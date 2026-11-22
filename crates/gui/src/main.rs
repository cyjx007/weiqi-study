//! weiqi-gui：GTK4 + libadwaita 原生窗口。
//!
//! 棋盘用 Cairo 画，状态与棋谱库复用 `app` crate，和 TUI 完全同一份数据。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use app::i18n::{color_name, fill, strings};
use app::{Action, App, AppConfig, AppEvent};
use gtk::cairo;
use gtk::gdk;
use gtk::glib;

const WOOD: (f64, f64, f64) = (0.85, 0.68, 0.42);
const GRID: (f64, f64, f64) = (0.18, 0.12, 0.05);
const BG: (f64, f64, f64) = (0.10, 0.10, 0.11);

struct Ui {
    app: App,
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
    hover: Option<goban::Point>,
    area: gtk::DrawingArea,
    game_count: usize,
    updating_games: bool,

    winrate: gtk::ProgressBar,
    turn: gtk::Label,
    score: gtk::Label,
    moves: gtk::Label,
    cand: gtk::Label,
    comment: gtk::Label,
    status: gtk::Label,
    games: gtk::DropDown,
    sec_winrate: gtk::Label,
    sec_position: gtk::Label,
    sec_predict: gtk::Label,
    sec_comment: gtk::Label,
    lang_btn: gtk::Button,
    new_btn: gtk::Button,
    analyse_btn: gtk::Button,
    review_btn: gtk::Button,
    undo_btn: gtk::Button,
}

fn main() -> glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("io.github.cyjx007.WeiqiStudy")
        .build();
    application.connect_activate(build_ui);
    application.run()
}

fn build_ui(application: &adw::Application) {
    let config = AppConfig::default();
    let mut app = App::new(19, 7.5, config.clone());
    app.refresh_games();
    if let Some(first) = app.games.first() {
        if let Ok(text) = app.store.read(&first.path) {
            if let Ok(sg) = sgf::parse(&text) {
                let _ = app.open_sgf_file(first.path.clone(), &sg);
            }
        }
    }

    let (tx, rx) = mpsc::channel();

    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_content_width(640);
    area.set_content_height(640);

    let winrate = gtk::ProgressBar::new();
    winrate.set_show_text(true);
    winrate.set_fraction(0.5);
    winrate.set_text(Some("50.0%"));

    let turn = label("");
    let score = label("");
    let moves = label("");
    let cand = label("");
    cand.set_wrap(true);
    let comment = label("");
    comment.set_wrap(true);
    comment.set_valign(gtk::Align::Start);
    let status = label("");
    status.add_css_class("dim-label");
    status.set_wrap(true);

    let games = gtk::DropDown::from_strings(&["—"]);
    games.set_hexpand(true);

    let lang_btn = gtk::Button::with_label("EN");
    let new_btn = gtk::Button::with_label("");
    let analyse_btn = gtk::Button::with_label("");
    let review_btn = gtk::Button::with_label("");
    let undo_btn = gtk::Button::with_label("");
    let prev_btn = gtk::Button::with_label("‹");
    let next_btn = gtk::Button::with_label("›");

    let side = gtk::Box::new(gtk::Orientation::Vertical, 8);
    side.set_size_request(340, -1);
    side.set_margin_top(12);
    side.set_margin_bottom(12);
    side.set_margin_start(12);
    side.set_margin_end(12);
    let sec_winrate = section("");
    let sec_position = section("");
    let sec_predict = section("");
    let sec_comment = section("");
    side.append(&sec_winrate);
    side.append(&winrate);
    side.append(&sec_position);
    side.append(&turn);
    side.append(&score);
    side.append(&moves);
    side.append(&sec_predict);
    side.append(&cand);
    side.append(&sec_comment);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&comment));
    scroll.set_vexpand(true);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    side.append(&scroll);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.append(&new_btn);
    actions.append(&analyse_btn);
    actions.append(&review_btn);
    actions.append(&undo_btn);
    side.append(&actions);
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    nav.append(&prev_btn);
    nav.append(&next_btn);
    nav.append(&games);
    side.append(&nav);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    footer.append(&status);
    footer.append(&lang_btn);
    status.set_hexpand(true);
    side.append(&footer);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&area);
    body.append(&side);

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Weiqi Study")
        .default_width(1000)
        .default_height(800)
        .build();
    let header = adw::HeaderBar::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&body);
    window.set_content(Some(&root));
    window.present();

    let ui = Rc::new(RefCell::new(Ui {
        app,
        tx,
        rx,
        hover: None,
        area: area.clone(),
        game_count: 0,
        updating_games: false,
        winrate,
        turn,
        score,
        moves,
        cand,
        comment,
        status,
        games: games.clone(),
        sec_winrate,
        sec_position,
        sec_predict,
        sec_comment,
        lang_btn: lang_btn.clone(),
        new_btn: new_btn.clone(),
        analyse_btn: analyse_btn.clone(),
        review_btn: review_btn.clone(),
        undo_btn: undo_btn.clone(),
    }));

    // 画棋盘
    {
        let ui = ui.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let ui = ui.borrow();
            draw_board(cr, w as f64, h as f64, &ui.app, ui.hover);
        });
    }

    // 点击落子
    {
        let ui = ui.clone();
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, x, y| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            let n = ui.app.size as f64;
            if let Some(p) = point_at(x, y, w_of(&ui.area), h_of(&ui.area), n) {
                ui.app.cursor = p;
                match ui.app.play_at_cursor() {
                    Ok(()) => spawn_analysis(&mut ui),
                    Err(e) => ui.app.status = e,
                }
                refresh(&mut ui);
            }
        });
        area.add_controller(click);
    }

    // 悬停
    {
        let ui = ui.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            let n = ui.app.size as f64;
            ui.hover = point_at(x, y, w_of(&ui.area), h_of(&ui.area), n);
            ui.area.queue_draw();
        });
        area.add_controller(motion);
    }

    // 键盘
    {
        let ui = ui.clone();
        let window_c = window.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::h | gdk::Key::Left => ui.app.move_cursor(-1, 0),
                gdk::Key::l | gdk::Key::Right => ui.app.move_cursor(1, 0),
                gdk::Key::k | gdk::Key::Up => ui.app.move_cursor(0, -1),
                gdk::Key::j | gdk::Key::Down => ui.app.move_cursor(0, 1),
                gdk::Key::Return => {
                    if let Some(p) = ui.hover {
                        ui.app.cursor = p;
                    }
                    match ui.app.play_at_cursor() {
                        Ok(()) => spawn_analysis(&mut ui),
                        Err(e) => ui.app.status = e,
                    }
                }
                gdk::Key::u => ui.app.prev_move(),
                gdk::Key::n => ui.app.next_move(),
                gdk::Key::p => ui.app.prev_move(),
                gdk::Key::g => ui.app.goto_start(),
                gdk::Key::G => ui.app.goto_end(),
                gdk::Key::t => {
                    ui.app.start_play();
                    spawn_analysis(&mut ui);
                }
                gdk::Key::v => {
                    ui.app.toggle_ai();
                    pump(&mut ui);
                }
                gdk::Key::a => spawn_analysis(&mut ui),
                gdk::Key::A => spawn_review(&mut ui),
                gdk::Key::b => ui.app.next_blunder(),
                gdk::Key::q => {
                    window_c.close();
                    return glib::Propagation::Stop;
                }
                _ => return glib::Propagation::Proceed,
            }
            refresh(&mut ui);
            glib::Propagation::Stop
        });
        window.add_controller(keys);
    }

    // 按钮
    {
        let ui = ui.clone();
        new_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.app.start_play();
            spawn_analysis(&mut ui);
            reload_games(&mut ui);
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        analyse_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            spawn_analysis(&mut ui);
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        review_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            spawn_review(&mut ui);
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        undo_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.app.prev_move();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        prev_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.app.prev_move();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        next_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.app.next_move();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        lang_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.app.lang = if ui.app.lang.is_zh() {
                app::i18n::Lang::En
            } else {
                app::i18n::Lang::Zh
            };
            reload_games(&mut ui);
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        games.connect_selected_notify(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            if ui.updating_games {
                return;
            }
            let idx = ui.games.selected() as usize;
            if idx < ui.app.games.len() {
                ui.app.game_cursor = idx;
                ui.app.open_selected_game();
                reload_games(&mut ui);
            }
            refresh(&mut ui);
        });
    }

    // 定时器：收分析结果、推进 AI、刷新界面
    {
        let ui = ui.clone();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return glib::ControlFlow::Continue;
            };
            while let Ok(ev) = ui.rx.try_recv() {
                ui.app.apply(ev);
            }
            pump(&mut ui);
            refresh(&mut ui);
            glib::ControlFlow::Continue
        });
    }

    {
        let mut ui = ui.borrow_mut();
        reload_games(&mut ui);
        refresh(&mut ui);
    }
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l
}

fn section(text: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.set_markup(&format!("<b>{text}</b>"));
    l
}

fn w_of(area: &gtk::DrawingArea) -> f64 {
    area.width() as f64
}

fn h_of(area: &gtk::DrawingArea) -> f64 {
    area.height() as f64
}

fn spawn_analysis(ui: &mut Ui) {
    if ui.app.busy {
        return;
    }
    let pos = ui.app.pos();
    ui.app.busy = true;
    ui.app.status = fill(strings(ui.app.lang).analyzing, ui.app.config.engine.visits);
    let game = ui.app.snapshot();
    let cfg = ui.app.config.engine.clone();
    let tx = ui.tx.clone();
    std::thread::spawn(move || {
        let result = engine::analyze_position(&cfg, &game);
        let _ = tx.send(AppEvent::Analysis { pos, result });
    });
}

fn spawn_review(ui: &mut Ui) {
    if ui.app.busy {
        return;
    }
    ui.app.busy = true;
    ui.app.status = strings(ui.app.lang).reviewing.to_string();
    let game = match ui.app.full_game() {
        Ok(g) => g,
        Err(e) => {
            ui.app.busy = false;
            ui.app.status = e;
            return;
        }
    };
    let cfg = ui.app.config.engine.clone();
    let tx = ui.tx.clone();
    std::thread::spawn(move || {
        let result = engine::analyze_game(&cfg, &game, |_, _| {})
            .map(|evals| review::build(&game, &evals, 5.0));
        let _ = tx.send(AppEvent::Review(result));
    });
}

fn pump(ui: &mut Ui) {
    match ui.app.next_action() {
        Action::None => {}
        Action::Analyze => spawn_analysis(ui),
        Action::AiMove(mv) => {
            if ui.app.play_move(mv).is_ok() {
                ui.app.status = strings(ui.app.lang).ai_played.to_string();
                spawn_analysis(ui);
            }
        }
    }
}

fn reload_games(ui: &mut Ui) {
    ui.app.refresh_games();
    let names: Vec<String> = if ui.app.games.is_empty() {
        vec![strings(ui.app.lang).no_games.to_string()]
    } else {
        ui.app
            .games
            .iter()
            .map(|g| {
                let name = g
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                format!("{}  {}", g.label, name)
            })
            .collect()
    };
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    ui.updating_games = true;
    ui.games.set_model(Some(&gtk::StringList::new(&refs)));
    ui.updating_games = false;
    ui.game_count = ui.app.games.len();
}

fn refresh(ui: &mut Ui) {
    let s = strings(ui.app.lang);
    ui.sec_winrate.set_markup(&format!("<b>{}</b>", s.winrate));
    ui.sec_position.set_markup(&format!("<b>{}</b>", s.position));
    ui.sec_predict.set_markup(&format!("<b>{}</b>", s.predict));
    ui.sec_comment.set_markup(&format!("<b>{}</b>", s.commentary));
    ui.new_btn.set_label(s.new_game_btn);
    ui.analyse_btn.set_label(s.analyse_btn);
    ui.review_btn.set_label(s.review_btn);
    ui.undo_btn.set_label(s.undo_btn);
    ui.lang_btn
        .set_label(if ui.app.lang.is_zh() { "EN" } else { "中文" });

    let wr = ui.app.analysis().map(|a| a.winrate_black).unwrap_or(0.5);
    ui.winrate.set_fraction(wr.clamp(0.0, 1.0) as f64);
    ui.winrate
        .set_text(Some(&format!("{:.1}%", wr * 100.0)));

    let who = if !ui.app.ai_enabled {
        s.review.to_string()
    } else if ui.app.to_move() == ui.app.human {
        s.you.to_string()
    } else {
        s.ai.to_string()
    };
    ui.turn.set_text(&format!(
        "{}: {}({})",
        s.turn,
        color_name(ui.app.to_move(), ui.app.lang),
        who
    ));
    let score = ui.app.analysis().map(|a| a.score_black).unwrap_or(0.0);
    ui.score.set_text(&format!("{}: {score:+.1}", s.score));
    ui.moves
        .set_text(&format!("{}: {} / {}", s.moves, ui.app.pos(), ui.app.total_moves()));

    let mut cand = String::new();
    for (i, (mv, wr)) in ui.app.prediction().iter().enumerate() {
        cand.push_str(&format!(
            "{}. {}  {:.1}%\n",
            i + 1,
            move_text(*mv, ui.app.size),
            wr * 100.0
        ));
    }
    if let Some(a) = ui.app.analysis() {
        if let Some(best) = a.candidates.first() {
            let pv: Vec<String> = best
                .pv
                .iter()
                .take(6)
                .map(|m| move_text(*m, ui.app.size))
                .collect();
            if !pv.is_empty() {
                cand.push_str(&format!("{}: {}", s.line, pv.join(" ")));
            }
        }
    }
    if cand.is_empty() {
        cand.push_str(s.press_a);
    }
    ui.cand.set_text(&cand);

    let mut text = ui.app.last_comment.clone().unwrap_or_default();
    if ui.app.ai_enabled && ui.app.to_move() == ui.app.human {
        if let Some(a) = ui.app.analysis() {
            if let Some(best) = a.candidates.first() {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&format!(
                    "{} {}（{:.1}%）",
                    s.suggestion,
                    move_text(best.mv, ui.app.size),
                    best.winrate_black * 100.0
                ));
            }
        }
    }
    if text.is_empty() {
        text.push_str(s.fallback_comment);
    }
    ui.comment.set_text(&text);
    ui.status.set_text(&ui.app.status);

    if ui.app.games.len() != ui.game_count {
        reload_games(ui);
    }
    ui.area.queue_draw();
}

fn move_text(mv: goban::Move, size: u8) -> String {
    match mv {
        goban::Move::Play(p) => engine::gtp_coord(p, size),
        goban::Move::Pass => "pass".to_string(),
    }
}

#[derive(Clone, Copy)]
struct Geo {
    ox: f64,
    oy: f64,
    cell: f64,
    board: f64,
}

fn geometry(w: f64, h: f64, n: f64) -> Geo {
    let margin = 58.0;
    let avail = (w.min(h) - 2.0 * margin).max(60.0);
    let cell = avail / (n - 1.0);
    let board = cell * (n - 1.0);
    Geo {
        ox: (w - board) / 2.0,
        oy: (h - board) / 2.0,
        cell,
        board,
    }
}

fn point_at(x: f64, y: f64, w: f64, h: f64, n: f64) -> Option<goban::Point> {
    let g = geometry(w, h, n);
    let gx = ((x - g.ox) / g.cell).round();
    let gy = ((y - g.oy) / g.cell).round();
    if gx < 0.0 || gy < 0.0 || gx >= n || gy >= n {
        return None;
    }
    let dx = x - (g.ox + gx * g.cell);
    let dy = y - (g.oy + gy * g.cell);
    if dx * dx + dy * dy > (g.cell * 0.5).powi(2) {
        return None;
    }
    Some(goban::Point::new(gx as u8, gy as u8))
}

fn rgb(cr: &cairo::Context, c: (f64, f64, f64)) {
    cr.set_source_rgb(c.0, c.1, c.2);
}

fn draw_board(cr: &cairo::Context, w: f64, h: f64, app: &App, hover: Option<goban::Point>) {
    rgb(cr, BG);
    cr.paint().ok();

    let n = app.size as f64;
    let g = geometry(w, h, n);
    let pad = g.cell * 1.35;

    rgb(cr, WOOD);
    cr.rectangle(g.ox - pad, g.oy - pad, g.board + 2.0 * pad, g.board + 2.0 * pad);
    cr.fill().ok();

    // 网格
    rgb(cr, GRID);
    cr.set_line_width(1.0);
    for i in 0..n as i32 {
        let p = g.ox + i as f64 * g.cell;
        cr.move_to(p, g.oy);
        cr.line_to(p, g.oy + g.board);
        let q = g.oy + i as f64 * g.cell;
        cr.move_to(g.ox, q);
        cr.line_to(g.ox + g.board, q);
    }
    cr.stroke().ok();

    // 星位
    let stars = star_points(n as usize);
    for &sx in stars {
        for &sy in stars {
            let x = g.ox + sx as f64 * g.cell;
            let y = g.oy + sy as f64 * g.cell;
            cr.arc(x, y, g.cell * 0.09, 0.0, std::f64::consts::TAU);
            cr.fill().ok();
        }
    }

    // 坐标
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(g.cell * 0.36);
    for i in 0..n as i32 {
        let label = format!("{}", i + 1);
        let ext = cr.text_extents(&label).map(|e| e.width()).unwrap_or(0.0);
        let x = g.ox + i as f64 * g.cell - ext / 2.0;
        cr.move_to(x, g.oy - g.cell * 0.45);
        cr.show_text(&label).ok();
        cr.move_to(x, g.oy + g.board + g.cell * 0.85);
        cr.show_text(&label).ok();

        let ext = cr.text_extents(&label).map(|e| e.width()).unwrap_or(0.0);
        let y = g.oy + i as f64 * g.cell + g.cell * 0.15;
        cr.move_to(g.ox - g.cell * 0.55 - ext, y);
        cr.show_text(&label).ok();
        cr.move_to(g.ox + g.board + g.cell * 0.45, y);
        cr.show_text(&label).ok();
    }

    let board = &app.game().position().board;
    // 候选
    for (rank, (mv, _)) in app.prediction().iter().enumerate() {
        if let goban::Move::Play(p) = mv {
            let x = g.ox + p.x as f64 * g.cell;
            let y = g.oy + p.y as f64 * g.cell;
            cr.set_source_rgba(0.15, 0.45, 0.85, 0.45);
            cr.arc(x, y, g.cell * 0.42, 0.0, std::f64::consts::TAU);
            cr.fill().ok();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
            let t = format!("{}", rank + 1);
            let ext = cr.text_extents(&t).map(|e| e.width()).unwrap_or(0.0);
            cr.move_to(x - ext / 2.0, y + g.cell * 0.12);
            cr.show_text(&t).ok();
        }
    }

    // 棋子
    for (p, c) in board.stones() {
        let x = g.ox + p.x as f64 * g.cell;
        let y = g.oy + p.y as f64 * g.cell;
        draw_stone(cr, x, y, g.cell * 0.46, c == goban::Color::Black);
    }

    // 最后一手
    if let Some(goban::Move::Play(p)) = app.game().position().last_move {
        let x = g.ox + p.x as f64 * g.cell;
        let y = g.oy + p.y as f64 * g.cell;
        cr.set_source_rgba(0.85, 0.15, 0.15, 0.9);
        cr.arc(x, y, g.cell * 0.12, 0.0, std::f64::consts::TAU);
        cr.fill().ok();
    }

    // 光标/悬停
    let cursor = hover.or(Some(app.cursor));
    if let Some(p) = cursor {
        let x = g.ox + p.x as f64 * g.cell;
        let y = g.oy + p.y as f64 * g.cell;
        cr.set_source_rgba(0.2, 0.6, 1.0, 0.55);
        cr.set_line_width(2.0);
        cr.arc(x, y, g.cell * 0.5, 0.0, std::f64::consts::TAU);
        cr.stroke().ok();
    }
}

fn draw_stone(cr: &cairo::Context, x: f64, y: f64, r: f64, black: bool) {
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
    cr.arc(x + r * 0.08, y + r * 0.12, r, 0.0, std::f64::consts::TAU);
    cr.fill().ok();

    let grad = cairo::LinearGradient::new(x - r, y - r, x + r, y + r);
    if black {
        grad.add_color_stop_rgb(0.0, 0.32, 0.32, 0.35);
        grad.add_color_stop_rgb(1.0, 0.02, 0.02, 0.03);
    } else {
        grad.add_color_stop_rgb(0.0, 1.0, 1.0, 1.0);
        grad.add_color_stop_rgb(1.0, 0.78, 0.78, 0.78);
    }
    cr.set_source(&grad).ok();
    cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
    cr.fill().ok();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    cr.set_line_width(1.0);
    cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
    cr.stroke().ok();
}

fn star_points(n: usize) -> &'static [usize] {
    match n {
        19 => &[3, 9, 15],
        13 => &[3, 6, 9],
        9 => &[2, 4, 6],
        _ => &[],
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_at_maps_intersections() {
        let n = 19.0;
        let w = 800.0;
        let h = 800.0;
        let g = geometry(w, h, n);
        let p = point_at(g.ox + 3.0 * g.cell, g.oy + 3.0 * g.cell, w, h, n).unwrap();
        assert_eq!(p, goban::Point::new(3, 3));
        // 远离交叉点 -> None
        assert!(point_at(g.ox + 3.5 * g.cell, g.oy + 3.5 * g.cell, w, h, n).is_none());
        assert!(point_at(-10.0, -10.0, w, h, n).is_none());
    }

    #[test]
    fn geometry_is_centred() {
        let g = geometry(1000.0, 700.0, 19.0);
        let left = g.ox;
        let right = 1000.0 - (g.ox + g.board);
        assert!((left - right).abs() < 0.001);
    }
}
