//! weiqi-tui：本地围棋学习界面（ratatui）。

mod ui;

use std::sync::mpsc;
use std::time::Duration;

use app::{Action, App, AppConfig, AppEvent};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

fn main() {
    if let Err(e) = run() {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut path: Option<String> = None;
    let mut visits: Option<u32> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--visits" => {
                visits = Some(
                    args.next()
                        .ok_or("--visits 需要数字")?
                        .parse()
                        .map_err(|_| "--visits 不是数字")?,
                );
            }
            "-h" | "--help" => {
                println!(
                    "weiqi-tui — 本地围棋学习界面\n\n用法：weiqi-tui [棋谱.sgf] [--visits N]\n\n\
                     t 开始人机对局，Enter 落子，a 分析当前局面，A 全盘复盘。\n\
                     不给棋谱时：有 samples/demo.sgf 就打开它，否则空盘。\n\
                     --visits 默认 120。"
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
            App::from_sgf(&sg, config)?
        }
        None => {
            // 没给棋谱：先找仓库里的示例，再退回空盘
            let demo = std::path::Path::new("samples/demo.sgf");
            if demo.exists() {
                let text = std::fs::read_to_string(demo)?;
                let sg = sgf::parse(&text)?;
                App::from_sgf(&sg, config)?
            } else {
                App::new(19, 7.5, config)
            }
        }
    };

    let (tx, rx) = mpsc::channel::<AppEvent>();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, app, tx, rx);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: App,
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    match key.code {
                        KeyCode::Char('c') if ctrl => return Ok(()),
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Esc => {
                            if app.show_help {
                                app.show_help = false;
                            } else {
                                return Ok(());
                            }
                        }
                        KeyCode::Char('?') => app.show_help = !app.show_help,
                        KeyCode::Left | KeyCode::Char('h') => app.move_cursor(-1, 0),
                        KeyCode::Right | KeyCode::Char('l') => app.move_cursor(1, 0),
                        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(0, -1),
                        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(0, 1),
                        KeyCode::Enter => match app.play_at_cursor() {
                            Ok(()) => {
                                app.status = "落子".to_string();
                                spawn_analysis(&mut app, &tx);
                            }
                            Err(e) => app.status = e,
                        },
                        KeyCode::Char('t') => {
                            app.start_play();
                            spawn_analysis(&mut app, &tx);
                        }
                        KeyCode::Char('v') => {
                            app.toggle_ai();
                            pump(&mut app, &tx);
                        }
                        KeyCode::Char('u') => {
                            app.prev_move();
                            app.status = "撤回一手".to_string();
                        }
                        KeyCode::Char('n') => app.next_move(),
                        KeyCode::Char('p') => app.prev_move(),
                        KeyCode::Home | KeyCode::Char('g') => app.goto_start(),
                        KeyCode::End | KeyCode::Char('G') => app.goto_end(),
                        KeyCode::Char('a') => spawn_analysis(&mut app, &tx),
                        KeyCode::Char('A') => spawn_review(&mut app, &tx),
                        KeyCode::Char('b') => app.next_blunder(),
                        _ => {}
                    }
                }
            }
        }

        while let Ok(ev) = rx.try_recv() {
            app.apply(ev);
        }
        pump(&mut app, &tx);
    }
}

/// 人机模式下推进一步：轮到 AI 就落子，否则保证当前局面有分析结果。
fn pump(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    match app.next_action() {
        Action::None => {}
        Action::Analyze => spawn_analysis(app, tx),
        Action::AiMove(mv) => match app.play_move(mv) {
            Ok(()) => {
                app.status = "AI 落子".to_string();
                spawn_analysis(app, tx);
            }
            Err(e) => app.status = e,
        },
    }
}

fn spawn_analysis(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    if app.busy {
        app.status = "上一个分析还没结束".to_string();
        return;
    }
    let pos = app.pos();
    app.busy = true;
    app.status = format!("分析中（{} visits）…", app.config.engine.visits);
    let game = app.snapshot();
    let cfg = app.config.engine.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = engine::analyze_position(&cfg, &game);
        let _ = tx.send(AppEvent::Analysis { pos, result });
    });
}

fn spawn_review(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    if app.busy {
        app.status = "上一个分析还没结束".to_string();
        return;
    }
    app.busy = true;
    app.status = "全盘复盘中…".to_string();
    let game = match app.full_game() {
        Ok(g) => g,
        Err(e) => {
            app.busy = false;
            app.status = e;
            return;
        }
    };
    let cfg = app.config.engine.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = engine::analyze_game(&cfg, &game, |_, _| {})
            .map(|evals| review::build(&game, &evals, 5.0));
        let _ = tx.send(AppEvent::Review(result));
    });
}
