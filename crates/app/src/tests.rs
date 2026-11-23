//! app 单元测试：导航、落子、规则、分析事件、存储交互。
//!
//! 这里可以用 crate 私有字段（`mod tests;` 是 crate 根的子模块）。

use super::*;
use engine::Candidate;
use goban::{KoRule, Move};

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

#[test]
fn sgf_rules_select_the_full_rule_set() {
    let jp = sgf::parse("(;SZ[9]RU[Japanese];B[cc];W[gg])").unwrap();
    let app = App::from_sgf(&jp, AppConfig::default()).unwrap();
    assert_eq!(app.game().rules(), RuleSet::JAPANESE);
    assert_eq!(app.game().ko_rule(), KoRule::Simple);

    let cn = sgf::parse("(;SZ[9]RU[Chinese];B[cc];W[gg])").unwrap();
    let app = App::from_sgf(&cn, AppConfig::default()).unwrap();
    assert_eq!(app.game().rules(), RuleSet::CHINESE);

    // AGA 过去被折叠成 Korean，现在必须保住自己的名字和计分方式
    let aga = sgf::parse("(;SZ[9]RU[AGA];B[cc];W[gg])").unwrap();
    let app = App::from_sgf(&aga, AppConfig::default()).unwrap();
    assert_eq!(app.game().rules(), RuleSet::AGA);
    assert_eq!(app.game().rules().scoring, goban::Scoring::Area);

    // 没有 RU 时保持默认
    let none = sgf::parse("(;SZ[9];B[cc];W[gg])").unwrap();
    let app = App::from_sgf(&none, AppConfig::default()).unwrap();
    assert_eq!(app.game().rules(), RuleSet::default());
    assert_eq!(app.game().ko_rule(), KoRule::default());
}

#[test]
fn unknown_rules_warn_and_fall_back_visibly() {
    let sg = sgf::parse("(;SZ[9]RU[My House Rules];B[cc];W[gg])").unwrap();
    let app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    assert_eq!(app.game().rules(), RuleSet::CHINESE, "未知规则回退默认");
    assert!(
        app.status.contains("My House Rules"),
        "回退必须让用户看见：{}",
        app.status
    );

    // 支持 AGA 后，写回不能退化成 Korean
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    let sg = sgf::parse("(;SZ[9]RU[AGA];B[cc];W[gg])").unwrap();
    app.open_sgf_file(std::env::temp_dir().join("weiqi-aga.sgf"), &sg)
        .unwrap();
    let saved = app.store.read(app.current_file.as_ref().unwrap()).unwrap();
    assert!(saved.contains("RU[AGA]"), "写回丢了规则名：{saved}");
    let back = sgf::parse(&saved).unwrap();
    assert_eq!(
        sgf::recognized_rule_set(back.rules.as_deref().unwrap()),
        Some(RuleSet::AGA)
    );
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
}

#[test]
fn list_open_keeps_warnings_in_status() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    let path = app.store.create().unwrap();
    std::fs::write(&path, "(;SZ[9]KM[abc];B[cc])").unwrap();
    app.refresh_games();
    assert_eq!(app.games.len(), 1);
    app.game_cursor = 0;
    app.open_selected_game();
    assert!(app.status.contains("KM"), "{}", app.status);
    assert!(app.status.contains('⚠'), "{}", app.status);

    // 开新局要清掉上一盘的 warning
    app.start_play();
    assert!(app.warnings.is_empty());
}

#[test]
fn sgf_warnings_show_in_status() {
    let sg = sgf::parse("(;SZ[9]KM[abc];B[cc])").unwrap();
    let app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    assert!(app.status.contains("KM"), "{}", app.status);
}

#[test]
fn start_play_on_sets_board_size() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.start_play_on(19);
    assert_eq!(app.size, 19);
    assert_eq!(app.game().size(), 19);
    assert_eq!(app.total_moves(), 0);
}

#[test]
fn setup_game_replays_with_declared_colors() {
    // AW 摆白子、黑先：cc 必须落成黑子，gg 必须落成白子
    let sg = sgf::parse("(;SZ[9]AW[dd];B[cc];W[gg])").unwrap();
    let app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    let board = &app.game().position().board;
    assert_eq!(board.at(Point::new(2, 2)), Some(Color::Black), "cc 应是黑");
    assert_eq!(board.at(Point::new(6, 6)), Some(Color::White), "gg 应是白");
}

/// 生成 n 手的合法棋谱（19 路，从左上角开始铺，不会互相提子）。
fn long_game(n: usize) -> App {
    let letters = b"abcdefghijklmnopqrs";
    let mut src = String::from("(;SZ[19]KM[7.5]");
    for i in 0..n {
        let color = if i.is_multiple_of(2) { 'B' } else { 'W' };
        let x = i % 19;
        let y = i / 19;
        src.push_str(&format!(
            ";{color}[{}{}]",
            letters[x] as char, letters[y] as char
        ));
    }
    src.push(')');
    let sg = sgf::parse(&src).unwrap();
    App::from_sgf(&sg, AppConfig::default()).unwrap()
}

#[test]
fn branching_from_an_old_position_drops_later_evals() {
    let mut app = long_game(25);
    for k in 20..=22 {
        app.evals.insert(k, eval(k, Color::Black, 0.5, Move::Pass));
    }
    let gen = app.generation();

    app.jump_to_turn(20);
    app.cursor = Point::new(10, 5);
    app.play_at_cursor().unwrap();

    assert_ne!(app.generation(), gen, "改走分支必须算新版本");
    assert!(app.evals.contains_key(&20), "分支点的分析仍然有效");
    assert!(
        !app.evals.contains_key(&21) && !app.evals.contains_key(&22),
        "旧分支后续的分析必须清掉：{:?}",
        app.evals.keys().collect::<Vec<_>>()
    );
}

#[test]
fn branching_clears_the_old_failed_pos() {
    let mut app = long_game(25);
    app.jump_to_turn(20);
    app.failed_pos = Some(21);
    app.cursor = Point::new(10, 5);
    app.play_at_cursor().unwrap();
    assert_eq!(
        app.last_failed_pos(),
        None,
        "旧分支的失败标记不能挡住新局面"
    );
}

#[test]
fn stale_events_from_the_old_branch_are_ignored() {
    let mut app = long_game(25);
    let old_gen = app.generation();
    app.jump_to_turn(20);
    app.cursor = Point::new(10, 5);
    app.play_at_cursor().unwrap();

    // 模拟“新分析正在跑”
    app.busy = true;
    app.apply(AppEvent::Analysis {
        generation: old_gen,
        pos: 21,
        result: Ok(eval(21, Color::Black, 0.9, Move::Pass)),
    });
    app.apply(AppEvent::Review {
        generation: old_gen,
        revision: app.review_revision(),
        result: Err("旧分支的复盘".to_string()),
    });

    assert!(app.analysis().is_none(), "旧分支的结果不能写进缓存");
    assert!(app.busy, "旧结果不能复位新任务的 busy");
    assert!(app.review.is_none());
}

#[test]
fn appending_a_move_keeps_the_previous_analysis() {
    let mut app = long_game(5);
    app.evals.insert(5, eval(5, Color::Black, 0.5, Move::Pass));
    let gen = app.generation();

    app.cursor = Point::new(10, 5);
    app.play_at_cursor().unwrap();

    assert_eq!(app.generation(), gen, "在末尾续下不是换线");
    assert!(
        app.evals.contains_key(&5),
        "上一手的分析回来仍应有效（最近一手点评要用）"
    );
}

#[test]
fn illegal_move_leaves_everything_untouched() {
    let mut app = long_game(25);
    app.jump_to_turn(20);
    app.evals
        .insert(20, eval(20, Color::Black, 0.5, Move::Pass));
    app.failed_pos = Some(20);
    let before = (
        app.generation(),
        app.pos(),
        app.total_moves(),
        app.evals.len(),
        app.last_failed_pos(),
        app.busy,
    );

    // 第 20 手之前 (0,0) 上已经有子
    app.cursor = Point::new(0, 0);
    assert!(app.play_at_cursor().is_err());

    assert_eq!(
        (
            app.generation(),
            app.pos(),
            app.total_moves(),
            app.evals.len(),
            app.last_failed_pos(),
            app.busy,
        ),
        before
    );
}

fn dummy_review() -> Review {
    Review {
        english: false,
        size: 9,
        header: "旧的复盘".to_string(),
        deltas: Vec::new(),
        phases: Vec::new(),
        blunders: Vec::new(),
        summary: Vec::new(),
        details: Vec::new(),
    }
}

#[test]
fn appending_a_move_invalidates_review_but_keeps_analysis_generation() {
    let mut app = long_game(5);
    app.evals.insert(5, eval(5, Color::Black, 0.5, Move::Pass));
    let gen = app.generation();
    let rev = app.review_revision();

    app.cursor = Point::new(10, 5);
    app.play_at_cursor().unwrap();

    assert_eq!(app.generation(), gen, "追加落子不换分析版本");
    assert_ne!(app.review_revision(), rev, "但整盘复盘必须失效");
    assert!(app.evals.contains_key(&5), "上一手分析仍然有效");
}

#[test]
fn stale_review_after_append_is_dropped() {
    let mut app = long_game(5);
    let (gen, rev) = (app.generation(), app.review_revision());
    app.cursor = Point::new(10, 5);
    app.play_at_cursor().unwrap();

    app.busy = true;
    app.apply(AppEvent::Review {
        generation: gen,
        revision: rev,
        result: Ok(dummy_review()),
    });
    assert!(app.review.is_none(), "旧复盘不能显示");
    assert!(app.busy, "旧复盘不改 busy（由 Controller 统一收尾）");

    // 失败事件同样不能覆盖状态
    let status = app.status.clone();
    app.apply(AppEvent::Review {
        generation: gen,
        revision: rev,
        result: Err("旧错误".to_string()),
    });
    assert_eq!(app.status, status);
}

#[test]
fn branching_and_new_game_also_invalidate_review() {
    let mut app = long_game(25);
    app.jump_to_turn(20);
    let rev = app.review_revision();
    app.cursor = Point::new(10, 5);
    app.play_at_cursor().unwrap();
    assert_ne!(app.review_revision(), rev);

    let rev = app.review_revision();
    app.start_play();
    assert_ne!(app.review_revision(), rev);
}

#[test]
fn illegal_move_does_not_cancel_the_active_preview() {
    let mut app = long_game(10);
    app.evals.insert(
        4,
        eval(4, Color::White, 0.5, Move::Play(Point::new(10, 10))),
    );
    app.jump_to_turn(4);
    app.start_preview();
    assert!(app.preview_active());

    // (0,0) 上已经有子
    app.cursor = Point::new(0, 0);
    assert!(app.play_at_cursor().is_err());
    assert!(app.preview_active(), "非法落子不应取消变化预览");
}

#[test]
fn committing_a_preview_drops_evals_after_its_base() {
    let mut app = long_game(10);
    for k in 4..=7 {
        app.evals.insert(k, eval(k, Color::White, 0.5, Move::Pass));
    }
    // 让 start_preview 有候选 PV 可走
    app.evals.insert(
        4,
        eval(4, Color::White, 0.5, Move::Play(Point::new(10, 10))),
    );
    let gen = app.generation();

    app.jump_to_turn(4);
    app.start_preview();
    app.preview_next();
    app.commit_preview();

    assert_ne!(app.generation(), gen, "采用变化也算新版本");
    assert!(app.evals.contains_key(&4), "base 位置的分析仍然有效");
    assert!(
        !app.evals.contains_key(&5) && !app.evals.contains_key(&7),
        "base 之后的分析必须清掉：{:?}",
        app.evals.keys().collect::<Vec<_>>()
    );
    assert_eq!(app.last_failed_pos(), None);
}

#[test]
fn white_first_sgf_keeps_colors_through_rebuild_and_full_game() {
    // 白先、没有摆子：from_sgf 走 rebuild，最容易把先手丢掉
    let sg = sgf::parse("(;SZ[9]PL[W];W[cc];B[gg])").unwrap();
    let app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    assert_eq!(
        app.game().color_of_move(1),
        Color::White,
        "from_sgf 重建后丢了白先"
    );
    let board = &app.game().position().board;
    assert_eq!(board.at(Point::new(2, 2)), Some(Color::White), "cc 应是白");
    assert_eq!(board.at(Point::new(6, 6)), Some(Color::Black), "gg 应是黑");

    // full_game 是复盘用的整盘快照，也必须保持同一先手
    let full = app.full_game().unwrap();
    assert_eq!(full.color_of_move(1), Color::White, "full_game 丢了白先");
    assert_eq!(
        full.position().board.at(Point::new(2, 2)),
        Some(Color::White)
    );
    assert_eq!(
        full.position().board.at(Point::new(6, 6)),
        Some(Color::Black)
    );

    // 前后跳转（会再 rebuild 一次）不能改变先手语义
    let mut app = app;
    app.goto_start();
    assert_eq!(app.game().color_of_move(1), Color::White);
    app.goto_end();
    assert_eq!(app.game().color_of_move(1), Color::White);
}

#[test]
fn explicit_black_first_with_setup_survives_rebuild() {
    // AB 摆黑子通常意味着让子（白先），但 PL[B] 显式写了黑先就得听文件的
    let sg = sgf::parse("(;SZ[9]PL[B]AB[dd];B[cc];W[gg])").unwrap();
    let app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    assert_eq!(
        app.game().color_of_move(1),
        Color::Black,
        "PL[B] 被摆子盖掉"
    );
    let board = &app.game().position().board;
    assert_eq!(board.at(Point::new(2, 2)), Some(Color::Black), "cc 应是黑");
    assert_eq!(board.at(Point::new(6, 6)), Some(Color::White), "gg 应是白");
}

#[test]
fn autosave_switches_to_a_conflict_copy_when_the_file_changed() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.start_play();
    let file = app.current_file.clone().expect("新局应有文件");
    // 空局不落盘；先落一手拿到指纹
    app.play_move(Move::Play(Point::new(0, 0))).unwrap();
    assert!(app.file_stamp.is_some(), "第一手保存后要记住指纹");

    // 另一个进程改了这个文件；我们手上的 mtime 已经过期
    std::fs::write(&file, "(;SZ[9]KM[0.5];B[dd])").unwrap();
    app.file_stamp = Some(super::storage::FileStamp {
        mtime: std::time::UNIX_EPOCH,
        len: 0,
        hash: 0,
    });

    app.play_move(Move::Play(Point::new(3, 3))).unwrap();

    let current = app.current_file.clone().unwrap();
    assert_ne!(current, file, "不能覆盖外部修改过的文件");
    assert!(
        current
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains("conflict")),
        "{}",
        current.display()
    );
    assert!(app.status.contains("conflict"), "{}", app.status);
    // 外部内容原样保留
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "(;SZ[9]KM[0.5];B[dd])"
    );
    // 下一次保存走新副本，不再反复产生冲突
    app.file_stamp = super::storage::FileStamp::of(&current).unwrap();
    app.play_move(Move::Play(Point::new(4, 4))).unwrap();
    assert_eq!(app.current_file, Some(current));
}

#[test]
fn new_game_does_not_inherit_a_stale_mtime() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.start_play();
    let first = app.current_file.clone().unwrap();
    assert!(!first.exists(), "空局不落盘");
    assert_eq!(app.file_stamp, None, "新局还没有指纹");

    // 第一手写出文件、建立指纹
    app.play_move(Move::Play(Point::new(0, 0))).unwrap();
    assert_eq!(
        app.file_stamp,
        super::storage::FileStamp::of(&first).unwrap()
    );

    // 再开新局：不能沿用上一盘的文件名/指纹
    app.start_play();
    let second = app.current_file.clone().unwrap();
    assert_ne!(first, second);
    assert_eq!(app.file_stamp, None, "新局必须清掉上一盘的指纹");
    assert!(!second.exists(), "新局第一手之前不落盘");
}

#[test]
fn open_detects_size_komi_or_rule_changes_between_reads() {
    let mut app = App::new(9, super::DEFAULT_KOMI, AppConfig::default());
    app.store = test_store();
    let path = app.store.dir().join("semantics.sgf");
    let original = "(;SZ[9]KM[7.5];B[cc];W[gg])";
    let sg = sgf::parse(original).unwrap();

    for changed in [
        "(;SZ[13]KM[7.5];B[cc];W[gg])",            // 只改棋盘大小
        "(;SZ[9]KM[0.5];B[cc];W[gg])",             // 只改贴目
        "(;SZ[9]KM[7.5]RU[Japanese];B[cc];W[gg])", // 只改规则（映射到不同 RuleSet）
        // 归一到同一个 RuleSet、但“是否可识别”变了，也必须发现
        "(;SZ[9]KM[7.5]RU[New Zealand];B[cc];W[gg])",
    ] {
        std::fs::write(&path, original).unwrap();
        app.open_sgf_file(path.clone(), &sg).unwrap();
        std::fs::write(&path, changed).unwrap();
        let err = app.open_sgf_file(path.clone(), &sg).unwrap_err();
        assert!(
            err.contains("修改"),
            "{changed} 的语义变化必须被发现：{err}"
        );
    }
}

#[test]
fn failed_open_keeps_current_state() {
    let mut app = demo();
    let before_moves = app.total_moves();
    let before_game = app.game().clone();
    let before_size = app.size;
    let before_file = app.current_file.clone();

    // 白棋下在已有黑子上：解析能过，重放必失败
    let bad = sgf::parse("(;SZ[19]KM[7.5];B[cc];W[cc])").unwrap();
    let err = app
        .open_sgf_file(std::env::temp_dir().join("weiqi-bad.sgf"), &bad)
        .unwrap_err();
    assert!(err.contains("重放失败"), "{err}");

    assert_eq!(app.size, before_size);
    assert_eq!(app.total_moves(), before_moves);
    assert_eq!(app.game().position(), before_game.position());
    assert_eq!(app.game().moves(), before_game.moves());
    assert_eq!(app.current_file, before_file);
}

#[test]
fn opening_sgf_file_updates_ko_rule() {
    // GUI / TUI 切谱走 open_sgf_file，不能沿用上一盘的规则
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();

    let jp = sgf::parse("(;SZ[9]RU[Japanese];B[cc];W[gg])").unwrap();
    let dir = std::env::temp_dir();
    app.open_sgf_file(dir.join("weiqi-ko-jp.sgf"), &jp).unwrap();
    assert_eq!(app.game().rules(), RuleSet::JAPANESE);

    let cn = sgf::parse("(;SZ[9]RU[Chinese];B[cc];W[gg])").unwrap();
    app.open_sgf_file(dir.join("weiqi-ko-cn.sgf"), &cn).unwrap();
    assert_eq!(app.game().rules(), RuleSet::CHINESE);

    // 新开人机局要回到默认规则，不继承上一盘
    app.start_play();
    assert_eq!(app.game().rules(), RuleSet::default());
}

#[test]
fn opening_a_stored_game_enters_study_mode() {
    // 从人机对局切到棋谱：打开后不能继续自动分析/应手（只清 vs_ai 不够）
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    start(&mut app);
    app.ai_enabled = true;
    app.human_enabled = true; // 拟人偏好本身保留，用户想再对局时重新开 AI
    assert!(matches!(
        app.next_action(),
        Action::AiMove(_) | Action::Analyze
    ));

    let sg = sgf::parse("(;SZ[9]KM[7.5];B[cc];W[gg])").unwrap();
    app.open_sgf_file(std::env::temp_dir().join("weiqi-study-mode.sgf"), &sg)
        .unwrap();

    assert!(!app.ai_enabled, "打开棋谱应进入纯研究模式");
    assert!(app.human_enabled, "拟人偏好保留，等用户重新开 AI");
    assert_eq!(app.next_action(), Action::None, "不会自动分析或应手");

    // 用户显式重新开启 AI 后仍然可用
    app.toggle_ai();
    assert!(app.ai_enabled);
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
fn new_game_is_not_written_until_the_first_move() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.start_play();
    let path = app.current_file.clone().expect("新局应预留文件名");
    assert!(!path.exists(), "空局不应落盘");
    assert!(app.games.is_empty(), "库里不应出现空棋谱");

    app.play_at_cursor().unwrap();
    assert!(path.exists(), "第一手后要自动保存");
    let text = app.store.read(&path).unwrap();
    assert!(
        sgf::parse(&text).is_ok(),
        "写出的必须是可解析的 SGF：{text}"
    );
    assert_eq!(app.games.len(), 1, "保存后棋谱列表要立刻更新");
}

#[test]
fn new_game_resets_komi_from_the_previous_sgf() {
    let sg = sgf::parse("(;SZ[9]KM[0.5]RU[Japanese];B[cc];W[gg])").unwrap();
    let mut app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    app.store = test_store();
    assert_eq!(app.game().komi(), 0.5);

    app.start_play();
    assert_eq!(app.game().rules(), RuleSet::default(), "规则回到默认");
    assert_eq!(app.komi, DEFAULT_KOMI, "新局不能继承上一盘的 0.5 贴目");
    assert_eq!(app.game().komi(), DEFAULT_KOMI);
}

#[test]
fn open_latest_skips_broken_files_and_reports_when_none_open() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.lang = Lang::Zh;
    let dir = app.store.dir().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    // 最新的条目损坏，下一盘有效：应跳过坏的
    std::fs::write(dir.join("game-9.sgf"), "not an sgf").unwrap();
    std::fs::write(dir.join("game-2.sgf"), "(;SZ[9]KM[7.5];B[cc])").unwrap();

    assert!(app.open_latest_game(), "应跳过损坏的最新文件");
    assert_eq!(app.total_moves(), 1);

    std::fs::remove_file(dir.join("game-2.sgf")).unwrap();
    assert!(!app.open_latest_game(), "全都打不开时要返回失败");
    assert!(
        app.status.contains("打不开"),
        "状态栏要说明原因：{}",
        app.status
    );
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
fn switching_sides_lets_the_engine_open() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.lang = Lang::Zh;
    app.start_play();
    let gen = app.generation();

    // 你执白：空盘轮到黑，AI 先分析
    app.set_human_color(Color::White);
    assert_eq!(app.human, Color::White);
    assert!(app.generation() > gen, "换边要让旧事件失效");
    assert_eq!(app.next_action(), Action::Analyze);
    assert!(app.status.contains('白'), "{}", app.status);

    // 空盘已有分析时，AI 直接落第一子，不需要重新分析
    let best = Move::Play(Point::new(2, 2));
    app.evals.insert(0, eval(0, Color::Black, 0.5, best));
    assert!(
        matches!(app.next_action(), Action::AiMove(_)),
        "有分析时应当直接应手，而不是再分析一次"
    );

    // 换回执黑：轮到你，已有分析就不再重复分析
    app.swap_sides();
    assert_eq!(app.human, Color::Black);
    assert_eq!(app.next_action(), Action::None);
}

#[test]
fn new_game_keeps_the_chosen_side() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.set_human_color(Color::White);
    app.start_play();
    assert_eq!(app.human, Color::White, "开新局不应把选边重置成黑");
    assert_eq!(app.next_action(), Action::Analyze, "人执白时 AI 先走");
}

#[test]
fn changing_sides_clears_stale_comment_and_failed_pos_but_keeps_evals() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = test_store();
    app.start_play();
    app.failed_pos = Some(3);
    app.last_comment = Some("旧点评".to_string());
    app.evals
        .insert(1, eval(1, Color::White, 0.4, Move::Play(Point::new(1, 1))));

    let gen = app.generation();
    app.set_human_color(Color::White);
    assert!(app.generation() > gen);
    assert_eq!(app.failed_pos, None);
    assert!(app.last_comment.is_none());
    assert!(app.evals.contains_key(&1), "分析是黑方视角，换边后仍然有效");

    // 同一个颜色再设一次是空操作
    let gen = app.generation();
    app.set_human_color(Color::White);
    assert_eq!(app.generation(), gen);
}

#[test]
fn human_cannot_move_on_ai_turn() {
    let mut app = demo();
    start(&mut app);
    app.evals
        .insert(0, eval(0, Color::Black, 1.0, Move::Play(Point::new(2, 2))));
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
    app.evals
        .insert(0, eval(0, Color::Black, 0.5, Move::Play(Point::new(2, 2))));
    assert_eq!(app.next_action(), Action::None);
    // 人落子后轮到 AI，有评估 → 直接要求 AI 落子
    app.cursor = Point::new(2, 2);
    app.play_at_cursor().unwrap();
    app.evals
        .insert(1, eval(1, Color::White, 0.55, Move::Play(Point::new(4, 4))));
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
fn human_policy_length_must_match_the_board_exactly() {
    let sg = sgf::parse("(;SZ[9];B[cc];W[gg];B[dd])").unwrap();
    let mut app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    app.ai_enabled = true;
    app.human = Color::Black; // 轮白，AI
    app.human_enabled = true;
    let fallback = Move::Play(Point::new(0, 0));

    // 恰好缺 Pass 槽（9*9=81）→ 整份策略作废，回退普通候选
    let mut policy = vec![0.0f32; 9 * 9];
    policy[4 * 9 + 4] = 1.0; // E5
    let mut e = eval(3, Color::White, 0.5, fallback);
    e.human_policy = Some(policy);
    app.evals.insert(3, e);
    assert_eq!(app.ai_choice(), Some(fallback));

    // 多一个元素同样作废
    let mut policy = vec![0.0f32; 9 * 9 + 2];
    policy[4 * 9 + 4] = 1.0;
    let mut e = eval(3, Color::White, 0.5, fallback);
    e.human_policy = Some(policy);
    app.evals.insert(3, e);
    assert_eq!(app.ai_choice(), Some(fallback));

    // 完整 82 位才按策略走
    let mut policy = vec![0.0f32; 9 * 9 + 1];
    policy[4 * 9 + 4] = 1.0;
    let mut e = eval(3, Color::White, 0.5, fallback);
    e.human_policy = Some(policy);
    app.evals.insert(3, e);
    assert_eq!(app.ai_choice(), Some(Move::Play(Point::new(4, 4))));
}

#[test]
fn swapping_sides_during_preview_leaves_the_main_line_alone() {
    let mut app = App::new(9, 7.5, AppConfig::default());
    start(&mut app);
    let best = Move::Play(Point::new(2, 2));
    let mut e = eval(0, Color::Black, 0.5, best);
    e.candidates[0].pv = vec![best, Move::Play(Point::new(3, 3))];
    app.evals.insert(0, e);

    app.start_preview();
    assert!(app.preview_active(), "{}", app.status);

    app.swap_sides();
    assert_eq!(app.human, Color::White);
    assert_eq!(app.next_action(), Action::None, "预览期间不能自动应手");
    assert!(app.preview_active(), "换边不应退出预览");
    assert_eq!(app.total_moves(), 0);
    assert_eq!(app.pos(), 0);
    assert_eq!(app.game().moves().len(), 0, "主线一手都不能多");

    app.exit_preview();
    assert!(
        matches!(app.next_action(), Action::AiMove(_)),
        "退出预览后 AI 按新执子方应手"
    );
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
fn replays_sgf_with_two_passes() {
    let sg = sgf::parse("(;SZ[9];B[cc];W[gg];B[];W[])").unwrap();
    let app = App::from_sgf(&sg, AppConfig::default()).unwrap();
    assert_eq!(app.total_moves(), 4);
    assert_eq!(app.pos(), 4);
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
    let src = "(;SZ[9];B[aa])";
    std::fs::write(&path, src).unwrap();
    let sg = sgf::parse(src).unwrap();
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
