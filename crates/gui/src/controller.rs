//! GTK 窗口与事件绑定：把按键、点击、按钮、定时器接到 `app::Controller`。
//!
//! 这里只做「输入 → Controller 方法 → 刷新控件」，不实现规则或分析逻辑。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use app::i18n::strings;
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
    let rename_btn = panels.rename_btn.clone();
    let delete_btn = panels.delete_btn.clone();
    let variation_btn = panels.variation_btn.clone();
    let commit_btn = panels.commit_btn.clone();
    let visits = panels.visits.clone();
    let human_switch = panels.human_switch.clone();
    let rank = panels.rank.clone();

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
                gdk::Key::u => {
                    if ui.ctrl.app.preview_active() {
                        ui.ctrl.preview_prev();
                    } else {
                        ui.ctrl.app.prev_move();
                    }
                }
                gdk::Key::n => {
                    if ui.ctrl.app.preview_active() {
                        ui.ctrl.preview_next();
                    } else {
                        ui.ctrl.app.next_move();
                    }
                }
                gdk::Key::p => {
                    if ui.ctrl.app.preview_active() {
                        ui.ctrl.preview_prev();
                    } else {
                        ui.ctrl.app.prev_move();
                    }
                }
                gdk::Key::V => ui.ctrl.start_preview(),
                gdk::Key::c => {
                    if ui.ctrl.app.preview_active() {
                        ui.ctrl.commit_preview();
                    }
                }
                gdk::Key::Escape => {
                    if ui.ctrl.app.preview_active() {
                        ui.ctrl.exit_preview();
                    } else {
                        return glib::Propagation::Proceed;
                    }
                }
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
        let window_d = window.clone();
        delete_btn.connect_clicked(move |_| {
            let (heading, body, cancel_label, ok_label) = {
                let ui = ui.borrow();
                let s = strings(ui.ctrl.app.lang);
                (
                    s.delete.to_string(),
                    s.trash_body.to_string(),
                    s.cancel.to_string(),
                    s.ok.to_string(),
                )
            };
            let dialog = adw::AlertDialog::builder()
                .heading(heading)
                .body(body)
                .build();
            dialog.add_response("cancel", &cancel_label);
            dialog.add_response("delete", &ok_label);
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            let ui_c = ui.clone();
            dialog.choose(
                Some(&window_d),
                None::<&gtk::gio::Cancellable>,
                move |response| {
                    if response == "delete" {
                        if let Ok(mut ui) = ui_c.try_borrow_mut() {
                            let _ = ui.ctrl.app.delete_selected_game();
                            reload_games(&mut ui);
                            refresh(&mut ui);
                        }
                    }
                },
            );
        });
    }
    {
        let ui = ui.clone();
        let window_r = window.clone();
        rename_btn.connect_clicked(move |_| {
            let (title, initial, cancel_label, ok_label) = {
                let ui = ui.borrow();
                let s = strings(ui.ctrl.app.lang);
                let initial = ui
                    .ctrl
                    .app
                    .selected_game()
                    .and_then(|g| g.path.file_stem())
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                (
                    s.rename.to_string(),
                    initial,
                    s.cancel.to_string(),
                    s.ok.to_string(),
                )
            };

            let win = gtk::Window::builder()
                .title(title)
                .modal(true)
                .default_width(360)
                .build();
            win.set_transient_for(Some(&window_r));

            let entry = gtk::Entry::new();
            entry.set_text(&initial);
            entry.set_hexpand(true);
            let cancel = gtk::Button::with_label(&cancel_label);
            let ok = gtk::Button::with_label(&ok_label);
            ok.add_css_class("suggested-action");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_halign(gtk::Align::End);
            row.append(&cancel);
            row.append(&ok);
            let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
            root.set_margin_top(14);
            root.set_margin_bottom(14);
            root.set_margin_start(14);
            root.set_margin_end(14);
            root.append(&entry);
            root.append(&row);
            win.set_child(Some(&root));

            {
                let win = win.clone();
                cancel.connect_clicked(move |_| win.close());
            }
            {
                let win = win.clone();
                let ui = ui.clone();
                let entry = entry.clone();
                ok.connect_clicked(move |_| {
                    if let Ok(mut ui) = ui.try_borrow_mut() {
                        let name = entry.text().to_string();
                        if ui.ctrl.app.rename_selected_game(&name).is_ok() {
                            reload_games(&mut ui);
                            refresh(&mut ui);
                            win.close();
                        } else {
                            refresh(&mut ui);
                        }
                    }
                });
            }
            {
                let ok = ok.clone();
                entry.connect_activate(move |_| ok.emit_clicked());
            }
            win.present();
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
            // 不要在 DropDown 的 selected-notify 里同步重建棋局/刷新控件：
            // GTK 此时正在关闭 popover，重入很容易让它卡在 frame 循环里。
            // 先取出 index，等主循环空闲再做。
            let idx = {
                let Ok(ui) = ui.try_borrow() else {
                    return;
                };
                if ui.panels.is_updating_games() {
                    return;
                }
                let idx = ui.panels.games.selected() as usize;
                if idx >= ui.ctrl.app.games.len() {
                    return;
                }
                idx
            };
            let ui = ui.clone();
            glib::idle_add_local_once(move || {
                let Ok(mut ui) = ui.try_borrow_mut() else {
                    return;
                };
                if ui.panels.is_updating_games() {
                    return;
                }
                ui.ctrl.app.game_cursor = idx;
                ui.ctrl.open_selected_game();
                refresh(&mut ui);
            });
        });
    }

    {
        let ui = ui.clone();
        variation_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.ctrl.start_preview();
            refresh(&mut ui);
        });
    }
    {
        let ui = ui.clone();
        commit_btn.connect_clicked(move |_| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            ui.ctrl.commit_preview();
            refresh(&mut ui);
        });
    }

    // AI 强度 / 拟人网
    {
        let ui = ui.clone();
        visits.connect_value_changed(move |spin| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            if ui.panels.is_updating_controls() {
                return;
            }
            let value = spin.value() as u32;
            if value != ui.ctrl.app.config.engine.visits {
                ui.ctrl.set_visits(value);
                refresh(&mut ui);
            }
        });
    }
    {
        let ui = ui.clone();
        human_switch.connect_active_notify(move |switch| {
            let Ok(mut ui) = ui.try_borrow_mut() else {
                return;
            };
            if ui.panels.is_updating_controls() {
                return;
            }
            if switch.is_active() != ui.ctrl.app.human_enabled {
                ui.ctrl.toggle_human();
                refresh(&mut ui);
            }
        });
    }
    {
        let ui = ui.clone();
        rank.connect_selected_notify(move |dropdown| {
            let idx = {
                let Ok(ui) = ui.try_borrow() else {
                    return;
                };
                if ui.panels.is_updating_controls() {
                    return;
                }
                let idx = dropdown.selected() as usize;
                if idx == ui.ctrl.app.human_rank {
                    return;
                }
                idx
            };
            let ui = ui.clone();
            glib::idle_add_local_once(move || {
                let Ok(mut ui) = ui.try_borrow_mut() else {
                    return;
                };
                if ui.panels.is_updating_controls() {
                    return;
                }
                ui.ctrl.set_human_rank(idx);
                refresh(&mut ui);
            });
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
