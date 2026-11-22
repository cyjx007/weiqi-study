//! TUI 图片棋盘：把局面画成 RGBA 位图，再交给 ratatui-image 输出。
//!
//! 图片按棋盘区域的实际像素尺寸生成并居中，避免缩放造成线条粗细不均；
//! 坐标文字用 `font8x8` 点阵字体，避免依赖系统字体。

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use app::App;
use font8x8::{UnicodeFonts, BASIC_FONTS};
use goban::Move;
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

const MIN_SIDE: u32 = 240;
const MAX_SIDE: u32 = 900;
const GTP_COLS: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";

const WOOD: [u8; 3] = [214, 176, 124];
const GRID: [u8; 4] = [126, 96, 62, 255];
const STAR: [u8; 4] = [64, 40, 16, 255];
const COORD: [u8; 4] = [70, 45, 18, 255];
const CAND_BG: [u8; 4] = [30, 105, 210, 150];
const WHITE_TEXT: [u8; 4] = [255, 255, 255, 255];
const CURSOR: [u8; 4] = [45, 140, 245, 230];
const LAST: [u8; 4] = [215, 40, 40, 235];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Auto,
    Image,
    Text,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(Mode::Auto),
            "image" => Ok(Mode::Image),
            "text" => Ok(Mode::Text),
            other => Err(format!("--mode 只能是 auto|image|text，收到 {other}")),
        }
    }
}

/// 图片棋盘状态：终端协议 + 当前局面的协议缓存。
pub struct ImageBoard {
    picker: Picker,
    protocol: Option<StatefulProtocol>,
    fingerprint: Option<u64>,
    /// 当前源图的像素边长；区域变化时按新尺寸重画，避免缩放
    side: u32,
    protocol_name: &'static str,
}

impl ImageBoard {
    /// 按 `--mode` 决定是否启用图片棋盘。
    ///
    /// - `text`：始终返回 `None`
    /// - `auto`：探测失败或只支持 halfblocks 时返回 `None`（由调用方回退文本）
    /// - `image`：探测失败直接报错
    pub fn new(mode: Mode) -> Result<Option<Self>, String> {
        if mode == Mode::Text {
            return Ok(None);
        }
        // 不支持图片的终端不会回任何响应，别让启动卡 2 秒：
        // 先看环境变量判断是不是常见图形终端；是的话等满 2s（Kitty 在
        // 大窗口/首次查询时可能 >300ms），否则 auto 只等 500ms。
        let timeout = if mode == Mode::Image || likely_image_terminal() {
            Duration::from_millis(2000)
        } else {
            Duration::from_millis(500)
        };
        let picker = match Picker::from_query_stdio_with_options(QueryStdioOptions {
            timeout,
            ..Default::default()
        }) {
            Ok(p) => p,
            Err(e) => {
                return match mode {
                    Mode::Image => Err(format!("终端不支持图片协议：{e}")),
                    _ => Ok(None),
                };
            }
        };
        // 有些终端不回答 CSI 16t（字体像素尺寸），ratatui-image 会退回
        // Halfblocks；环境变量已经明确是 kitty/iTerm2/sixel 时按环境强制协议。
        let mut picker = picker;
        if picker.protocol_type() == ProtocolType::Halfblocks {
            if let Some(proto) = env_protocol() {
                picker.set_protocol_type(proto);
            }
        }
        if picker.protocol_type() == ProtocolType::Halfblocks {
            return match mode {
                Mode::Image => {
                    Err("终端只支持 halfblocks；图片模式需要 kitty / sixel / iTerm2".to_string())
                }
                _ => Ok(None),
            };
        }
        let protocol_name = match picker.protocol_type() {
            ProtocolType::Kitty => "kitty",
            ProtocolType::Sixel => "sixel",
            ProtocolType::Iterm2 => "iTerm2",
            ProtocolType::Halfblocks => "halfblocks",
        };
        Ok(Some(ImageBoard {
            picker,
            protocol: None,
            fingerprint: None,
            side: 0,
            protocol_name,
        }))
    }

    pub fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }

    /// 把棋盘画到 `area`；局面变化或区域变化时重新生成并编码图片。
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, app: &App) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let side = self.target_side(area);
        let fp = fingerprint(app);
        if self.fingerprint != Some(fp) || self.side != side {
            let img = render_board(app, side);
            self.protocol = Some(
                self.picker
                    .new_resize_protocol(DynamicImage::ImageRgba8(img)),
            );
            self.fingerprint = Some(fp);
            self.side = side;
        }
        if let Some(state) = self.protocol.as_mut() {
            // StatefulImage 从区域左上角开始画；按实际 cell 尺寸算一个居中矩形
            let size = state.size_for(
                Resize::Fit(None),
                ratatui::layout::Size::new(area.width, area.height),
            );
            let rect = centered_rect(area, size.width, size.height);
            frame.render_stateful_widget(
                StatefulImage::new().resize(Resize::Fit(None)),
                rect,
                state,
            );
        }
    }

    /// 目标源图边长：取区域实际像素的短边，限制在 240–900。
    fn target_side(&self, area: Rect) -> u32 {
        let font = self.picker.font_size();
        let w = area.width as u32 * font.width.max(1) as u32;
        let h = area.height as u32 * font.height.max(1) as u32;
        w.min(h).clamp(MIN_SIDE, MAX_SIDE)
    }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// 终端环境变量给出的协议提示（查询没拿到字体尺寸时兜底）。
fn env_protocol() -> Option<ProtocolType> {
    if std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM").is_ok_and(|t| t.to_ascii_lowercase().contains("kitty"))
    {
        return Some(ProtocolType::Kitty);
    }
    if std::env::var_os("WEZTERM_EXECUTABLE").is_some()
        || std::env::var("TERM").is_ok_and(|t| t.to_ascii_lowercase().contains("wezterm"))
    {
        // WezTerm 支持 kitty graphics
        return Some(ProtocolType::Kitty);
    }
    if std::env::var_os("ITERM_SESSION_ID").is_some()
        || std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "iTerm.app")
    {
        return Some(ProtocolType::Iterm2);
    }
    if std::env::var("TERM").is_ok_and(|t| {
        let t = t.to_ascii_lowercase();
        t.contains("sixel") || t.contains("foot") || t.contains("mlterm")
    }) {
        return Some(ProtocolType::Sixel);
    }
    None
}

/// 常见支持图片协议的终端；用来决定 auto 查询等多久。
fn likely_image_terminal() -> bool {
    env_protocol().is_some()
}

/// 影响棋盘画面的状态；任何一项变化都要重新生成图片。
fn fingerprint(app: &App) -> u64 {
    let mut h = DefaultHasher::new();
    app.display_pos().hash(&mut h);
    app.preview_progress().hash(&mut h);
    app.size.hash(&mut h);
    app.cursor.x.hash(&mut h);
    app.cursor.y.hash(&mut h);
    match app.display_game().position().last_move {
        Some(Move::Play(p)) => {
            1u8.hash(&mut h);
            p.x.hash(&mut h);
            p.y.hash(&mut h);
        }
        Some(Move::Pass) => 2u8.hash(&mut h),
        None => 0u8.hash(&mut h),
    }
    for (mv, wr) in app.prediction() {
        match mv {
            Move::Play(p) => {
                1u8.hash(&mut h);
                p.x.hash(&mut h);
                p.y.hash(&mut h);
            }
            Move::Pass => 2u8.hash(&mut h),
        }
        wr.to_bits().hash(&mut h);
    }
    if let Some(a) = app.analysis() {
        a.winrate_black.to_bits().hash(&mut h);
        a.score_black.to_bits().hash(&mut h);
    }
    h.finish()
}

struct Metrics {
    margin: f64,
    cell: f64,
}

fn metrics(size: u8, side: f64) -> Metrics {
    let margin = (side * 0.09).max(10.0);
    let board = side - 2.0 * margin;
    Metrics {
        margin,
        cell: board / (size as f64 - 1.0).max(1.0),
    }
}

fn center(m: &Metrics, p: goban::Point) -> (i32, i32) {
    (
        (m.margin + p.x as f64 * m.cell).round() as i32,
        (m.margin + p.y as f64 * m.cell).round() as i32,
    )
}

fn render_board(app: &App, side: u32) -> RgbaImage {
    let mut canvas = Canvas::new(side, side, [WOOD[0], WOOD[1], WOOD[2], 255]);
    let m = metrics(app.size, side as f64);
    let board = m.cell * (app.size as f64 - 1.0);
    let n = app.size as i32;

    // 网格
    for i in 0..n {
        let p = (m.margin + i as f64 * m.cell).round() as i32;
        let start = m.margin.round() as i32;
        let end = (m.margin + board).round() as i32;
        canvas.line(p, start, p, end, GRID, 2);
        canvas.line(start, p, end, p, GRID, 2);
    }

    // 星位
    let stars = star_points(app.size as usize);
    for &sx in stars {
        for &sy in stars {
            let x = (m.margin + sx as f64 * m.cell).round() as i32;
            let y = (m.margin + sy as f64 * m.cell).round() as i32;
            canvas.circle(x, y, (m.cell * 0.09).round() as i32, STAR);
        }
    }

    // 坐标：数字/字母点阵字，按格子大小缩放
    let scale = ((m.cell * 0.32 / 8.0).round() as i32).clamp(1, 3);
    let glyph = 8 * scale;
    for (i, col) in GTP_COLS.iter().enumerate().take(n as usize) {
        let x = (m.margin + i as f64 * m.cell).round() as i32;
        let letter = (*col as char).to_string();
        let w = text_width(&letter, scale);
        canvas.text(
            &letter,
            x - w / 2,
            m.margin as i32 - glyph - 12,
            scale,
            COORD,
        );
        canvas.text(
            &letter,
            x - w / 2,
            (m.margin + board) as i32 + 12,
            scale,
            COORD,
        );

        let row = (n as usize - i).to_string();
        let w = text_width(&row, scale);
        let y = (m.margin + i as f64 * m.cell).round() as i32 - glyph / 2;
        canvas.text(&row, m.margin as i32 - w - 12, y, scale, COORD);
        canvas.text(&row, (m.margin + board) as i32 + 12, y, scale, COORD);
    }

    // 候选点
    for (rank, (mv, _)) in app.prediction().iter().enumerate() {
        if let Move::Play(p) = mv {
            let (x, y) = center(&m, *p);
            canvas.circle(x, y, (m.cell * 0.42).round() as i32, CAND_BG);
            let label = (rank + 1).to_string();
            let w = text_width(&label, scale);
            canvas.text(&label, x - w / 2, y - glyph / 2, scale, WHITE_TEXT);
        }
    }

    // 棋子
    for (p, c) in app.display_game().position().board.stones() {
        let (x, y) = center(&m, p);
        draw_stone(
            &mut canvas,
            x,
            y,
            (m.cell * 0.46).round() as i32,
            c == goban::Color::Black,
        );
    }

    // 最后一手
    if let Some(Move::Play(p)) = app.display_game().position().last_move {
        let (x, y) = center(&m, p);
        canvas.circle(x, y, (m.cell * 0.12).round() as i32, LAST);
    }

    // 光标
    let (x, y) = center(&m, app.cursor);
    canvas.ring(x, y, (m.cell * 0.5).round() as i32, 3, CURSOR);

    canvas.img
}

fn draw_stone(c: &mut Canvas, x: i32, y: i32, r: i32, black: bool) {
    c.circle(x + 2, y + 3, r, [0, 0, 0, 70]);
    if black {
        c.circle(x, y, r, [20, 20, 23, 255]);
        c.ring(x, y, r, 1, [80, 80, 88, 255]);
        c.circle(x - r / 3, y - r / 3, (r / 5).max(1), [75, 75, 82, 255]);
    } else {
        c.circle(x, y, r, [245, 245, 242, 255]);
        c.ring(x, y, r, 1, [120, 120, 120, 255]);
    }
}

fn text_width(s: &str, scale: i32) -> i32 {
    s.chars().count() as i32 * 8 * scale
}

fn star_points(n: usize) -> &'static [usize] {
    match n {
        19 => &[3, 9, 15],
        13 => &[3, 6, 9],
        9 => &[2, 4, 6],
        _ => &[],
    }
}

struct Canvas {
    img: RgbaImage,
}

impl Canvas {
    fn new(w: u32, h: u32, bg: [u8; 4]) -> Self {
        Canvas {
            img: RgbaImage::from_pixel(w, h, Rgba(bg)),
        }
    }

    fn blend(&mut self, x: i32, y: i32, src: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.img.width() as i32 || y >= self.img.height() as i32 {
            return;
        }
        let dst = self.img.get_pixel_mut(x as u32, y as u32);
        let a = src[3] as u32;
        if a >= 255 {
            *dst = Rgba([src[0], src[1], src[2], 255]);
            return;
        }
        let ia = 255 - a;
        dst.0[0] = ((src[0] as u32 * a + dst.0[0] as u32 * ia) / 255) as u8;
        dst.0[1] = ((src[1] as u32 * a + dst.0[1] as u32 * ia) / 255) as u8;
        dst.0[2] = ((src[2] as u32 * a + dst.0[2] as u32 * ia) / 255) as u8;
        dst.0[3] = 255;
    }

    fn dot(&mut self, x: i32, y: i32, w: i32, color: [u8; 4]) {
        let w = w.max(1);
        // 2px 线要占 x-1 和 x，居中而不是偏右下
        let start = -(w / 2);
        for dy in start..start + w {
            for dx in start..start + w {
                self.blend(x + dx, y + dy, color);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: [u8; 4], w: i32) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.dot(x0, y0, w, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn circle(&mut self, cx: i32, cy: i32, r: i32, color: [u8; 4]) {
        let r2 = r * r;
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= r2 {
                    self.blend(cx + x, cy + y, color);
                }
            }
        }
    }

    fn ring(&mut self, cx: i32, cy: i32, r: i32, w: i32, color: [u8; 4]) {
        let inner = (r - w / 2).max(0);
        let outer = r + w / 2;
        let (i2, o2) = (inner * inner, outer * outer);
        for y in -outer..=outer {
            for x in -outer..=outer {
                let d = x * x + y * y;
                if d >= i2 && d <= o2 {
                    self.blend(cx + x, cy + y, color);
                }
            }
        }
    }

    fn text(&mut self, s: &str, x: i32, y: i32, scale: i32, color: [u8; 4]) {
        let mut pen = x;
        for ch in s.chars() {
            if let Some(glyph) = BASIC_FONTS.get(ch) {
                for (row, bits) in glyph.iter().enumerate() {
                    for col in 0..8 {
                        if bits & (1 << col) != 0 {
                            for dy in 0..scale {
                                for dx in 0..scale {
                                    self.blend(
                                        pen + col * scale + dx,
                                        y + row as i32 * scale + dy,
                                        color,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            pen += 8 * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::{App, AppConfig};

    fn demo(size: u8, setup: &str) -> App {
        let sg = sgf::parse(&format!("(;SZ[{size}]KM[7.5]{setup})")).unwrap();
        App::from_sgf(&sg, AppConfig::default()).unwrap()
    }

    #[test]
    fn mode_parses() {
        assert_eq!(Mode::parse("auto").unwrap(), Mode::Auto);
        assert_eq!(Mode::parse("image").unwrap(), Mode::Image);
        assert_eq!(Mode::parse("text").unwrap(), Mode::Text);
        assert!(Mode::parse("svg").is_err());
    }

    #[test]
    fn renders_stones_and_empty_points() {
        let app = demo(9, ";B[cc]");
        let img = render_board(&app, 720);
        assert_eq!(img.width(), 720);
        assert_eq!(img.height(), 720);

        let m = metrics(9, 720.0);
        let (sx, sy) = center(&m, goban::Point::new(2, 2));
        // 中心是最后一手红点，往旁边挪半格看棋子本体
        let off = (m.cell * 0.25) as i32;
        let stone = img.get_pixel((sx + off) as u32, sy as u32).0;
        assert!(stone[0] < 60 && stone[1] < 60, "落子点应是黑子: {stone:?}");
        let last = img.get_pixel(sx as u32, sy as u32).0;
        assert!(last[0] > 180 && last[1] < 80, "最后一手应是红点: {last:?}");

        let (ex, ey) = center(&m, goban::Point::new(3, 3));
        let empty = img.get_pixel((ex + off) as u32, (ey + off) as u32).0;
        assert!(empty[0] > 150, "空点应是木色: {empty:?}");
    }

    #[test]
    fn centered_rect_puts_the_board_in_the_middle() {
        let r = centered_rect(Rect::new(0, 0, 80, 40), 50, 20);
        assert_eq!((r.x, r.y, r.width, r.height), (15, 10, 50, 20));
    }

    #[test]
    fn fingerprint_tracks_cursor_and_moves() {
        let mut app = demo(9, ";B[cc]");
        let a = fingerprint(&app);
        app.move_cursor(1, 0);
        let b = fingerprint(&app);
        assert_ne!(a, b);
        app.play_at_cursor().unwrap();
        assert_ne!(b, fingerprint(&app));
    }
}
