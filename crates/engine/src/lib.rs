//! KataGo 进程接入：GTP 坐标转换 + analysis 批处理。
//!
//! 唯一允许碰进程和 IO 的 crate。对外只暴露「局面 → 评估」这一层。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use goban::{Color, Game, Move, Point};
use serde_json::{json, Value};

const GTP_COLS: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub katago: PathBuf,
    pub model: PathBuf,
    pub config: PathBuf,
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
        if let Some(p) = largest_net(&dir) {
            return Some(p);
        }
    }
    None
}

fn largest_net(dir: &std::path::Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("gz") {
            continue;
        }
        // 拟人网（human SL）不能当普通模型加载
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains("human"))
        {
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
}

pub fn gtp_coord(p: Point, size: u8) -> String {
    let col = GTP_COLS.get(p.x as usize).copied().unwrap_or(b'?') as char;
    format!("{}{}", col, size - p.y)
}

pub fn parse_gtp_move(s: &str, size: u8) -> Move {
    if s.eq_ignore_ascii_case("pass") {
        return Move::Pass;
    }
    let b = s.as_bytes();
    if b.len() < 2 {
        return Move::Pass;
    }
    let col = GTP_COLS.iter().position(|&c| c == b[0].to_ascii_uppercase());
    let row = s[1..].parse::<u8>().ok();
    match (col, row) {
        (Some(x), Some(r)) if r >= 1 && r <= size => Move::Play(Point::new(x as u8, size - r)),
        _ => Move::Pass,
    }
}

/// 整盘分析：0..=最后一手全部局面。
pub fn analyze_game<F>(
    cfg: &EngineConfig,
    game: &Game,
    progress: F,
) -> Result<Vec<TurnEval>, String>
where
    F: FnMut(usize, usize),
{
    let turns: Vec<u16> = (0..=game.moves().len() as u16).collect();
    analyze_turns(cfg, game, &turns, progress)
}

/// 只分析当前局面（实时看胜率/候选点时用）。
pub fn analyze_position(cfg: &EngineConfig, game: &Game) -> Result<TurnEval, String> {
    let turn = game.moves().len() as u16;
    let mut evals = analyze_turns(cfg, game, &[turn], |_, _| {})?;
    evals
        .pop()
        .ok_or_else(|| "没有拿到当前局面的分析结果".to_string())
}

/// 指定局面集合的分析：一次拉起 KataGo，分批送 query，按 turn 收集。
pub fn analyze_turns<F>(
    cfg: &EngineConfig,
    game: &Game,
    turns: &[u16],
    mut progress: F,
) -> Result<Vec<TurnEval>, String>
where
    F: FnMut(usize, usize),
{
    if cfg.katago.components().count() > 1 && !cfg.katago.exists() {
        return Err(format!("找不到 katago: {}", cfg.katago.display()));
    }
    if !cfg.model.exists() {
        return Err(format!(
            "找不到网络文件: {}（用 KATAGO_MODEL 指定 .bin.gz）",
            cfg.model.display()
        ));
    }
    if !cfg.config.exists() {
        return Err(format!("找不到配置: {}", cfg.config.display()));
    }

    let mut turns: Vec<u16> = turns.to_vec();
    turns.sort_unstable();
    turns.dedup();
    let total = turns.len();
    if total == 0 {
        return Ok(Vec::new());
    }

    let size = game.size();
    let moves_json: Vec<Value> = game
        .moves()
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
        .collect();

    let setup_json: Vec<Value> = game
        .setup()
        .iter()
        .map(|(c, p)| json!([c.sgf_char().to_string(), gtp_coord(*p, size)]))
        .collect();

    let mut queries = Vec::new();
    for chunk in turns.chunks(cfg.chunk.max(1)) {
        queries.push(json!({
            "id": format!("chunk-{}", chunk.first().copied().unwrap_or(0)),
            "initialStones": setup_json,
            "moves": moves_json,
            "rules": "chinese",
            "komi": game.komi(),
            "boardXSize": size,
            "boardYSize": size,
            "analyzeTurns": chunk,
            "maxVisits": cfg.visits,
            "analysisPVLen": 6,
        }));
    }

    let mut child = Command::new(&cfg.katago)
        .arg("analysis")
        .arg("-model")
        .arg(&cfg.model)
        .arg("-config")
        .arg(&cfg.config)
        .arg("-override-config")
        .arg("logToStderr=false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("启动 KataGo 失败: {e}"))?;

    {
        let stdin = child.stdin.as_mut().ok_or("打不开 KataGo stdin")?;
        for q in &queries {
            writeln!(stdin, "{q}").map_err(|e| format!("写入 query 失败: {e}"))?;
        }
        stdin.flush().ok();
    }
    drop(child.stdin.take());

    let stdout = child.stdout.take().ok_or("打不开 KataGo stdout")?;
    let reader = BufReader::new(stdout);
    let mut evals: Vec<TurnEval> = Vec::with_capacity(total);
    let mut failure: Option<String> = None;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                failure = Some(format!("读 KataGo 输出失败: {e}"));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                failure = Some(format!("解析 JSON 失败: {e}: {line}"));
                break;
            }
        };
        if let Some(err) = value.get("error").and_then(|x| x.as_str()) {
            failure = Some(format!("KataGo 报错: {err}"));
            break;
        }
        let Some(turn) = value.get("turnNumber").and_then(|x| x.as_u64()) else {
            continue;
        };
        match eval_from_json(&value, game, turn as u16) {
            Ok(ev) => evals.push(ev),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
        progress(evals.len(), total);
    }
    // 出错时别留孤儿子进程
    if failure.is_some() {
        let _ = child.kill();
    }
    let _ = child.wait();
    if let Some(e) = failure {
        return Err(e);
    }

    evals.sort_by_key(|e| e.turn);
    if evals.len() != total {
        return Err(format!(
            "只收到 {} / {} 个局面的分析结果",
            evals.len(),
            total
        ));
    }
    Ok(evals)
}

fn eval_from_json(v: &Value, game: &Game, turn: u16) -> Result<TurnEval, String> {
    let to_move = game.to_move_at(turn);
    let root = v.get("rootInfo").ok_or("结果缺少 rootInfo")?;
    let wr = root.get("winrate").and_then(|x| x.as_f64()).unwrap_or(0.5) as f32;
    let score = root
        .get("scoreLead")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0) as f32;
    let (winrate_black, score_black) = if to_move == Color::Black {
        (wr, score)
    } else {
        (1.0 - wr, -score)
    };

    let mut candidates = Vec::new();
    if let Some(arr) = v.get("moveInfos").and_then(|x| x.as_array()) {
        for m in arr.iter().take(8) {
            let mv = m
                .get("move")
                .and_then(|x| x.as_str())
                .map(|s| parse_gtp_move(s, game.size()))
                .unwrap_or(Move::Pass);
            let cwr = m.get("winrate").and_then(|x| x.as_f64()).unwrap_or(0.5) as f32;
            let csc = m
                .get("scoreLead")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0) as f32;
            let (cwr_b, csc_b) = if to_move == Color::Black {
                (cwr, csc)
            } else {
                (1.0 - cwr, -csc)
            };
            let pv = m
                .get("pv")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .map(|s| parse_gtp_move(s, game.size()))
                        .collect()
                })
                .unwrap_or_default();
            let visits = m.get("visits").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            candidates.push(Candidate {
                mv,
                winrate_black: cwr_b,
                score_black: csc_b,
                pv,
                visits,
            });
        }
    }

    Ok(TurnEval {
        turn,
        to_move,
        winrate_black,
        score_black,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtp_roundtrip() {
        assert_eq!(gtp_coord(Point::new(15, 3), 19), "Q16");
        assert_eq!(gtp_coord(Point::new(3, 15), 19), "D4");
        assert_eq!(parse_gtp_move("Q16", 19), Move::Play(Point::new(15, 3)));
        assert_eq!(parse_gtp_move("pass", 19), Move::Pass);
        // 跳过 I
        assert_eq!(gtp_coord(Point::new(8, 18), 19), "J1");
    }
}
