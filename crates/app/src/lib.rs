//! 与界面无关的应用状态：导航、落子、人机对战、分析缓存。
//!
//! 前端只负责把按键映射成 [`Controller`] 的调用，以及把状态画出来。
//! 后台分析线程与事件回填都在 [`controller`] 里，TUI/GUI 共用同一份。

pub mod controller;
pub mod i18n;
mod preview;
pub mod storage;

pub use controller::Controller;

use preview::Preview;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use i18n::{strings, Lang};
use storage::{GameEntry, GameStore};

use engine::{EngineConfig, TurnEval};
use goban::{Color, Game, Move, Point, RuleSet};
use review::Review;
use sgf::SgfGame;

#[derive(Clone, Debug, Default)]
pub struct AppConfig {
    pub engine: EngineConfig,
}

/// 前端每轮问一次「现在该干什么」，避免把待机判断散落在 UI 循环里。
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Analyze,
    AiMove(Move),
}

#[derive(Debug)]
pub enum AppEvent {
    Analysis {
        /// 起这次分析的棋局版本；切棋谱/换拟人设置后旧结果会被丢弃
        generation: u64,
        pos: usize,
        result: Result<TurnEval, String>,
    },
    Review {
        generation: u64,
        result: Result<Review, String>,
    },
}

/// 拟人网可选段位（KataGo humanv0 的 profile 名）。
/// 每步思考量的四个快捷档位（visits）。
pub const VISIT_PRESETS: [u32; 4] = [60, 120, 300, 600];

pub const HUMAN_RANKS: [&str; 9] = [
    "rank_20k", "rank_15k", "rank_10k", "rank_5k", "rank_1k", "rank_1d", "rank_3d", "rank_5d",
    "rank_9d",
];

pub struct App {
    pub size: u8,
    pub komi: f32,
    pub cursor: Point,
    pub review: Option<Review>,
    pub status: String,
    pub busy: bool,
    pub show_help: bool,
    pub config: AppConfig,
    /// 人机对战：AI 是否自动应手
    pub ai_enabled: bool,
    /// 人执哪一方
    pub human: Color,
    /// 拟人网模式（用 humanPolicy 按概率选点）
    pub human_enabled: bool,
    /// `HUMAN_RANKS` 的索引
    pub human_rank: usize,
    /// 最近一手的即时点评
    pub last_comment: Option<String>,
    pub lang: Lang,
    pub engine: Arc<engine::Engine>,
    pub store: GameStore,
    pub current_file: Option<PathBuf>,
    /// 上次读/写 `current_file` 时看到的版本指纹（mtime + 长度 + 内容哈希）
    pub file_stamp: Option<storage::FileStamp>,
    pub games: Vec<GameEntry>,
    pub show_games: bool,
    pub show_info: bool,
    pub game_cursor: usize,
    setup: Vec<(Color, Point)>,
    /// 当前棋局的完整规则（SGF `RU` 或默认 Chinese）
    rules: RuleSet,
    /// 重放时的先手方（SGF 声明优先）
    first: Color,
    moves: Vec<Move>,
    pos: usize,
    game: Game,
    evals: HashMap<usize, TurnEval>,
    failed_pos: Option<usize>,
    /// 拟人网选点的随机种子；测试可以固定它
    ai_seed: u64,
    /// 是否人机对局（start_play 开始，读 SGF 时清掉）
    vs_ai: bool,
    /// 最近一次读谱产生的 warning
    pub warnings: Vec<String>,
    /// 棋局/引擎设置版本；旧的分析结果回来时按这个丢弃
    generation: u64,
    /// 引擎推荐变化的临时预览（不动主线）
    preview: Option<Preview>,
}

/// 状态栏用的 warning 摘要（只显示第一条 + 总数）。
fn warning_text(warnings: &[String]) -> Option<String> {
    warnings.first().map(|w| {
        if warnings.len() > 1 {
            format!("⚠ {} 条警告：{w}", warnings.len())
        } else {
            format!("⚠ {w}")
        }
    })
}

impl App {
    /// 空盘新局（不读 SGF）。
    pub fn new(size: u8, komi: f32, config: AppConfig) -> Self {
        let engine = Arc::new(engine::Engine::new(config.engine.clone()));
        App {
            engine,
            size,
            komi,
            cursor: Point::new(size / 2, size / 2),
            review: None,
            status: strings(Lang::from_env()).ready.to_string(),
            busy: false,
            show_help: false,
            config,
            ai_enabled: false,
            human: Color::Black,
            human_enabled: false,
            human_rank: 3,
            last_comment: None,
            lang: Lang::from_env(),
            store: GameStore::open(),
            current_file: None,
            file_stamp: None,
            games: Vec::new(),
            show_games: false,
            show_info: false,
            game_cursor: 0,
            setup: Vec::new(),
            rules: RuleSet::default(),
            first: Color::Black,
            moves: Vec::new(),
            pos: 0,
            game: Game::new(size, komi),
            evals: HashMap::new(),
            failed_pos: None,
            ai_seed: seed(),
            vs_ai: false,
            warnings: Vec::new(),
            generation: 0,
            preview: None,
        }
    }

    pub fn from_sgf(sg: &SgfGame, config: AppConfig) -> Result<Self, String> {
        let mut app = Self::new(sg.size, sg.komi.unwrap_or(7.5), config);
        app.setup = sg.setup.clone();
        app.rules = sgf::rule_set_from_rules(sg.rules.as_deref());
        app.first = sg.first_to_move();
        app.moves = sg.moves.clone();
        app.pos = sg.moves.len();
        app.rebuild()?;
        app.warnings = sg.warnings.clone();
        if let Some(w) = warning_text(&app.warnings) {
            app.status = w;
        }
        Ok(app)
    }

    fn rebuild(&mut self) -> Result<(), String> {
        // 先手是棋谱事实（SGF 首手颜色 / PL），不能用 with_setup_and_rule 重新猜
        let mut game = Game::with_setup_rules_first(
            self.size,
            self.komi,
            self.setup.clone(),
            self.rules,
            self.first,
        );
        for (i, mv) in self.moves[..self.pos].iter().enumerate() {
            game.play(*mv)
                .map_err(|e| format!("第 {} 手重放失败：{e}", i + 1))?;
        }
        self.game = game;
        Ok(())
    }

    pub fn game(&self) -> &Game {
        &self.game
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// 棋局/引擎设置版本；分析线程带着它回来，过期的结果直接丢弃。
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 换了棋局或引擎设置：旧分析作废，允许立刻发起新的分析。
    fn next_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
    }

    pub fn total_moves(&self) -> usize {
        self.moves.len()
    }

    pub fn snapshot(&self) -> Game {
        self.game.clone()
    }

    /// 整盘快照（复盘用）。
    pub fn full_game(&self) -> Result<Game, String> {
        let mut game = Game::with_setup_rules_first(
            self.size,
            self.komi,
            self.setup.clone(),
            self.rules,
            self.first,
        );
        for (i, mv) in self.moves.iter().enumerate() {
            game.play(*mv)
                .map_err(|e| format!("第 {} 手重放失败：{e}", i + 1))?;
        }
        Ok(game)
    }

    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        let x = (self.cursor.x as i32 + dx).clamp(0, self.size as i32 - 1) as u8;
        let y = (self.cursor.y as i32 + dy).clamp(0, self.size as i32 - 1) as u8;
        self.cursor = Point::new(x, y);
    }

    /// 在光标处落子（人）。人机模式下轮到 AI 时会拒绝。
    pub fn play_at_cursor(&mut self) -> Result<(), String> {
        if self.ai_enabled && self.to_move() != self.human {
            return Err(strings(self.lang).turn_ai.to_string());
        }
        self.play_move(Move::Play(self.cursor))
    }

    /// 落任意一手（人或 AI 共用）。
    pub fn play_move(&mut self, mv: Move) -> Result<(), String> {
        self.preview = None;
        let mut probe = self.snapshot();
        // 先验合法性：失败时不能动 generation / 主线 / 缓存 / busy
        probe.play(mv).map_err(|e| format!("{e}"))?;

        let base = self.pos;
        // 只有「从中间另一手改走」才是换线；在末尾续下时历史前缀没变，
        // 已发出的上一手分析回来仍然有效（否则最近一手的点评会凭空消失）。
        let branching = base < self.moves.len();
        if branching {
            // 换线 = 新棋局版本：旧分支的在途分析和结果一律作废
            self.next_generation();
        }
        self.moves.truncate(base);
        self.moves.push(mv);
        self.pos = base + 1;
        self.game = probe;
        // 只保留分支点及之前的分析；更深位置属于旧分支
        self.evals.retain(|&k, _| k <= base);
        if branching {
            self.failed_pos = None;
        }
        self.review = None;
        self.autosave();
        Ok(())
    }

    /// 把当前局面写回它对应的文件（如果有）。
    ///
    /// 文件被别的进程改过时不会覆盖它，而是另存冲突副本并切换过去。
    fn autosave(&mut self) {
        let Some(path) = self.current_file.clone() else {
            return;
        };
        let setup = self.setup.clone();
        let result = self.store.save(
            &path,
            self.size,
            self.komi,
            &setup,
            &self.moves,
            storage::WriteOptions {
                rules: self.rules,
                first: self.first,
                expected: self.file_stamp,
            },
        );
        match result {
            Ok(storage::SaveOutcome::Written(stamp)) => {
                self.file_stamp = Some(stamp);
            }
            Ok(storage::SaveOutcome::Conflict(alt)) => {
                match storage::FileStamp::of(&alt) {
                    Ok(st) => self.file_stamp = st,
                    // 读到一半失败就保留旧指纹：下次保存会变成冲突副本，而不是无条件覆盖
                    Err(e) => self.status = e,
                }
                self.current_file = Some(alt.clone());
                self.status = i18n::fill(
                    strings(self.lang).conflict_saved,
                    alt.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                );
            }
            Err(e) => self.status = e,
        }
    }

    /// 指定棋盘大小开新局（9/13/19）。
    pub fn start_play_on(&mut self, size: u8) {
        self.size = size;
        self.cursor = Point::new(size / 2, size / 2);
        self.start_play();
    }

    /// 开始一盘人机对局：空盘，你执黑，AI 执白。
    pub fn start_play(&mut self) {
        self.preview = None;
        self.next_generation();
        self.rules = RuleSet::default();
        self.first = Color::Black;
        self.warnings.clear();
        self.setup.clear();
        self.moves.clear();
        self.pos = 0;
        self.evals.clear();
        self.review = None;
        self.last_comment = None;
        self.failed_pos = None;
        self.ai_enabled = true;
        self.vs_ai = true;
        self.human = Color::Black;
        self.cursor = Point::new(self.size / 2, self.size / 2);
        if let Err(e) = self.rebuild() {
            self.status = e;
            return;
        }
        match self.store.create() {
            Ok(path) => {
                self.current_file = Some(path.clone());
                // 新文件还没有“上次看到的指纹”，否则会误判成冲突
                self.file_stamp = None;
                self.autosave();
                self.status = i18n::fill(
                    strings(self.lang).new_game,
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                );
            }
            Err(e) => self.status = e,
        }
    }

    /// 打开一盘棋。
    ///
    /// 外部文件**永远不原地改写**：先复制到 `games/`，之后的自动保存只写副本。
    /// 已经在 `games/` 里的文件直接续写。
    pub fn open_sgf_file(&mut self, path: PathBuf, sg: &SgfGame) -> Result<(), String> {
        // 事务性：先在局部把棋局重放成功、把副本写好，再一次性提交。
        // 任何一步失败都保持当前会话原样，避免"旧棋盘 + 新元数据"或覆盖旧文件。
        let size = sg.size;
        let komi = sg.komi.unwrap_or(7.5);
        let rules = sgf::rule_set_from_rules(sg.rules.as_deref());
        let first = sg.first_to_move();
        let setup = sg.setup.clone();
        let moves = sg.moves.clone();

        let mut game = Game::with_setup_rules_first(size, komi, setup.clone(), rules, first);
        for (i, mv) in moves.iter().enumerate() {
            game.play(*mv)
                .map_err(|e| format!("第 {} 手重放失败：{e}", i + 1))?;
        }

        let inside_store = path.starts_with(self.store.dir());
        let mut stamp: Option<storage::FileStamp> = None;
        let new_file = if inside_store {
            // 在锁内读一次：确认磁盘上仍是调用方解析过的那份，
            // 并且拿到与这些字节对应的指纹（否则会“读旧内容、记新指纹”从而静默覆盖别人）
            let (text, st) = self
                .store
                .read_stamped(&path)?
                .ok_or_else(|| format!("棋谱不存在：{}", path.display()))?;
            let current =
                sgf::parse(&text).map_err(|e| format!("重新读取 {} 失败：{e}", path.display()))?;
            if current.moves != moves
                || current.setup != setup
                || current.first != sg.first
                || current.rules != sg.rules
            {
                return Err("棋谱在打开期间被其他程序修改，请重新打开".to_string());
            }
            stamp = Some(st);
            Some(path)
        } else {
            match self.store.create() {
                Ok(copy) => {
                    match self.store.save(
                        &copy,
                        size,
                        komi,
                        &setup,
                        &moves,
                        storage::WriteOptions {
                            rules,
                            first,
                            expected: None,
                        },
                    )? {
                        storage::SaveOutcome::Written(st) => stamp = Some(st),
                        storage::SaveOutcome::Conflict(_) => {
                            return Err("新建棋谱时发生冲突，请重试".to_string())
                        }
                    }
                    Some(copy)
                }
                Err(_) => None,
            }
        };

        // 到这里才动 self
        self.preview = None;
        self.next_generation();
        self.size = size;
        self.komi = komi;
        self.rules = rules;
        self.first = first;
        self.setup = setup;
        self.moves = moves;
        self.pos = self.moves.len();
        self.evals.clear();
        self.review = None;
        self.last_comment = None;
        self.failed_pos = None;
        self.cursor = Point::new(size / 2, size / 2);
        self.game = game;
        self.vs_ai = false;
        self.file_stamp = stamp;
        self.current_file = new_file;
        self.warnings = sg.warnings.clone();
        if let Some(w) = warning_text(&self.warnings) {
            self.status = w;
        }
        Ok(())
    }

    pub fn toggle_games(&mut self) {
        self.show_games = !self.show_games;
        if self.show_games {
            self.refresh_games();
            self.game_cursor = 0;
        }
    }

    pub fn refresh_games(&mut self) {
        self.games = self.store.list();
    }

    pub fn move_game_cursor(&mut self, delta: i32) {
        if self.games.is_empty() {
            return;
        }
        let n = self.games.len() as i32;
        let mut i = self.game_cursor as i32 + delta;
        if i < 0 {
            i = n - 1;
        } else if i >= n {
            i = 0;
        }
        self.game_cursor = i as usize;
    }

    pub fn open_selected_game(&mut self) {
        let Some(entry) = self.games.get(self.game_cursor).cloned() else {
            return;
        };
        let result = self
            .store
            .read(&entry.path)
            .and_then(|text| sgf::parse(&text));
        match result {
            Ok(sg) => match self.open_sgf_file(entry.path.clone(), &sg) {
                Ok(()) => {
                    self.show_games = false;
                    let loaded = i18n::fill(
                        strings(self.lang).game_loaded,
                        entry
                            .path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?"),
                    );
                    self.status = match warning_text(&self.warnings) {
                        Some(w) => format!("{loaded} · {w}"),
                        None => loaded,
                    };
                }
                Err(e) => self.status = e,
            },
            Err(e) => self.status = e,
        }
    }

    pub fn toggle_ai(&mut self) {
        self.ai_enabled = !self.ai_enabled;
        let st = strings(self.lang);
        self.status = if self.ai_enabled {
            i18n::fill(st.ai_on, i18n::color_name(self.human, self.lang))
        } else {
            st.ai_off.to_string()
        };
    }

    pub fn human_model_available(&self) -> bool {
        self.config
            .engine
            .human_model
            .as_ref()
            .is_some_and(|p| p.exists())
    }

    pub fn human_profile(&self) -> &'static str {
        HUMAN_RANKS[self.human_rank.min(HUMAN_RANKS.len() - 1)]
    }

    /// 调整每次分析的 visits（10–5000），下一次分析立即生效。
    pub fn set_visits(&mut self, visits: u32) {
        let visits = visits.clamp(10, 5000);
        self.config.engine.visits = visits;
        self.engine.set_visits(visits);
        self.failed_pos = None;
        self.status = i18n::fill(strings(self.lang).strength_set, visits);
    }

    /// 开关拟人网；切换后清掉普通模型的评估，避免 AI 用旧结果选点。
    pub fn toggle_human(&mut self) {
        let st = strings(self.lang);
        if !self.human_model_available() {
            self.status = st.no_human_model.to_string();
            return;
        }
        self.human_enabled = !self.human_enabled;
        self.next_generation();
        let profile = self.human_enabled.then(|| self.human_profile().to_string());
        self.engine.set_human_profile(profile);
        self.evals.clear();
        self.failed_pos = None;
        self.review = None;
        self.last_comment = None;
        self.status = if self.human_enabled {
            i18n::fill(
                st.human_on,
                i18n::rank_label(self.human_profile(), self.lang),
            )
        } else {
            st.human_off.to_string()
        };
    }

    /// 切换拟人段位（`delta` 为 ±1，循环）。
    pub fn cycle_human_rank(&mut self, delta: i32) {
        let n = HUMAN_RANKS.len() as i32;
        let idx = (self.human_rank as i32 + delta).rem_euclid(n) as usize;
        self.set_human_rank(idx);
    }

    /// 直接选中某个拟人段位（下拉框用）。没有拟人网也允许预先选，
    /// 只是状态里提示不可用。
    pub fn set_human_rank(&mut self, idx: usize) {
        if idx >= HUMAN_RANKS.len() {
            return;
        }
        self.human_rank = idx;
        let st = strings(self.lang);
        if !self.human_model_available() {
            self.status = st.no_human_model.to_string();
            return;
        }
        if self.human_enabled {
            self.next_generation();
            self.engine
                .set_human_profile(Some(self.human_profile().to_string()));
            self.evals.clear();
            self.failed_pos = None;
        }
        self.status = i18n::fill(
            st.human_rank_set,
            i18n::rank_label(self.human_profile(), self.lang),
        );
    }

    /// 当前选中的棋谱（棋谱列表里光标所指）。
    pub fn selected_game(&self) -> Option<&GameEntry> {
        self.games.get(self.game_cursor)
    }

    /// 重命名选中的棋谱；当前打开的棋谱会跟着改路径。
    pub fn rename_selected_game(&mut self, name: &str) -> Result<(), String> {
        let Some(entry) = self.selected_game().cloned() else {
            return Err(strings(self.lang).no_games.to_string());
        };
        let new_path = self.store.rename(&entry.path, name)?;
        if self.current_file.as_deref() == Some(entry.path.as_path()) {
            self.current_file = Some(new_path.clone());
            match storage::FileStamp::of(&new_path) {
                Ok(st) => self.file_stamp = st,
                Err(e) => self.status = e,
            }
        }
        self.refresh_games();
        let label = new_path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        self.status = i18n::fill(strings(self.lang).renamed, label);
        Ok(())
    }

    /// 删除选中的棋谱：移到 `games/.trash/`，并清掉当前文件的自动保存目标。
    pub fn delete_selected_game(&mut self) -> Result<(), String> {
        let Some(entry) = self.selected_game().cloned() else {
            return Err(strings(self.lang).no_games.to_string());
        };
        let trashed = self.store.trash(&entry.path)?;
        if self.current_file.as_deref() == Some(entry.path.as_path()) {
            self.current_file = None;
            self.file_stamp = None;
        }
        self.refresh_games();
        self.game_cursor = self.game_cursor.min(self.games.len().saturating_sub(1));
        let label = trashed.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        self.status = i18n::fill(strings(self.lang).deleted, label);
        Ok(())
    }

    pub fn prev_move(&mut self) {
        self.preview = None;
        if self.pos > 0 {
            self.pos -= 1;
            if let Err(e) = self.rebuild() {
                self.status = e;
            }
        }
    }

    pub fn next_move(&mut self) {
        self.preview = None;
        if self.pos < self.moves.len() {
            self.pos += 1;
            if let Err(e) = self.rebuild() {
                self.status = e;
            }
        }
    }

    pub fn goto_start(&mut self) {
        self.jump_to_turn(0);
    }

    pub fn goto_end(&mut self) {
        self.jump_to_turn(self.moves.len() as u16);
    }

    pub fn jump_to_turn(&mut self, turn: u16) {
        self.preview = None;
        self.pos = (turn as usize).min(self.moves.len());
        if let Err(e) = self.rebuild() {
            self.status = e;
        }
    }

    pub fn next_blunder(&mut self) {
        let Some(review) = &self.review else {
            self.status = strings(self.lang).need_review.to_string();
            return;
        };
        if review.blunders.is_empty() {
            self.status = strings(self.lang).no_blunders.to_string();
            return;
        }
        let turns: Vec<u16> = review
            .blunders
            .iter()
            .map(|&i| review.deltas[i].turn)
            .collect();
        let next = turns
            .iter()
            .copied()
            .find(|&t| t as usize > self.pos)
            .or_else(|| turns.first().copied());
        if let Some(turn) = next {
            self.jump_to_turn(turn);
            self.status = i18n::fill(strings(self.lang).jump_turn, turn);
        }
    }

    /// 当前展示的棋盘：预览变化时是预览局面，否则是主线局面。
    /// 当前该做的动作：分析当前局面、让 AI 落子，或者什么都不做。
    pub fn next_action(&self) -> Action {
        if self.busy || !self.ai_enabled {
            return Action::None;
        }
        // 翻旧棋时不自动应手
        if self.pos != self.moves.len() {
            return Action::None;
        }
        // 分析失败不自动重试，避免死循环
        if self.failed_pos == Some(self.pos) {
            return Action::None;
        }
        if self.to_move() == self.human {
            return if self.evals.contains_key(&self.pos) {
                Action::None
            } else {
                Action::Analyze
            };
        }
        match self.ai_choice() {
            Some(mv) => Action::AiMove(mv),
            // 已经有分析但没有合法候选：不要再无限触发分析
            None if self.analysis().is_some() => Action::None,
            None => Action::Analyze,
        }
    }

    pub fn analysis(&self) -> Option<&TurnEval> {
        self.evals.get(&self.pos)
    }

    pub fn eval_at(&self, pos: usize) -> Option<&TurnEval> {
        self.evals.get(&pos)
    }

    /// 当前局面 AI 会选哪一手（用于应手）。
    pub fn ai_choice(&self) -> Option<Move> {
        if !self.ai_enabled || self.to_move() == self.human {
            return None;
        }
        let analysis = self.analysis()?;
        if self.human_enabled {
            if let Some(policy) = &analysis.human_policy {
                if let Some(mv) = self.sample_human_policy(policy) {
                    return Some(mv);
                }
            }
        }
        let candidates: Vec<&engine::Candidate> = analysis
            .candidates
            .iter()
            .take(3)
            .filter(|c| self.is_legal(c.mv))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        // 前 3 个按 7:2:1 挑，避免每盘都一模一样的应手；用局面号做种子，可复现
        let roll = mix64(self.pos as u64 * 0x9E37 + 7) % 10;
        let idx = if roll < 7 {
            0
        } else if roll < 9 {
            1
        } else {
            2
        };
        candidates
            .get(idx)
            .or_else(|| candidates.first())
            .map(|c| c.mv)
    }

    /// 按 humanPolicy 概率抽一手；最后一位是 pass。
    fn sample_human_policy(&self, policy: &[f32]) -> Option<Move> {
        let size = self.size as usize;
        let cells = size * size;
        if policy.len() < cells {
            return None;
        }
        let mut choices: Vec<(usize, f32)> = Vec::new();
        for (i, p) in policy.iter().take(cells + 1).enumerate() {
            if *p <= 0.0 {
                continue;
            }
            let mv = if i == cells {
                Move::Pass
            } else {
                Move::Play(Point::new((i % size) as u8, (i / size) as u8))
            };
            if self.is_legal(mv) {
                choices.push((i, *p));
            }
        }
        let total: f64 = choices.iter().map(|(_, p)| *p as f64).sum();
        if total <= 0.0 {
            return None;
        }
        let mut roll = (mix64(self.ai_seed ^ (self.pos as u64).wrapping_mul(0x9E37_79B9))
            % 1_000_000) as f64
            / 1_000_000.0
            * total;
        let mut last = None;
        for (i, p) in &choices {
            last = Some(*i);
            roll -= *p as f64;
            if roll < 0.0 {
                return Some(if *i == cells {
                    Move::Pass
                } else {
                    Move::Play(Point::new((i % size) as u8, (i / size) as u8))
                });
            }
        }
        last.map(|i| {
            if i == cells {
                Move::Pass
            } else {
                Move::Play(Point::new((i % size) as u8, (i / size) as u8))
            }
        })
    }

    /// 这手在当前局面是否合法（防止旧分析/引擎 bug 让自动应手卡死）。
    fn is_legal(&self, mv: Move) -> bool {
        let mut game = self.game.clone();
        game.play(mv).is_ok()
    }

    /// 给"你"的下一步预测（当前局面的前三个候选）；变化预览时不显示。
    pub fn prediction(&self) -> Vec<(Move, f32)> {
        if self.preview_active() {
            return Vec::new();
        }
        self.analysis()
            .map(|a| {
                a.candidates
                    .iter()
                    .take(3)
                    .map(|c| (c.mv, c.winrate_black))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn apply(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Analysis {
                generation,
                pos,
                result,
            } => {
                if generation != self.generation {
                    return;
                }
                self.busy = false;
                match result {
                    Ok(t) => {
                        self.failed_pos = None;
                        self.evals.insert(pos, t);
                        self.status = strings(self.lang).analysis_done.to_string();
                        if pos > 0 && pos <= self.moves.len() {
                            let before = self.evals.get(&(pos - 1)).cloned();
                            let after = self.evals.get(&pos).cloned();
                            if let (Some(before), Some(after)) = (before, after) {
                                let color = self.game.color_of_move(pos as u16);
                                let mv = self.moves[pos - 1];
                                let comment =
                                    self.game.position_at(pos as u16 - 1).map(|pos_before| {
                                        review::quick_comment(
                                            !self.lang.is_zh(),
                                            pos_before,
                                            &before,
                                            &after,
                                            color,
                                            mv,
                                            if self.vs_ai { Some(self.human) } else { None },
                                        )
                                    });
                                self.last_comment = comment;
                            }
                        }
                    }
                    Err(e) => {
                        self.failed_pos = Some(pos);
                        self.status = i18n::fill(strings(self.lang).analysis_failed, e);
                    }
                }
            }
            AppEvent::Review { generation, result } => {
                if generation != self.generation {
                    return;
                }
                match result {
                    Ok(r) => {
                        self.review = Some(r);
                        self.busy = false;
                        self.status = strings(self.lang).review_done.to_string();
                    }
                    Err(e) => {
                        self.busy = false;
                        self.status = i18n::fill(strings(self.lang).review_failed, e);
                    }
                }
            }
        }
    }

    /// 当前局面是否因为分析失败而卡住（避免自动重试死循环）
    pub fn last_failed_pos(&self) -> Option<usize> {
        self.failed_pos
    }

    /// AI 应手落子失败（通常是旧分析结果/非法候选）：停在当前局面，别再每帧重试。
    pub fn fail_ai_move(&mut self, message: String) {
        self.failed_pos = Some(self.pos);
        self.status = message;
    }

    pub fn to_move(&self) -> Color {
        self.game.position().to_move
    }

    pub fn candidate_rank(&self, p: Point) -> Option<usize> {
        if self.preview_active() {
            return None;
        }
        self.analysis()?
            .candidates
            .iter()
            .take(3)
            .position(|c| c.mv == Move::Play(p))
            .map(|i| i + 1)
    }
}

/// 把一手棋格式化成界面上的 GTP 坐标（A–T，跳过 I）。
pub fn move_text(mv: Move, size: u8) -> String {
    match mv {
        Move::Play(p) => engine::gtp_coord(p, size),
        Move::Pass => "pass".to_string(),
    }
}

fn seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests;
