//! 界面渲染：只读 `App` 状态，不修改任何东西。

use app::App;
use goban::{Color, Move, Point};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color as C, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
    Frame,
};

const WOOD: C = C::Rgb(196, 158, 108);
const LINE: C = C::Rgb(110, 82, 48);
const CURSOR: C = C::Rgb(90, 140, 200);
const GTP_COLS: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";
const CAND: [&str; 3] = ["1", "2", "3"];

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks =
        Layout::horizontal([Constraint::Min(42), Constraint::Length(36)]).split(area);
    draw_board(frame, app, chunks[0]);
    draw_side(frame, app, chunks[1]);
    if app.show_help {
        draw_help(frame, area);
    }
}

fn draw_board(frame: &mut Frame, app: &App, area: Rect) {
    let who = if app.ai_enabled {
        if app.to_move() == app.human { "你" } else { "AI" }
    } else {
        "复盘"
    };
    let title = format!(" 对局 · 轮到{}({}) ", app.to_move().name_zh(), who);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Block::default().style(Style::default().bg(WOOD)), inner);

    let n = app.size as usize;
    let bw = n * 2;
    let bh = n + 1;
    if inner.width < (bw + 4) as u16 || inner.height < bh as u16 {
        frame.render_widget(
            Paragraph::new("窗口太小，放大一点").style(Style::default().fg(C::Gray).bg(WOOD)),
            inner,
        );
        return;
    }
    let x0 = inner.x + (inner.width - (bw as u16 + 3)) / 2;
    let y0 = inner.y + (inner.height - bh as u16) / 2;
    let buf = frame.buffer_mut();
    let wood = Style::default().bg(WOOD);

    // 列字母
    for (j, letter) in GTP_COLS.iter().take(n).enumerate() {
        buf.set_string(
            x0 + 3 + j as u16 * 2,
            y0,
            (*letter as char).to_string(),
            wood.fg(LINE),
        );
    }

    for i in 0..n {
        let row = n - i;
        buf.set_string(
            x0,
            y0 + 1 + i as u16,
            format!("{row:>2}"),
            wood.fg(LINE),
        );
        for j in 0..n {
            let p = Point::new(j as u8, i as u8);
            let x = x0 + 3 + j as u16 * 2;
            let y = y0 + 1 + i as u16;
            let stone = app.game().position().board.at(p);
            let is_last = app.game().position().last_move == Some(Move::Play(p));
            let mut style = wood;
            if is_last {
                style = style.add_modifier(Modifier::BOLD);
            }
            if app.cursor == p {
                style = style.bg(CURSOR);
            }
            let (sym, fg) = match stone {
                Some(Color::Black) => ("●", C::Black),
                Some(Color::White) => ("○", C::White),
                None => match app.candidate_rank(p) {
                    Some(rank) => (CAND[rank - 1], C::Yellow),
                    None => ("·", LINE),
                },
            };
            buf.set_string(x, y, format!("{sym} "), style.fg(fg));
        }
    }
}

fn draw_side(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(7),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(area);

    let wr = app.analysis().map(|a| a.winrate_black).unwrap_or(0.5);
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" 胜率（黑） "))
        .gauge_style(Style::default().fg(C::Rgb(235, 235, 235)).bg(C::Rgb(45, 45, 45)))
        .ratio(wr.clamp(0.0, 1.0) as f64)
        .label(format!("{:.1}%", wr * 100.0));
    frame.render_widget(gauge, chunks[0]);

    let score = app.analysis().map(|a| a.score_black).unwrap_or(0.0);
    let who = if !app.ai_enabled {
        "复盘".to_string()
    } else if app.to_move() == app.human {
        "你".to_string()
    } else {
        "AI".to_string()
    };
    let info = Paragraph::new(vec![
        Line::from(format!("目差   黑 {score:+.1}")),
        Line::from(format!("手数   {} / {}", app.pos(), app.total_moves())),
        Line::from(format!("轮到   {}（{}）", app.to_move().name_zh(), who)),
    ])
    .block(Block::default().borders(Borders::ALL).title(" 局面 "));
    frame.render_widget(info, chunks[1]);

    let mut lines = Vec::new();
    let pred = app.prediction();
    if pred.is_empty() {
        lines.push(Line::from("按 a 分析当前局面"));
    } else {
        for (i, (mv, wr)) in pred.iter().enumerate() {
            lines.push(Line::from(format!(
                "{}. {:<4} 黑 {:.1}%",
                i + 1,
                mv_text(*mv, app.size),
                wr * 100.0
            )));
        }
        if let Some(a) = app.analysis() {
            if let Some(best) = a.candidates.first() {
                let pv: Vec<String> = best
                    .pv
                    .iter()
                    .take(5)
                    .map(|m| mv_text(*m, app.size))
                    .collect();
                if !pv.is_empty() {
                    lines.push(Line::from(format!("变化 {}", pv.join(" "))));
                }
            }
        }
    }
    let cand = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" 预测 / 候选 "))
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
                    "建议 {}（黑 {:.1}%）",
                    mv_text(best.mv, app.size),
                    best.winrate_black * 100.0
                ));
                let pv: Vec<String> = best
                    .pv
                    .iter()
                    .take(4)
                    .map(|m| mv_text(*m, app.size))
                    .collect();
                if !pv.is_empty() {
                    text.push_str(&format!("，之后大致 {}", pv.join(" ")));
                }
                text.push('。');
            }
        }
    }
    if text.is_empty() {
        text.push_str("按 t 开始人机对局；你落子后这里会写这手亏了多少、该下哪。");
    }
    let c = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(" 讲解 "))
        .wrap(Wrap { trim: true });
    frame.render_widget(c, chunks[3]);

    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(C::Gray))
        .block(Block::default().borders(Borders::ALL).title(" 状态 "));
    frame.render_widget(status, chunks[4]);
}

fn mv_text(mv: Move, size: u8) -> String {
    match mv {
        Move::Play(p) => engine::gtp_coord(p, size),
        Move::Pass => "pass".to_string(),
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "按键",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("hjkl / 方向键   移动光标"),
        Line::from("Enter           在光标处落子"),
        Line::from("u               撤回一手"),
        Line::from("n / p           下一手 / 上一手"),
        Line::from("g / G           回到开局 / 跳到末尾"),
        Line::from("t               开始人机对局（你执黑）"),
        Line::from("v               开关 AI 应手"),
        Line::from("a               分析当前局面"),
        Line::from("A               全盘复盘（生成失误清单）"),
        Line::from("b               跳到下一个失误"),
        Line::from("?               关闭帮助"),
        Line::from("q / Esc         退出"),
    ];
    let w = 44.min(area.width);
    let h = (text.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    let p = Paragraph::new(text)
        .alignment(Alignment::Left)
        .block(Block::default().borders(Borders::ALL).title(" 帮助 "))
        .style(Style::default().bg(C::Black).fg(C::Gray));
    frame.render_widget(p, rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::{App, AppConfig};
    use ratatui::{backend::TestBackend, Terminal};

    fn test_app() -> App {
        let sg = sgf::parse("(;SZ[9]KM[7.5];B[cc];W[gg];B[dd])").unwrap();
        App::from_sgf(&sg, AppConfig::default()).unwrap()
    }

    fn render_to_string(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn renders_board_and_panels() {
        let app = test_app();
        // 宽字符在 buffer 里会占两个 cell，先把空白去掉再断言
        let s: String = render_to_string(&app, 100, 30)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(s.contains("对局"));
        assert!(s.contains("胜率"));
        assert!(s.contains("候选"));
        assert!(s.contains('●')); // 黑子
        assert!(s.contains('○')); // 白子
        assert!(s.contains('A')); // 列字母
    }

    #[test]
    fn help_overlay_toggles() {
        let mut app = test_app();
        app.show_help = true;
        let s: String = render_to_string(&app, 100, 30)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(s.contains("按键"));
        assert!(s.contains("全盘复盘"));
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let app = test_app();
        let _ = render_to_string(&app, 20, 8);
    }
}
