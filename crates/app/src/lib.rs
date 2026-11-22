//! 与界面无关的应用状态：导航、落子、人机对战、分析缓存。
//!
//! 前端只负责把按键映射成这里的调用，以及把状态画出来。
//! 本 crate 不碰终端、不 spawn 线程——分析由前端起线程后把结果送回来。

pub mod i18n;
pub mod storage;

use std::collections::HashMap;
use std::path::PathBuf;

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
        pos: usize,
        result: Result<TurnEval, String>,
    },
    Review(Result<Review, String>),
}

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
    /// 最近一手的即时点评
    pub last_comment: Option<String>,
    pub lang: Lang,
    pub store: GameStore,
    pub current_file: Option<PathBuf>,
    pub games: Vec<GameEntry>,
    pub show_games: bool,
    pub game_cursor: usize,
    setup: Vec<(Color, Point)>,
    moves: Vec<Move>,
    pos: usize,
    game: Game,
    evals: HashMap<usize, TurnEval>,
    failed_pos: Option<usize>,
}

impl App {
    /// 空盘新局（不读 SGF）。
    pub fn new(size: u8, komi: f32, config: AppConfig) -> Self {
        App {
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
            last_comment: None,
            lang: Lang::from_env(),
            store: GameStore::open(),
            current_file: None,
            games: Vec::new(),
            show_games: false,
            game_cursor: 0,
            setup: Vec::new(),
            moves: Vec::new(),
            pos: 0,
            game: Game::new(size, komi),
            evals: HashMap::new(),
            failed_pos: None,
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

    pub fn open_sgf_file(
        &mut self,
        path: PathBuf,
        sg: &SgfGame,
    ) -> Result<(), String> {
        self.size = sg.size;
        self.komi = sg.komi.unwrap_or(7.5);
        self.setup = sg.setup.clone();
        self.moves = sg.moves.clone();
        self.pos = self.moves.len();
        self.current_file = Some(path);
        self.evals.clear();
        self.review = None;
        self.last_comment = None;
        self.failed_pos = None;
        self.cursor = Point::new(self.size / 2, self.size / 2);
        self.rebuild()
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

    pub fn prev_move(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
            if let Err(e) = self.rebuild() {
                self.status = e;
            }
        }
    }

    pub fn next_move(&mut self) {
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
        let candidates: Vec<&engine::Candidate> =
            self.analysis()?.candidates.iter().take(3).collect();
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

    /// 给"你"的下一步预测（当前局面的前三个候选）。
    pub fn prediction(&self) -> Vec<(Move, f32)> {
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
            AppEvent::Analysis { pos, result } => {
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
                                            pos_before, &before, &after, color, mv, size,
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
            AppEvent::Review(Ok(r)) => {
                self.review = Some(r);
                self.busy = false;
                self.status = strings(self.lang).review_done.to_string();
            }
            AppEvent::Review(Err(e)) => {
                self.busy = false;
                self.status = i18n::fill(strings(self.lang).review_failed, e);
            }
        }
    }

    /// 当前局面是否因为分析失败而卡住（避免自动重试死循环）
    pub fn last_failed_pos(&self) -> Option<usize> {
        self.failed_pos
    }

    pub fn to_move(&self) -> Color {
        self.game.position().to_move
    }

    pub fn candidate_rank(&self, p: Point) -> Option<usize> {
        self.analysis()?
            .candidates
            .iter()
            .take(3)
            .position(|c| c.mv == Move::Play(p))
            .map(|i| i + 1)
    }
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
                    let result = engine::analyze_position(&app.config.engine, &app.snapshot());
                    let pos = app.pos();
                    app.apply(AppEvent::Analysis { pos, result });
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
    fn failed_analysis_does_not_retry_forever() {
        let mut app = demo();
        start(&mut app);
        app.apply(AppEvent::Analysis {
            pos: 0,
            result: Err("boom".to_string()),
        });
        assert_eq!(app.next_action(), Action::None);
    }

    #[test]
    fn analysis_produces_quick_comment() {
        let mut app = demo();
        start(&mut app);
        app.cursor = Point::new(2, 2);
        app.play_at_cursor().unwrap();
        // 推荐手故意和实际下的不一样，才能测到「推荐」那句
        app.evals
            .insert(0, eval(0, Color::Black, 0.70, Move::Play(Point::new(6, 6))));
        // 黑下完之后胜率掉到 40%
        app.apply(AppEvent::Analysis {
            pos: 1,
            result: Ok(eval(1, Color::White, 0.60, Move::Play(Point::new(6, 6)))),
        });
        let c = app.last_comment.as_deref().unwrap();
        assert!(c.contains("胜率"));
        assert!(c.contains("推荐"));
    }
}
