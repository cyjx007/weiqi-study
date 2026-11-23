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
    /// 已有一个后台线程（分析或复盘）在跑；同一时间只允许一个
    working: bool,
    /// 工作期间局面又变了：等它结束后补一次当前局面的分析
    pending: bool,
    /// 测试用：实际 spawn 了几次
    #[cfg(test)]
    spawns: usize,
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
            working: false,
            pending: false,
            #[cfg(test)]
            spawns: 0,
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
        let mut finished = false;
        while let Ok(ev) = self.rx.try_recv() {
            self.app.apply(ev);
            self.working = false;
            finished = true;
            self.dirty = true;
        }
        // 工作期间用户又落了子/换了局面：合并成一次补分析，
        // 而不是每落一手就新起一个线程排队堵在引擎锁后面。
        if finished {
            if self.pending {
                self.pending = false;
                self.spawn_analysis();
            } else {
                // 物理任务结束、也没有排队的分析：即使结果是过期被丢弃，也要放开 busy，
                // 否则界面会永远停在“分析中”。
                self.app.busy = false;
            }
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

    /// 分析当前局面（后台线程）。同一时间只保留一个后台任务；
    /// 期间局面变了就记成 pending，等当前任务结束后合并成一次补分析。
    pub fn analyze(&mut self) {
        self.dirty = true;
        if self.working {
            self.pending = true;
            self.app.busy = true;
            return;
        }
        if self.app.busy {
            self.app.status = strings(self.app.lang).busy.to_string();
            return;
        }
        self.spawn_analysis();
    }

    fn spawn_analysis(&mut self) {
        self.pending = false;
        self.working = true;
        #[cfg(test)]
        {
            self.spawns += 1;
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
        if self.app.busy || self.working {
            self.app.status = strings(self.app.lang).busy.to_string();
            return;
        }
        self.working = true;
        self.app.busy = true;
        self.app.status = strings(self.app.lang).reviewing.to_string();
        let game = match self.app.full_game() {
            Ok(g) => g,
            Err(e) => {
                self.app.busy = false;
                self.working = false;
                self.app.status = e;
                return;
            }
        };
        let engine = self.app.engine.clone();
        let english = !self.app.lang.is_zh();
        let generation = self.app.generation();
        let revision = self.app.review_revision();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = engine
                .analyze_game(&game, |_, _| {})
                .map(|evals| review::build(english, &game, &evals, 5.0));
            let _ = tx.send(AppEvent::Review {
                generation,
                revision,
                result,
            });
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

    /// 指定棋盘大小开新局。
    pub fn start_play_on(&mut self, size: u8) {
        self.dirty = true;
        self.app.start_play_on(size);
        self.analyze();
    }

    /// 开关 AI 应手，并按新状态推进一手。
    pub fn toggle_ai(&mut self) {
        self.dirty = true;
        self.app.toggle_ai();
        self.pump();
    }

    /// 选择人机对局里你执哪一方（GUI 下拉框用）；换边后立刻推进 AI 的一手。
    pub fn set_human_color(&mut self, color: goban::Color) {
        self.dirty = true;
        self.app.set_human_color(color);
        self.pump();
    }

    /// 交换人机双方的执子颜色（TUI 快捷键用）。
    pub fn swap_sides(&mut self) {
        self.dirty = true;
        self.app.swap_sides();
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
    use engine::{Candidate, EngineConfig, TurnEval};
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
    fn stale_review_releases_busy_without_touching_review() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;
        // 模拟：Review 物理任务还在跑
        ctrl.working = true;
        ctrl.app.busy = true;
        // 隔离：不让收尾后的 pump() 自动补分析，否则 busy 会因“新任务”变回 true
        ctrl.app.ai_enabled = false;
        let (gen, rev) = (ctrl.app.generation(), ctrl.app.review_revision());

        ctrl.app.play_move(Move::Play(Point::new(2, 2))).unwrap();
        ctrl.sender()
            .send(AppEvent::Review {
                generation: gen,
                revision: rev,
                result: Err("旧复盘".to_string()),
            })
            .unwrap();
        ctrl.tick();

        assert!(ctrl.app.review.is_none());
        assert!(!ctrl.app.busy, "物理任务结束后不能一直卡在 busy");
    }

    #[test]
    fn stale_review_with_pending_analysis_spawns_exactly_once() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;
        ctrl.working = true;
        ctrl.pending = true;
        ctrl.app.busy = true;
        let (gen, rev) = (ctrl.app.generation(), ctrl.app.review_revision());
        ctrl.app.play_move(Move::Play(Point::new(2, 2))).unwrap();

        ctrl.sender()
            .send(AppEvent::Review {
                generation: gen,
                revision: rev,
                result: Err("旧复盘".to_string()),
            })
            .unwrap();
        ctrl.tick();

        assert_eq!(ctrl.spawns, 1, "只补发一次分析");
        assert!(!ctrl.pending);
        assert!(ctrl.app.busy);
    }

    #[test]
    fn rapid_moves_are_coalesced_into_one_pending_analysis() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;

        // 第一手：真正起一个线程
        ctrl.analyze();
        assert_eq!(ctrl.spawns, 1);

        // 线程还没回来时又落了好几手：只记 pending，不再 spawn
        for p in [
            Point::new(2, 2),
            Point::new(4, 4),
            Point::new(6, 6),
            Point::new(8, 8),
        ] {
            ctrl.app.play_move(Move::Play(p)).unwrap();
            ctrl.analyze();
        }
        assert_eq!(ctrl.spawns, 1, "工作期间不能每手都新起线程");
        assert!(ctrl.pending);

        // 旧线程结束（离线引擎立刻失败）后，只补一次
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while ctrl.spawns < 2 && std::time::Instant::now() < deadline {
            ctrl.tick();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(ctrl.spawns, 2, "合并后只补一次分析");
        assert!(!ctrl.pending);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while ctrl.app.busy && std::time::Instant::now() < deadline {
            ctrl.tick();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!ctrl.app.busy);
    }

    #[test]
    fn late_old_branch_result_does_not_reset_the_new_analysis() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;
        for p in [Point::new(2, 2), Point::new(4, 4), Point::new(6, 6)] {
            ctrl.app.play_move(Move::Play(p)).unwrap();
        }

        let old_gen = ctrl.app.generation();
        ctrl.app.jump_to_turn(2);
        ctrl.app.play_move(Move::Play(Point::new(8, 8))).unwrap();
        assert_ne!(ctrl.app.generation(), old_gen);

        // 新分支的分析正在跑，此时旧分支的结果才回来
        ctrl.app.busy = true;
        let tx = ctrl.sender();
        tx.send(AppEvent::Analysis {
            generation: old_gen,
            pos: 3,
            result: Ok(eval(3, Color::Black, 0.9, Move::Pass)),
        })
        .unwrap();
        ctrl.tick();

        assert!(ctrl.app.busy, "旧分支的结果不能复位新任务的 busy");
        assert!(ctrl.app.analysis().is_none(), "旧分支结果不能写进缓存");
    }

    #[test]
    fn engine_failure_resets_busy_and_allows_another_analysis() {
        use std::time::{Duration, Instant};

        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        ctrl.app.busy = false;

        ctrl.analyze();
        assert!(ctrl.app.busy, "发起分析后应处于 busy");

        let deadline = Instant::now() + Duration::from_secs(10);
        while ctrl.app.busy && Instant::now() < deadline {
            ctrl.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!ctrl.app.busy, "引擎失败后 busy 必须复位");
        assert!(ctrl.app.analysis().is_none());
        assert!(!ctrl.app.status.is_empty(), "失败原因要显示出来");

        // 关键：失败之后还能再发起，不是永久卡死
        ctrl.analyze();
        assert!(ctrl.app.busy, "失败后应允许重新分析");

        let deadline = Instant::now() + Duration::from_secs(10);
        while ctrl.app.busy && Instant::now() < deadline {
            ctrl.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!ctrl.app.busy);
    }

    #[test]
    fn unsupported_board_size_error_is_visible_and_does_not_stick_busy() {
        use std::path::PathBuf;
        use std::time::{Duration, Instant};

        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("weiqi-ctrl-board-{}-{}", std::process::id(), n));
        std::fs::create_dir(&dir).unwrap();
        let model = dir.join("model.bin.gz");
        let config = dir.join("analysis.cfg");
        std::fs::write(&model, "fake").unwrap();
        std::fs::write(&config, "fake").unwrap();

        let script = r#"#!/bin/sh
while IFS= read -r line; do
  echo '{"error":"boardXSize 25 is larger than compiled MAX_LEN 19"}'
done
"#;
        let config = AppConfig {
            engine: EngineConfig {
                katago: PathBuf::from("/bin/sh"),
                model,
                config,
                human_model: None,
                visits: 10,
                chunk: 30,
                timeout: Duration::from_secs(10),
                extra_args: vec![
                    "-c".to_string(),
                    script.to_string(),
                    dir.to_string_lossy().into_owned(),
                ],
            },
        };
        let mut app = App::new(25, 7.5, config);
        app.store = test_store();
        app.lang = Lang::Zh;
        app.start_play_on(25);

        let mut ctrl = Controller::new(app);
        ctrl.app.busy = false;
        ctrl.analyze();

        let deadline = Instant::now() + Duration::from_secs(10);
        while ctrl.app.busy && Instant::now() < deadline {
            ctrl.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        ctrl.tick();

        assert!(!ctrl.app.busy, "错误必须让 busy 复位，不能永久卡住");
        assert!(
            ctrl.app.status.contains("boardXSize"),
            "用户要能看到引擎原始错误：{}",
            ctrl.app.status
        );
        assert!(
            ctrl.app.status.contains("MAX_LEN>19"),
            "用户要看到可操作提示：{}",
            ctrl.app.status
        );
        assert_eq!(ctrl.app.last_failed_pos(), Some(0), "同一局面不自动重试");

        // 失败后仍能手动重新发起分析
        ctrl.analyze();
        assert!(ctrl.app.busy, "失败之后应允许重新分析");

        let deadline = Instant::now() + Duration::from_secs(10);
        while ctrl.app.busy && Instant::now() < deadline {
            ctrl.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!ctrl.app.busy);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn swapping_sides_makes_the_engine_open_the_game() {
        let mut ctrl = Controller::new(offline_app());
        ctrl.app.start_play();
        assert!(!ctrl.app.busy);

        ctrl.swap_sides();
        assert_eq!(ctrl.app.human, Color::White);
        assert_eq!(ctrl.spawns, 1, "人执白时空盘第一手应由 AI 接手");
        assert!(ctrl.app.busy, "AI 的第一手在后台分析中");
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
