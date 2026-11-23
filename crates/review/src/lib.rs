//! 讲评：把引擎评估翻译成人话。
//!
//! 纯函数、确定性输出（不读时钟、不随机），方便做快照测试。

use engine::{gtp_coord, TurnEval};
use goban::{Color, Game, Move, Point, Position};

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
    pub english: bool,
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

/// 一手棋的战术特征（全部由棋盘本身算出来，不靠猜）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveFacts {
    pub line: u8,
    pub contact: bool,
    pub captures: u16,
    pub atari: bool,
    pub connects: bool,
    pub self_atari: bool,
}

pub fn move_facts(before: &goban::Position, mv: Move) -> Option<MoveFacts> {
    let p = match mv {
        Move::Play(p) => p,
        Move::Pass => return None,
    };
    let mover = before.to_move;
    let after = before.try_play(mv).ok()?;
    let size = before.board.size();

    // 每手的提子差最多是一整盘子（≤625），u16 足够；转换只作防御。
    let captures =
        u16::try_from(after.captures[mover.index()].saturating_sub(before.captures[mover.index()]))
            .unwrap_or(u16::MAX);
    let (_, own_liberties) = after.board.group_and_liberties(p);

    let mut contact = false;
    let mut own_neighbors = 0;
    for n in before.board.neighbors(p) {
        match before.board.at(n) {
            Some(c) if c != mover => contact = true,
            Some(_) => own_neighbors += 1,
            None => {}
        }
    }

    let mut atari = false;
    for n in after.board.neighbors(p) {
        if let Some(c) = after.board.at(n) {
            if c != mover {
                let (_, libs) = after.board.group_and_liberties(n);
                if libs.len() == 1 {
                    atari = true;
                }
            }
        }
    }

    let dx = p.x.min(size - 1 - p.x);
    let dy = p.y.min(size - 1 - p.y);

    Some(MoveFacts {
        line: 1 + dx.min(dy),
        contact,
        captures,
        atari,
        connects: own_neighbors >= 2,
        self_atari: own_liberties.len() == 1,
    })
}

fn verdict_zh(loss: f32) -> &'static str {
    if loss >= 15.0 {
        "，这手是明显失误。"
    } else if loss >= 8.0 {
        "，这手亏了不少。"
    } else if loss >= 5.0 {
        "，这手有点缓。"
    } else if loss >= 2.0 {
        "，小问题。"
    } else if loss <= -4.0 {
        "，这手不错。"
    } else {
        "。"
    }
}

fn verdict_en(loss: f32) -> &'static str {
    if loss >= 15.0 {
        " — a clear blunder."
    } else if loss >= 8.0 {
        " — a big loss."
    } else if loss >= 5.0 {
        " — a slack move."
    } else if loss >= 2.0 {
        " — a minor slip."
    } else if loss <= -4.0 {
        " — a good move."
    } else {
        "."
    }
}

fn line_name_zh(line: u8) -> &'static str {
    match line {
        1 => "一路",
        2 => "二路",
        3 => "三路",
        4 => "四路",
        _ => "中腹附近",
    }
}

fn line_name_en(line: u8) -> &'static str {
    match line {
        1 => "1st line",
        2 => "2nd line",
        3 => "3rd line",
        4 => "4th line",
        _ => "the centre",
    }
}

fn facts_text(english: bool, f: &MoveFacts) -> String {
    let mut parts = if english {
        vec![line_name_en(f.line).to_string()]
    } else {
        vec![line_name_zh(f.line).to_string()]
    };
    if f.captures > 0 {
        parts.push(if english {
            format!("captured {} stone(s)", f.captures)
        } else {
            format!("提掉 {} 子", f.captures)
        });
    }
    if f.atari && f.captures == 0 {
        parts.push(if english {
            "atari on the opponent".into()
        } else {
            "打吃对方".into()
        });
    }
    if f.connects {
        parts.push(if english {
            "connected to your stones".into()
        } else {
            "和自己的棋连上".into()
        });
    }
    if f.contact {
        parts.push(if english {
            "contact play against the opponent".into()
        } else {
            "贴着对方下（接触战）".into()
        });
    }
    if f.self_atari {
        parts.push(if english {
            "your group is left with one liberty".into()
        } else {
            "自己只剩一口气".into()
        });
    }
    if parts.len() == 1 && f.line >= 5 {
        parts.push(if english {
            "away from the edges".into()
        } else {
            "远离边角，偏向中腹".into()
        });
    }
    parts.join(if english { ", " } else { "，" })
}

fn zone_en(p: Point, size: u8) -> &'static str {
    match zone_name(p, size) {
        "角" => "corner",
        "边" => "side",
        _ => "centre",
    }
}

fn score_phrase(
    english: bool,
    mover: Color,
    human: Option<Color>,
    score: f32,
    winrate: f32,
) -> String {
    // 人机模式：只有人下的那手用"你"，AI 那手写"AI"；复盘模式（human=None）保持原文案
    let is_ai = human.is_some() && human != Some(mover);
    let who = match (english, is_ai, mover) {
        (true, true, Color::Black) => "AI (Black)",
        (true, true, Color::White) => "AI (White)",
        (true, false, Color::Black) => "you (Black)",
        (true, false, Color::White) => "you (White)",
        (false, true, Color::Black) => "AI（黑）",
        (false, true, Color::White) => "AI（白）",
        (false, false, Color::Black) => "你（黑）",
        (false, false, Color::White) => "你（白）",
    };
    if english {
        if score.abs() < 0.5 {
            format!("{who} are roughly level ({score:+.1}); win rate {winrate:.1}%")
        } else if score > 0.0 {
            format!("{who} are ahead by {score:.1} points; win rate {winrate:.1}% (the edge comes from the score)")
        } else {
            format!(
                "{who} are behind by {:.1} points; win rate {winrate:.1}%",
                -score
            )
        }
    } else if score.abs() < 0.5 {
        format!("{who}和对方差不多（{score:+.1} 目），胜率 {winrate:.1}%")
    } else if score > 0.0 {
        format!("{who}领先 {score:.1} 目，胜率 {winrate:.1}%（优势主要来自目数）")
    } else {
        format!("{who}落后 {:.1} 目，胜率 {winrate:.1}%", -score)
    }
}

/// 把整盘分析结果转成复盘。
///
/// 前置条件由函数自己检查：`evals` 必须恰好覆盖 `turn 0..=moves.len()` 且顺序一致。
/// 不满足时返回带诊断信息的错误，而不是索引越界 panic。
pub fn build(
    english: bool,
    game: &Game,
    evals: &[TurnEval],
    threshold: f32,
) -> Result<Review, String> {
    let size = game.size();
    let n = game.moves().len();
    let expected = n + 1;
    if evals.len() != expected {
        return Err(format!(
            "全盘复盘需要 {expected} 个局面结果（turn 0..={n}），实际收到 {}",
            evals.len()
        ));
    }
    if let Some((i, ev)) = evals
        .iter()
        .enumerate()
        .find(|(i, ev)| ev.turn as usize != *i)
    {
        return Err(format!(
            "复盘结果顺序不符：第 {i} 个结果的 turn 是 {}，应为 {i}",
            ev.turn
        ));
    }
    let mut deltas = Vec::with_capacity(n);

    for i in 1..=n {
        let before = &evals[i - 1];
        let after = &evals[i];
        let color = game.color_of_move(i as u16);
        let wr_before = winrate_for(color, before.winrate_black);
        let wr_after = winrate_for(color, after.winrate_black);
        let best = before
            .candidates
            .first()
            .map(|c| (c.mv, winrate_for(color, c.winrate_black)));
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
    let bounds = [
        (if english { "Opening" } else { "布局" }, 1u16, t1),
        (if english { "Middlegame" } else { "中盘" }, t1 + 1, t2),
        (if english { "Endgame" } else { "官子" }, t2 + 1, n as u16),
    ];
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

    let header = if english {
        format!("{size}×{size} · komi {:.1} · {n} moves", game.komi())
    } else {
        format!("{size} 路 · 贴目 {:.1} · 共 {n} 手", game.komi())
    };

    let mut summary = Vec::new();
    if n > 0 {
        let avg = deltas.iter().map(|d| d.loss).sum::<f32>() / n as f32;
        if english {
            summary.push(format!("Overall: average loss {avg:.1} points per move."));
            for p in &phases {
                summary.push(format!(
                    "{} (moves {}–{}): average {:.1}, worst {:+.1} (move {})",
                    p.label, p.from, p.to, p.avg_loss, -p.worst_loss, p.worst_turn
                ));
            }
        } else {
            summary.push(format!("整体：平均每手损失 {avg:.1} 个百分点。"));
            for p in &phases {
                summary.push(format!(
                    "{}（第 {}–{} 手）：平均 {:.1}，最差 {:+.1}（第 {} 手）",
                    p.label, p.from, p.to, p.avg_loss, -p.worst_loss, p.worst_turn
                ));
            }
        }
        if let Some(&worst) = blunders.first() {
            let d = &deltas[worst];
            let col = if english {
                if d.color == Color::Black {
                    "Black"
                } else {
                    "White"
                }
            } else {
                d.color.name_zh()
            };
            summary.push(if english {
                format!(
                    "Biggest blunder: move {} {} {} ({:+.1})",
                    d.turn,
                    col,
                    move_text(d.mv, size),
                    -d.loss
                )
            } else {
                format!(
                    "最大失误：第 {} 手 {} {}（{:+.1}）",
                    d.turn,
                    col,
                    move_text(d.mv, size),
                    -d.loss
                )
            });
        }
        if let Some(best) = deltas
            .iter()
            .min_by(|a, b| a.loss.partial_cmp(&b.loss).unwrap())
        {
            if best.loss < -2.0 && n >= 10 {
                let col = if english {
                    if best.color == Color::Black {
                        "Black"
                    } else {
                        "White"
                    }
                } else {
                    best.color.name_zh()
                };
                summary.push(if english {
                    format!(
                        "Best move: move {} {} {} ({:+.1})",
                        best.turn,
                        col,
                        move_text(best.mv, size),
                        -best.loss
                    )
                } else {
                    format!(
                        "下得最好的一手：第 {} 手 {} {}（{:+.1}）",
                        best.turn,
                        col,
                        move_text(best.mv, size),
                        -best.loss
                    )
                });
            }
        }
    }

    let mut details = Vec::new();
    for &i in blunders.iter().take(5) {
        let d = &deltas[i];
        let col = if english {
            if d.color == Color::Black {
                "Black"
            } else {
                "White"
            }
        } else {
            d.color.name_zh()
        };
        let mut s = if english {
            format!(
                "Move {} {} {}: win rate {:.1}% → {:.1}% ({:+.1}){}",
                d.turn,
                col,
                move_text(d.mv, size),
                d.winrate_before,
                d.winrate_after,
                -d.loss,
                verdict_en(d.loss)
            )
        } else {
            format!(
                "第 {} 手 {} {}：胜率 {:.1}% → {:.1}%（{:+.1}）{}",
                d.turn,
                col,
                move_text(d.mv, size),
                d.winrate_before,
                d.winrate_after,
                -d.loss,
                verdict_zh(d.loss)
            )
        };
        if let Some((best_mv, best_wr)) = d.best {
            if best_mv != d.mv {
                s.push_str(&if english {
                    format!(
                        "\n    Better: {} ({:.1}%)",
                        move_text(best_mv, size),
                        best_wr
                    )
                } else {
                    format!(
                        "\n    当时该下 {}（{:.1}%）",
                        move_text(best_mv, size),
                        best_wr
                    )
                });
                if !d.best_pv.is_empty() {
                    let pv: Vec<String> = d
                        .best_pv
                        .iter()
                        .take(6)
                        .map(|m| move_text(*m, size))
                        .collect();
                    s.push_str(&if english {
                        format!(", then {}.", pv.join(" "))
                    } else {
                        format!("，之后大致 {}。", pv.join(" "))
                    });
                } else {
                    s.push(if english { '.' } else { '。' });
                }
            }
        }
        s.push_str(&if english {
            format!(
                "\n    Score lead {:+.1} → {:+.1}.",
                d.score_before, d.score_after
            )
        } else {
            format!(
                "\n    目差 {:+.1} → {:+.1}。",
                d.score_before, d.score_after
            )
        });
        details.push(s);
    }

    Ok(Review {
        english,
        size,
        header,
        deltas,
        phases,
        blunders,
        summary,
        details,
    })
}

impl Review {
    /// 文本渲染：表格（只显示超过阈值的手）+ 总结 + 失误详情。
    pub fn render(&self, min_loss: f32, show_all: bool) -> String {
        let mut out = String::new();
        out.push_str(&format!("{}\n", self.header));
        if self.english {
            out.push_str("\nMove  Col  Point  Win rate                    Score\n");
        } else {
            out.push_str("\n手数  执  落点   胜率变化                    目差变化\n");
        }
        out.push_str("─────────────────────────────────────────────────────────────\n");
        for d in &self.deltas {
            if !show_all && d.loss < min_loss {
                continue;
            }
            let col = if self.english {
                if d.color == Color::Black {
                    "B"
                } else {
                    "W"
                }
            } else {
                d.color.name_zh()
            };
            out.push_str(&format!(
                "{:>4}  {}  {:<4}  {:5.1}% → {:5.1}%  ({:+.1})   {:+.1} → {:+.1}\n",
                d.turn,
                col,
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
            out.push_str(if self.english {
                "\n──── Details ────\n"
            } else {
                "\n──── 失误详情 ────\n"
            });
            for d in &self.details {
                out.push_str(d);
                out.push('\n');
            }
        }
        out
    }

    fn coord(&self, d: &MoveDelta) -> String {
        match d.mv {
            Move::Pass => "pass".to_string(),
            Move::Play(p) => gtp_coord(p, self.size),
        }
    }
}

/// 一手棋的即时点评：局面 + 这手的特点 + 引擎预演的后果 + 推荐。
pub fn quick_comment(
    english: bool,
    before: &Position,
    before_eval: &TurnEval,
    after_eval: &TurnEval,
    mover: Color,
    mv: Move,
    human: Option<Color>,
) -> String {
    let size = before.board.size();
    let wr_before = winrate_for(mover, before_eval.winrate_black);
    let wr_after = winrate_for(mover, after_eval.winrate_black);
    let loss = wr_before - wr_after;
    let score_before = score_for(mover, before_eval.score_black);
    let opp = mover.opposite();

    let mut lines = Vec::new();
    lines.push(if english {
        format!(
            "Position: {}.",
            score_phrase(true, mover, human, score_before, wr_before)
        )
    } else {
        format!(
            "局面：{}。",
            score_phrase(false, mover, human, score_before, wr_before)
        )
    });

    let zone = match mv {
        Move::Play(p) => {
            if english {
                format!("（{}）", zone_en(p, size))
            } else {
                format!("（{}）", zone_name(p, size))
            }
        }
        Move::Pass => String::new(),
    };
    lines.push(if english {
        format!(
            "{}{}: win rate {:.1}% → {:.1}% ({:+.1}){}",
            move_text(mv, size),
            zone,
            wr_before,
            wr_after,
            -loss,
            verdict_en(loss)
        )
    } else {
        format!(
            "{}{}：胜率 {:.1}% → {:.1}%（{:+.1}）{}",
            move_text(mv, size),
            zone,
            wr_before,
            wr_after,
            -loss,
            verdict_zh(loss)
        )
    });

    if let Some(f) = move_facts(before, mv) {
        lines.push(if english {
            format!("Shape: {}.", facts_text(true, &f))
        } else {
            format!("特点：{}。", facts_text(false, &f))
        });
    }

    if let Some(pv) = after_eval.candidates.first().map(|c| &c.pv) {
        if !pv.is_empty() {
            let pv: Vec<String> = pv.iter().take(4).map(|m| move_text(*m, size)).collect();
            lines.push(if english {
                format!(
                    "Follow-up: {} will likely play {}.",
                    if opp == Color::Black {
                        "Black"
                    } else {
                        "White"
                    },
                    pv.join(" ")
                )
            } else {
                format!(
                    "后果：之后{}大致 {}。",
                    if opp == Color::Black {
                        "黑棋"
                    } else {
                        "白棋"
                    },
                    pv.join(" ")
                )
            });
        }
    }

    if let Some(best) = before_eval.candidates.first() {
        if best.mv != mv {
            let best_wr = winrate_for(mover, best.winrate_black);
            let best_zone = match best.mv {
                Move::Play(p) => {
                    if english {
                        zone_en(p, size)
                    } else {
                        zone_name(p, size)
                    }
                }
                Move::Pass => {
                    if english {
                        "pass"
                    } else {
                        "停一手"
                    }
                }
            };
            // 推荐手相对“实战这一手”的改善量；同样是当前行棋方视角。
            // 不能用 -loss（那是实战手相对上一手的下跌），否则会显示“比实战高 -10 点”。
            let improvement = best_wr - wr_after;
            let mut line = if english {
                if improvement >= 0.0 {
                    format!(
                        "Better: {} ({}, {:.1}%, {:+.1} points better)",
                        move_text(best.mv, size),
                        best_zone,
                        best_wr,
                        improvement
                    )
                } else {
                    format!(
                        "Better: {} ({}, {:.1}%, {:.1} points worse)",
                        move_text(best.mv, size),
                        best_zone,
                        best_wr,
                        -improvement
                    )
                }
            } else if improvement >= 0.0 {
                format!(
                    "推荐 {}（{}，{:.1}%，比实战高 {:.1} 点）",
                    move_text(best.mv, size),
                    best_zone,
                    best_wr,
                    improvement
                )
            } else {
                format!(
                    "推荐 {}（{}，{:.1}%，比实战低 {:.1} 点）",
                    move_text(best.mv, size),
                    best_zone,
                    best_wr,
                    -improvement
                )
            };
            if !best.pv.is_empty() {
                let pv: Vec<String> = best
                    .pv
                    .iter()
                    .take(4)
                    .map(|m| move_text(*m, size))
                    .collect();
                line.push_str(&if english {
                    format!(": then {}.", pv.join(" "))
                } else {
                    format!("：之后大致 {}。", pv.join(" "))
                });
            } else {
                line.push(if english { '.' } else { '。' });
            }
            lines.push(line);
        } else {
            lines.push(if english {
                "This is the engine's top move.".into()
            } else {
                "这手和引擎推荐一致。".into()
            });
        }
    }

    lines.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Candidate;
    use goban::{Game, Move, Point, Position};

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
            human_policy: None,
        }
    }

    #[test]
    fn facts_detect_capture_contact_and_line() {
        let mut pos = Position::new(9);
        pos.board.set(Point::new(1, 0), Some(Color::Black));
        pos.board.set(Point::new(0, 0), Some(Color::White));
        pos.to_move = Color::Black;
        let f = move_facts(&pos, Move::Play(Point::new(0, 1))).unwrap();
        assert_eq!(f.captures, 1);
        assert_eq!(f.line, 1);
        assert!(f.contact);
    }

    #[test]
    fn quick_comment_explains_facts_and_recommendation() {
        let mut pos = Position::new(9);
        pos.board.set(Point::new(1, 0), Some(Color::Black));
        pos.board.set(Point::new(0, 0), Some(Color::White));
        pos.to_move = Color::Black;
        let before = fake_eval(0, Color::Black, 0.55);
        let after = fake_eval(1, Color::White, 0.60);
        let text = quick_comment(
            false,
            &pos,
            &before,
            &after,
            Color::Black,
            Move::Play(Point::new(0, 1)),
            None,
        );
        assert!(text.contains("局面："));
        assert!(text.contains("特点："));
        assert!(text.contains("推荐") || text.contains("一致"));
    }

    #[test]
    fn recommendation_improvement_uses_the_same_view_and_units() {
        let mut pos = Position::new(9);
        pos.to_move = Color::Black;
        // 实战从 55% 掉到 45%，推荐手 56% → 比实战高 11 点
        let mut best = fake_eval(0, Color::Black, 0.56);
        best.candidates[0].winrate_black = 0.56;
        let after = fake_eval(1, Color::White, 0.45);

        let zh = quick_comment(
            false,
            &pos,
            &best,
            &after,
            Color::Black,
            Move::Play(Point::new(2, 2)),
            None,
        );
        assert!(zh.contains("比实战高 11.0 点"), "{zh}");

        let en = quick_comment(
            true,
            &pos,
            &best,
            &after,
            Color::Black,
            Move::Play(Point::new(2, 2)),
            None,
        );
        assert!(en.contains("+11.0 points better"), "{en}");

        // 白方视角：同样的胜率差，符号和数值也要对
        pos.to_move = Color::White;
        let mut best_w = fake_eval(0, Color::White, 0.44); // 白 56%
        best_w.candidates[0].winrate_black = 0.44;
        let after_w = fake_eval(1, Color::Black, 0.55); // 白 45%
        let zh_w = quick_comment(
            false,
            &pos,
            &best_w,
            &after_w,
            Color::White,
            Move::Play(Point::new(2, 2)),
            None,
        );
        assert!(zh_w.contains("比实战高 11.0 点"), "{zh_w}");

        // 推荐手反而更差：显示“低 N 点”，不是负的“高”
        let mut best_bad = fake_eval(0, Color::Black, 0.50);
        best_bad.candidates[0].winrate_black = 0.50;
        let after_bad = fake_eval(1, Color::White, 0.60);
        let zh_bad = quick_comment(
            false,
            &pos,
            &best_bad,
            &after_bad,
            Color::Black,
            Move::Play(Point::new(2, 2)),
            None,
        );
        assert!(zh_bad.contains("比实战低 10.0 点"), "{zh_bad}");
    }

    #[test]
    fn ai_move_is_not_called_you() {
        let mut pos = Position::new(9);
        pos.board.set(Point::new(1, 0), Some(Color::Black));
        pos.board.set(Point::new(0, 0), Some(Color::White));
        pos.to_move = Color::White;
        let before = fake_eval(0, Color::White, 0.45);
        let after = fake_eval(1, Color::Black, 0.50);
        let text = quick_comment(
            false,
            &pos,
            &before,
            &after,
            Color::White,
            Move::Play(Point::new(0, 1)),
            Some(Color::Black),
        );
        assert!(text.contains("AI（白）"), "{text}");
        assert!(!text.contains("你（白）"), "{text}");
    }

    #[test]
    fn english_templates_use_english_terms() {
        let mut pos = Position::new(9);
        pos.board.set(Point::new(0, 0), Some(Color::White));
        pos.to_move = Color::Black;
        let before = fake_eval(0, Color::Black, 0.55);
        let after = fake_eval(1, Color::White, 0.60);
        let text = quick_comment(
            true,
            &pos,
            &before,
            &after,
            Color::Black,
            Move::Play(Point::new(0, 1)),
            None,
        );
        assert!(text.contains("Position:"));
        assert!(text.contains("win rate"));
        assert!(text.contains("Shape:"));
        assert!(!text.contains("胜率"));
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
        let r = build(false, &game, &evals, 5.0).unwrap();
        // 黑方胜率 60% -> 40%，亏 20 个点
        assert!((r.deltas[0].loss - 20.0).abs() < 0.01);
        // 白方视角：60% -> 60%，没亏没赚
        assert!(r.deltas[1].loss.abs() < 0.01);
    }

    #[test]
    fn build_rejects_incomplete_or_misordered_eval_lists() {
        let mut game = Game::new(9, 7.5);
        game.play(Move::Play(Point::new(2, 2))).unwrap(); // B
        game.play(Move::Play(Point::new(6, 6))).unwrap(); // W
                                                          // 2 手需要 turn 0..=2 共 3 个结果
        let short = vec![
            fake_eval(0, Color::Black, 0.5),
            fake_eval(1, Color::White, 0.5),
        ];
        let err = build(false, &game, &short, 5.0).unwrap_err();
        assert!(err.contains("3 个"), "{err}");
        assert!(err.contains("实际收到 2"), "{err}");

        let long = vec![
            fake_eval(0, Color::Black, 0.5),
            fake_eval(1, Color::White, 0.5),
            fake_eval(2, Color::Black, 0.5),
            fake_eval(3, Color::White, 0.5),
        ];
        let err = build(false, &game, &long, 5.0).unwrap_err();
        assert!(err.contains("实际收到 4"), "{err}");

        // 数量对但顺序错：不能让索引错位算出错误的点评
        let swapped = vec![
            fake_eval(1, Color::Black, 0.5),
            fake_eval(0, Color::White, 0.5),
            fake_eval(2, Color::Black, 0.5),
        ];
        let err = build(false, &game, &swapped, 5.0).unwrap_err();
        assert!(err.contains("顺序"), "{err}");
        assert!(err.contains("第 0 个"), "{err}");

        // 空棋谱 + 一个结果仍然合法
        let empty = Game::new(9, 7.5);
        let r = build(false, &empty, &[fake_eval(0, Color::Black, 0.5)], 5.0).unwrap();
        assert_eq!(r.deltas.len(), 0);
    }

    #[test]
    fn render_is_deterministic() {
        let mut game = Game::new(19, 7.5);
        game.play(Move::Play(Point::new(15, 3))).unwrap();
        let evals = vec![
            fake_eval(0, Color::Black, 0.5),
            fake_eval(1, Color::White, 0.5),
        ];
        let r = build(false, &game, &evals, 5.0).unwrap();
        assert_eq!(r.render(5.0, true), r.render(5.0, true));
    }
}
