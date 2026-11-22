//! SGF 读取（跟随主线，跳过其它分支），支持常用属性子集。
//!
//! 支持：`SZ` `KM` `RU` `B` `W` `AB` `AW`。
//! 分支规则按 SGF 规范：第一个子分支就是主线。

use goban::{Color, Move, Point};

#[derive(Clone, Debug, Default)]
pub struct SgfGame {
    pub size: u8,
    pub komi: Option<f32>,
    pub rules: Option<String>,
    pub setup: Vec<(Color, Point)>,
    pub moves: Vec<Move>,
}

type Props = Vec<(String, Vec<String>)>;

pub fn parse(src: &str) -> Result<SgfGame, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < chars.len() && chars[i] != '(' {
        i += 1;
    }
    if i >= chars.len() {
        return Err("不是 SGF：找不到起始的 (".to_string());
    }
    let nodes = parse_tree(&chars, &mut i)?;

    let mut out = SgfGame {
        size: 19,
        ..Default::default()
    };

    // 第一遍：读盘面参数（根节点在前）
    for props in &nodes {
        for (ident, values) in props {
            match ident.as_str() {
                "SZ" => {
                    if let Some(v) = values.first() {
                        if let Ok(n) = v.split(':').next().unwrap_or(v).parse::<u8>() {
                            if (2..=25).contains(&n) {
                                out.size = n;
                            }
                        }
                    }
                }
                "KM" => {
                    if out.komi.is_none() {
                        if let Some(v) = values.first() {
                            out.komi = v.trim().parse::<f32>().ok();
                        }
                    }
                }
                "RU" if out.rules.is_none() => {
                    if let Some(v) = values.first() {
                        out.rules = Some(v.trim().to_string());
                    }
                }
                _ => {}
            }
        }
    }

    // 第二遍：摆子 + 落子
    for props in &nodes {
        for (ident, values) in props {
            match ident.as_str() {
                "AB" => {
                    for v in values {
                        let p = parse_coord(v, out.size)
                            .map_err(|e| format!("AB 摆子坐标非法 [{v}]：{e}"))?;
                        out.setup.push((Color::Black, p));
                    }
                }
                "AW" => {
                    for v in values {
                        let p = parse_coord(v, out.size)
                            .map_err(|e| format!("AW 摆子坐标非法 [{v}]：{e}"))?;
                        out.setup.push((Color::White, p));
                    }
                }
                "B" | "W" => {
                    let color = if ident == "B" {
                        Color::Black
                    } else {
                        Color::White
                    };
                    let raw = values.first().map(String::as_str).unwrap_or("");
                    let mv = parse_move(raw, out.size).map_err(|e| {
                        format!("第 {} 手坐标非法 [{raw}]：{e}", out.moves.len() + 1)
                    })?;
                    let expected = if out.setup.is_empty() {
                        if out.moves.len().is_multiple_of(2) {
                            Color::Black
                        } else {
                            Color::White
                        }
                    } else if out.moves.len().is_multiple_of(2) {
                        Color::White
                    } else {
                        Color::Black
                    };
                    // 轮次对不上的（残缺棋谱）跳过，避免整盘解析失败
                    if color == expected {
                        out.moves.push(mv);
                    }
                }
                _ => {}
            }
        }
    }

    if out.moves.is_empty() && out.setup.is_empty() {
        return Err("没有找到任何落子（可能是空棋谱或只含变化图）".to_string());
    }
    Ok(out)
}

fn parse_tree(chars: &[char], i: &mut usize) -> Result<Vec<Props>, String> {
    if *i >= chars.len() || chars[*i] != '(' {
        return Err("SGF 结构错误：期望 (".to_string());
    }
    *i += 1;

    let mut nodes: Vec<Props> = Vec::new();
    loop {
        skip_ws(chars, i);
        if *i < chars.len() && chars[*i] == ';' {
            nodes.push(parse_node(chars, i));
        } else {
            break;
        }
    }

    // 子分支：第一个是主线，其余跳过
    skip_ws(chars, i);
    if *i < chars.len() && chars[*i] == '(' {
        let first = parse_tree(chars, i)?;
        nodes.extend(first);
        loop {
            skip_ws(chars, i);
            if *i < chars.len() && chars[*i] == '(' {
                skip_tree(chars, i);
            } else {
                break;
            }
        }
    }

    skip_ws(chars, i);
    if *i >= chars.len() || chars[*i] != ')' {
        return Err("SGF 结构错误：括号不匹配".to_string());
    }
    *i += 1;
    Ok(nodes)
}

fn skip_ws(chars: &[char], i: &mut usize) {
    while *i < chars.len() && chars[*i].is_whitespace() {
        *i += 1;
    }
}

fn parse_node(chars: &[char], i: &mut usize) -> Props {
    *i += 1; // ';'
    let mut props = Vec::new();
    loop {
        skip_ws(chars, i);
        if *i >= chars.len() || !chars[*i].is_ascii_uppercase() {
            break;
        }
        let start = *i;
        while *i < chars.len() && chars[*i].is_ascii_uppercase() {
            *i += 1;
        }
        let ident: String = chars[start..*i].iter().collect();
        let mut values: Vec<String> = Vec::new();
        while *i < chars.len() && chars[*i] == '[' {
            *i += 1;
            let mut v = String::new();
            while *i < chars.len() && chars[*i] != ']' {
                if chars[*i] == '\\' {
                    *i += 1;
                    if *i < chars.len() {
                        v.push(chars[*i]);
                        *i += 1;
                    }
                } else {
                    v.push(chars[*i]);
                    *i += 1;
                }
            }
            *i += 1; // ]
            values.push(v);
            skip_ws(chars, i);
        }
        props.push((ident, values));
    }
    props
}

fn skip_tree(chars: &[char], i: &mut usize) {
    let mut depth = 0;
    while *i < chars.len() {
        match chars[*i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    *i += 1;
                    return;
                }
            }
            _ => {}
        }
        *i += 1;
    }
}

fn parse_coord(s: &str, size: u8) -> Result<Point, String> {
    let b: Vec<char> = s.chars().collect();
    if b.len() != 2 {
        return Err(format!("坐标长度应为 2，实际 {}（{s:?}）", b.len()));
    }
    let x = (b[0] as u8)
        .checked_sub(b'a')
        .ok_or_else(|| format!("横坐标不是小写字母：{}", b[0]))?;
    let y = (b[1] as u8)
        .checked_sub(b'a')
        .ok_or_else(|| format!("纵坐标不是小写字母：{}", b[1]))?;
    if x >= size || y >= size {
        return Err(format!("坐标超出 {} 路棋盘", size));
    }
    Ok(Point::new(x, y))
}

fn parse_move(s: &str, size: u8) -> Result<Move, String> {
    // SGF 空坐标是 Pass；19 路及以下 `tt` 也是约定俗成的 Pass
    if s.is_empty() || (s == "tt" && size <= 19) {
        return Ok(Move::Pass);
    }
    parse_coord(s, size).map(Move::Play)
}

/// 把局面写回 SGF（只写主线，带摆子和贴目）。
pub fn write_game(size: u8, komi: f32, setup: &[(Color, Point)], moves: &[Move]) -> String {
    fn coord(p: Point) -> String {
        let mut s = String::new();
        s.push((b'a' + p.x) as char);
        s.push((b'a' + p.y) as char);
        s
    }
    let mut out = String::from("(;GM[1]FF[4]CA[UTF-8]");
    out.push_str(&format!("SZ[{size}]KM[{komi:.1}]RU[Chinese]"));
    let blacks: Vec<String> = setup
        .iter()
        .filter(|(c, _)| *c == Color::Black)
        .map(|(_, p)| format!("[{}]", coord(*p)))
        .collect();
    let whites: Vec<String> = setup
        .iter()
        .filter(|(c, _)| *c == Color::White)
        .map(|(_, p)| format!("[{}]", coord(*p)))
        .collect();
    if !blacks.is_empty() {
        out.push_str("AB");
        out.push_str(&blacks.concat());
    }
    if !whites.is_empty() {
        out.push_str("AW");
        out.push_str(&whites.concat());
    }

    let first = if setup.is_empty() {
        Color::Black
    } else {
        Color::White
    };
    for (i, mv) in moves.iter().enumerate() {
        let color = if i.is_multiple_of(2) {
            first
        } else {
            first.opposite()
        };
        out.push(';');
        out.push(color.sgf_char());
        out.push('[');
        if let Move::Play(p) = mv {
            out.push_str(&coord(*p));
        }
        out.push(']');
    }
    out.push(')');
    out
}

/// SGF 坐标 -> 0-based Point（给测试和外部工具复用）
pub fn point_from_sgf(s: &str, size: u8) -> Option<Point> {
    parse_coord(s, size).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use goban::Move;

    #[test]
    fn parses_basic_game() {
        let src = "(;GM[1]FF[4]SZ[19]KM[7.5]RU[Chinese];B[pd];W[dp];B[pp];W[dd])";
        let g = parse(src).unwrap();
        assert_eq!(g.size, 19);
        assert_eq!(g.komi, Some(7.5));
        assert_eq!(g.rules.as_deref(), Some("Chinese"));
        assert_eq!(g.moves.len(), 4);
        assert_eq!(g.moves[0], Move::Play(Point::new(15, 3))); // Q16
        assert_eq!(g.moves[1], Move::Play(Point::new(3, 15))); // D4
    }

    #[test]
    fn follows_first_variation() {
        let src = "(;SZ[9];B[cc](;W[gg];B[dd])(;W[ee]))";
        let g = parse(src).unwrap();
        assert_eq!(g.moves.len(), 3);
        assert_eq!(g.moves[1], Move::Play(Point::new(6, 6))); // G7
        assert_eq!(g.moves[2], Move::Play(Point::new(3, 3)));
    }

    #[test]
    fn writes_and_reparses() {
        let moves = vec![
            Move::Play(Point::new(15, 3)),
            Move::Pass,
            Move::Play(Point::new(3, 15)),
        ];
        let text = write_game(19, 7.5, &[], &moves);
        let back = parse(&text).unwrap();
        assert_eq!(back.size, 19);
        assert_eq!(back.komi, Some(7.5));
        assert_eq!(back.moves, moves);
    }

    #[test]
    fn writes_and_reparses_two_passes() {
        let moves = vec![Move::Play(Point::new(2, 2)), Move::Pass, Move::Pass];
        let text = write_game(9, 7.5, &[], &moves);
        let back = parse(&text).unwrap();
        assert_eq!(back.moves, moves);
    }

    #[test]
    fn handles_pass() {
        let src = "(;SZ[9];B[];W[ee])";
        let g = parse(src).unwrap();
        assert_eq!(g.moves[0], Move::Pass);
    }

    #[test]
    fn tt_is_pass_on_19_or_smaller() {
        let g = parse("(;SZ[19];B[tt];W[dd])").unwrap();
        assert_eq!(g.moves[0], Move::Pass);
        let g = parse("(;SZ[9];B[tt])").unwrap();
        assert_eq!(g.moves[0], Move::Pass);
    }

    #[test]
    fn rejects_invalid_move_coordinates() {
        let err = parse("(;SZ[9];B[zz])").unwrap_err();
        assert!(err.contains("坐标非法"), "{err}");
        let err = parse("(;SZ[9];B[Z9])").unwrap_err();
        assert!(err.contains("坐标非法"), "{err}");
        let err = parse("(;SZ[9];B[abc])").unwrap_err();
        assert!(err.contains("坐标非法"), "{err}");
    }

    #[test]
    fn rejects_invalid_setup_coordinates() {
        let err = parse("(;SZ[9]AB[zz])").unwrap_err();
        assert!(err.contains("AB"), "{err}");
        let err = parse("(;SZ[9]AW[Z9])").unwrap_err();
        assert!(err.contains("AW"), "{err}");
    }

    #[test]
    fn skips_out_of_turn_moves() {
        // 两个黑连着下：第二个应被忽略，保证整盘可解析
        let src = "(;SZ[9];B[cc];B[dd];W[ee])";
        let g = parse(src).unwrap();
        assert_eq!(g.moves.len(), 2);
    }
}
