//! weiqi-tui：本地围棋学习界面（ratatui）。

mod image_board;
mod ui;

use std::time::Duration;

use app::i18n::{fill, strings};
use app::{App, AppConfig, Controller};
use image_board::{ImageBoard, Mode};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

/// 事件轮询间隔；也决定 AI 应手、分析结果回到界面的节奏。
const FRAME: Duration = Duration::from_millis(50);

fn main() {
    if let Err(e) = run() {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut path: Option<String> = None;
    let mut visits: Option<u32> = None;
    let mut mode = Mode::Auto;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--visits" | "-visits" => {
                visits = Some(
                    args.next()
                        .ok_or("--visits 需要数字")?
                        .parse()
                        .map_err(|_| "--visits 不是数字")?,
                );
            }
            "--mode" | "-mode" => {
                let value = args.next().ok_or("--mode 需要 auto|image|text")?;
                mode = Mode::parse(&value)?;
            }
            "-V" | "--version" | "-version" => {
                println!("weiqi-tui {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" | "-help" => {
                println!(
                    "weiqi-tui {} — local Go study board / 本地围棋学习界面\n\n\
                     用法: weiqi-tui [game.sgf] [--visits N] [--mode auto|image|text]\n\
                     --mode  棋盘渲染：auto 探测 kitty/sixel/iTerm2（失败回退文本）,\n\
                              image 强制图片（不支持则报错）, text 强制文本点阵\n\
                     t 人机对局 / play the engine, s 换边 / swap sides, Enter 落子, L 棋谱 / games, ? 帮助 / help\n\
                     不给棋谱时：有 samples/demo.sgf 就打开它，否则空盘。",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            other if other.starts_with('-') => return Err(format!("未知参数 {other}").into()),
            other => path = Some(other.to_string()),
        }
    }

    let mut config = AppConfig::default();
    if let Some(v) = visits {
        config.engine.visits = v;
    }

    let app = match path {
        Some(path) => {
            let text = std::fs::read_to_string(&path)?;
            let sg = sgf::parse(&text)?;
            let mut app = App::from_sgf(&sg, config)?;
            app.open_sgf_file(std::path::PathBuf::from(path), &sg)?;
            app.show_games = false;
            app
        }
        None => {
            let demo = std::path::Path::new("samples/demo.sgf");
            if demo.exists() {
                let text = std::fs::read_to_string(demo)?;
                let sg = sgf::parse(&text)?;
                let mut app = App::from_sgf(&sg, config)?;
                app.open_sgf_file(demo.to_path_buf(), &sg)?;
                app
            } else {
                App::new(19, 7.5, config)
            }
        }
    };

    let mut terminal = ratatui::init();
    let mut image = match ImageBoard::new(mode) {
        Ok(image) => image,
        Err(e) => {
            ratatui::restore();
            return Err(e.into());
        }
    };
    let mut app = app;
    if app.warnings.is_empty() {
        if let Some(img) = &image {
            app.status = fill(strings(app.lang).image_board, img.protocol_name());
        } else if mode != Mode::Text {
            app.status = strings(app.lang).image_fallback.to_string();
        }
    }
    let result = event_loop(&mut terminal, Controller::new(app), &mut image);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut ctrl: Controller,
    image: &mut Option<ImageBoard>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut dirty = true;
    let mut overlay: Option<ui::Overlay> = None;
    loop {
        if dirty {
            terminal.draw(|frame| ui::draw(frame, &ctrl.app, image, overlay.as_ref()))?;
            dirty = false;
        }

        if event::poll(FRAME)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let ctrl_key = key.modifiers.contains(KeyModifiers::CONTROL);
                    if ctrl_key && key.code == KeyCode::Char('c') {
                        return Ok(());
                    }

                    if ctrl.app.preview_active() {
                        match key.code {
                            KeyCode::Right | KeyCode::Char('n') | KeyCode::Char(']') => {
                                ctrl.preview_next()
                            }
                            KeyCode::Left | KeyCode::Char('p') | KeyCode::Char('[') => {
                                ctrl.preview_prev()
                            }
                            KeyCode::Char('c') => ctrl.commit_preview(),
                            KeyCode::Esc => ctrl.exit_preview(),
                            KeyCode::Char('q') => return Ok(()),
                            _ => {}
                        }
                    } else if ctrl.app.show_games {
                        if let Some(prompt) = overlay.as_mut() {
                            match prompt {
                                ui::Overlay::ConfirmDelete => match key.code {
                                    KeyCode::Char('d') | KeyCode::Char('y') => {
                                        let _ = ctrl.app.delete_selected_game();
                                        overlay = None;
                                    }
                                    KeyCode::Esc | KeyCode::Char('n') => overlay = None,
                                    _ => {}
                                },
                                ui::Overlay::Rename(text) => match key.code {
                                    KeyCode::Enter => {
                                        let name = text.clone();
                                        if ctrl.app.rename_selected_game(&name).is_ok() {
                                            overlay = None;
                                        }
                                    }
                                    KeyCode::Esc => overlay = None,
                                    KeyCode::Backspace => {
                                        text.pop();
                                    }
                                    KeyCode::Char(c) => text.push(c),
                                    _ => {}
                                },
                            }
                        } else {
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('L') => {
                                    ctrl.app.toggle_games();
                                    overlay = None;
                                }
                                KeyCode::Up | KeyCode::Char('k') => ctrl.app.move_game_cursor(-1),
                                KeyCode::Down | KeyCode::Char('j') => ctrl.app.move_game_cursor(1),
                                KeyCode::Enter => ctrl.open_selected_game(),
                                KeyCode::Char('d') => overlay = Some(ui::Overlay::ConfirmDelete),
                                KeyCode::Char('r') => {
                                    if let Some(entry) = ctrl.app.selected_game() {
                                        let name = entry
                                            .path
                                            .file_stem()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("")
                                            .to_string();
                                        overlay = Some(ui::Overlay::Rename(name));
                                    }
                                }
                                KeyCode::Char('q') => return Ok(()),
                                _ => {}
                            }
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('q') => return Ok(()),
                            KeyCode::Esc => {
                                if ctrl.app.show_help {
                                    ctrl.app.show_help = false;
                                } else {
                                    return Ok(());
                                }
                            }
                            KeyCode::Char('?') => ctrl.app.show_help = !ctrl.app.show_help,
                            KeyCode::Char('L') => ctrl.app.toggle_games(),
                            KeyCode::Char('i') => ctrl.app.show_info = !ctrl.app.show_info,
                            KeyCode::Left | KeyCode::Char('h') => ctrl.app.move_cursor(-1, 0),
                            KeyCode::Right | KeyCode::Char('l') => ctrl.app.move_cursor(1, 0),
                            KeyCode::Up | KeyCode::Char('k') => ctrl.app.move_cursor(0, -1),
                            KeyCode::Down | KeyCode::Char('j') => ctrl.app.move_cursor(0, 1),
                            KeyCode::Enter => {
                                let _ = ctrl.play_at_cursor();
                            }
                            KeyCode::Char('t') => ctrl.start_play(),
                            KeyCode::Char('s') => ctrl.swap_sides(),
                            KeyCode::Char('v') => ctrl.toggle_ai(),
                            KeyCode::Char('V') => ctrl.start_preview(),
                            KeyCode::Char('u') => {
                                ctrl.app.prev_move();
                                ctrl.app.status = strings(ctrl.app.lang).undone.to_string();
                            }
                            KeyCode::Char('n') => ctrl.app.next_move(),
                            KeyCode::Char('p') => ctrl.app.prev_move(),
                            KeyCode::Home | KeyCode::Char('g') => ctrl.app.goto_start(),
                            KeyCode::End | KeyCode::Char('G') => ctrl.app.goto_end(),
                            KeyCode::Char('a') => ctrl.analyze(),
                            KeyCode::Char('A') => ctrl.review(),
                            KeyCode::Char('b') => ctrl.app.next_blunder(),
                            KeyCode::Char('+') | KeyCode::Char('=') => ctrl.adjust_visits(20),
                            KeyCode::Char('-') => ctrl.adjust_visits(-20),
                            KeyCode::Char('m') => ctrl.toggle_human(),
                            KeyCode::Char('[') => ctrl.cycle_human_rank(-1),
                            KeyCode::Char(']') => ctrl.cycle_human_rank(1),
                            _ => {}
                        }
                    }
                }
            }
            // 任何输入（按键、窗口尺寸变化）都可能改变界面
            ctrl.invalidate();
        }

        dirty |= ctrl.tick();
    }
}
