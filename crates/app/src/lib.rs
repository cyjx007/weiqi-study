//! 与界面无关的应用状态：导航、落子、分析结果缓存。
//!
//! 前端只负责把按键映射成这里的调用，以及把状态画出来。
//! 本 crate 不碰终端、不 spawn 线程——分析由前端起线程后把结果送回来。

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

#[derive(Debug)]
pub enum AppEvent {
    Analysis(Result<TurnEval, String>),
    Review(Result<Review, String>),
}

pub struct App {
    pub size: u8,
    pub komi: f32,
    pub cursor: Point,
    pub analysis: Option<TurnEval>,
    pub review: Option<Review>,
    pub status: String,
    pub busy: bool,
    pub show_help: bool,
    pub config: AppConfig,
    setup: Vec<(Color, Point)>,
    moves: Vec<Move>,
    pos: usize,
    game: Game,
}

impl App {
    pub fn from_sgf(sg: &SgfGame, config: AppConfig) -> Result<Self, String> {
        let mut app = App {
            size: sg.size,
            komi: sg.komi.unwrap_or(7.5),
            cursor: Point::new(sg.size / 2, sg.size / 2),
            analysis: None,
            review: None,
            status: "就绪".to_string(),
            busy: false,
            show_help: false,
            config,
            setup: sg.setup.clone(),
            moves: sg.moves.clone(),
            pos: sg.moves.len(),
            game: Game::new(sg.size, sg.komi.unwrap_or(7.5)),
        };
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

    /// 当前局面快照（交给分析线程用）。
    pub fn snapshot(&self) -> Game {
        self.game.clone()
    }

    /// 整盘快照（复盘用，不含当前未落子的光标位置）。
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

    /// 在光标处落子。若当前不在末尾，会截断后面的手。
    pub fn play_at_cursor(&mut self) -> Result<(), String> {
        let mut probe = self.snapshot();
        probe
            .play(Move::Play(self.cursor))
            .map_err(|e| format!("{e}"))?;
        self.moves.truncate(self.pos);
        self.moves.push(Move::Play(self.cursor));
        self.pos += 1;
        self.game = probe;
        self.invalidate();
        Ok(())
    }

    pub fn prev_move(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
            if let Err(e) = self.rebuild() {
                self.status = e;
                return;
            }
            self.invalidate();
        }
    }

    pub fn next_move(&mut self) {
        if self.pos < self.moves.len() {
            self.pos += 1;
            if let Err(e) = self.rebuild() {
                self.status = e;
                return;
            }
            self.invalidate();
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
            return;
        }
        self.invalidate();
    }

    pub fn next_blunder(&mut self) {
        let Some(review) = &self.review else {
            self.status = "还没有全盘复盘（按 A）".to_string();
            return;
        };
        if review.blunders.is_empty() {
            self.status = "没有超过阈值的失误".to_string();
            return;
        }
        // 先收集，避免持有 review 的同时改 self
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
            self.status = format!("跳到第 {turn} 手");
        }
    }

    fn invalidate(&mut self) {
        self.analysis = None;
    }

    pub fn apply(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Analysis(Ok(t)) => {
                self.analysis = Some(t);
                self.busy = false;
                self.status = "分析完成".to_string();
            }
            AppEvent::Analysis(Err(e)) => {
                self.busy = false;
                self.status = format!("分析失败：{e}");
            }
            AppEvent::Review(Ok(r)) => {
                self.review = Some(r);
                self.busy = false;
                self.status = "全盘复盘完成（b 跳到下一个失误）".to_string();
            }
            AppEvent::Review(Err(e)) => {
                self.busy = false;
                self.status = format!("复盘失败：{e}");
            }
        }
    }

    /// 当前局面轮到谁
    pub fn to_move(&self) -> Color {
        self.game.position().to_move
    }

    /// 光标处是不是一个候选点，返回名次（1-based）
    pub fn candidate_rank(&self, p: Point) -> Option<usize> {
        let analysis = self.analysis.as_ref()?;
        analysis
            .candidates
            .iter()
            .take(3)
            .position(|c| c.mv == Move::Play(p))
            .map(|i| i + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goban::Move;

    fn demo() -> App {
        let sg = sgf::parse("(;SZ[9]KM[7.5];B[cc];W[gg];B[dd];W[ee])").unwrap();
        App::from_sgf(&sg, AppConfig::default()).unwrap()
    }

    #[test]
    fn starts_at_end_and_navigates() {
        let mut app = demo();
        assert_eq!(app.pos(), 4);
        app.goto_start();
        assert_eq!(app.pos(), 0);
        assert_eq!(app.game().moves().len(), 0);
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
        app.cursor = Point::new(2, 2); // 第一手黑子处
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
}
