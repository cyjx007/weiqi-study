//! 与界面无关的应用状态：导航、落子、人机对战、分析缓存。
//!
//! 前端只负责把按键映射成 [`Controller`] 的调用，以及把状态画出来。
//! 后台分析线程与事件回填都在 [`controller`] 里，TUI/GUI 共用同一份。

pub mod controller;
pub mod i18n;
pub mod storage;

pub use controller::Controller;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use i18n::{strings, Lang};
use storage::{GameEntry, GameStore};

use engine::{EngineConfig, TurnEval};
use goban::{Color, Game, Move, Point};
use review::Review;
use sgf::SgfGame;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub engine: EngineConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            engine: EngineConfig::default(),
        }
    }
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
pub const HUMAN_RANKS: [&str; 9] = [
    "rank_20k",
    "rank_15k",
    "rank_10k",
    "rank_5k",
    "rank_1k",
    "rank_1d",
    "rank_3d",
    "rank_5d",
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
    pub games: Vec<GameEntry>,
    pub show_games: bool,
    pub show_info: bool,
    pub game_cursor: usize,
    setup: Vec<(Color, Point)>,
    moves: Vec<Move>,
    pos: usize,
    game: Game,
    evals: HashMap<usize, TurnEval>,
    failed_pos: Option<usize>,
    /// 拟人网选点的随机种子；测试可以固定它
    ai_seed: u64,
    /// 棋局/引擎设置版本；旧的分析结果回来时按这个丢弃
    generation: u64,
    /// 引擎推荐变化的临时预览（不动主线）
    preview: Option<Preview>,
}

/// 从 `base` 开始预览 `pv` 的前 `step` 手；`game` 是预览到当前步的棋盘。
struct Preview {
    base: usize,
    pv: Vec<Move>,
    step: usize,
    game: Game,
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
            games: Vec::new(),
            show_games: false,
            show_info: false,
            game_cursor: 0,
            setup: Vec::new(),
            moves: Vec::new(),
            pos: 0,
            game: Game::new(size, komi),
            evals: HashMap::new(),
            failed_pos: None,
            ai_seed: seed(),
            generation: 0,
            preview: None,
        }
    }

    pub fn from_sgf(sg: &SgfGame, config: AppConfig) -> Result<Self, String> {
        let mut app = Self::new(sg.size, sg.komi.unwrap_or(7.5), config);
        app.setup = sg.setup.clone();
        app.moves = sg.moves.clone();
        app.pos = sg.moves.len();
        app.rebuild()?;
        Ok(app)
    }

    fn rebuild(&mut self) -> Result<(), String> {
        let mut game = Game::with_setup(self.size, self.komi, self.setup.clone());
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
        let mut game = Game::with_setup(self.size, self.komi, self.setup.clone());
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
        probe.play(mv).map_err(|e| format!("{e}"))?;
        self.moves.truncate(self.pos);
        self.moves.push(mv);
        self.pos += 1;
        self.game = probe;
        // 新分支之后的评估全部作废
        self.evals.retain(|&k, _| k <= self.pos);
        self.review = None;
        self.autosave();
        Ok(())
    }

    /// 把当前局面写回它对应的文件（如果有）。
    fn autosave(&mut self) {
        let Some(path) = self.current_file.clone() else {
            return;
        };
        let setup = self.setup.clone();
        if let Err(e) = self.store.save(&path, self.size, self.komi, &setup, &self.moves) {
            self.status = e;
        }
    }

    /// 开始一盘人机对局：空盘，你执黑，AI 执白。
    pub fn start_play(&mut self) {
        self.preview = None;
        self.next_generation();
        self.setup.clear();
        self.moves.clear();
        self.pos = 0;
        self.evals.clear();
        self.review = None;
        self.last_comment = None;
        self.failed_pos = None;
        self.ai_enabled = true;
        self.human = Color::Black;
        self.cursor = Point::new(self.size / 2, self.size / 2);
        if let Err(e) = self.rebuild() {
            self.status = e;
            return;
        }
        match self.store.create() {
            Ok(path) => {
                self.current_file = Some(path.clone());
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
        self.preview = None;
        self.next_generation();
        self.size = sg.size;
        self.komi = sg.komi.unwrap_or(7.5);
        self.setup = sg.setup.clone();
        self.moves = sg.moves.clone();
        self.pos = self.moves.len();
        self.evals.clear();
        self.review = None;
        self.last_comment = None;
        self.failed_pos = None;
        self.cursor = Point::new(self.size / 2, self.size / 2);
        self.rebuild()?;

        let inside_store = path.starts_with(self.store.dir());
        self.current_file = if inside_store {
            Some(path)
        } else {
            match self.store.create() {
                Ok(copy) => {
                    self.store.save(&copy, self.size, self.komi, &self.setup, &self.moves)?;
                    Some(copy)
                }
                Err(_) => None,
            }
        };
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
                    self.status = i18n::fill(
                        strings(self.lang).game_loaded,
                        entry.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    );
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
            i18n::fill(st.human_on, self.human_profile())
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
        self.status = i18n::fill(st.human_rank_set, self.human_profile());
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
        }
        self.refresh_games();
        let label = new_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
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
        }
        self.refresh_games();
        self.game_cursor = self.game_cursor.min(self.games.len().saturating_sub(1));
        let label = trashed
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
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
    pub fn display_game(&self) -> &Game {
        self.preview
            .as_ref()
            .map(|p| &p.game)
            .unwrap_or(&self.game)
    }

    pub fn display_pos(&self) -> usize {
        self.preview
            .as_ref()
            .map(|p| p.base + p.step)
            .unwrap_or(self.pos)
    }

    pub fn display_to_move(&self) -> Color {
        self.display_game().position().to_move
    }

    pub fn preview_active(&self) -> bool {
        self.preview.is_some()
    }

    /// (已走步数, 变化总手数)
    pub fn preview_progress(&self) -> Option<(usize, usize)> {
        self.preview.as_ref().map(|p| (p.step, p.pv.len()))
    }

    /// 用当前分析的第一候选 PV 进入变化预览，不修改主线。
    pub fn start_preview(&mut self) {
        let Some(analysis) = self.analysis() else {
            self.status = strings(self.lang).no_variation.to_string();
            return;
        };
        let pv: Vec<Move> = analysis
            .candidates
            .first()
            .map(|c| c.pv.iter().take(12).copied().collect())
            .unwrap_or_default();
        if pv.is_empty() {
            self.status = strings(self.lang).no_variation.to_string();
            return;
        }
        let base = self.pos;
        self.preview = Some(Preview {
            base,
            step: 0,
            pv,
            game: self.game.clone(),
        });
        self.status = i18n::fill(strings(self.lang).variation_start, self.preview_len());
    }

    fn preview_len(&self) -> usize {
        self.preview.as_ref().map(|p| p.pv.len()).unwrap_or(0)
    }

    fn preview_status(&self) -> String {
        match self.preview_progress() {
            Some((step, total)) => {
                format!("{} {step}/{total}", strings(self.lang).variation)
            }
            None => String::new(),
        }
    }

    pub fn preview_next(&mut self) {
        let Some(p) = self.preview.as_mut() else {
            return;
        };
        if p.step >= p.pv.len() {
            return;
        }
        let mv = p.pv[p.step];
        let mut game = p.game.clone();
        match game.play(mv) {
            Ok(()) => {
                p.step += 1;
                p.game = game;
                self.status = self.preview_status();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    pub fn preview_prev(&mut self) {
        let Some(p) = self.preview.as_mut() else {
            return;
        };
        if p.step == 0 {
            return;
        }
        let step = p.step - 1;
        let mut game = self.game.clone();
        for mv in p.pv.iter().take(step) {
            if game.play(*mv).is_err() {
                return;
            }
        }
        p.step = step;
        p.game = game;
        self.status = self.preview_status();
    }

    /// 采用预览到的变化：截断主线并落子，之后照常自动保存。
    pub fn commit_preview(&mut self) {
        let Some(p) = self.preview.take() else {
            return;
        };
        if p.step == 0 {
            return;
        }
        let step = p.step;
        self.next_generation();
        self.moves.truncate(p.base);
        self.moves.extend(p.pv.iter().take(step).copied());
        self.pos = self.moves.len();
        if let Err(e) = self.rebuild() {
            self.status = e;
            return;
        }
        self.evals.retain(|&k, _| k <= self.pos);
        self.review = None;
        self.autosave();
        self.status = i18n::fill(strings(self.lang).variation_commit, step);
    }

    pub fn exit_preview(&mut self) {
        self.preview = None;
    }

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
                                let size = self.size;
                                let comment = self
                                    .game
                                    .position_at(pos as u16 - 1)
                                    .map(|pos_before| {
                                        review::quick_comment(
                                            !self.lang.is_zh(),
                                            pos_before,
                                            &before,
                                            &after,
                                            color,
                                            mv,
                                            size,
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
mod tests {
    use super::*;
    use engine::Candidate;
    use goban::Move;

    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_store() -> storage::GameStore {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        storage::GameStore::at(std::env::temp_dir().join(format!(
            "weiqi-app-test-{}-{}",
            std::process::id(),
            n
        )))
    }

    fn start(app: &mut App) {
        app.store = test_store();
        app.start_play();
    }

    fn demo() -> App {
        let sg = sgf::parse("(;SZ[9]KM[7.5];B[cc];W[gg];B[dd];W[ee])").unwrap();
        App::from_sgf(&sg, AppConfig::default()).unwrap()
    }

    fn eval(pos: usize, to_move: Color, wr: f32, best: Move) -> TurnEval {
        TurnEval {
            turn: pos as u16,
            to_move,
            winrate_black: wr,
            score_black: 0.0,
            candidates: vec![
                Candidate {
                    mv: best,
                    winrate_black: wr,
                    score_black: 0.0,
                    pv: vec![best],
                    visits: 100,
                },
                Candidate {
                    mv: Move::Play(Point::new(0, 0)),
                    winrate_black: wr - 0.01,
                    score_black: 0.0,
                    pv: vec![Move::Play(Point::new(0, 0))],
                    visits: 50,
                },
            ],
            human_policy: None,
        }
    }

    #[test]
    fn starts_at_end_and_navigates() {
        let mut app = demo();
        assert_eq!(app.pos(), 4);
        app.goto_start();
        assert_eq!(app.pos(), 0);
        app.next_move();
        assert_eq!(app.pos(), 1);
        assert_eq!(app.game().moves()[0], Move::Play(Point::new(2, 2)));
        app.prev_move();
        assert_eq!(app.pos(), 0);
    }

    #[test]
    fn plays_and_undoes_at_cursor() {
        let mut app = demo();
        app.cursor = Point::new(0, 0);
        app.play_at_cursor().unwrap();
        assert_eq!(app.total_moves(), 5);
        assert_eq!(app.game().moves()[4], Move::Play(Point::new(0, 0)));
        app.prev_move();
        assert_eq!(app.pos(), 4);
    }

    #[test]
    fn rejects_occupied_point() {
        let mut app = demo();
        app.cursor = Point::new(2, 2);
        let before = app.total_moves();
        assert!(app.play_at_cursor().is_err());
        assert_eq!(app.total_moves(), before);
    }

    #[test]
    fn jumping_truncates_future_moves_when_playing() {
        let mut app = demo();
        app.jump_to_turn(2);
        app.cursor = Point::new(6, 2);
        app.play_at_cursor().unwrap();
        assert_eq!(app.total_moves(), 3);
    }

    #[test]
    fn cursor_is_clamped() {
        let mut app = demo();
        app.move_cursor(-100, -100);
        assert_eq!(app.cursor, Point::new(0, 0));
        app.move_cursor(100, 100);
        assert_eq!(app.cursor, Point::new(8, 8));
    }

    #[test]
    fn start_play_gives_empty_board_to_human() {
        let mut app = demo();
        start(&mut app);
        assert_eq!(app.total_moves(), 0);
        assert_eq!(app.to_move(), Color::Black);
        assert!(app.ai_enabled);
        assert_eq!(app.human, Color::Black);
    }

    #[test]
    fn human_cannot_move_on_ai_turn() {
        let mut app = demo();
        start(&mut app);
        app.evals.insert(
            0,
            eval(0, Color::Black, 1.0, Move::Play(Point::new(2, 2))),
        );
        app.cursor = Point::new(2, 2);
        app.play_at_cursor().unwrap(); // 人（黑）
        assert!(app.play_at_cursor().is_err()); // 现在轮到 AI
    }

    #[test]
    fn ai_choice_comes_from_candidates() {
        let mut app = demo();
        start(&mut app);
        app.cursor = Point::new(2, 2);
        app.play_at_cursor().unwrap(); // 黑
        app.evals
            .insert(1, eval(1, Color::White, 0.6, Move::Play(Point::new(4, 4))));
        let mv = app.ai_choice().unwrap();
        assert!(mv == Move::Play(Point::new(4, 4)) || mv == Move::Play(Point::new(0, 0)));
    }

    #[test]
    fn next_action_requests_analysis_then_ai_move() {
        let mut app = demo();
        start(&mut app);
        // 空盘、轮到人：没有评估 → 要求分析
        assert_eq!(app.next_action(), Action::Analyze);
        app.evals.insert(0, eval(0, Color::Black, 0.5, Move::Play(Point::new(2, 2))));
        assert_eq!(app.next_action(), Action::None);
        // 人落子后轮到 AI，有评估 → 直接要求 AI 落子
        app.cursor = Point::new(2, 2);
        app.play_at_cursor().unwrap();
        app.evals.insert(1, eval(1, Color::White, 0.55, Move::Play(Point::new(4, 4))));
        assert!(matches!(app.next_action(), Action::AiMove(_)));
    }

    /// 端到端：拟人网返回 humanPolicy（默认跳过，需要本机 humanv0 模型）。
    #[test]
    #[ignore = "需要本机 KataGo + humanv0 网络"]
    fn human_model_returns_a_policy() {
        let mut config = AppConfig::default();
        config.engine.visits = 1;
        assert!(
            config.engine.human_model.is_some(),
            "本机应能找到 humanv0 网络"
        );
        let engine = engine::Engine::new(config.engine.clone());
        engine.set_human_profile(Some("rank_5k".to_string()));
        let eval = engine.analyze_position(&goban::Game::new(9, 7.5)).unwrap();
        let policy = eval.human_policy.expect("humanPolicy 应该存在");
        assert_eq!(policy.len(), 9 * 9 + 1);
        assert!(policy.iter().any(|p| *p > 0.0));
    }

    /// 端到端：真的起 KataGo，走 6 手人机对局（默认跳过，需要本机引擎）。
    #[test]
    #[ignore = "需要本机 KataGo + 网络"]
    fn plays_against_real_engine() {
        let mut config = AppConfig::default();
        config.engine.visits = 40;
        let mut app = App::new(9, 7.5, config);
        start(&mut app);

        let mut guard = 0;
        while app.total_moves() < 6 && guard < 20 {
            guard += 1;
            match app.next_action() {
                Action::Analyze => {
                    let result = app.engine.analyze_position(&app.snapshot());
                    let pos = app.pos();
                    let generation = app.generation();
                    app.apply(AppEvent::Analysis {
                        generation,
                        pos,
                        result,
                    });
                }
                Action::AiMove(mv) => {
                    app.play_move(mv).unwrap();
                }
                Action::None => {
                    // 轮到人：替人下 AI 推荐的第一手
                    let Some(&(mv, _)) = app.prediction().first() else {
                        break;
                    };
                    app.play_move(mv).unwrap();
                }
            }
        }
        assert!(app.total_moves() >= 4, "应该至少走了几手");
        assert!(app.last_comment.is_some(), "应该生成了讲解");
    }

    #[test]
    fn saves_moves_and_reloads() {
        let mut app = demo();
        start(&mut app);
        app.cursor = Point::new(2, 2);
        app.play_at_cursor().unwrap();
        let file = app.current_file.clone().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let sg = sgf::parse(&text).unwrap();
        assert_eq!(sg.moves.len(), 1);
        app.refresh_games();
        assert_eq!(app.games.len(), 1);

        let mut reopened = App::new(9, 7.5, AppConfig::default());
        reopened.store = app.store.clone();
        reopened.open_sgf_file(file, &sg).unwrap();
        assert_eq!(reopened.total_moves(), 1);
        assert_eq!(reopened.pos(), 1);
    }

    #[test]
    fn opening_external_sgf_never_writes_it() {
        use std::io::Write;
        let store = test_store();
        let external = std::env::temp_dir().join(format!(
            "weiqi-external-{}-{}.sgf",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let original = "(;GM[1]FF[4]SZ[9]KM[7.5]PB[Alice];B[cc];W[gg])";
        std::fs::File::create(&external)
            .unwrap()
            .write_all(original.as_bytes())
            .unwrap();

        let sg = sgf::parse(original).unwrap();
        let mut app = App::new(9, 7.5, AppConfig::default());
        app.store = store.clone();
        app.open_sgf_file(external.clone(), &sg).unwrap();

        // 副本在 games 目录，原文件路径不再被引用
        let current = app.current_file.clone().unwrap();
        assert!(current.starts_with(store.dir()));
        assert_ne!(current, external);

        // 继续落子只写副本
        app.cursor = Point::new(4, 4);
        app.play_at_cursor().unwrap();
        let after = std::fs::read_to_string(&external).unwrap();
        assert_eq!(after, original);
        let copy = std::fs::read_to_string(&current).unwrap();
        assert!(copy.contains(";B[ee]") || copy.contains(";W[ee]") || copy.contains("[ee]"));

        let _ = std::fs::remove_file(&external);
        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn renames_and_trashes_the_current_game() {
        let mut app = demo();
        start(&mut app);
        app.cursor = Point::new(2, 2);
        app.play_at_cursor().unwrap();
        let old = app.current_file.clone().unwrap();
        app.refresh_games();
        app.game_cursor = 0;

        app.rename_selected_game("my study").unwrap();
        let renamed = app.current_file.clone().unwrap();
        assert_eq!(
            renamed.file_name().and_then(|n| n.to_str()),
            Some("my study.sgf")
        );
        assert!(!old.exists());
        assert_eq!(app.games.len(), 1);

        app.delete_selected_game().unwrap();
        assert!(app.current_file.is_none());
        assert!(app.games.is_empty());
        let trash: Vec<_> = std::fs::read_dir(app.store.dir().join(".trash"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(trash.len(), 1, "删除应进入回收站而不是真删");

        let _ = std::fs::remove_dir_all(app.store.dir());
    }

    #[test]
    fn visits_are_shared_with_the_engine() {
        let mut app = demo();
        app.set_visits(333);
        assert_eq!(app.config.engine.visits, 333);
        assert_eq!(app.engine.visits(), 333);
        app.set_visits(1);
        assert_eq!(app.engine.visits(), 10, "visits 下限 10");
    }

    #[test]
    fn human_mode_requires_a_model() {
        let mut app = demo();
        app.config.engine.human_model = None;
        app.toggle_human();
        assert!(!app.human_enabled);
        assert_eq!(app.status, strings(app.lang).no_human_model);
    }

    #[test]
    fn human_rank_cycles() {
        let mut app = demo();
        app.human_rank = 0;
        app.cycle_human_rank(-1);
        assert_eq!(app.human_profile(), "rank_9d");
        app.cycle_human_rank(1);
        assert_eq!(app.human_profile(), "rank_20k");
    }

    #[test]
    fn human_policy_drives_the_ai_move() {
        let sg = sgf::parse("(;SZ[9];B[cc];W[gg];B[dd])").unwrap();
        let mut app = App::from_sgf(&sg, AppConfig::default()).unwrap();
        app.ai_enabled = true;
        app.human = Color::Black; // 轮白，AI
        app.human_enabled = true;
        app.human_rank = 3;

        let mut policy = vec![0.0f32; 9 * 9 + 1];
        policy[4 * 9 + 4] = 1.0; // E5
        let mut e = eval(3, Color::White, 0.5, Move::Play(Point::new(0, 0)));
        e.human_policy = Some(policy);
        app.evals.insert(3, e);
        assert_eq!(app.ai_choice(), Some(Move::Play(Point::new(4, 4))));

        // 最后一位是 pass
        let mut policy = vec![0.0f32; 9 * 9 + 1];
        policy[9 * 9] = 1.0;
        let mut e = eval(3, Color::White, 0.5, Move::Play(Point::new(0, 0)));
        e.human_policy = Some(policy);
        app.evals.insert(3, e);
        assert_eq!(app.ai_choice(), Some(Move::Pass));
    }

    #[test]
    fn previews_and_commits_a_variation() {
        let sg = sgf::parse("(;SZ[9];B[cc];W[gg];B[dd])").unwrap();
        let mut app = App::from_sgf(&sg, AppConfig::default()).unwrap();
        app.evals.insert(
            3,
            TurnEval {
                turn: 3,
                to_move: Color::White,
                winrate_black: 0.5,
                score_black: 0.0,
                candidates: vec![engine::Candidate {
                    mv: Move::Play(Point::new(4, 4)),
                    winrate_black: 0.5,
                    score_black: 0.0,
                    pv: vec![Move::Play(Point::new(4, 4)), Move::Play(Point::new(2, 6))],
                    visits: 100,
                }],
                human_policy: None,
            },
        );

        app.start_preview();
        assert!(app.preview_active());
        assert_eq!(app.preview_progress(), Some((0, 2)));
        assert_eq!(app.display_pos(), 3);
        app.preview_next();
        assert_eq!(app.display_pos(), 4);
        assert_eq!(
            app.display_game().position().board.at(Point::new(4, 4)),
            Some(Color::White)
        );
        app.preview_next();
        assert_eq!(app.display_pos(), 5);
        app.preview_prev();
        assert_eq!(app.display_pos(), 4);

        app.commit_preview();
        assert!(!app.preview_active());
        assert_eq!(app.pos(), 4);
        assert_eq!(app.total_moves(), 4);
        assert_eq!(
            app.game().position().board.at(Point::new(4, 4)),
            Some(Color::White)
        );
    }

    #[test]
    fn preview_without_analysis_is_refused() {
        let mut app = demo();
        app.start_preview();
        assert!(!app.preview_active());
        assert_eq!(app.status, strings(app.lang).no_variation);
    }

    #[test]
    fn move_text_uses_gtp_coordinates() {
        assert_eq!(move_text(Move::Play(Point::new(15, 3)), 19), "Q16");
        assert_eq!(move_text(Move::Pass, 19), "pass");
    }

    #[test]
    fn stale_analysis_after_switching_games_is_ignored() {
        let mut app = demo();
        let old_generation = app.generation();
        start(&mut app); // 新局 → generation +1
        assert_ne!(app.generation(), old_generation);

        app.busy = true; // 假装新局的分析正在跑
        app.apply(AppEvent::Analysis {
            generation: old_generation,
            pos: 0,
            result: Ok(eval(0, Color::Black, 0.5, Move::Play(Point::new(2, 2)))),
        });
        assert!(app.analysis().is_none(), "旧棋局的结果不能写进新棋局");
        assert!(app.busy, "旧结果也不能把新分析标记成空闲");
    }

    #[test]
    fn opening_a_game_resets_busy_for_the_new_analysis() {
        let mut app = demo();
        app.store = test_store();
        app.busy = true;
        let path = app.store.create().unwrap();
        let sg = sgf::parse("(;SZ[9];B[aa])").unwrap();
        app.open_sgf_file(path, &sg).unwrap();
        assert!(!app.busy);
        assert_eq!(app.total_moves(), 1);
    }

    #[test]
    fn illegal_candidates_do_not_spin_the_ai() {
        let sg = sgf::parse("(;SZ[9];B[cc];W[gg];B[dd])").unwrap();
        let mut app = App::from_sgf(&sg, AppConfig::default()).unwrap();
        app.ai_enabled = true;
        app.human = Color::Black; // 轮白，AI
        let mut e = eval(3, Color::White, 0.5, Move::Play(Point::new(2, 2)));
        e.candidates.truncate(1);
        e.candidates[0].mv = Move::Play(Point::new(2, 2)); // 已被黑子占住
        e.candidates[0].pv = vec![Move::Play(Point::new(2, 2))];
        app.evals.insert(3, e);
        assert_eq!(app.ai_choice(), None, "非法候选不能交给 AI");
        assert_eq!(
            app.next_action(),
            Action::None,
            "有分析但没有合法候选时不能每帧重新触发分析"
        );
    }

    #[test]
    fn failed_analysis_does_not_retry_forever() {
        let mut app = demo();
        start(&mut app);
        app.apply(AppEvent::Analysis {
            generation: app.generation(),
            pos: 0,
            result: Err("boom".to_string()),
        });
        assert_eq!(app.next_action(), Action::None);
    }

    #[test]
    fn analysis_produces_quick_comment() {
        let mut app = demo();
        app.lang = Lang::Zh;
        start(&mut app);
        app.cursor = Point::new(2, 2);
        app.play_at_cursor().unwrap();
        // 推荐手故意和实际下的不一样，才能测到「推荐」那句
        app.evals
            .insert(0, eval(0, Color::Black, 0.70, Move::Play(Point::new(6, 6))));
        // 黑下完之后胜率掉到 40%
        app.apply(AppEvent::Analysis {
            generation: app.generation(),
            pos: 1,
            result: Ok(eval(1, Color::White, 0.60, Move::Play(Point::new(6, 6)))),
        });
        let c = app.last_comment.as_deref().unwrap();
        assert!(c.contains("胜率"));
        assert!(c.contains("推荐"));
    }
}
