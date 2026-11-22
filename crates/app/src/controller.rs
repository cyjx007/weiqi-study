//! 前端共用的控制层：起后台分析、收结果、推进 AI、决定何时重画。
//!
//! TUI 和 GTK 只需要把输入映射成这里的方法调用，然后每帧 `tick()` 一次；
//! 不再各自维护一份 spawn/pump 逻辑。

use std::sync::mpsc;

use crate::i18n::{fill, strings};
use crate::{Action, App, AppEvent};

/// 一个前端控制器：拥有 `App`、后台事件通道和「需要重画」标记。
pub struct Controller {
    pub app: App,
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
    dirty: bool,
}

impl Controller {
    pub fn new(app: App) -> Self {
        let (tx, rx) = mpsc::channel();
        Controller {
            app,
            tx,
            rx,
            // 第一帧总要画
            dirty: true,
        }
    }

    /// 后台线程用的发送端（测试里也用它注入事件）。
    pub fn sender(&self) -> mpsc::Sender<AppEvent> {
        self.tx.clone()
    }

    /// 前端改过状态（移动光标之类）后调用，下一帧会重画。
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// 收后台结果 + 推进 AI。返回这一帧是否需要重画（并清掉标记）。
    pub fn tick(&mut self) -> bool {
        while let Ok(ev) = self.rx.try_recv() {
            self.app.apply(ev);
            self.dirty = true;
        }
        self.pump();
        std::mem::take(&mut self.dirty)
    }

    /// 人机模式推进一步：轮到 AI 就落子，否则保证当前局面有分析。
    fn pump(&mut self) {
        match self.app.next_action() {
            Action::None => {}
            Action::Analyze => self.analyze(),
            Action::AiMove(mv) => match self.app.play_move(mv) {
                Ok(()) => {
                    self.app.status = strings(self.app.lang).ai_played.to_string();
                    self.analyze();
                }
                Err(e) => {
                    self.app.fail_ai_move(e);
                    self.dirty = true;
                }
            },
        }
    }

    /// 分析当前局面（后台线程）。
    pub fn analyze(&mut self) {
        self.dirty = true;
        if self.app.busy {
            self.app.status = strings(self.app.lang).busy.to_string();
            return;
        }
        let pos = self.app.pos();
        let generation = self.app.generation();
        self.app.busy = true;
        self.app.status = fill(
            strings(self.app.lang).analyzing,
            self.app.config.engine.visits,
        );
        let game = self.app.snapshot();
        let engine = self.app.engine.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = engine.analyze_position(&game);
            let _ = tx.send(AppEvent::Analysis {
                generation,
                pos,
                result,
            });
        });
    }

    /// 全盘复盘（后台线程）。
    pub fn review(&mut self) {
        self.dirty = true;
        if self.app.busy {
            self.app.status = strings(self.app.lang).busy.to_string();
            return;
        }
        self.app.busy = true;
        self.app.status = strings(self.app.lang).reviewing.to_string();
        let game = match self.app.full_game() {
            Ok(g) => g,
            Err(e) => {
                self.app.busy = false;
                self.app.status = e;
                return;
            }
        };
        let engine = self.app.engine.clone();
        let english = !self.app.lang.is_zh();
        let generation = self.app.generation();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = engine
                .analyze_game(&game, |_, _| {})
                .map(|evals| review::build(english, &game, &evals, 5.0));
            let _ = tx.send(AppEvent::Review { generation, result });
        });
    }

    /// 在光标处落子；成功后自动分析新局面。
    pub fn play_at_cursor(&mut self) -> Result<(), String> {
        self.dirty = true;
        match self.app.play_at_cursor() {
            Ok(()) => {
                self.analyze();
                Ok(())
            }
            Err(e) => {
                self.app.status = e.clone();
                Err(e)
            }
        }
    }

    /// 开一盘人机对局，并立刻分析空盘。
    pub fn start_play(&mut self) {
        self.dirty = true;
        self.app.start_play();
        self.analyze();
    }

    /// 开关 AI 应手，并按新状态推进一手。
    pub fn toggle_ai(&mut self) {
        self.dirty = true;
        self.app.toggle_ai();
        self.pump();
    }

    /// 调整分析强度（visits），下一次分析生效。
    pub fn set_visits(&mut self, visits: u32) {
        self.dirty = true;
        self.app.set_visits(visits);
    }

    pub fn adjust_visits(&mut self, delta: i32) {
        let visits = (self.app.config.engine.visits as i32 + delta).max(10) as u32;
        self.set_visits(visits);
    }

    /// 开关拟人网（humanv0）。
    pub fn toggle_human(&mut self) {
        self.dirty = true;
        self.app.toggle_human();
        self.pump();
    }

    /// 切换拟人段位。
    pub fn cycle_human_rank(&mut self, delta: i32) {
        self.dirty = true;
        self.app.cycle_human_rank(delta);
        self.pump();
    }

    pub fn set_human_rank(&mut self, idx: usize) {
        self.dirty = true;
        self.app.set_human_rank(idx);
        self.pump();
    }

    /// 用当前分析的第一候选 PV 进入变化预览。
    pub fn start_preview(&mut self) {
        self.dirty = true;
        self.app.start_preview();
    }

    pub fn preview_next(&mut self) {
        self.dirty = true;
        self.app.preview_next();
    }

    pub fn preview_prev(&mut self) {
        self.dirty = true;
        self.app.preview_prev();
    }

    pub fn exit_preview(&mut self) {
        self.dirty = true;
        self.app.exit_preview();
    }

    pub fn commit_preview(&mut self) {
        self.dirty = true;
        self.app.commit_preview();
    }

    /// 打开棋谱列表里选中的那盘。
    pub fn open_selected_game(&mut self) {
        self.dirty = true;
        self.app.open_selected_game();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use crate::storage::GameStore;
    use crate::AppConfig;
    use engine::{Candidate, TurnEval};
    use goban::{Color, Move, Point};

    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_store() -> GameStore {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        GameStore::at(std::env::temp_dir().join(format!(
            "weiqi-ctrl-test-{}-{}",
            std::process::id(),
            n
        )))
    }

    /// 引擎路径故意指向不存在的位置：分析线程会立刻失败，不会真的起 KataGo。
    fn offline_app() -> App {
        let mut config = AppConfig::default();
        config.engine.katago = "/nonexistent/katago".into();
        config.engine.model = "/nonexistent/model.bin.gz".into();
        config.engine.config = "/nonexistent/analysis.cfg".into();
        let mut app = App::new(9, 7.5, config);
        app.store = test_store();
        app.lang = Lang::Zh;
        app
    }

    fn eval(pos: usize, to_move: Color, wr: f32, best: Move) -> TurnEval {
        TurnEval {
            turn: pos as u16,
            to_move,
            winrate_black: wr,
            score_black: 0.0,
            candidates: vec![Candidate {
                mv: best,
                winrate_black: wr,
                score_black: 0.0,
                pv: vec![best],
                visits: 100,
            }],
            human_policy: None,
        }
    }

    #[test]
    fn tick_applies_background_events_and_clears_dirty() {
        let mut ctrl = Controller::new(offline_app());
        assert!(ctrl.tick(), "第一帧应当要求重画");
        assert!(!ctrl.tick(), "没有变化时不需要重画");

        let tx = ctrl.sender();
        tx.send(AppEvent::Analysis {
            generation: ctrl.app.generation(),
            pos: 0,
            result: Ok(eval(0, Color::Black, 0.5, Move::Play(Point::new(2, 2)))),
        })
        .unwrap();
        assert!(ctrl.tick(), "收到分析结果后需要重画");
        assert!(ctrl.app.analysis().is_some());
        assert!(!ctrl.app.busy);
    }

    #[test]
    fn play_at_cursor_plays_and_starts_analysis() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;
        ctrl.app.cursor = Point::new(2, 2);

        ctrl.play_at_cursor().unwrap();
        assert_eq!(ctrl.app.total_moves(), 1);
        assert_eq!(ctrl.app.pos(), 1);
        assert!(ctrl.app.busy, "落子后应自动起一次分析");
        assert_eq!(
            ctrl.app.status,
            fill(
                strings(ctrl.app.lang).analyzing,
                ctrl.app.config.engine.visits
            )
        );
    }

    #[test]
    fn tick_lets_the_engine_reply() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;
        ctrl.app.cursor = Point::new(2, 2);
        ctrl.app.play_at_cursor().unwrap(); // 人执黑
        ctrl.app.busy = false; // 假装分析已完成
        ctrl.app
            .evals
            .insert(1, eval(1, Color::White, 0.6, Move::Play(Point::new(4, 4))));

        assert!(ctrl.tick(), "AI 应手后需要重画");
        assert_eq!(ctrl.app.total_moves(), 2);
        assert_eq!(ctrl.app.game().moves()[1], Move::Play(Point::new(4, 4)));
        assert!(ctrl.app.busy, "AI 落子后应继续分析");
    }

    #[test]
    fn busy_analysis_reports_instead_of_spawning_twice() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.busy = true;
        ctrl.analyze();
        assert_eq!(ctrl.app.status, strings(ctrl.app.lang).busy);
        assert!(ctrl.app.busy);
    }

    #[test]
    fn review_requires_a_complete_game() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;
        ctrl.app.cursor = Point::new(2, 2);
        ctrl.app.play_at_cursor().unwrap();
        ctrl.app.busy = false;

        ctrl.review();
        assert!(ctrl.app.busy);
        assert_eq!(ctrl.app.status, strings(ctrl.app.lang).reviewing);
    }
}
