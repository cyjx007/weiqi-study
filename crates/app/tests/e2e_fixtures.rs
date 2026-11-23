//! 固定 SGF fixtures 的端到端回归：SGF → App → Goban → 写回 → 再读。
//!
//! 这里只走公开 API（`open_sgf_file` 会把外部文件复制进 `games/`），
//! 所以覆盖的是用户真实经过的那条链路，而不是某个 helper。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use app::storage::GameStore;
use app::{App, AppConfig};
use goban::{Color, KoRule, Move, Point, RuleSet, Scoring};
use sgf::SgfGame;

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("weiqi-e2e-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 把 fixture 写到 store 外面，再按用户的方式打开（copy-on-open）。
fn open(src: &str) -> (App, PathBuf) {
    let dir = temp_dir();
    let input = dir.join("input.sgf");
    std::fs::write(&input, src).unwrap();

    let sg = sgf::parse(src).unwrap();
    let mut app = App::new(9, 7.5, AppConfig::default());
    app.store = GameStore::at(dir.join("games"));
    app.open_sgf_file(input.clone(), &sg).unwrap();
    (app, input)
}

/// 把当前棋局写回的真实文件重新解析一遍。
fn reopened(app: &App) -> SgfGame {
    let path = app.current_file.as_deref().expect("打开后应有库存文件");
    let text = app.store.read(path).unwrap();
    sgf::parse(&text).unwrap()
}

fn cleanup(app: &App) {
    if let Some(dir) = app.store.dir().parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn pl_white_keeps_white_first_end_to_end() {
    let src = include_str!("fixtures/pl-white.sgf");
    let (app, input) = open(src);

    // 原文件只读，不能被工具改写
    assert_eq!(std::fs::read_to_string(&input).unwrap(), src);

    let game = app.game();
    assert_eq!(game.first_to_move(), Color::White);
    assert_eq!(game.color_of_move(1), Color::White);
    assert_eq!(game.color_of_move(2), Color::Black);
    assert_eq!(
        game.position().board.at(Point::new(2, 2)),
        Some(Color::White),
        "W[cc] 必须是白子"
    );
    assert_eq!(
        game.position().board.at(Point::new(6, 6)),
        Some(Color::Black),
        "B[gg] 必须是黑子"
    );

    // 写回再读：先手和颜色都不能翻转
    let back = reopened(&app);
    assert_eq!(back.first_to_move(), Color::White);
    assert_eq!(back.moves.len(), 3);
    assert_eq!(back.rules.as_deref(), Some("Chinese"));

    cleanup(&app);
}

#[test]
fn handicap_setup_defaults_to_white_but_keeps_declared_moves() {
    let src = include_str!("fixtures/handicap-ab.sgf");
    let (app, _input) = open(src);

    let game = app.game();
    assert_eq!(game.size(), 19);
    assert_eq!(game.first_to_move(), Color::White, "让子棋白先");
    assert_eq!(game.ko_rule(), KoRule::Simple, "RU[Japanese]");
    assert_eq!(game.rules().scoring, Scoring::Territory);
    assert_eq!(
        game.position().board.at(Point::new(15, 3)),
        Some(Color::Black)
    );
    assert_eq!(
        game.position().board.at(Point::new(3, 15)),
        Some(Color::Black)
    );
    assert_eq!(
        game.position().board.at(Point::new(3, 3)),
        Some(Color::White),
        "W[dd] 应在摆子之后落成白子"
    );

    let back = reopened(&app);
    assert_eq!(back.size, 19);
    assert_eq!(back.setup.len(), 2);
    assert_eq!(back.first_to_move(), Color::White);

    cleanup(&app);
}

#[test]
fn consecutive_passes_survive_the_round_trip() {
    let src = include_str!("fixtures/passes.sgf");
    let (app, _input) = open(src);

    let game = app.game();
    assert_eq!(game.moves().len(), 4);
    assert_eq!(game.moves()[2], Move::Pass);
    assert_eq!(game.moves()[3], Move::Pass);

    let back = reopened(&app);
    assert_eq!(back.moves, game.moves().to_vec());

    cleanup(&app);
}

#[test]
fn variations_file_follows_the_main_line_only() {
    let src = include_str!("fixtures/variations.sgf");
    let (app, _input) = open(src);

    // 声明过的限制：只跟主线，分支不读。主线的定义是“第一个子分支”。
    assert_eq!(
        app.game().moves(),
        &[
            Move::Play(Point::new(2, 2)),
            Move::Play(Point::new(6, 6)),
            Move::Play(Point::new(3, 3)),
        ]
    );
    assert_eq!(app.game().rules(), RuleSet::KOREAN);

    cleanup(&app);
}

#[test]
fn collection_warns_instead_of_silently_dropping_games() {
    let src = include_str!("fixtures/collection.sgf");
    let games = sgf::parse_collection(src).unwrap();
    assert_eq!(games.len(), 2);

    let (app, _input) = open(src);
    assert_eq!(app.game().size(), 9, "默认打开第一盘");
    assert!(
        app.warnings.iter().any(|w| w.contains("2 盘棋")),
        "{:?}",
        app.warnings
    );
    assert!(
        app.status.contains("盘棋"),
        "状态栏要能看见：{}",
        app.status
    );

    cleanup(&app);
}

#[test]
fn unknown_rule_warns_and_falls_back_visibly() {
    let src = include_str!("fixtures/unknown-ru.sgf");
    let (app, _input) = open(src);

    assert_eq!(app.game().rules(), RuleSet::CHINESE);
    assert!(
        app.warnings.iter().any(|w| w.contains("My House Rules")),
        "{:?}",
        app.warnings
    );
    let back = reopened(&app);
    assert_eq!(back.rules.as_deref(), Some("Chinese"), "写回用规范化名字");

    cleanup(&app);
}

#[test]
fn aga_rule_name_and_scoring_survive_the_round_trip() {
    let src = include_str!("fixtures/aga.sgf");
    let (app, _input) = open(src);

    let game = app.game();
    assert_eq!(game.rules(), RuleSet::AGA);
    assert_eq!(game.rules().scoring, Scoring::Area);
    assert_eq!(game.ko_rule(), KoRule::SituationalSuperko);

    let path = app.current_file.as_deref().unwrap();
    let text = app.store.read(path).unwrap();
    assert!(text.contains("RU[AGA]"), "AGA 不能退化成 Korean：{text}");

    let back = reopened(&app);
    assert_eq!(
        sgf::recognized_rule_set(back.rules.as_deref().unwrap()),
        Some(RuleSet::AGA)
    );
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);

    cleanup(&app);
}

#[test]
fn every_fixture_reports_a_parsable_file_path() {
    // 防御性检查：fixtures 目录里的每个 .sgf 都必须能被 parse_collection 读，
    // 避免以后加文件忘了同步测试。
    for (name, src) in [
        ("pl-white", include_str!("fixtures/pl-white.sgf")),
        ("handicap-ab", include_str!("fixtures/handicap-ab.sgf")),
        ("passes", include_str!("fixtures/passes.sgf")),
        ("variations", include_str!("fixtures/variations.sgf")),
        ("collection", include_str!("fixtures/collection.sgf")),
        ("unknown-ru", include_str!("fixtures/unknown-ru.sgf")),
        ("aga", include_str!("fixtures/aga.sgf")),
    ] {
        let games = sgf::parse_collection(src).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!games.is_empty(), "{name}");
    }
}
