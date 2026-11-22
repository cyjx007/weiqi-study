//! 界面渲染：只读 `App` 状态，不修改任何东西。

use crate::image_board::ImageBoard;

use app::i18n::{color_name, strings};
use app::{move_text, App};
use goban::{Color, Move, Point};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color as C, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap},
    Frame,
};

const WOOD: C = C::Rgb(214, 176, 124);
const LINE: C = C::Rgb(126, 96, 62);
const HOSHI: C = C::Rgb(64, 40, 16);
const CAND: C = C::Rgb(20, 40, 110);
const CURSOR: C = C::Rgb(80, 136, 196);
const GTP_COLS: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";
const CAND_LABELS: [&str; 3] = ["1", "2", "3"];

pub enum Overlay {
    ConfirmDelete,
    Rename(String),
}

pub fn draw(
    frame: &mut Frame,
    app: &App,
    image: &mut Option<ImageBoard>,
    overlay: Option<&Overlay>,
) {
    let area = frame.area();
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(area);
    let content = rows[0];
    let footer = rows[1];

    // 宽窗口才固定侧栏；窄窗口把棋盘铺满，信息用 i 临时覆盖显示
    let docked = content.width >= 78;
    let side_w = 38.min(content.width.saturating_sub(44));
    if docked {
        let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(side_w)]).split(content);
        draw_board(frame, app, cols[0], image);
        let mode = image.as_ref().map(|i| i.protocol_name()).unwrap_or("text");
        draw_side(frame, app, cols[1], mode);
    } else {
        draw_board(frame, app, content, image);
        if app.show_info {
            let w = 38.min(content.width);
            let rect = Rect {
                x: content.x + content.width.saturating_sub(w),
                y: content.y,
                width: w,
                height: content.height,
            };
            frame.render_widget(Clear, rect);
            let mode = image.as_ref().map(|i| i.protocol_name()).unwrap_or("text");
            draw_side(frame, app, rect, mode);
        }
    }

    let s = strings(app.lang);
    frame.render_widget(
        Paragraph::new(s.footer)
            .style(Style::default().fg(C::Gray))
            .wrap(Wrap { trim: true }),
        footer,
    );

    if app.show_help {
        draw_help(frame, app, area);
    }
    if app.show_games {
        draw_games(frame, app, area);
    }
    if let Some(overlay) = overlay {
        draw_overlay(frame, app, area, overlay);
    }
}

/// 保持像素接近正方形：字符高约为宽的两倍，所以行距 = 列距 / 2。
fn choose_scale(inner_w: u16, inner_h: u16, n: u16) -> Option<(u16, u16)> {
    for (spacing, row_step) in [(6u16, 3u16), (4, 2), (2, 1)] {
        let w = n * spacing + 4;
        let h = n * row_step + 2;
        if w <= inner_w && h <= inner_h {
            return Some((spacing, row_step));
        }
    }
    None
}

fn draw_board(frame: &mut Frame, app: &App, area: Rect, image: &mut Option<ImageBoard>) {
    let s = strings(app.lang);
    let who = if app.ai_enabled {
        if app.to_move() == app.human {
            s.you
        } else {
            s.ai
        }
    } else {
        s.review
    };
    let title = format!(
        " {} · {} {}({}) ",
        s.game,
        s.turn,
        color_name(app.display_to_move(), app.lang),
        who
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // 图片模式：Cairo/位图负责棋盘，文字侧栏照旧
    if let Some(img) = image.as_mut() {
        img.draw(frame, inner, app);
        return;
    }

    let n = app.size as u16;
    let Some((spacing, row_step)) = choose_scale(inner.width, inner.height, n) else {
        frame.render_widget(
            Paragraph::new("window too small / 窗口太小")
                .style(Style::default().fg(C::Gray)),
            inner,
        );
        return;
    };
    let need_w = n * spacing + 4;
    let need_h = n * row_step + 2;
    let x0 = inner.x + (inner.width - need_w) / 2;
    let y0 = inner.y + (inner.height - need_h) / 2;

    let wood_rect = Rect {
        x: x0,
        y: y0,
        width: need_w,
        height: need_h,
    };
    frame.render_widget(Block::default().style(Style::default().bg(WOOD)), wood_rect);

    let buf = frame.buffer_mut();
    let wood = Style::default().bg(WOOD);
    let stars = star_points(n as usize);
    let grid_x = x0 + 3;

    let pad = (spacing.saturating_sub(1)) / 2;
    for j in 0..n {
        let letter = GTP_COLS[j as usize] as char;
        buf.set_string(grid_x + j * spacing + pad, y0, letter.to_string(), wood.fg(LINE));
        buf.set_string(
            grid_x + j * spacing + pad,
            y0 + 1 + n * row_step,
            letter.to_string(),
            wood.fg(LINE),
        );
    }

    for i in 0..n {
        let row_label = (n - i).to_string();
        let y = y0 + 1 + i * row_step;
        buf.set_string(x0, y, format!("{row_label:>2}"), wood.fg(LINE));
        for j in 0..n {
            let p = Point::new(j as u8, i as u8);
            let x = grid_x + j * spacing;
            let stone = app.display_game().position().board.at(p);
            let is_last = app.display_game().position().last_move == Some(Move::Play(p));
            let mut style = wood;
            if is_last {
                style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            }
            let (sym, fg, bold) = match stone {
                Some(Color::Black) => ("●", C::Black, false),
                Some(Color::White) => ("●", C::White, true),
                None => match app.candidate_rank(p) {
                    Some(rank) => (CAND_LABELS[rank - 1], CAND, true),
                    None => {
                        let hoshi = stars.contains(&(i as usize)) && stars.contains(&(j as usize));
                        if hoshi {
                            ("•", HOSHI, true)
                        } else {
                            ("·", LINE, false)
                        }
                    }
                },
            };
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            buf.set_string(x, y, cell_text(sym, spacing), style.fg(fg));

            // 光标：用奇数宽的块罩住落点，保证点在方块正中（偶数格宽做不到精确居中）
            if app.cursor == p {
                let block_w = if spacing >= 3 { 3 } else { 1 };
                let pad = (spacing.saturating_sub(1)) / 2;
                let start = pad.saturating_sub(block_w / 2);
                buf.set_string(
                    x + start,
                    y,
                    cell_text(sym, block_w),
                    style.bg(CURSOR).fg(fg),
                );
            }
        }
    }
}

/// 把 1 列宽的符号放进 `spacing` 列宽的格子里并居中。
fn cell_text(sym: &str, spacing: u16) -> String {
    let w = spacing as usize;
    let pad = (w.saturating_sub(1)) / 2;
    let mut out = String::new();
    out.push_str(&" ".repeat(pad));
    out.push_str(sym);
    let used = pad + 1;
    out.push_str(&" ".repeat(w.saturating_sub(used)));
    out
}

fn star_points(n: usize) -> &'static [usize] {
    match n {
        19 => &[3, 9, 15],
        13 => &[3, 6, 9],
        9 => &[2, 4, 6],
        _ => &[],
    }
}

fn draw_side(frame: &mut Frame, app: &App, area: Rect, board_mode: &str) {
    let s = strings(app.lang);
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(area);

    let wr = app.analysis().map(|a| a.winrate_black).unwrap_or(0.5);
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.winrate)))
        .gauge_style(Style::default().fg(C::Rgb(235, 235, 235)).bg(C::Rgb(45, 45, 45)))
        .ratio(wr.clamp(0.0, 1.0) as f64)
        .label(format!("{:.1}%", wr * 100.0));
    frame.render_widget(gauge, chunks[0]);

    let score = app.analysis().map(|a| a.score_black).unwrap_or(0.0);
    let who = if !app.ai_enabled {
        s.review.to_string()
    } else if app.to_move() == app.human {
        s.you.to_string()
    } else {
        s.ai.to_string()
    };
    let info = Paragraph::new(vec![
        Line::from(format!("{}   - {score:+.1}", s.score)),
        Line::from(format!("{}   {} / {}", s.moves, app.display_pos(), app.total_moves())),
        Line::from(format!(
            "{}   {}({})",
            s.turn,
            color_name(app.display_to_move(), app.lang),
            who
        )),
        Line::from(format!("{}   {} visits", s.strength, app.config.engine.visits)),
        Line::from(format!(
            "{}   {}",
            s.human_net,
            if app.human_enabled {
                app.human_profile()
            } else {
                "-"
            }
        )),
        Line::from(format!("{}   {board_mode}", s.board_mode)),
    ])
    .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.position)));
    frame.render_widget(info, chunks[1]);

    let mut lines = Vec::new();
    let pred = app.prediction();
    if let Some((step, total)) = app.preview_progress() {
        lines.push(Line::from(format!("{} {step}/{total}", s.variation)));
        lines.push(Line::from(s.variation_hint));
    } else if pred.is_empty() {
        lines.push(Line::from(s.press_a));
    } else {
        for (i, (mv, wr)) in pred.iter().enumerate() {
            lines.push(Line::from(format!(
                "{}. {:<4} {:.1}%",
                i + 1,
                move_text(*mv, app.size),
                wr * 100.0
            )));
        }
        if let Some(a) = app.analysis() {
            if let Some(best) = a.candidates.first() {
                let pv: Vec<String> = best
                    .pv
                    .iter()
                    .take(5)
                    .map(|m| move_text(*m, app.size))
                    .collect();
                if !pv.is_empty() {
                    lines.push(Line::from(format!("{} {}", s.line, pv.join(" "))));
                }
            }
        }
    }
    let cand = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.predict)))
        .wrap(Wrap { trim: true });
    frame.render_widget(cand, chunks[2]);

    let mut text = String::new();
    if let Some(c) = &app.last_comment {
        text.push_str(c);
    }
    if app.ai_enabled && app.to_move() == app.human {
        if let Some(a) = app.analysis() {
            if let Some(best) = a.candidates.first() {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&format!(
                    "{} {}（{:.1}%）",
                    s.suggestion,
                    move_text(best.mv, app.size),
                    best.winrate_black * 100.0
                ));
                let pv: Vec<String> = best
                    .pv
                    .iter()
                    .take(4)
                    .map(|m| move_text(*m, app.size))
                    .collect();
                if !pv.is_empty() {
                    text.push_str(&format!("，{} {}", s.line, pv.join(" ")));
                }
                text.push('。');
            }
        }
    }
    if text.is_empty() {
        text.push_str(s.fallback_comment);
    }
    let c = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.commentary)))
        .wrap(Wrap { trim: true });
    frame.render_widget(c, chunks[3]);

    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(C::Gray))
        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.status)));
    frame.render_widget(status, chunks[4]);
}

fn draw_games(frame: &mut Frame, app: &App, area: Rect) {
    let s = strings(app.lang);
    let w = 52.min(area.width);
    let h = ((app.games.len() as u16) + 4).clamp(7.min(area.height), area.height);
    let rect = centered(area, w, h);
    frame.render_widget(Clear, rect);

    let mut lines = Vec::new();
    if app.games.is_empty() {
        lines.push(Line::from(s.no_games));
    } else {
        for (i, g) in app.games.iter().enumerate() {
            let style = if i == app.game_cursor {
                Style::default().fg(C::Black).bg(C::Rgb(230, 200, 150))
            } else {
                Style::default()
            };
            let name = g
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            lines.push(Line::styled(format!("{}  {}", g.label, name), style));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        s.games_hint,
        Style::default().fg(C::Gray),
    )));
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.games)));
    frame.render_widget(p, rect);
}

fn draw_overlay(frame: &mut Frame, app: &App, area: Rect, overlay: &Overlay) {
    let s = strings(app.lang);
    let (body, hint) = match overlay {
        Overlay::ConfirmDelete => (s.trash_confirm.to_string(), "d / Esc"),
        Overlay::Rename(text) => (format!("{}: {}_", s.rename, text), s.rename_hint),
    };
    let w = 60.min(area.width);
    let h = 7.min(area.height);
    let rect = centered(area, w, h);
    frame.render_widget(Clear, rect);
    let lines = vec![
        Line::from(Span::styled(
            body,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(hint, Style::default().fg(C::Gray))),
    ];
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.games)))
        .wrap(Wrap { trim: true });
    frame.render_widget(p, rect);
}

fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let s = strings(app.lang);
    let mut text = vec![Line::from(Span::styled(
        s.help,
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    text.push(Line::from(""));
    for (key, action) in s.help_lines {
        text.push(Line::from(format!("{key:<16} {action}")));
    }
    let w = 56.min(area.width);
    let h = (text.len() as u16 + 2).min(area.height);
    let rect = centered(area, w, h);
    frame.render_widget(Clear, rect);
    let p = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.help)))
        .style(Style::default().bg(C::Black).fg(C::Gray));
    frame.render_widget(p, rect);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::storage::GameEntry;
    use app::{App, AppConfig};
    use ratatui::{backend::TestBackend, Terminal};

    fn test_app() -> App {
        let sg = sgf::parse("(;SZ[9]KM[7.5];B[cc];W[gg];B[dd])").unwrap();
        App::from_sgf(&sg, AppConfig::default()).unwrap()
    }

    fn render_to_string(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut image = None;
        terminal.draw(|f| draw(f, app, &mut image, None)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    fn compact(s: String) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn renders_board_and_panels() {
        let app = test_app();
        let s = compact(render_to_string(&app, 100, 30));
        assert!(s.contains("对局") || s.contains("Game"));
        assert!(s.matches('●').count() >= 2); // 黑白都用实心圆，靠颜色区分
        assert!(s.contains('·'));
        assert!(s.contains('A'));
    }

    #[test]
    fn commentary_panel_shows_text() {
        let mut app = test_app();
        app.last_comment = Some("特点：二路，贴着对方下（接触战）。".to_string());
        let s = compact(render_to_string(&app, 110, 40));
        assert!(s.contains("特点"));
    }

    #[test]
    fn help_overlay_toggles() {
        let mut app = test_app();
        app.show_help = true;
        let s = compact(render_to_string(&app, 110, 40));
        assert!(s.contains("帮助") || s.contains("Help"));
    }

    #[test]
    fn games_overlay_lists_entries() {
        let mut app = test_app();
        app.show_games = true;
        app.games = vec![GameEntry {
            path: std::path::PathBuf::from("/tmp/game-1.sgf"),
            label: "01-02 03:04".to_string(),
        }];
        let s = compact(render_to_string(&app, 110, 40));
        assert!(s.contains("01-02"));
    }

    #[test]
    fn games_overlay_empty_message() {
        use app::i18n::Lang;
        let mut app = test_app();
        app.show_games = true;

        app.lang = Lang::Zh;
        let zh = compact(render_to_string(&app, 110, 40));
        assert!(zh.contains("还没有保存的棋谱"));

        app.lang = Lang::En;
        let en = compact(render_to_string(&app, 110, 40));
        assert!(en.contains("Nosavedgamesyet"));
    }

    #[test]
    fn scale_keeps_square_aspect() {
        assert_eq!(choose_scale(200, 80, 19), Some((6, 3)));
        assert_eq!(choose_scale(100, 40, 19), Some((4, 2)));
        assert_eq!(choose_scale(60, 40, 19), Some((2, 1)));
        assert_eq!(choose_scale(30, 12, 19), None);
    }

    #[test]
    fn overlays_render_management_prompts() {
        let app = test_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut image = None;
        terminal
            .draw(|f| {
                draw(
                    f,
                    &app,
                    &mut image,
                    Some(&Overlay::Rename("opening study".to_string())),
                )
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("opening study"));
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let app = test_app();
        let _ = render_to_string(&app, 20, 8);
    }

    #[test]
    fn tiny_games_overlay_does_not_panic() {
        let mut app = test_app();
        app.show_games = true;
        let _ = render_to_string(&app, 20, 4);
    }
}
