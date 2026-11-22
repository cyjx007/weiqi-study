//! GTK 窗口与事件绑定：把按键、点击、按钮、定时器接到 `app::Controller`。
//!
//! 这里只做「输入 → Controller 方法 → 刷新控件」，不实现规则或分析逻辑。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use app::{App, AppConfig, Controller};
use gtk::{gdk, glib};

use crate::board::{draw_board, point_at};
use crate::panels::{install_css, Panels};

/// 控制器 tick 间隔：分析结果/AI 应手最多延迟这么久反映到界面。
const TICK: Duration = Duration::from_millis(50);

struct Ui {
    ctrl: Controller,
    hover: Option<goban::Point>,
    area: gtk::DrawingArea,
    panels: Panels,
}

pub fn build_ui(application: &adw::Application) {
    install_css();
    let mut app = App::new(19, 7.5, AppConfig::default());
    app.refresh_games();
    if let Some(first) = app.games.first() {
        if let Ok(text) = app.store.read(&first.path) {
            if let Ok(sg) = sgf::parse(&text) {
                let _ = app.open_sgf_file(first.path.clone(), &sg);
            }
        }
    }

    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_content_width(640);
    area.set_content_height(640);

    let (side, panels) = Panels::build();
    let games = panels.games.clone();
    let lang_btn = panels.lang_btn.clone();
    let new_btn = panels.new_btn.clone();
    let analyse_btn = panels.analyse_btn.clone();
    let review_btn = panels.review_btn.clone();
    let undo_btn = panels.undo_btn.clone();
    let prev_btn = panels.prev_btn.clone();
    let next_btn = panels.next_btn.clone();

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
        ctrl: Controller::new(app),
        hover: None,
        area: area.clone(),
        panels,
    }));

    // 画棋盘
    {
        let ui = ui.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let ui = ui.borrow();
            draw_board(cr, w as f64, h as f64, &ui.ctrl.app, ui.hover);
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
            let n = ui.ctrl.app.size as f64;
            if let Some(p) = point_at(x, y, w_of(&ui.area), h_of(&ui.area), n) {
                ui.ctrl.app.cursor = p;
                let _ = ui.ctrl.play_at_cursor();
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
            let n = ui.ctrl.app.size as f64;
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
                gdk::Key::h | gdk::Key::Left => ui.ctrl.app.move_cursor(-1, 0),
                gdk::Key::l | gdk::Key::Right => ui.ctrl.app.move_cursor(1, 0),
                gdk::Key::k | gdk::Key::Up => ui.ctrl.app.move_cursor(0, -1),
                gdk::Key::j | gdk::Key::Down => ui.ctrl.app.move_cursor(0, 1),
                gdk::Key::Return => {
                    if let Some(p) = ui.hover {
                        ui.ctrl.app.cursor = p;
                    }
                    let _ = ui.ctrl.play_at_cursor();
                }
                gdk::Key::u => ui.ctrl.app.prev_move(),
                gdk::Key::n => ui.ctrl.app.next_move(),
                gdk::Key::p => ui.ctrl.app.prev_move(),
                gdk::Key::g => ui.ctrl.app.goto_start(),
                gdk::Key::G => ui.ctrl.app.goto_end(),
                gdk::Key::t => ui.ctrl.start_play(),
                gdk::Key::v => ui.ctrl.toggle_ai(),
                gdk::Key::a => ui.ctrl.analyze(),
                gdk::Key::A => ui.ctrl.review(),
                gdk::Key::b => ui.ctrl.app.next_blunder(),
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
            ui.ctrl.start_play();
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
            ui.ctrl.analyze();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        review_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.ctrl.review();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        undo_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.ctrl.app.prev_move();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        prev_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.ctrl.app.prev_move();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        next_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.ctrl.app.next_move();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        lang_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.ctrl.app.lang = if ui.ctrl.app.lang.is_zh() {
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
            if ui.panels.is_updating_games() {
                return;
            }
            let idx = ui.panels.games.selected() as usize;
            if idx < ui.ctrl.app.games.len() {
                ui.ctrl.app.game_cursor = idx;
                ui.ctrl.open_selected_game();
                reload_games(&mut ui);
            }
            refresh(&mut ui);
        });
    }

    // 定时器：收分析结果 + 推进 AI，只在状态变了才重画
    {
        let ui = ui.clone();
        glib::timeout_add_local(TICK, move || {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return glib::ControlFlow::Continue;
            };
            if ui.ctrl.tick() {
                refresh(&mut ui);
            }
            glib::ControlFlow::Continue
        });
    }

    {
        let mut ui = ui.borrow_mut();
        reload_games(&mut ui);
        refresh(&mut ui);
    }
}

fn refresh(ui: &mut Ui) {
    ui.panels.refresh(&mut ui.ctrl.app);
    ui.area.queue_draw();
}

fn reload_games(ui: &mut Ui) {
    ui.panels.reload_games(&mut ui.ctrl.app);
}

fn w_of(area: &gtk::DrawingArea) -> f64 {
    area.width() as f64
}

fn h_of(area: &gtk::DrawingArea) -> f64 {
    area.height() as f64
}
