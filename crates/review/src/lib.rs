//! 讲评：把引擎评估翻译成人话。
//!
//! 纯函数、确定性输出（不读时钟、不随机），方便做快照测试。

use engine::{gtp_coord, TurnEval};
use goban::{Color, Game, Move, Point};

#[derive(Clone, Debug)]
pub struct MoveDelta {
    pub turn: u16,
    pub color: Color,
    pub mv: Move,
    /// 落子方视角的胜率（%）
    pub winrate_before: f32,
    pub winrate_after: f32,
    /// 正数 = 亏损的百分点
    pub loss: f32,
    pub score_before: f32,
    pub score_after: f32,
    pub best: Option<(Move, f32)>,
    pub best_pv: Vec<Move>,
}

#[derive(Clone, Debug)]
pub struct PhaseStat {
    pub label: &'static str,
    pub from: u16,
    pub to: u16,
    pub moves: u16,
    pub avg_loss: f32,
    pub worst_loss: f32,
    pub worst_turn: u16,
}

#[derive(Clone, Debug)]
pub struct Review {
    pub size: u8,
    pub header: String,
    pub deltas: Vec<MoveDelta>,
    pub phases: Vec<PhaseStat>,
    /// deltas 中失误的下标，按严重程度降序
    pub blunders: Vec<usize>,
    pub summary: Vec<String>,
    pub details: Vec<String>,
}

/// 把「回合 + 黑方视角胜率」翻译成落子方视角的胜率（%）。
fn winrate_for(color: Color, winrate_black: f32) -> f32 {
    let w = if color == Color::Black {
        winrate_black
    } else {
        1.0 - winrate_black
    };
    w * 100.0
}

fn score_for(color: Color, score_black: f32) -> f32 {
    if color == Color::Black {
        score_black
    } else {
        -score_black
    }
}

fn move_text(mv: Move, size: u8) -> String {
    match mv {
        Move::Play(p) => gtp_coord(p, size),
        Move::Pass => "停一手".to_string(),
    }
}

fn verdict(loss: f32) -> &'static str {
    if loss >= 15.0 {
        "这手是明显失误。"
    } else if loss >= 8.0 {
        "这手亏了不少。"
    } else if loss >= 5.0 {
        "这手有点缓。"
    } else if loss >= 2.0 {
        "小问题。"
    } else if loss <= -4.0 {
        "这手不错。"
    } else {
        ""
    }
}

pub fn build(game: &Game, evals: &[TurnEval], threshold: f32) -> Review {
    let size = game.size();
    let n = game.moves().len();
    let mut deltas = Vec::with_capacity(n);

    for i in 1..=n {
        let before = &evals[i - 1];
        let after = &evals[i];
        let color = game.color_of_move(i as u16);
        let wr_before = winrate_for(color, before.winrate_black);
        let wr_after = winrate_for(color, after.winrate_black);
        let best = before.candidates.first().map(|c| {
            (
                c.mv,
                winrate_for(color, c.winrate_black),
            )
        });
        deltas.push(MoveDelta {
            turn: i as u16,
            color,
            mv: game.moves()[i - 1],
            winrate_before: wr_before,
            winrate_after: wr_after,
            loss: wr_before - wr_after,
            score_before: score_for(color, before.score_black),
            score_after: score_for(color, after.score_black),
            best,
            best_pv: before
                .candidates
                .first()
                .map(|c| c.pv.clone())
                .unwrap_or_default(),
        });
    }

    let threshold = threshold.max(0.0);
    let mut blunders: Vec<usize> = (0..deltas.len())
        .filter(|&i| deltas[i].loss >= threshold)
        .collect();
    blunders.sort_by(|&a, &b| deltas[b].loss.partial_cmp(&deltas[a].loss).unwrap());

    // 阶段划分：按盘面大小缩放（19 路：布局 1–50、中盘 51–150、官子 151+）
    let scale = (size as f32 / 19.0).powi(2);
    let t1 = (50.0 * scale).round().max(6.0) as u16;
    let t2 = (150.0 * scale).round().max(20.0) as u16;
    let bounds = [("布局", 1u16, t1), ("中盘", t1 + 1, t2), ("官子", t2 + 1, n as u16)];
    let mut phases = Vec::new();
    for (label, from, to) in bounds {
        let slice: Vec<&MoveDelta> = deltas
            .iter()
            .filter(|d| d.turn >= from && d.turn <= to)
            .collect();
        if slice.is_empty() {
            continue;
        }
        let avg = slice.iter().map(|d| d.loss).sum::<f32>() / slice.len() as f32;
        let worst = slice
            .iter()
            .max_by(|a, b| a.loss.partial_cmp(&b.loss).unwrap())
            .unwrap();
        phases.push(PhaseStat {
            label,
            from,
            to,
            moves: slice.len() as u16,
            avg_loss: avg,
            worst_loss: worst.loss,
            worst_turn: worst.turn,
        });
    }

    let header = format!(
        "{} 路 · 贴目 {:.1} · 共 {} 手",
        size,
        game.komi(),
        n
    );

    let mut summary = Vec::new();
    if n > 0 {
        let avg = deltas.iter().map(|d| d.loss).sum::<f32>() / n as f32;
        summary.push(format!("整体：平均每手损失 {avg:.1} 个百分点。"));
        for p in &phases {
            summary.push(format!(
                "{}（第 {}–{} 手）：平均 {:.1}，最差 {:+.1}（第 {} 手）",
                p.label, p.from, p.to, p.avg_loss, -p.worst_loss, p.worst_turn
            ));
        }
        if let Some(&worst) = blunders.first() {
            let d = &deltas[worst];
            summary.push(format!(
                "最大失误：第 {} 手 {} {}（{:+.1}）",
                d.turn,
                d.color.name_zh(),
                move_text(d.mv, size),
                -d.loss
            ));
        }
        if let Some(best) = deltas
            .iter()
            .min_by(|a, b| a.loss.partial_cmp(&b.loss).unwrap())
        {
            if best.loss < -2.0 && n >= 10 {
                summary.push(format!(
                    "下得最好的一手：第 {} 手 {} {}（{:+.1}）",
                    best.turn,
                    best.color.name_zh(),
                    move_text(best.mv, size),
                    -best.loss
                ));
            }
        }
    }

    let mut details = Vec::new();
    for &i in blunders.iter().take(5) {
        let d = &deltas[i];
        let mut s = format!(
            "第 {} 手 {} {}：胜率 {:.1}% → {:.1}%（{:+.1}）。{}",
            d.turn,
            d.color.name_zh(),
            move_text(d.mv, size),
            d.winrate_before,
            d.winrate_after,
            -d.loss,
            verdict(d.loss)
        );
        if let Some((best_mv, best_wr)) = d.best {
            if best_mv != d.mv {
                s.push_str(&format!(
                    "\n    当时该下 {}（{:.1}%）",
                    move_text(best_mv, size),
                    best_wr
                ));
                if !d.best_pv.is_empty() {
                    let pv: Vec<String> = d
                        .best_pv
                        .iter()
                        .take(6)
                        .map(|m| move_text(*m, size))
                        .collect();
                    s.push_str(&format!("，之后大致 {}", pv.join(" ")));
                }
                s.push('。');
            }
        }
        s.push_str(&format!(
            "\n    目差 {:+.1} → {:+.1}。",
            d.score_before, d.score_after
        ));
        details.push(s);
    }

    Review {
        size,
        header,
        deltas,
        phases,
        blunders,
        summary,
        details,
    }
}

/// 一手棋的即时点评（TUI 每步调用，不依赖整盘 Review）。
pub fn quick_comment(
    before: &TurnEval,
    after: &TurnEval,
    mover: Color,
    mv: Move,
    size: u8,
) -> String {
    let wr_before = winrate_for(mover, before.winrate_black);
    let wr_after = winrate_for(mover, after.winrate_black);
    let loss = wr_before - wr_after;
    let zone = match mv {
        Move::Play(p) => format!("（{}）", zone_name(p, size)),
        Move::Pass => String::new(),
    };
    let mut s = format!(
        "{}{}：胜率 {:.1}% → {:.1}%（{:+.1}）。",
        move_text(mv, size),
        zone,
        wr_before,
        wr_after,
        -loss
    );
    match before.candidates.first() {
        Some(best) if best.mv != mv => {
            s.push_str(&format!(
                "推荐 {}（{:.1}%）",
                move_text(best.mv, size),
                winrate_for(mover, best.winrate_black)
            ));
            if !best.pv.is_empty() {
                let pv: Vec<String> = best
                    .pv
                    .iter()
                    .take(4)
                    .map(|m| move_text(*m, size))
                    .collect();
                s.push_str("，之后大致 ");
                s.push_str(&pv.join(" "));
            }
            s.push('。');
        }
        Some(_) => s.push_str("和推荐一致。"),
        None => {}
    }
    s
}

fn zone_name(p: Point, size: u8) -> &'static str {
    let dx = p.x.min(size.saturating_sub(1).saturating_sub(p.x));
    let dy = p.y.min(size.saturating_sub(1).saturating_sub(p.y));
    if dx <= 3 && dy <= 3 {
        "角"
    } else if dx <= 3 || dy <= 3 {
        "边"
    } else {
        "中腹"
    }
}

impl Review {
    /// 文本渲染：表格（只显示超过阈值的手）+ 总结 + 失误详情。
    pub fn render(&self, min_loss: f32, show_all: bool) -> String {
        let mut out = String::new();
        out.push_str(&format!("{}\n", self.header));
        out.push_str("\n手数  执  落点   胜率变化                    目差变化\n");
        out.push_str("─────────────────────────────────────────────────────────────\n");
        for d in &self.deltas {
            if !show_all && d.loss < min_loss {
                continue;
            }
            out.push_str(&format!(
                "{:>4}  {}  {:<4}  {:5.1}% → {:5.1}%  ({:+.1})   {:+.1} → {:+.1}\n",
                d.turn,
                d.color.name_zh(),
                self.coord(d),
                d.winrate_before,
                d.winrate_after,
                -d.loss,
                d.score_before,
                d.score_after
            ));
        }
        out.push('\n');
        for line in &self.summary {
            out.push_str(line);
            out.push('\n');
        }
        if !self.details.is_empty() {
            out.push_str("\n──── 失误详情 ────\n");
            for d in &self.details {
                out.push_str(d);
                out.push('\n');
            }
        }
        out
    }

    fn coord(&self, d: &MoveDelta) -> String {
        // 盘面大小在 header 里，这里用坐标长度推断：GTP 坐标不含盘面信息，
        // 直接按 Move 的 Point 还原（19 路默认，9/13 路也能正确显示列字母）。
        match d.mv {
            Move::Pass => "停".to_string(),
            Move::Play(p) => gtp_coord(p, self.size),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Candidate;
    use goban::{Game, Move, Point};

    fn fake_eval(turn: u16, to_move: Color, wr_black: f32) -> TurnEval {
        TurnEval {
            turn,
            to_move,
            winrate_black: wr_black,
            score_black: 0.0,
            candidates: vec![Candidate {
                mv: Move::Play(Point::new(15, 3)),
                winrate_black: wr_black,
                score_black: 0.0,
                pv: vec![Move::Play(Point::new(15, 3))],
                visits: 100,
            }],
        }
    }

    #[test]
    fn computes_loss_from_mover_perspective() {
        let mut game = Game::new(19, 7.5);
        game.play(Move::Play(Point::new(15, 3))).unwrap(); // B
        game.play(Move::Play(Point::new(3, 15))).unwrap(); // W
        // 黑方胜率 60% -> 40%
        let evals = vec![
            fake_eval(0, Color::Black, 0.60),
            fake_eval(1, Color::White, 0.40),
            fake_eval(2, Color::Black, 0.40),
        ];
        let r = build(&game, &evals, 5.0);
        // 黑方胜率 60% -> 40%，亏 20 个点
        assert!((r.deltas[0].loss - 20.0).abs() < 0.01);
        // 白方视角：60% -> 60%，没亏没赚
        assert!(r.deltas[1].loss.abs() < 0.01);
    }

    #[test]
    fn render_is_deterministic() {
        let mut game = Game::new(19, 7.5);
        game.play(Move::Play(Point::new(15, 3))).unwrap();
        let evals = vec![
            fake_eval(0, Color::Black, 0.5),
            fake_eval(1, Color::White, 0.5),
        ];
        let r = build(&game, &evals, 5.0);
        assert_eq!(r.render(5.0, true), r.render(5.0, true));
    }
}
