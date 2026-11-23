//! KataGo 进程接入：GTP 坐标转换 + analysis 批处理。
//!
//! 唯一允许碰进程和 IO 的 crate。对外只暴露「局面 → 评估」这一层。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use std::collections::{HashMap, HashSet};

use goban::{Color, Game, Move, Point, RuleSet};
use serde_json::{json, Value};

const GTP_COLS: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub katago: PathBuf,
    pub model: PathBuf,
    pub config: PathBuf,
    /// 拟人网 `humanv0`；留 None 表示不可用（不传 `-human-model`）
    pub human_model: Option<PathBuf>,
    pub visits: u32,
    /// 一次 query 分析多少个局面（控制内存与进度粒度）
    pub chunk: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            katago: std::env::var_os("KATAGO_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(find_katago),
            model: std::env::var_os("KATAGO_MODEL")
                .map(PathBuf::from)
                .or_else(find_model)
                .unwrap_or_default(),
            human_model: std::env::var_os("KATAGO_HUMAN_MODEL")
                .map(PathBuf::from)
                .or_else(find_human_model),
            config: std::env::var_os("KATAGO_CFG")
                .map(PathBuf::from)
                .unwrap_or_else(|| ensure_config(config_dir().join("analysis.cfg"))),
            visits: 120,
            chunk: 30,
        }
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("weiqi-study")
}

fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/state"))
        .join("weiqi-study")
}

fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

fn find_katago() -> PathBuf {
    if let Some(p) = which("katago") {
        return p;
    }
    // 常见的手动安装位置（Nix profile / 项目内自建）
    for candidate in [
        home_dir().join("go-study/katago/bin/katago"),
        PathBuf::from("/usr/bin/katago"),
        PathBuf::from("/usr/local/bin/katago"),
    ] {
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("katago")
}

/// 在 Steam 棋类合集里找一个网络（存在就用，找不到让用户显式设置 KATAGO_MODEL）。
fn find_model() -> Option<PathBuf> {
    let common = home_dir().join(".local/share/Steam/steamapps/common");
    for dir in [
        common.join("NinOneQi/engines/go"),
        common.join("Just Go/JustGo_Data/StreamingAssets/AI/Katago/networks/Analysis"),
    ] {
        if let Some(p) = largest_net(&dir, false) {
            return Some(p);
        }
    }
    None
}

/// 拟人网（human SL）通常和普通网络放在同一个目录，文件名带 `humanv0`。
fn find_human_model() -> Option<PathBuf> {
    let common = home_dir().join(".local/share/Steam/steamapps/common");
    for dir in [
        common.join("NinOneQi/engines/go"),
        common.join("Just Go/JustGo_Data/StreamingAssets/AI/Katago/networks/Analysis"),
    ] {
        if let Some(p) = largest_net(&dir, true) {
            return Some(p);
        }
    }
    None
}

fn largest_net(dir: &std::path::Path, human: bool) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("gz") {
            continue;
        }
        // 拟人网（human SL）和普通网络用途不同，不能混用
        let is_human = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains("human"));
        if is_human != human {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(s, _)| size > *s) {
            best = Some((size, path));
        }
    }
    best.map(|(_, p)| p)
}

/// 配置文件不存在时写一份最小可用的（OpenCL 友好）。
fn ensure_config(path: PathBuf) -> PathBuf {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let logs = state_dir().join("logs");
        let _ = std::fs::create_dir_all(&logs);
        let text = format!(
            "logDir = {}\nlogToStderr = false\nlogErrorsAndWarnings = true\n\
             numAnalysisThreads = 2\nnumSearchThreadsPerAnalysisThread = 8\n\
             nnMaxBatchSize = 16\nnnCacheSizePowerOfTwo = 20\n\
             reportAnalysisWinratesAs = SIDETOMOVE\nanalysisPVLen = 8\nmaxVisits = 120\n",
            logs.display()
        );
        let _ = std::fs::write(&path, text);
    }
    path
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub mv: Move,
    pub winrate_black: f32,
    pub score_black: f32,
    pub pv: Vec<Move>,
    pub visits: u32,
}

#[derive(Clone, Debug)]
pub struct TurnEval {
    pub turn: u16,
    /// 该局面轮到谁走
    pub to_move: Color,
    pub winrate_black: f32,
    pub score_black: f32,
    pub candidates: Vec<Candidate>,
    /// 拟人网给出的走子概率（含 pass 在最后一位）；普通模型为 None
    pub human_policy: Option<Vec<f32>>,
}

pub fn gtp_coord(p: Point, size: u8) -> String {
    let col = GTP_COLS.get(p.x as usize).copied().unwrap_or(b'?') as char;
    format!("{}{}", col, size - p.y)
}

pub fn parse_gtp_move(s: &str, size: u8) -> Result<Move, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("pass") {
        return Ok(Move::Pass);
    }
    if s.eq_ignore_ascii_case("resign") {
        return Err("引擎返回 resign".to_string());
    }
    let b = s.as_bytes();
    if b.len() < 2 {
        return Err(format!("GTP 坐标太短：{s:?}"));
    }
    let col = GTP_COLS
        .iter()
        .position(|&c| c == b[0].to_ascii_uppercase())
        .ok_or_else(|| format!("GTP 列非法：{s:?}"))?;
    let row = s[1..]
        .parse::<u8>()
        .map_err(|_| format!("GTP 行号非法：{s:?}"))?;
    if row < 1 || row > size || col >= size as usize {
        return Err(format!("GTP 坐标超出 {size} 路棋盘：{s:?}"));
    }
    Ok(Move::Play(Point::new(col as u8, size - row)))
}

/// 引擎句柄：每个前端持有自己的实例，内部只有一个常驻会话。
pub struct Engine {
    cfg: EngineConfig,
    session: std::sync::Mutex<Option<AnalysisSession>>,
    visits: std::sync::atomic::AtomicU32,
    /// 拟人设置 + 会话参数版本（一起读写，避免版本和段位错配）
    human: std::sync::Mutex<HumanState>,
}

#[derive(Clone, Default, PartialEq)]
struct HumanState {
    /// Some(profile) = 拟人模式开启，例如 `rank_5k`
    profile: Option<String>,
    /// 拟人设置变化就 +1，下一次 query 重建进程
    epoch: u64,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Self {
        Engine {
            visits: std::sync::atomic::AtomicU32::new(cfg.visits.max(1)),
            cfg,
            session: std::sync::Mutex::new(None),
            human: std::sync::Mutex::new(HumanState::default()),
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    pub fn visits(&self) -> u32 {
        self.visits.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 调整每次分析的 visits，下一次 query 生效（不需要重启进程）。
    pub fn set_visits(&self, visits: u32) {
        self.visits
            .store(visits.max(1), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn human_profile(&self) -> Option<String> {
        self.human
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .profile
            .clone()
    }

    fn human_state(&self) -> HumanState {
        self.human.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 开关拟人网 / 换段位。不在这里抢 session 锁：正在跑的分析可能占着它很久；
    /// 只把版本号 +1，下一次 analyze_turns 发现版本不一致再重建进程。
    pub fn set_human_profile(&self, profile: Option<String>) {
        let mut guard = self.human.lock().unwrap_or_else(|e| e.into_inner());
        if guard.profile != profile {
            guard.profile = profile;
            guard.epoch += 1;
        }
    }

    pub fn analyze_game<F>(&self, game: &Game, progress: F) -> Result<Vec<TurnEval>, String>
    where
        F: FnMut(usize, usize),
    {
        let turns: Vec<u16> = (0..=game.moves().len() as u16).collect();
        self.analyze_turns(game, &turns, progress)
    }

    pub fn analyze_position(&self, game: &Game) -> Result<TurnEval, String> {
        let turn = game.moves().len() as u16;
        let mut evals = self.analyze_turns(game, &[turn], |_, _| {})?;
        evals
            .pop()
            .ok_or_else(|| "没有拿到当前局面的分析结果".to_string())
    }

    pub fn analyze_turns<F>(
        &self,
        game: &Game,
        turns: &[u16],
        mut progress: F,
    ) -> Result<Vec<TurnEval>, String>
    where
        F: FnMut(usize, usize),
    {
        for (label, path) in [
            ("katago", &self.cfg.katago),
            ("model", &self.cfg.model),
            ("config", &self.cfg.config),
        ] {
            if label == "katago" {
                if self.cfg.katago.components().count() > 1 && !path.exists() {
                    return Err(format!("找不到 katago: {}", path.display()));
                }
            } else if !path.exists() {
                return Err(format!("找不到 {label}: {}", path.display()));
            }
        }

        let state = self.human_state();
        let human = state.profile.clone();
        let epoch = state.epoch;
        if human.is_some() {
            match &self.cfg.human_model {
                Some(p) if p.exists() => {}
                _ => return Err("拟人模式需要 humanv0 网络（设 KATAGO_HUMAN_MODEL）".to_string()),
            }
        }

        let mut turns: Vec<u16> = turns.to_vec();
        turns.sort_unstable();
        turns.dedup();
        if turns.is_empty() {
            return Ok(Vec::new());
        }

        let mut guard = self
            .session
            .lock()
            .map_err(|_| "分析会话锁失败".to_string())?;
        let need_start = match guard.as_mut() {
            Some(session) => session.is_dead() || session.epoch != epoch,
            None => true,
        };
        if need_start {
            let human_model = if human.is_some() {
                self.cfg.human_model.as_deref()
            } else {
                None
            };
            *guard = Some(AnalysisSession::start(&self.cfg, human_model, epoch)?);
        }
        let result = guard.as_mut().expect("会话已初始化").query(
            &self.cfg,
            self.visits(),
            human.as_deref(),
            game,
            &turns,
            &mut progress,
        );
        if result.is_err() {
            // 出错就丢掉会话（Drop 会 kill），下次重新拉起
            *guard = None;
        }
        result
    }
}

struct AnalysisSession {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    rx: std::sync::mpsc::Receiver<String>,
    seq: u64,
    epoch: u64,
}

impl AnalysisSession {
    fn start(
        cfg: &EngineConfig,
        human_model: Option<&std::path::Path>,
        epoch: u64,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(&cfg.katago);
        cmd.arg("analysis").arg("-model").arg(&cfg.model);
        if let Some(model) = human_model {
            cmd.arg("-human-model").arg(model);
        }
        cmd.arg("-config")
            .arg(&cfg.config)
            .arg("-override-config")
            .arg("logToStderr=false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| format!("启动 KataGo 失败: {e}"))?;
        let stdin = child.stdin.take().ok_or("打不开 KataGo stdin")?;
        let stdout = child.stdout.take().ok_or("打不开 KataGo stdout")?;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(AnalysisSession {
            child,
            stdin,
            rx,
            seq: 0,
            epoch,
        })
    }

    fn is_dead(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn query(
        &mut self,
        cfg: &EngineConfig,
        visits: u32,
        human_profile: Option<&str>,
        game: &Game,
        turns: &[u16],
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Vec<TurnEval>, String> {
        self.seq += 1;
        let base = self.seq;
        let size = game.size();
        let moves_json = history_json(game);
        let setup_json = setup_json(game);

        // 每个请求 ID 对应它负责的 turn 集合：响应必须同时匹配 (id, turn)，
        // 不能只看 id 在集合里、turn 在全局范围里就算过（review v6 4.1）。
        let mut expected: HashMap<String, HashSet<u16>> = HashMap::new();
        for (n, chunk) in turns.chunks(cfg.chunk.max(1)).enumerate() {
            let id = format!("q{base}-{n}");
            expected.insert(id.clone(), chunk.iter().copied().collect());
            let mut query = json!({
                "id": id,
                "initialStones": setup_json,
                "moves": moves_json,
                "rules": game.rules().katago_name(),
                "komi": game.komi(),
                "boardXSize": size,
                "boardYSize": size,
                "analyzeTurns": chunk,
                "maxVisits": visits,
                "analysisPVLen": 6,
            });
            if let Some(profile) = human_profile {
                query["includePolicy"] = json!(true);
                query["overrideSettings"] = json!({
                    "humanSLProfile": profile,
                    "ignorePreRootHistory": false,
                });
            }
            writeln!(self.stdin, "{query}").map_err(|e| format!("写入 query 失败: {e}"))?;
        }
        self.stdin
            .flush()
            .map_err(|e| format!("刷新 query 失败: {e}"))?;

        // 调用方重复传同一个 turn 时，只分析一次，否则 total 会虚高
        let mut uniq_turns: Vec<u16> = Vec::with_capacity(turns.len());
        for &t in turns {
            if !uniq_turns.contains(&t) {
                uniq_turns.push(t);
            }
        }
        let turns: &[u16] = &uniq_turns;

        let total = turns.len();
        let mut received: HashMap<String, HashSet<u16>> = HashMap::new();
        for id in expected.keys() {
            received.insert(id.clone(), HashSet::new());
        }
        let mut evals: Vec<TurnEval> = Vec::with_capacity(total);
        while evals.len() < total {
            let line = match self.rx.recv_timeout(std::time::Duration::from_secs(90)) {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err("KataGo 分析超时（90s 无输出）".to_string())
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("KataGo 意外退出".to_string())
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let value: Value =
                serde_json::from_str(&line).map_err(|e| format!("解析 JSON 失败: {e}: {line}"))?;
            if let Some(err) = value.get("error").and_then(|x| x.as_str()) {
                return Err(format!("KataGo 报错: {err}"));
            }
            let id = value
                .get("id")
                .and_then(|x| x.as_str())
                .ok_or_else(|| format!("响应缺少 id: {line}"))?;
            let Some(allowed) = expected.get(id) else {
                return Err(format!(
                    "响应 id 不属于本次请求: {id}（请求 {base}，共 {} 批）",
                    expected.len()
                ));
            };
            let turn = value
                .get("turnNumber")
                .and_then(|x| x.as_u64())
                .ok_or_else(|| format!("响应缺少 turnNumber: {line}"))?
                as u16;
            if !allowed.contains(&turn) {
                return Err(format!(
                    "响应 turnNumber 越界: id={id} turn={turn}（该批次负责 {} 个 turn）",
                    allowed.len()
                ));
            }
            let got = received.get_mut(id).expect("id 已在 expected 里登记");
            if !got.insert(turn) {
                return Err(format!("响应 turnNumber 重复: id={id} turn={turn}"));
            }
            evals.push(eval_from_json(&value, game, turn)?);
            progress(evals.len(), total);
        }
        evals.sort_by_key(|e| e.turn);
        Ok(evals)
    }
}

impl Drop for AnalysisSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 提交给 KataGo 的历史手：`[[颜色, 坐标], ...]`。
///
/// 颜色必须来自 `Game`（SGF 首手 / `PL` 决定），不能按“第一手是黑”硬编码；
/// 白先棋谱整盘黑白对调就是这么来的（review v6 P1）。
fn history_json(game: &Game) -> Vec<Value> {
    let size = game.size();
    game.moves()
        .iter()
        .enumerate()
        .map(|(i, mv)| {
            let color = game.color_of_move(i as u16 + 1);
            let coord = match mv {
                Move::Play(p) => gtp_coord(*p, size),
                Move::Pass => "pass".to_string(),
            };
            json!([color.sgf_char().to_string(), coord])
        })
        .collect()
}

/// 提交给 KataGo 的开局摆子（AB/AW）。
fn setup_json(game: &Game) -> Vec<Value> {
    let size = game.size();
    game.setup()
        .iter()
        .map(|(c, p)| json!([c.sgf_char().to_string(), gtp_coord(*p, size)]))
        .collect()
}

/// 本地规则集 → KataGo 的 `rules` 预设名。直接用规范名，不再先折叠成 KoRule。
pub fn katago_rules(rule: RuleSet) -> &'static str {
    rule.katago_name()
}

/// 取一个必填的有限浮点字段；缺失、类型错误、NaN/Infinity 都算坏数据。
fn finite_f64(v: &Value, key: &str) -> Result<f64, String> {
    let x = v
        .get(key)
        .and_then(|x| x.as_f64())
        .ok_or_else(|| format!("缺少 {key}"))?;
    if !x.is_finite() {
        return Err(format!("{key} 不是有限数值: {x}"));
    }
    Ok(x)
}

/// 解析一条候选。协议必需字段（move/winrate/scoreLead/visits）缺任何一个都返回 Err，
/// 由调用方跳过该候选；只有 `pv` 是可选字段（缺省为空）。
fn candidate_from_json(m: &Value, game: &Game, to_move: Color) -> Result<Candidate, String> {
    let raw = m
        .get("move")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "候选缺少 move".to_string())?;
    let mv = parse_gtp_move(raw, game.size()).map_err(|e| format!("move={raw}: {e}"))?;

    let cwr = finite_f64(m, "winrate")?;
    if !(0.0..=1.0).contains(&cwr) {
        return Err(format!("winrate 越界: {cwr}"));
    }
    let csc = finite_f64(m, "scoreLead")?;

    let raw_visits = finite_f64(m, "visits")?;
    if raw_visits < 0.0 {
        return Err(format!("visits 为负: {raw_visits}"));
    }
    if raw_visits > u32::MAX as f64 {
        return Err(format!("visits 溢出 u32: {raw_visits}"));
    }
    let visits = raw_visits as u32;

    let (cwr_b, csc_b) = if to_move == Color::Black {
        (cwr as f32, csc as f32)
    } else {
        (1.0 - cwr as f32, -(csc as f32))
    };
    let pv = m
        .get("pv")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .filter_map(|s| parse_gtp_move(s, game.size()).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(Candidate {
        mv,
        winrate_black: cwr_b,
        score_black: csc_b,
        pv,
        visits,
    })
}

fn eval_from_json(v: &Value, game: &Game, turn: u16) -> Result<TurnEval, String> {
    let to_move = game.to_move_at(turn);
    let root = v.get("rootInfo").ok_or("结果缺少 rootInfo")?;
    let wr = finite_f64(root, "winrate").map_err(|e| format!("rootInfo {e}"))? as f32;
    if !(0.0..=1.0).contains(&wr) {
        return Err(format!("rootInfo.winrate 越界: {wr}"));
    }
    let score = finite_f64(root, "scoreLead").map_err(|e| format!("rootInfo {e}"))? as f32;
    let (winrate_black, score_black) = if to_move == Color::Black {
        (wr, score)
    } else {
        (1.0 - wr, -score)
    };

    let mut candidates = Vec::new();
    if let Some(arr) = v.get("moveInfos").and_then(|x| x.as_array()) {
        // 先过滤再截断：否则前 8 条全坏时，第 9 条好棋会被误判成"全部无法解析"
        let mut first_err: Option<String> = None;
        for m in arr.iter() {
            // 候选级容错：引擎可能给出 resign / unknown / 越界坐标，
            // 跳过这一个候选即可，不要因为一个坏值丢掉整轮分析。
            match candidate_from_json(m, game, to_move) {
                Ok(c) => candidates.push(c),
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        if candidates.is_empty() && !arr.is_empty() {
            return Err(format!(
                "moveInfos 里的候选全部无法解析：{}",
                first_err.unwrap_or_default()
            ));
        }
        candidates.truncate(8);
    }

    let human_policy = v.get("humanPolicy").and_then(|x| x.as_array()).map(|a| {
        a.iter()
            .map(|x| x.as_f64().unwrap_or(-1.0) as f32)
            .collect::<Vec<f32>>()
    });

    Ok(TurnEval {
        turn,
        to_move,
        winrate_black,
        score_black,
        candidates,
        human_policy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use goban::KoRule;

    #[test]
    fn gtp_roundtrip() {
        assert_eq!(gtp_coord(Point::new(15, 3), 19), "Q16");
        assert_eq!(gtp_coord(Point::new(3, 15), 19), "D4");
        assert_eq!(
            parse_gtp_move("Q16", 19).unwrap(),
            Move::Play(Point::new(15, 3))
        );
        assert_eq!(parse_gtp_move("pass", 19).unwrap(), Move::Pass);
        // 跳过 I
        assert_eq!(gtp_coord(Point::new(8, 18), 19), "J1");
        assert!(parse_gtp_move("I5", 19).is_err());
        assert!(parse_gtp_move("Z99", 19).is_err());
        assert!(parse_gtp_move("Q16", 9).is_err());
        assert!(parse_gtp_move("", 19).is_err());
        assert!(parse_gtp_move("resign", 19).is_err());
    }

    #[test]
    fn bad_candidate_is_skipped_not_fatal() {
        let game = Game::new(9, 7.5);
        let v = json!({
            "rootInfo": {"winrate": 0.5, "scoreLead": 0.0},
            "moveInfos": [
                {"move": "resign", "winrate": 0.5, "scoreLead": 0.0, "pv": ["resign"], "visits": 1},
                {"move": "D4", "winrate": 0.5, "scoreLead": 0.0, "pv": ["D4", "resign", "E5"], "visits": 10}
            ]
        });
        let ev = eval_from_json(&v, &game, 0).unwrap();
        assert_eq!(ev.candidates.len(), 1, "坏候选跳过，好候选保留");
        assert_eq!(ev.candidates[0].mv, Move::Play(Point::new(3, 5))); // 9 路盘的 D4
        assert_eq!(
            ev.candidates[0].pv,
            vec![Move::Play(Point::new(3, 5)), Move::Play(Point::new(4, 4))]
        );
    }

    #[test]
    fn all_candidates_bad_is_an_error() {
        let game = Game::new(9, 7.5);
        let v = json!({
            "rootInfo": {"winrate": 0.5, "scoreLead": 0.0},
            "moveInfos": [{"move": "resign"}, {"move": "unknown"}]
        });
        let err = eval_from_json(&v, &game, 0).unwrap_err();
        assert!(err.contains("全部无法解析"), "{err}");
    }

    #[test]
    fn eval_converts_to_black_view_under_white_first() {
        // 白先棋谱：turn 0 的行棋方是白，KataGo 给的 winrate/scoreLead 是白方视角
        let mut game =
            Game::with_setup_rules_first(9, 7.5, Vec::new(), RuleSet::CHINESE, Color::White);
        let v = json!({
            "rootInfo": {"winrate": 0.7, "scoreLead": 3.0},
            "moveInfos": [{"move": "D4", "winrate": 0.7, "scoreLead": 3.0, "visits": 10}]
        });

        let ev = eval_from_json(&v, &game, 0).unwrap();
        assert_eq!(ev.to_move, Color::White);
        assert!(
            (ev.winrate_black - 0.3).abs() < 1e-6,
            "{}",
            ev.winrate_black
        );
        assert!((ev.score_black + 3.0).abs() < 1e-6, "{}", ev.score_black);
        assert!((ev.candidates[0].winrate_black - 0.3).abs() < 1e-6);

        // 白落一手后轮到黑，视角不再翻转
        game.play(Move::Play(Point::new(2, 2))).unwrap();
        let ev = eval_from_json(&v, &game, 1).unwrap();
        assert_eq!(ev.to_move, Color::Black);
        assert!(
            (ev.winrate_black - 0.7).abs() < 1e-6,
            "{}",
            ev.winrate_black
        );
        assert!((ev.score_black - 3.0).abs() < 1e-6, "{}", ev.score_black);
    }

    #[test]
    fn incomplete_candidate_fields_are_rejected_not_defaulted() {
        let game = Game::new(9, 7.5);
        let base = json!({"rootInfo": {"winrate": 0.5, "scoreLead": 0.0}});
        for bad in [
            json!({"move": "D4", "scoreLead": 0.0, "visits": 1}), // 缺 winrate
            json!({"move": "D4", "winrate": 1.2, "scoreLead": 0.0, "visits": 1}), // 越界
            json!({"move": "D4", "winrate": 0.5, "scoreLead": 0.0}), // 缺 visits
            json!({"move": "D4", "winrate": 0.5, "scoreLead": 0.0, "visits": -1}), // 负数
            json!({"move": "D4", "winrate": 0.5, "scoreLead": 0.0, "visits": 5e9}), // 溢出 u32
        ] {
            let mut v = base.clone();
            v["moveInfos"] = json!([bad]);
            let err = eval_from_json(&v, &game, 0).unwrap_err();
            assert!(err.contains("全部无法解析"), "{err}");
        }

        // rootInfo 的必需字段同样不能缺
        let v = json!({"rootInfo": {"winrate": 0.5}, "moveInfos": []});
        assert!(eval_from_json(&v, &game, 0).is_err());
        let v = json!({"rootInfo": {"winrate": 0.5, "scoreLead": 0.0}});
        assert!(eval_from_json(&v, &game, 0).is_ok());

        // 字段齐全的候选照常通过
        let mut v = base;
        v["moveInfos"] = json!([
            {"move": "D4", "winrate": 0.5, "scoreLead": 1.0, "visits": 42}
        ]);
        let ev = eval_from_json(&v, &game, 0).unwrap();
        assert_eq!(ev.candidates.len(), 1);
        assert_eq!(ev.candidates[0].visits, 42);
    }

    #[test]
    fn good_candidate_after_eight_bad_ones_survives() {
        let game = Game::new(9, 7.5);
        let mut infos = Vec::new();
        for _ in 0..8 {
            infos.push(json!({"move": "resign"}));
        }
        infos.push(json!({"move": "D4", "winrate": 0.5, "scoreLead": 0.0, "visits": 3}));
        let v = json!({
            "rootInfo": {"winrate": 0.5, "scoreLead": 0.0},
            "moveInfos": infos
        });
        let ev = eval_from_json(&v, &game, 0).unwrap();
        assert_eq!(ev.candidates.len(), 1);
        assert_eq!(ev.candidates[0].mv, Move::Play(Point::new(3, 5)));
    }

    #[test]
    fn response_with_wrong_id_is_an_error() {
        let (engine, dir) = fake_engine(
            r#"#!/bin/sh
while IFS= read -r line; do
  echo '{"id":"wrong","turnNumber":0,"rootInfo":{"winrate":0.5,"scoreLead":0.0}}'
done
"#,
        );
        let err = engine.analyze_position(&Game::new(9, 7.5)).unwrap_err();
        assert!(err.contains("不属于本次请求"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn response_with_out_of_range_turn_is_an_error() {
        let (engine, dir) = fake_engine(
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  echo "{\"id\":\"$id\",\"turnNumber\":99,\"rootInfo\":{\"winrate\":0.5,\"scoreLead\":0.0}}"
done
"#,
        );
        let err = engine.analyze_position(&Game::new(9, 7.5)).unwrap_err();
        assert!(err.contains("越界"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn turn_from_another_chunk_is_rejected() {
        // chunk=1：请求 q-0 只负责 turn 0，q-1 负责 turn 1。
        // 即使 id 合法、turn 也在全局范围内，只要不属于这个 id 就必须报错。
        let (engine, dir) = fake_engine_chunk(
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  echo "{\"id\":\"$id\",\"turnNumber\":1,\"rootInfo\":{\"winrate\":0.5,\"scoreLead\":0.0}}"
done
"#,
            1,
        );
        let err = engine
            .analyze_turns(&Game::new(9, 7.5), &[0, 1], |_, _| {})
            .unwrap_err();
        assert!(err.contains("越界"), "{err}");
        assert!(err.contains("id=q"), "错误里要带请求 id：{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_turn_within_a_request_is_rejected() {
        // 两批合成一个请求里的重复响应：第一次接受，第二次必须报重复
        let (engine, dir) = fake_engine(
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  echo "{\"id\":\"$id\",\"turnNumber\":0,\"rootInfo\":{\"winrate\":0.5,\"scoreLead\":0.0}}"
  echo "{\"id\":\"$id\",\"turnNumber\":0,\"rootInfo\":{\"winrate\":0.5,\"scoreLead\":0.0}}"
done
"#,
        );
        let err = engine
            .analyze_turns(&Game::new(9, 7.5), &[0, 1], |_, _| {})
            .unwrap_err();
        assert!(err.contains("重复"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_root_info_is_an_error() {
        let (engine, dir) = fake_engine(
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  echo "{\"id\":\"$id\",\"turnNumber\":0}"
done
"#,
        );
        let err = engine.analyze_position(&Game::new(9, 7.5)).unwrap_err();
        assert!(err.contains("rootInfo"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn history_json_uses_declared_colors() {
        // 白先棋谱：第一手必须是 W，不能因为空盘就写死 B
        let mut game =
            Game::with_setup_rules_first(9, 7.5, Vec::new(), RuleSet::CHINESE, Color::White);
        game.play(Move::Play(Point::new(2, 2))).unwrap();
        game.play(Move::Play(Point::new(6, 6))).unwrap();
        assert_eq!(
            history_json(&game),
            vec![json!(["W", "C7"]), json!(["B", "G3"])]
        );

        // 黑先 + 摆子：仍然听文件的
        let mut game = Game::with_setup_rules_first(
            9,
            7.5,
            vec![(Color::Black, Point::new(0, 0))],
            RuleSet::CHINESE,
            Color::Black,
        );
        game.play(Move::Play(Point::new(2, 2))).unwrap();
        assert_eq!(history_json(&game), vec![json!(["B", "C7"])]);
        assert_eq!(setup_json(&game), vec![json!(["B", "A9"])]);
    }

    #[test]
    fn katago_rules_follow_the_rule_set() {
        assert_eq!(katago_rules(RuleSet::CHINESE), "chinese");
        assert_eq!(katago_rules(RuleSet::JAPANESE), "japanese");
        assert_eq!(katago_rules(RuleSet::KOREAN), "korean");
        // 以前 AGA / Ing 会被折叠成 korean，现在原样传下去
        assert_eq!(katago_rules(RuleSet::AGA), "aga");
        assert_eq!(katago_rules(RuleSet::ING), "ing");
    }

    #[test]
    fn query_sends_the_games_rules() {
        // 假引擎只认 japanese；如果 query 还写死 chinese 就会返回 error
        let (engine, dir) = fake_engine(
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"rules":"japanese"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
      echo "{\"id\":\"$id\",\"turnNumber\":0,\"rootInfo\":{\"winrate\":0.5,\"scoreLead\":0.0},\"moveInfos\":[]}" ;;
    *) echo '{"error":"wrong rules"}' ;;
  esac
done"#,
        );
        let game = Game::with_rule(9, 7.5, KoRule::Simple);
        engine.analyze_position(&game).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 假 KataGo：不依赖 GPU/模型，验证协议解析与错误路径。
    fn fake_engine(script: &str) -> (Engine, std::path::PathBuf) {
        fake_engine_chunk(script, 30)
    }

    fn fake_engine_chunk(script: &str, chunk: usize) -> (Engine, std::path::PathBuf) {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "weiqi-fake-engine-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // 0700 私有目录：/tmp 里别让别的用户提前创建同名目录
        std::fs::create_dir(&dir).unwrap();
        let mut dir_perms = std::fs::metadata(&dir).unwrap().permissions();
        dir_perms.set_mode(0o700);
        std::fs::set_permissions(&dir, dir_perms).unwrap();
        let bin = dir.join("katago");
        {
            let mut f = std::fs::File::create(&bin).unwrap();
            f.write_all(script.as_bytes()).unwrap();
            f.sync_all().unwrap();
        } // 显式关闭：否则紧接着 exec 可能拿到 ETXTBSY
        let mut perms = std::fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms).unwrap();
        let model = dir.join("model.bin.gz");
        let config = dir.join("analysis.cfg");
        std::fs::write(&model, "fake").unwrap();
        std::fs::write(&config, "fake").unwrap();
        let engine = Engine::new(EngineConfig {
            katago: bin,
            model,
            config,
            human_model: None,
            visits: 10,
            chunk,
        });
        (engine, dir)
    }

    #[test]
    fn fake_engine_parses_analysis_json() {
        let (engine, dir) = fake_engine(
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *analyzeTurns*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
      echo "{\"id\":\"$id\",\"turnNumber\":0,\"rootInfo\":{\"winrate\":0.5,\"scoreLead\":0.0},\"moveInfos\":[{\"move\":\"D4\",\"winrate\":0.5,\"scoreLead\":0.0,\"pv\":[\"D4\",\"E5\"],\"visits\":1}]}" ;;
  esac
done
"#,
        );
        let eval = engine
            .analyze_position(&Game::new(9, 7.5))
            .expect("假引擎应返回结果");
        assert_eq!(eval.turn, 0);
        assert_eq!(eval.candidates[0].mv, Move::Play(Point::new(3, 5))); // D4 on 9x9
        assert_eq!(eval.candidates[0].pv.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fake_engine_invalid_json_is_an_error() {
        let (engine, dir) = fake_engine(
            r#"#!/bin/sh
read -r line
echo 'not json'
"#,
        );
        let err = engine.analyze_position(&Game::new(9, 7.5)).unwrap_err();
        assert!(err.contains("解析 JSON"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fake_engine_exit_is_an_error() {
        let (engine, dir) = fake_engine("#!/bin/sh\nexit 1\n");
        let err = engine.analyze_position(&Game::new(9, 7.5)).unwrap_err();
        assert!(
            err.contains("意外退出") || err.contains("写入 query"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
