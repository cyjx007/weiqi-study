//! 棋盘绘制与命中测试：纯 Cairo + `App` 快照，不碰 GTK 控件。

use app::App;
use goban::Point;
use gtk::cairo;

const WOOD: (f64, f64, f64) = (0.85, 0.68, 0.42);
const GRID: (f64, f64, f64) = (0.18, 0.12, 0.05);
const BG: (f64, f64, f64) = (0.10, 0.10, 0.11);

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

/// 把窗口坐标换算成最近的交叉点；离交叉点太远返回 None。
pub fn point_at(x: f64, y: f64, w: f64, h: f64, n: f64) -> Option<Point> {
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
    Some(Point::new(gx as u8, gy as u8))
}

fn rgb(cr: &cairo::Context, c: (f64, f64, f64)) {
    cr.set_source_rgb(c.0, c.1, c.2);
}

pub fn draw_board(cr: &cairo::Context, w: f64, h: f64, app: &App, hover: Option<Point>) {
    rgb(cr, BG);
    cr.paint().ok();

    let n = app.size as f64;
    let g = geometry(w, h, n);
    let pad = g.cell * 1.35;

    rgb(cr, WOOD);
    cr.rectangle(
        g.ox - pad,
        g.oy - pad,
        g.board + 2.0 * pad,
        g.board + 2.0 * pad,
    );
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
            cr.new_sub_path();
            cr.arc(x, y, g.cell * 0.09, 0.0, std::f64::consts::TAU);
            cr.fill().ok();
        }
    }

    // 坐标
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(g.cell * 0.36);
    const COLS: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";
    // 横轴：A–T（跳过 I），和 GTP/讲解里的 Q5、R5 对应
    for (i, col) in COLS.iter().enumerate().take(n as usize) {
        let label = (*col as char).to_string();
        let ext = cr.text_extents(&label).map(|e| e.width()).unwrap_or(0.0);
        let x = g.ox + i as f64 * g.cell - ext / 2.0;
        cr.move_to(x, g.oy - g.cell * 0.45);
        cr.show_text(&label).ok();
        cr.move_to(x, g.oy + g.board + g.cell * 0.85);
        cr.show_text(&label).ok();
    }
    // 纵轴：19 在上、1 在下
    for i in 0..n as usize {
        let label = (n as usize - i).to_string();
        let ext = cr.text_extents(&label).map(|e| e.width()).unwrap_or(0.0);
        let y = g.oy + i as f64 * g.cell + g.cell * 0.15;
        cr.move_to(g.ox - g.cell * 0.55 - ext, y);
        cr.show_text(&label).ok();
        cr.move_to(g.ox + g.board + g.cell * 0.45, y);
        cr.show_text(&label).ok();
    }
    // 文字用的 move_to 会留下当前点，后面画圆必须先开新子路径，
    // 否则 arc() 会自动从那个点连一条线过来（就是那条斜线）。
    cr.new_path();

    let board = &app.display_game().position().board;
    // 候选
    for (rank, (mv, _)) in app.prediction().iter().enumerate() {
        if let goban::Move::Play(p) = mv {
            let x = g.ox + p.x as f64 * g.cell;
            let y = g.oy + p.y as f64 * g.cell;
            cr.set_source_rgba(0.15, 0.45, 0.85, 0.45);
            cr.new_sub_path();
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
    if let Some(goban::Move::Play(p)) = app.display_game().position().last_move {
        let x = g.ox + p.x as f64 * g.cell;
        let y = g.oy + p.y as f64 * g.cell;
        cr.set_source_rgba(0.85, 0.15, 0.15, 0.9);
        cr.new_sub_path();
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
        cr.new_sub_path();
        cr.arc(x, y, g.cell * 0.5, 0.0, std::f64::consts::TAU);
        cr.stroke().ok();
    }
}

fn draw_stone(cr: &cairo::Context, x: f64, y: f64, r: f64, black: bool) {
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
    cr.new_sub_path();
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
    cr.new_sub_path();
    cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
    cr.fill().ok();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    cr.set_line_width(1.0);
    cr.new_sub_path();
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
        assert_eq!(p, Point::new(3, 3));
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
