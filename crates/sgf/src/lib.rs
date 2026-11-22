//! SGF 读取（跟随主线，跳过其它分支），支持常用属性子集。
//!
//! 支持：`SZ` `KM` `RU` `B` `W` `AB` `AW`。
//! 分支规则按 SGF 规范：第一个子分支就是主线。

use goban::{Color, KoRule, Move, Point};

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

    // 先确定第一手颜色：SGF 的 B/W 是文件里的事实，不能用"有没有摆子"去猜。
    // 摆子只影响轮次校验的起点，不影响文件已经写明的颜色。
    let first_color = nodes
        .iter()
        .flat_map(|props| props.iter())
        .find_map(|(ident, _)| match ident.as_str() {
            "B" => Some(Color::Black),
            "W" => Some(Color::White),
            _ => None,
        })
        .unwrap_or(Color::Black);

    // 第一遍：读盘面参数（根节点在前）
    // `move_nodes` 是源文件里的手节点序号（含被跳过的乱序手），只用于报错定位。
    let mut move_nodes = 0usize;
    for props in &nodes {
        for (ident, values) in props {
            match ident.as_str() {
                "SZ" => {
                    let Some(v) = values.first() else { continue };
                    let (w, h) = match v.split_once(':') {
                        Some((w, h)) => (w.trim(), h.trim()),
                        None => (v.trim(), v.trim()),
                    };
                    let w: u8 = w.parse().map_err(|_| format!("SZ 不是数字：{v:?}"))?;
                    let h: u8 = h.parse().map_err(|_| format!("SZ 不是数字：{v:?}"))?;
                    if w != h {
                        return Err(format!("不支持矩形棋盘 SZ[{v}]（当前只支持正方形）"));
                    }
                    if !(2..=25).contains(&w) {
                        return Err(format!("棋盘大小超出 2..=25：{v:?}"));
                    }
                    out.size = w;
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
    // 摆子只在还没有任何 B/W 之前允许（根节点拆成多个节点也兼容）
    let mut saw_move = false;
    for (node_idx, props) in nodes.iter().enumerate() {
        for (ident, values) in props {
            match ident.as_str() {
                "AB" | "AW" => {
                    if saw_move {
                        // 中途摆子会改变此前棋步的合法性，不能当成开局摆子提前应用
                        return Err(format!(
                            "不支持中途摆子：第 {} 个节点的 {ident}",
                            node_idx + 1
                        ));
                    }
                    let color = if ident == "AB" {
                        Color::Black
                    } else {
                        Color::White
                    };
                    for v in values {
                        let p = parse_coord(v, out.size)
                            .map_err(|e| format!("{ident} 摆子坐标非法 [{v}]：{e}"))?;
                        out.setup.push((color, p));
                    }
                }
                "B" | "W" => {
                    saw_move = true;
                    move_nodes += 1;
                    let color = if ident == "B" {
                        Color::Black
                    } else {
                        Color::White
                    };
                    let expected = if out.moves.len().is_multiple_of(2) {
                        first_color
                    } else {
                        first_color.opposite()
                    };
                    // 轮次对不上的（残缺棋谱）整手丢弃，连坐标也不校验：
                    // 否则一个多余的坏坐标会让整盘打不开。
                    if color != expected {
                        continue;
                    }
                    let raw = values.first().map(String::as_str).unwrap_or("");
                    let mv = parse_move(raw, out.size)
                        .map_err(|e| format!("第 {move_nodes} 手坐标非法 [{raw}]：{e}"))?;
                    out.moves.push(mv);
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
    let mut in_value = false;
    while *i < chars.len() {
        let c = chars[*i];
        if in_value {
            // 属性值里的括号不是结构括号；反斜杠转义下一个字符
            match c {
                '\\' => *i += 1,
                ']' => in_value = false,
                _ => {}
            }
        } else {
            match c {
                '[' => in_value = true,
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
        }
        *i += 1;
    }
}

/// SGF 坐标轴字符 → 0-based 下标。
///
/// `char as u8` 是截断转换：`'š'` 的低 8 位正好是 `'a'`，不先判 ASCII 就会被当成 0。
fn coord_axis(c: char, which: &str) -> Result<u8, String> {
    if !c.is_ascii_lowercase() {
        return Err(format!("{which}坐标不是小写字母：{c}"));
    }
    Ok(c as u8 - b'a')
}

fn parse_coord(s: &str, size: u8) -> Result<Point, String> {
    let b: Vec<char> = s.chars().collect();
    if b.len() != 2 {
        return Err(format!("坐标长度应为 2，实际 {}（{s:?}）", b.len()));
    }
    let x = coord_axis(b[0], "横")?;
    let y = coord_axis(b[1], "纵")?;
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

/// 从 SGF `RU` 属性猜规则；认不出来就用默认（位置超级劫）。
///
/// 宁可放宽也不要收紧：映射错了顶多漏禁一手，收紧会让合法棋谱打不开。
/// - 日本 / 新西兰：只禁简单劫（NZ 允许循环局面）
/// - AGA / Ing / 韩国：情境超级劫（把行棋方算进重复判定）
/// - 中国及未知值：位置超级劫（默认）
pub fn ko_rule_from_rules(rules: Option<&str>) -> KoRule {
    let Some(r) = rules else {
        return KoRule::default();
    };
    let r = r.trim().to_ascii_lowercase();
    // 用前缀而不是 contains：否则 "something else" 会因为含 "ing" 被认成 Ing 规则
    if r.starts_with("japanese") || r.starts_with("new zealand") {
        KoRule::Simple
    } else if r.starts_with("aga") || r.starts_with("ing") || r.starts_with("korean") {
        KoRule::SituationalSuperko
    } else {
        KoRule::PositionalSuperko
    }
}

/// 写回 SGF `RU` 时用的名字。
pub fn rules_for_ko(ko: KoRule) -> &'static str {
    match ko {
        KoRule::Simple => "Japanese",
        KoRule::PositionalSuperko => "Chinese",
        KoRule::SituationalSuperko => "Korean",
    }
}

/// 把局面写回 SGF（只写主线，带摆子和贴目）。
pub fn write_game(
    size: u8,
    komi: f32,
    setup: &[(Color, Point)],
    moves: &[Move],
    ko: KoRule,
) -> String {
    fn coord(p: Point) -> String {
        let mut s = String::new();
        s.push((b'a' + p.x) as char);
        s.push((b'a' + p.y) as char);
        s
    }
    let mut out = String::from("(;GM[1]FF[4]CA[UTF-8]");
    out.push_str(&format!("SZ[{size}]KM[{komi:.1}]RU[{}]", rules_for_ko(ko)));
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
        let text = write_game(19, 7.5, &[], &moves, KoRule::default());
        let back = parse(&text).unwrap();
        assert_eq!(back.size, 19);
        assert_eq!(back.komi, Some(7.5));
        assert_eq!(back.moves, moves);
    }

    #[test]
    fn writes_and_reparses_two_passes() {
        let moves = vec![Move::Play(Point::new(2, 2)), Move::Pass, Move::Pass];
        let text = write_game(9, 7.5, &[], &moves, KoRule::default());
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
    fn rejects_non_ascii_coordinates() {
        // 'š' 的低 8 位是 'a'，不能靠 `char as u8` 截断后当成合法坐标
        let err = parse("(;SZ[9];B[ša])").unwrap_err();
        assert!(err.contains("坐标非法"), "{err}");
        let err = parse("(;SZ[9];B[aš])").unwrap_err();
        assert!(err.contains("坐标非法"), "{err}");
    }

    #[test]
    fn out_of_turn_move_with_bad_coordinate_is_skipped() {
        // 残缺棋谱里多出来的一手：整手丢弃，不该让整盘打不开
        let g = parse("(;SZ[9];B[dd];B[zz])").unwrap();
        assert_eq!(g.moves, vec![Move::Play(Point::new(3, 3))]);
    }

    #[test]
    fn error_reports_source_node_index() {
        // 第 2 手乱序被跳过，第 3 手坐标非法：报错要按源文件的手数
        let err = parse("(;SZ[9];B[dd];B[cc];W[zz])").unwrap_err();
        assert!(err.contains("第 3 手"), "{err}");
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

    // ---- 属性测试：随机合法棋谱写出再读回，必须完全一致 ----

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// `seed | 1` 会把 2 和 3 折叠成同一个状态；先做一次 splitmix 扩散。
    fn seeded(seed: u64) -> u64 {
        let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (x ^ (x >> 31)) | 1
    }

    fn random_legal_game(seed: u64, size: u8, plies: usize) -> goban::Game {
        let mut rng = Rng(seeded(seed));
        let mut g = goban::Game::new(size, 7.5);
        let mut played = 0;
        while played < plies {
            if rng.below(12) == 0 {
                g.play(Move::Pass).unwrap();
                played += 1;
                continue;
            }
            let x = rng.below(size as u64) as u8;
            let y = rng.below(size as u64) as u8;
            if g.play(Move::Play(Point::new(x, y))).is_ok() {
                played += 1;
            }
        }
        g
    }

    #[test]
    fn random_games_round_trip_through_sgf() {
        for (seed, size) in [(1u64, 9u8), (2, 13), (3, 19)] {
            let game = random_legal_game(seed, size, 90);
            let text = write_game(size, 7.5, &[], game.moves(), KoRule::default());
            let back = parse(&text).unwrap_or_else(|e| panic!("seed {seed}: 解析失败：{e}"));

            assert_eq!(back.size, size, "seed {seed}");
            assert_eq!(back.komi, Some(7.5), "seed {seed}");
            assert_eq!(back.moves, game.moves(), "seed {seed}: 手顺必须一致");

            // 重放读回的手顺，终局必须和原局相同
            let mut replay = goban::Game::new(size, 7.5);
            for mv in &back.moves {
                replay.play(*mv).expect("读回的棋谱必须能重放");
            }
            assert_eq!(
                replay.position(),
                game.position(),
                "seed {seed}: 终局不一致"
            );
        }
    }

    #[test]
    fn setup_stones_round_trip() {
        let setup = vec![
            (Color::Black, Point::new(2, 2)),
            (Color::Black, Point::new(6, 6)),
            (Color::White, Point::new(2, 6)),
            (Color::White, Point::new(6, 2)),
        ];
        let moves = vec![Move::Play(Point::new(4, 4)), Move::Pass];
        let text = write_game(9, 0.5, &setup, &moves, KoRule::default());
        let back = parse(&text).unwrap();

        assert_eq!(back.size, 9);
        assert_eq!(back.komi, Some(0.5));
        assert_eq!(back.moves, moves);
        let mut got = back.setup;
        let mut want = setup;
        got.sort_by_key(|(c, p)| (c.index(), p.y, p.x));
        want.sort_by_key(|(c, p)| (c.index(), p.y, p.x));
        assert_eq!(got, want, "摆子必须原样读回");
    }

    #[test]
    fn consecutive_passes_round_trip_at_every_size() {
        for size in [5u8, 9, 13, 19] {
            let moves = vec![Move::Pass, Move::Pass, Move::Pass];
            let text = write_game(size, 7.5, &[], &moves, KoRule::default());
            let back = parse(&text).unwrap();
            assert_eq!(back.moves, moves, "{size} 路盘连续 Pass 必须读回");
        }
    }

    #[test]
    fn ru_survives_write_and_parse() {
        for ko in [
            KoRule::Simple,
            KoRule::PositionalSuperko,
            KoRule::SituationalSuperko,
        ] {
            let text = write_game(9, 7.5, &[], &[Move::Play(Point::new(2, 2))], ko);
            let back = parse(&text).unwrap();
            assert_eq!(ko_rule_from_rules(back.rules.as_deref()), ko, "{text}");
        }
    }

    #[test]
    fn ko_rule_mapping() {
        assert_eq!(ko_rule_from_rules(Some("Japanese")), KoRule::Simple);
        assert_eq!(ko_rule_from_rules(Some("Japanese rules")), KoRule::Simple);
        assert_eq!(ko_rule_from_rules(Some("New Zealand")), KoRule::Simple);
        assert_eq!(ko_rule_from_rules(Some("AGA")), KoRule::SituationalSuperko);
        assert_eq!(
            ko_rule_from_rules(Some("Korean")),
            KoRule::SituationalSuperko
        );
        assert_eq!(
            ko_rule_from_rules(Some("Chinese")),
            KoRule::PositionalSuperko
        );
        assert_eq!(ko_rule_from_rules(None), KoRule::default());
        assert_eq!(
            ko_rule_from_rules(Some("something else")),
            KoRule::default()
        );
    }

    /// 文件里的 B/W 是事实：有开局摆子也不能把第一手吞掉。
    #[test]
    fn declared_colors_are_preserved_with_setup() {
        // 根节点被拆成两个节点、摆子仍在前：允许
        let g = parse("(;SZ[9];AB[ee];B[dd])").unwrap();
        assert_eq!(g.setup.len(), 1);
        assert_eq!(g.moves.len(), 1);

        // 让子棋：AW 摆白子，黑先
        let g = parse("(;SZ[9]AW[dd];B[cc];W[gg])").unwrap();
        assert_eq!(g.moves.len(), 2, "AW + 黑先不能丢第一手");
        assert_eq!(g.moves[0], Move::Play(Point::new(2, 2)));

        // 真正的乱序（两个黑连着下）仍然丢弃第二个
        let g = parse("(;SZ[9];B[cc];B[dd];W[ee])").unwrap();
        assert_eq!(g.moves.len(), 2);
    }

    #[test]
    fn midgame_setup_is_rejected() {
        // 中途 AB/AW 不能当成开局摆子提前应用
        let err = parse("(;SZ[9];B[cc];W[gg];AB[ee];B[dd])").unwrap_err();
        assert!(err.contains("不支持中途摆子"), "{err}");
    }

    #[test]
    fn rejects_rectangular_board() {
        let err = parse("(;SZ[19:13];B[dd])").unwrap_err();
        assert!(err.contains("矩形"), "{err}");
        let err = parse("(;SZ[13:19];B[dd])").unwrap_err();
        assert!(err.contains("矩形"), "{err}");
        // 宽高相同的写法可以接受
        assert!(parse("(;SZ[9:9];B[dd])").is_ok());
    }

    #[test]
    fn rejects_invalid_size() {
        assert!(parse("(;SZ[abc];B[dd])").is_err());
        assert!(parse("(;SZ[1];B[dd])").is_err());
        assert!(parse("(;SZ[99];B[dd])").is_err());
    }

    #[test]
    fn skips_variations_with_parens_in_comments() {
        // 第二条变化分支的注释里有不配对括号：不能破坏深度计数
        let src = "(;SZ[9];B[cc](;W[gg];B[dd])(;W[ee]C[a ( b];B[ff]))";
        let g = parse(src).unwrap();
        assert_eq!(g.moves.len(), 3, "主线三手，第二条变化被跳过");
        assert_eq!(g.moves[1], Move::Play(Point::new(6, 6)));

        let src = "(;SZ[9];B[cc](;W[gg])(;W[ee]C[a ) b];B[ff]))";
        let g = parse(src).unwrap();
        assert_eq!(g.moves.len(), 2);

        // 转义的括号和方括号
        let src = "(;SZ[9];B[cc](;W[gg])(;W[ee]C[x \\) y \\] z];B[ff]))";
        let g = parse(src).unwrap();
        assert_eq!(g.moves.len(), 2);
    }
}
