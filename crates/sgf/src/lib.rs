//! SGF 读取（跟随主线，跳过其它分支），支持常用属性子集。
//!
//! 支持：`SZ` `KM` `RU` `B` `W` `AB` `AW`。
//! 分支规则按 SGF 规范：第一个子分支就是主线。

use goban::{Color, KoRule, Move, Point, RuleSet};

#[derive(Clone, Debug, Default)]
pub struct SgfGame {
    pub size: u8,
    pub komi: Option<f32>,
    pub rules: Option<String>,
    /// 先手方：根 PL 或首手 B/W；两者都没有时为 None（交给 `first_to_move` 推断）
    pub first: Option<Color>,
    /// 让子数（`HA`）。只作为元数据保留，先手推断会在缺 `PL`/首手时参考它。
    pub handicap: Option<u8>,
    /// 对局结果（`RE`），原样保留。
    pub result: Option<String>,
    pub setup: Vec<(Color, Point)>,
    pub moves: Vec<Move>,
    /// 宽松解析时遇到的问题（非法 KM、乱序手、无法识别的 PL 等）
    pub warnings: Vec<String>,
}

impl SgfGame {
    /// 重放时的先手方：优先用文件声明的颜色，没有落子再按让子惯例推断。
    pub fn first_to_move(&self) -> Color {
        self.first.unwrap_or_else(|| {
            // 让子棋白先：有黑摆子，或 HA 声明了让子数
            let handicap = self.handicap.is_some_and(|n| n >= 2);
            if handicap || self.setup.iter().any(|(c, _)| *c == Color::Black) {
                Color::White
            } else {
                Color::Black
            }
        })
    }
}

type Props = Vec<(String, Vec<String>)>;

/// 本工具真正解析的属性。
const SUPPORTED_PROPS: &[&str] = &[
    "SZ", "KM", "PL", "RU", "AB", "AW", "AE", "B", "W", "HA", "RE",
];

/// 常见但不影响对局语义的 SGF 属性：读到就跳过，不报“不支持”。
/// 白名单是有意放宽的——否则一般棋谱（PB/PW/DT/C/LB...）全会被 warning 淹掉。
const KNOWN_METADATA: &[&str] = &[
    "GM", "FF", "CA", "ST", "AP", "DT", "PB", "PW", "BR", "WR", "BT", "WT", "US", "AN", "SO", "CP",
    "ON", "GC", "TM", "OT", "GN", "EV", "RO", "PC", "C", "N", "V", "DM", "GB", "GW", "PM", "FG",
    "VW", "LB", "TR", "SQ", "CR", "MA", "DD", "AR", "LN", "MN", "TB", "TW", "BL", "WL", "OB", "OW",
    "TC", "AT", "M",
];

/// 解析一个 SGF collection：标准允许一个文件里放多盘棋。
///
/// 每盘只读主线；盘内的其它分支仍然跳过（变化树另见 DESIGN）。
pub fn parse_collection(src: &str) -> Result<Vec<SgfGame>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    // 起始括号前只允许空白 / BOM，别的内容要报位置
    while i < chars.len() && chars[i] != '(' {
        let c = chars[i];
        if !c.is_whitespace() && c != '\u{feff}' {
            return Err(format!("SGF 之前有多余内容：第 {} 个字符 {:?}", i + 1, c));
        }
        i += 1;
    }
    if i >= chars.len() {
        return Err("不是 SGF：找不到起始的 (".to_string());
    }

    let mut games = Vec::new();
    loop {
        skip_ws(&chars, &mut i);
        if i >= chars.len() {
            break;
        }
        if chars[i] != '(' {
            return Err(format!(
                "SGF 尾部有多余内容：第 {} 个字符 {:?}",
                i + 1,
                chars[i]
            ));
        }
        let nodes = parse_tree(&chars, &mut i)?;
        games.push(build_game(&nodes)?);
    }
    if games.is_empty() {
        return Err("不是 SGF：找不到起始的 (".to_string());
    }
    Ok(games)
}

/// 只读第一盘；文件里还有更多棋谱时给 warning，绝不静默截断。
pub fn parse(src: &str) -> Result<SgfGame, String> {
    let mut games = parse_collection(src)?;
    let mut first = games.remove(0);
    if !games.is_empty() {
        first
            .warnings
            .push(format!("文件含 {} 盘棋，只打开第一盘", games.len() + 1));
    }
    Ok(first)
}

fn build_game(nodes: &[Props]) -> Result<SgfGame, String> {
    let mut out = SgfGame {
        size: 19,
        ..Default::default()
    };

    // 先确定第一手颜色：SGF 的 B/W 是文件里的事实，不能用"有没有摆子"去猜。
    // 摆子只影响轮次校验的起点，不影响文件已经写明的颜色。
    let first_move = nodes
        .iter()
        .flat_map(|props| props.iter())
        .find_map(|(ident, _)| match ident.as_str() {
            "B" => Some(Color::Black),
            "W" => Some(Color::White),
            _ => None,
        });
    // 根节点被拆成多个节点时，PL 仍算"开局前"
    let first_move_node = nodes
        .iter()
        .position(|props| props.iter().any(|(ident, _)| ident == "B" || ident == "W"));
    let first_color = first_move.unwrap_or(Color::Black);
    let mut pl: Option<Color> = None;

    // 第一遍：读盘面参数（根节点在前）
    // `move_nodes` 是源文件里的手节点序号（含被跳过的乱序手），只用于报错定位。
    let mut move_nodes = 0usize;
    let (mut saw_sz, mut saw_km, mut saw_ha, mut saw_re) = (false, false, false, false);
    for (node_idx, props) in nodes.iter().enumerate() {
        for (ident, values) in props {
            match ident.as_str() {
                "SZ" => {
                    if saw_sz {
                        out.warnings.push("重复的 SZ，只取第一个".to_string());
                        continue;
                    }
                    saw_sz = true;
                    if values.len() > 1 {
                        out.warnings.push("SZ 有多个值，只取第一个".to_string());
                    }
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
                    // 用独立标记：第一个 KM 即使非法/为空，也算“已经出现过”
                    if saw_km {
                        out.warnings.push("重复的 KM，只取第一个".to_string());
                        continue;
                    }
                    saw_km = true;
                    {
                        let Some(v) = values.first() else { continue };
                        if values.len() > 1 {
                            out.warnings.push("KM 有多个值，只取第一个".to_string());
                        }
                        let raw = v.trim();
                        if raw.is_empty() {
                            out.warnings.push("KM 为空，按默认贴目".to_string());
                        } else {
                            // 部分编辑器用逗号当小数点
                            match raw.replace(',', ".").parse::<f32>() {
                                Ok(k) if k.is_finite() => out.komi = Some(k),
                                Ok(_) => out
                                    .warnings
                                    .push(format!("KM 不是有限数：{v:?}，按默认贴目")),
                                Err(_) => {
                                    out.warnings.push(format!("KM 不是数字：{v:?}，按默认贴目"))
                                }
                            }
                        }
                    }
                }
                "PL" => {
                    if values.len() > 1 {
                        out.warnings.push(format!(
                            "第 {} 个节点的 PL 有多个值，只取第一个",
                            node_idx + 1
                        ));
                    }
                    let v = values.first().map(|s| s.trim()).unwrap_or("");
                    let color = match v {
                        "B" => Some(Color::Black),
                        "W" => Some(Color::White),
                        _ => None,
                    };
                    match color {
                        Some(c) if first_move_node.is_none_or(|m| node_idx <= m) => {
                            if pl.is_some() {
                                out.warnings.push("重复的 PL，使用最后一个".to_string());
                            }
                            pl = Some(c);
                        }
                        Some(_) => out
                            .warnings
                            .push(format!("忽略第 {} 个节点的 PL", node_idx + 1)),
                        None => out
                            .warnings
                            .push(format!("第 {} 个节点的 PL 值无法识别：{v:?}", node_idx + 1)),
                    }
                }
                "RU" => {
                    if out.rules.is_some() {
                        out.warnings.push("重复的 RU，只取第一个".to_string());
                    } else if values.len() > 1 {
                        out.warnings.push("RU 有多个值，只取第一个".to_string());
                        if let Some(v) = values.first() {
                            out.rules = Some(v.trim().to_string());
                        }
                    } else if let Some(v) = values.first() {
                        out.rules = Some(v.trim().to_string());
                    }
                }
                "HA" => {
                    if saw_ha {
                        out.warnings.push("重复的 HA，只取第一个".to_string());
                        continue;
                    }
                    saw_ha = true;
                    if values.len() > 1 {
                        out.warnings.push("HA 有多个值，只取第一个".to_string());
                    }
                    let raw = values.first().map(|s| s.trim()).unwrap_or("");
                    match raw.parse::<u8>() {
                        Ok(n) if n >= 2 => out.handicap = Some(n),
                        Ok(n) => out
                            .warnings
                            .push(format!("HA[{n}] 不是有效让子数（至少 2），已忽略")),
                        Err(_) => out.warnings.push(format!("HA[{raw}] 不是数字，已忽略")),
                    }
                }
                "RE" => {
                    if saw_re {
                        out.warnings.push("重复的 RE，只取第一个".to_string());
                        continue;
                    }
                    saw_re = true;
                    if values.len() > 1 {
                        out.warnings.push("RE 有多个值，只取第一个".to_string());
                    }
                    if let Some(v) = values.first() {
                        let v = v.trim();
                        if !v.is_empty() {
                            out.result = Some(v.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    out.first = match (first_move, pl) {
        (Some(f), Some(p)) => {
            if p != f {
                out.warnings
                    .push("PL 与首手颜色不一致，按首手颜色重放".to_string());
            }
            Some(f)
        }
        (Some(f), None) => Some(f),
        (None, Some(p)) => Some(p),
        // 没有落子也没有 PL：交给 first_to_move() 按摆子惯例推断
        (None, None) => None,
    };

    // 第二遍：摆子 + 落子
    // 摆子只在还没有任何 B/W 之前允许（根节点拆成多个节点也兼容）
    let mut saw_move = false;
    let mut cleared: Vec<Point> = Vec::new();
    let mut ignored_props: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (node_idx, props) in nodes.iter().enumerate() {
        // 按节点判断：同一节点里 AB 写在 B 后面也算开局摆子
        let move_before_node = saw_move;
        for (ident, values) in props {
            match ident.as_str() {
                "AB" | "AW" => {
                    if move_before_node {
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
                "AE" => {
                    if move_before_node {
                        return Err(format!("不支持中途清子：第 {} 个节点的 AE", node_idx + 1));
                    }
                    for v in values {
                        let p = parse_coord(v, out.size)
                            .map_err(|e| format!("AE 坐标非法 [{v}]：{e}"))?;
                        cleared.push(p);
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
                        out.warnings.push(format!(
                            "第 {move_nodes} 手颜色与轮次不符（{ident}），已跳过"
                        ));
                        continue;
                    }
                    let raw = values.first().map(String::as_str).unwrap_or("");
                    let mv = parse_move(raw, out.size)
                        .map_err(|e| format!("第 {move_nodes} 手坐标非法 [{raw}]：{e}"))?;
                    out.moves.push(mv);
                }
                _ => {
                    // 常见元数据静默跳过；真正不认识的属性汇总成一条提示。
                    // 不要逐个 warning：实战棋谱动辄十几个属性，会把状态栏淹掉。
                    if !SUPPORTED_PROPS.contains(&ident.as_str())
                        && !KNOWN_METADATA.contains(&ident.as_str())
                    {
                        ignored_props.insert(ident.clone());
                    }
                }
            }
        }
    }

    // AE 在 AB/AW 之后统一生效（同一节点里的先后顺序不影响结果）
    out.setup.retain(|(_, p)| !cleared.contains(p));

    if out.moves.is_empty() && out.setup.is_empty() {
        return Err("没有找到任何落子（可能是空棋谱或只含变化图）".to_string());
    }
    // 规则不认识就明确告警，不能静默按 Chinese 处理
    if let Some(w) = rules_warning(out.rules.as_deref()) {
        out.warnings.push(w);
    }
    if !ignored_props.is_empty() {
        let total = ignored_props.len();
        let mut list: Vec<String> = ignored_props.into_iter().collect();
        list.truncate(8);
        let more = if total > list.len() {
            format!(" 等 {total} 个")
        } else {
            String::new()
        };
        // 状态栏只显示前 5 条，这条插到最前才不会被挤掉
        out.warnings
            .insert(0, format!("忽略了不支持的属性：{}{more}", list.join(", ")));
    }
    // 状态栏只有一行，warning 太多反而看不清
    if out.warnings.len() > 5 {
        let rest = out.warnings.len() - 5;
        out.warnings.truncate(5);
        out.warnings.push(format!("…还有 {rest} 条警告"));
    }
    Ok(out)
}

/// 严格模式：有任何 warning 就报错。
pub fn parse_strict(src: &str) -> Result<SgfGame, String> {
    let g = parse(src)?;
    if let Some(w) = g.warnings.first() {
        return Err(format!("严格模式：{w}（共 {} 条警告）", g.warnings.len()));
    }
    Ok(g)
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
            let node_no = nodes.len() + 1;
            nodes.push(parse_node(chars, i).map_err(|e| format!("第 {node_no} 个节点：{e}"))?);
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

/// 解析一个节点（`;` 之后到下一个 `;` / `(` / `)` 之前）。
///
/// SGF 要求每个属性至少有一个 `[...]` 值；`B` / `W` 的空值 `B[]` 才是合法 Pass。
/// 缺值、未闭合的 `[`、值末尾孤立的反斜杠都必须报错，不能悄悄当成 Pass。
fn parse_node(chars: &[char], i: &mut usize) -> Result<Props, String> {
    *i += 1; // ';'
    let mut props = Vec::new();
    let mut move_prop: Option<String> = None;
    loop {
        skip_ws(chars, i);
        if *i >= chars.len() || !chars[*i].is_ascii_uppercase() {
            if *i < chars.len() && chars[*i].is_ascii_lowercase() {
                return Err(format!(
                    "第 {} 个字符：SGF 属性名必须大写（{:?}）",
                    *i + 1,
                    chars[*i]
                ));
            }
            break;
        }
        let start = *i;
        while *i < chars.len() && chars[*i].is_ascii_uppercase() {
            *i += 1;
        }
        let ident: String = chars[start..*i].iter().collect();
        // 属性名和 `[` 之间允许空白（手写棋谱常见 `B [cc]`）
        skip_ws(chars, i);
        let mut values: Vec<String> = Vec::new();
        while *i < chars.len() && chars[*i] == '[' {
            let open = *i;
            *i += 1;
            let mut v = String::new();
            let mut closed = false;
            while *i < chars.len() {
                match chars[*i] {
                    ']' => {
                        *i += 1;
                        closed = true;
                        break;
                    }
                    '\\' => {
                        *i += 1;
                        if *i >= chars.len() {
                            return Err(format!(
                                "第 {} 个字符：属性 {ident} 的值以反斜杠结尾",
                                open + 1
                            ));
                        }
                        // SGF：反斜杠 + 换行表示行续接，两者都从值里删掉
                        match chars[*i] {
                            '\r' => {
                                *i += 1;
                                if *i < chars.len() && chars[*i] == '\n' {
                                    *i += 1;
                                }
                            }
                            '\n' => *i += 1,
                            c => {
                                v.push(c);
                                *i += 1;
                            }
                        }
                    }
                    c => {
                        v.push(c);
                        *i += 1;
                    }
                }
            }
            if !closed {
                return Err(format!(
                    "第 {} 个字符：属性 {ident} 的 [ 没有闭合",
                    open + 1
                ));
            }
            values.push(v);
            skip_ws(chars, i);
        }
        if values.is_empty() {
            // `YBar` 这种：属性名只能由大写字母组成，下面跟着小写说明是写错了
            if *i < chars.len() && chars[*i].is_ascii_lowercase() {
                return Err(format!(
                    "第 {} 个字符：SGF 属性名必须大写（{:?} 紧跟 {ident}）",
                    *i + 1,
                    chars[*i]
                ));
            }
            return Err(format!(
                "第 {} 个字符：属性 {ident} 缺少 [...] 值",
                start + 1
            ));
        }
        // 落子属性：一个节点只能有一手，且 B/W 只能有一个值
        if ident == "B" || ident == "W" {
            if values.len() != 1 {
                return Err(format!(
                    "第 {} 个字符：{ident} 只能有一个值（实际 {} 个）",
                    start + 1,
                    values.len()
                ));
            }
            if let Some(prev) = &move_prop {
                return Err(format!(
                    "第 {} 个字符：同一节点不能同时有 {prev} 和 {ident}",
                    start + 1
                ));
            }
            move_prop = Some(ident.clone());
        }
        props.push((ident, values));
    }
    Ok(props)
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

/// 精确识别 SGF `RU`（大小写不敏感，允许 `Australian rules` 这类后缀）。
///
/// 只返回项目**声明支持**的规则集；New Zealand 因允许自杀暂不支持，
/// Ing 的超级劫/循环处理也只按通用 SSK 近似。
pub fn recognized_rule_set(ru: &str) -> Option<RuleSet> {
    let r = ru.trim().to_ascii_lowercase();
    if r.starts_with("chinese") {
        Some(RuleSet::CHINESE)
    } else if r.starts_with("japanese") {
        Some(RuleSet::JAPANESE)
    } else if r.starts_with("korean") {
        Some(RuleSet::KOREAN)
    } else if r.starts_with("aga") {
        Some(RuleSet::AGA)
    } else if r.starts_with("ing") {
        Some(RuleSet::ING)
    } else {
        None
    }
}

/// SGF `RU` → 内部规则集。未识别时回退默认（Chinese），
/// 调用方应同时用 [`rules_warning`] 告诉用户发生了回退，不能静默。
pub fn rule_set_from_rules(rules: Option<&str>) -> RuleSet {
    rules.and_then(recognized_rule_set).unwrap_or_default()
}

/// 规则不认识或被近似处理时的提示；明确支持的规则返回 `None`。
pub fn rules_warning(rules: Option<&str>) -> Option<String> {
    let trimmed = rules?.trim();
    if trimmed.is_empty() || recognized_rule_set(trimmed).is_some() {
        return None;
    }
    if trimmed.to_ascii_lowercase().starts_with("new zealand") {
        // NZ 允许自杀（自己填气也算合法），本工具 Goban 一律禁止自杀
        return Some(format!(
            "暂不支持 RU[{trimmed}]（允许自杀），按 Chinese 处理"
        ));
    }
    Some(format!("无法识别的 RU[{trimmed}]，按 Chinese 处理"))
}

/// 从 SGF `RU` 只取劫规则（保留给只需要 KoRule 的调用方）。
pub fn ko_rule_from_rules(rules: Option<&str>) -> KoRule {
    rule_set_from_rules(rules).ko
}

/// 写回 SGF `RU` 时用的名字。
pub fn rules_for_ko(ko: KoRule) -> &'static str {
    RuleSet::from_ko(ko).name
}

/// 把局面写回 SGF（只写主线，带摆子和贴目）。
pub fn write_game(
    size: u8,
    komi: f32,
    setup: &[(Color, Point)],
    moves: &[Move],
    rule: RuleSet,
    first: Color,
) -> String {
    fn coord(p: Point) -> String {
        let mut s = String::new();
        s.push((b'a' + p.x) as char);
        s.push((b'a' + p.y) as char);
        s
    }
    let mut out = String::from("(;GM[1]FF[4]CA[UTF-8]");
    out.push_str(&format!("SZ[{size}]KM[{komi:.1}]RU[{}]", rule.name));
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

    // 没有落子时用 PL 记住先手（否则重载后只能按摆子惯例猜）
    if moves.is_empty() {
        out.push_str(&format!("PL[{}]", first.sgf_char()));
    }
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
    use goban::Scoring;

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
        let text = write_game(19, 7.5, &[], &moves, RuleSet::default(), Color::Black);
        let back = parse(&text).unwrap();
        assert_eq!(back.size, 19);
        assert_eq!(back.komi, Some(7.5));
        assert_eq!(back.moves, moves);
    }

    #[test]
    fn writes_and_reparses_two_passes() {
        let moves = vec![Move::Play(Point::new(2, 2)), Move::Pass, Move::Pass];
        let text = write_game(9, 7.5, &[], &moves, RuleSet::default(), Color::Black);
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
    fn property_name_and_value_may_be_separated_by_whitespace() {
        let g = parse("(;SZ[9];B [cc];W\n[gg])").unwrap();
        assert_eq!(
            g.moves,
            vec![Move::Play(Point::new(2, 2)), Move::Play(Point::new(6, 6))]
        );
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    }

    #[test]
    fn move_properties_must_be_single_valued() {
        for src in ["(;SZ[9];B[cc][dd];W[ee])", "(;SZ[9];B[cc];W[dd][ee])"] {
            let err = parse(src).unwrap_err();
            assert!(err.contains("只能有一个值"), "{src}: {err}");
            assert!(err.contains("个节点"), "要带节点号：{err}");
            assert!(err.contains("个字符"), "要带字符位置：{err}");
        }
    }

    #[test]
    fn one_node_cannot_hold_two_moves() {
        let err = parse("(;SZ[9];B[cc]W[dd])").unwrap_err();
        assert!(err.contains("同一节点不能同时有"), "{err}");
    }

    #[test]
    fn ignored_pl_does_not_also_claim_it_won() {
        let g = parse("(;SZ[9];B[cc];PL[W])").unwrap();
        assert!(
            g.warnings
                .iter()
                .any(|w| w.contains("忽略第 3 个节点的 PL")),
            "{:?}",
            g.warnings
        );
        assert!(
            !g.warnings.iter().any(|w| w.contains("使用最后一个")),
            "被忽略的 PL 不能说成生效：{:?}",
            g.warnings
        );
    }

    #[test]
    fn scalar_properties_warn_on_extra_or_duplicate_values() {
        // 多值：只取第一个，但必须说出来
        for (src, needle) in [
            ("(;SZ[9][13];B[cc])", "SZ 有多个值"),
            ("(;SZ[9]RU[Chinese][Japanese];B[cc])", "RU 有多个值"),
            ("(;SZ[9]HA[2][3];W[cc])", "HA 有多个值"),
            ("(;SZ[9]RE[W+R][B+R];B[cc])", "RE 有多个值"),
        ] {
            let g = parse(src).unwrap();
            assert!(
                g.warnings.iter().any(|w| w.contains(needle)),
                "{src} 应提示 {needle}：{:?}",
                g.warnings
            );
        }
        // 重复属性：只取第一个
        for (src, needle) in [
            ("(;SZ[9]SZ[13];B[cc])", "重复的 SZ"),
            ("(;SZ[9]KM[6.5]KM[7.5];B[cc])", "重复的 KM"),
            ("(;SZ[9]RU[Chinese]RU[Japanese];B[cc])", "重复的 RU"),
            ("(;SZ[9]PL[B]PL[W];B[cc])", "重复的 PL"),
        ] {
            let g = parse(src).unwrap();
            assert!(
                g.warnings.iter().any(|w| w.contains(needle)),
                "{src} 应提示 {needle}：{:?}",
                g.warnings
            );
        }
    }

    #[test]
    fn setup_properties_still_accept_multiple_values() {
        let g = parse("(;SZ[9]AB[aa][bb][cc]AW[dd][ee]AE[aa];W[ff])").unwrap();
        // AB 三个、AW 两个，AE 清掉 aa
        assert_eq!(g.setup.len(), 4, "{:?}", g.setup);
        assert!(!g.setup.iter().any(|(_, p)| *p == Point::new(0, 0)));
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    }

    #[test]
    fn property_without_a_value_is_an_error() {
        // B/W 缺 [] 不能被当成 Pass
        for src in [
            "(;SZ[9];B;W[cc])",
            "(;SZ[9];B[cc];W)",
            "(;SZ[9];SZ)",
            "(;SZ[9]C[later];B[cc];W[dd];C)",
        ] {
            let err = parse(src).unwrap_err();
            assert!(err.contains("缺少 [...] 值"), "{src} 应报缺值，实际：{err}");
        }
    }

    #[test]
    fn empty_brackets_are_still_a_pass() {
        let g = parse("(;SZ[9];B[];W[cc];B[];W[dd])").unwrap();
        assert_eq!(
            g.moves,
            vec![
                Move::Pass,
                Move::Play(Point::new(2, 2)),
                Move::Pass,
                Move::Play(Point::new(3, 3)),
            ]
        );
    }

    #[test]
    fn truncated_input_reports_a_position_and_never_panics() {
        let err = parse("(;SZ[9];B[cc").unwrap_err();
        assert!(err.contains("没有闭合"), "{err}");
        assert!(err.contains("第 "), "要带字符位置：{err}");

        let err = parse("(;SZ[9];B[cc]").unwrap_err();
        assert!(err.contains("括号不匹配"), "{err}");

        // 值末尾孤立的反斜杠
        let err = parse("(;SZ[9]C[abc\\").unwrap_err();
        assert!(
            err.contains("反斜杠结尾") || err.contains("没有闭合"),
            "{err}"
        );
    }

    #[test]
    fn escaped_brackets_and_line_continuations_follow_sgf_rules() {
        // 值里的 \] 是字面 ]，不会提前结束属性
        let g = parse("(;SZ[9]C[a \\] b];B[cc])").unwrap();
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);

        // 反斜杠 + 换行 = 行续接，两者都删掉
        let g = parse("(;SZ[9]C[ab\\\ncd];B[cc])").unwrap();
        assert_eq!(g.moves, vec![Move::Play(Point::new(2, 2))]);
        let g = parse("(;SZ[9]C[ab\\\r\ncd];B[cc])").unwrap();
        assert_eq!(g.moves, vec![Move::Play(Point::new(2, 2))]);
    }

    #[test]
    fn lowercase_property_names_are_rejected_with_a_hint() {
        let err = parse("(;SZ[9];B[cc]YBar[1])").unwrap_err();
        assert!(err.contains("必须大写"), "{err}");
    }

    #[test]
    fn a_malformed_first_game_does_not_swallow_the_next_one() {
        let err = parse_collection("(;SZ[9];B;W[cc])(;SZ[9];B[dd])").unwrap_err();
        assert!(err.contains("缺少 [...] 值"), "{err}");
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
    fn parses_a_collection_of_games() {
        let src = "(;SZ[9]KM[5.5];B[cc];W[gg])(;SZ[13]RU[Japanese];B[dd];W[ee])";
        let games = parse_collection(src).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].size, 9);
        assert_eq!(games[0].moves.len(), 2);
        assert_eq!(games[1].size, 13);
        assert_eq!(games[1].rules.as_deref(), Some("Japanese"));

        // 单盘读取只拿第一盘，但必须提示还有别的，不能静默截断
        let first = parse(src).unwrap();
        assert_eq!(first.size, 9);
        assert!(
            first.warnings.iter().any(|w| w.contains("2 盘棋")),
            "{:?}",
            first.warnings
        );
        // --strict 会把这条 warning 变成错误
        assert!(parse_strict(src).unwrap_err().contains("盘棋"));
    }

    #[test]
    fn collection_boundaries_survive_brackets_and_variations() {
        // 注释里的括号不能打乱树边界；第一盘里面的变化分支要跳过，
        // 第二个顶层 tree 仍然要能读到。
        let src = "(;SZ[9]C[a ) and ( inside];B[cc](;W[dd])(;W[gg]))(;SZ[9];B[ff])";
        let games = parse_collection(src).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(
            games[0].moves,
            vec![Move::Play(Point::new(2, 2)), Move::Play(Point::new(3, 3))]
        );
        assert_eq!(games[1].moves, vec![Move::Play(Point::new(5, 5))]);
    }

    #[test]
    fn trailing_and_leading_junk_report_position() {
        let err = parse("(;SZ[9];B[cc]) oops").unwrap_err();
        assert!(err.contains("尾部有多余内容"), "{err}");
        assert!(err.contains("第 "), "要报字符位置：{err}");

        let err = parse("junk(;SZ[9];B[cc])").unwrap_err();
        assert!(err.contains("之前有多余内容"), "{err}");

        // 空白和 BOM 是合法的前缀
        assert!(parse("\u{feff}\n  (;SZ[9];B[cc])").is_ok());
        assert!(parse("(;SZ[9];B[cc])\n\n").is_ok());
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
            let text = write_game(
                size,
                7.5,
                &[],
                game.moves(),
                RuleSet::default(),
                Color::Black,
            );
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
        let text = write_game(9, 0.5, &setup, &moves, RuleSet::default(), Color::White);
        let back = parse(&text).unwrap();

        assert_eq!(back.size, 9);
        assert_eq!(back.komi, Some(0.5));
        assert_eq!(back.moves, moves);
        assert_eq!(back.first_to_move(), Color::White, "让子棋白先");
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
            let text = write_game(size, 7.5, &[], &moves, RuleSet::default(), Color::Black);
            let back = parse(&text).unwrap();
            assert_eq!(back.moves, moves, "{size} 路盘连续 Pass 必须读回");
        }
    }

    #[test]
    fn ru_survives_write_and_parse() {
        for rule in [
            RuleSet::CHINESE,
            RuleSet::JAPANESE,
            RuleSet::KOREAN,
            RuleSet::AGA,
            RuleSet::ING,
        ] {
            let text = write_game(
                9,
                7.5,
                &[],
                &[Move::Play(Point::new(2, 2))],
                rule,
                Color::Black,
            );
            let back = parse(&text).unwrap();
            assert_eq!(
                recognized_rule_set(back.rules.as_deref().unwrap()),
                Some(rule),
                "规则名必须原样往返：{text}"
            );
        }
    }

    #[test]
    fn rule_mapping_is_explicit() {
        let rs = |s| rule_set_from_rules(Some(s));
        assert_eq!(rs("Japanese"), RuleSet::JAPANESE);
        assert_eq!(rs("Japanese rules"), RuleSet::JAPANESE);
        assert_eq!(rs("Korean"), RuleSet::KOREAN);
        assert_eq!(rs("Chinese"), RuleSet::CHINESE);
        // AGA / Ing 以前被折叠成 Korean，现在要保住规范名
        assert_eq!(rs("AGA"), RuleSet::AGA);
        assert_eq!(rs("Ing"), RuleSet::ING);
        assert_eq!(rs("Ing rules (SST)"), RuleSet::ING);
        assert_eq!(rs(" chinese "), RuleSet::CHINESE);
        // 大小写不能混进来："something else" 含 ing，但不是 Ing 规则
        assert_eq!(rs("something else"), RuleSet::default());
        assert_eq!(rule_set_from_rules(None), RuleSet::default());

        // 计分方式不能丢：日本/韩国是数目，其余是数子
        assert_eq!(RuleSet::JAPANESE.scoring, Scoring::Territory);
        assert_eq!(RuleSet::KOREAN.scoring, Scoring::Territory);
        assert_eq!(RuleSet::CHINESE.scoring, Scoring::Area);
        assert_eq!(RuleSet::AGA.scoring, Scoring::Area);
        assert_eq!(RuleSet::ING.scoring, Scoring::Area);
    }

    #[test]
    fn unknown_or_unsupported_rules_warn() {
        // 明确支持的不该有警告
        for ru in ["Chinese", "Japanese", "Korean", "AGA", "Ing"] {
            assert_eq!(rules_warning(Some(ru)), None, "{ru}");
        }
        // 不认识和暂不支持的都要说清楚，不能静默
        let w = rules_warning(Some("New Zealand")).unwrap();
        assert!(w.contains("New Zealand") && w.contains("自杀"), "{w}");
        let w = rules_warning(Some("My House Rules")).unwrap();
        assert!(w.contains("My House Rules"), "{w}");
        assert_eq!(rules_warning(None), None);

        // parse 要把警告带出来
        let g = parse("(;SZ[9]RU[Some Odd Rules];B[dd])").unwrap();
        assert!(
            g.warnings.iter().any(|w| w.contains("Some Odd Rules")),
            "{:?}",
            g.warnings
        );
        assert_eq!(rule_set_from_rules(g.rules.as_deref()), RuleSet::CHINESE);

        let g = parse("(;SZ[9]RU[New Zealand];B[dd])").unwrap();
        assert!(
            g.warnings.iter().any(|w| w.contains("New Zealand")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn ko_rule_mapping() {
        assert_eq!(ko_rule_from_rules(Some("Japanese")), KoRule::Simple);
        assert_eq!(ko_rule_from_rules(Some("Japanese rules")), KoRule::Simple);
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
        // New Zealand 允许自杀，本工具不支持：回退 Chinese（PSK）并告警，
        // 不再错误地映射成「只禁单劫」
        assert_eq!(
            ko_rule_from_rules(Some("New Zealand")),
            KoRule::PositionalSuperko
        );
    }

    /// 文件里的 B/W 是事实：有开局摆子也不能把第一手吞掉。
    #[test]
    fn ae_clears_setup_stones() {
        // AE 只清开局摆子；同节点里 AB/AW/AE 的先后顺序不影响结果
        // 没有 B/W 时先手才由摆子/HA 推断
        let g = parse("(;SZ[9]AB[dd][gg]AW[cc]AE[dd])").unwrap();
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);
        let points: Vec<(Color, Point)> = g.setup.clone();
        assert!(
            !points.iter().any(|(_, p)| *p == Point::new(3, 3)),
            "dd 应被 AE 清掉：{points:?}"
        );
        assert!(points.contains(&(Color::Black, Point::new(6, 6))));
        assert!(points.contains(&(Color::White, Point::new(2, 2))));
        // 清掉的正好是黑子 dd，剩下的还有黑子 gg → 仍是白先
        assert_eq!(g.first_to_move(), Color::White);

        // 有显式首手时以文件为准（B[ee] 就是黑先），不能被摆子推翻
        let g = parse("(;SZ[9]AB[gg];B[ee])").unwrap();
        assert_eq!(g.first_to_move(), Color::Black);

        // 把所有黑子都清掉：回到黑先
        let g = parse("(;SZ[9]AB[dd]AE[dd];B[cc])").unwrap();
        assert!(g.setup.is_empty());
        assert_eq!(g.first_to_move(), Color::Black);
    }

    #[test]
    fn mid_game_ae_is_rejected() {
        let err = parse("(;SZ[9];B[cc];AE[cc])").unwrap_err();
        assert!(err.contains("中途清子"), "{err}");
        assert!(err.contains("第 3 个节点"), "{err}");
    }

    #[test]
    fn handicap_is_kept_and_drives_white_first() {
        let g = parse("(;SZ[19]HA[2]AB[pd][dp];W[dd])").unwrap();
        assert_eq!(g.handicap, Some(2));
        assert_eq!(g.first_to_move(), Color::White);

        // 只有 HA、没有 AB 的棋谱也该白先
        let g = parse("(;SZ[19]HA[3];W[dd])").unwrap();
        assert_eq!(g.handicap, Some(3));
        assert_eq!(g.first_to_move(), Color::White);

        // 非法值只警告，不崩溃
        let g = parse("(;SZ[19]HA[x];B[dd])").unwrap();
        assert_eq!(g.handicap, None);
        assert!(
            g.warnings.iter().any(|w| w.contains("HA")),
            "{:?}",
            g.warnings
        );

        let g = parse("(;SZ[19]HA[1];B[dd])").unwrap();
        assert_eq!(g.handicap, None);
        assert!(
            g.warnings.iter().any(|w| w.contains("让子数")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn duplicate_km_uses_the_first_even_when_it_is_invalid() {
        let g = parse("(;SZ[9]KM[bad]KM[6.5];B[cc])").unwrap();
        assert_eq!(g.komi, None, "第一个 KM 无效时不能采用第二个");
        assert!(
            g.warnings.iter().any(|w| w.contains("重复的 KM")),
            "{:?}",
            g.warnings
        );

        let g = parse("(;SZ[9]KM[]KM[6.5];B[cc])").unwrap();
        assert_eq!(g.komi, None, "第一个 KM 为空时不能采用第二个");
        assert!(
            g.warnings.iter().any(|w| w.contains("重复的 KM")),
            "{:?}",
            g.warnings
        );

        let g = parse("(;SZ[9]KM[6.5]KM[7.5];B[cc])").unwrap();
        assert_eq!(g.komi, Some(6.5), "两个都有效时取第一个");
        assert!(
            g.warnings.iter().any(|w| w.contains("重复的 KM")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn result_metadata_is_kept_without_warning() {
        let g = parse("(;SZ[9]RE[W+2.5];B[cc];W[gg])").unwrap();
        assert_eq!(g.result.as_deref(), Some("W+2.5"));
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);

        let g = parse("(;SZ[9]RE[];B[cc])").unwrap();
        assert_eq!(g.result, None, "空 RE 不当作结果");
    }

    #[test]
    fn unknown_props_are_summarised_once() {
        // SGF 属性名必须大写；小写会被当成格式错误（这是规范行为）
        let g = parse("(;SZ[9]XFOO[1]YBAR[2];B[cc];W[gg])").unwrap();
        let hits: Vec<&String> = g
            .warnings
            .iter()
            .filter(|w| w.contains("不支持的属性"))
            .collect();
        assert_eq!(hits.len(), 1, "只汇总一条：{:?}", g.warnings);
        assert!(
            hits[0].contains("XFOO") && hits[0].contains("YBAR"),
            "{}",
            hits[0]
        );
    }

    #[test]
    fn common_metadata_is_silent_so_strict_mode_still_works() {
        let src = "(;GM[1]FF[4]CA[UTF-8]AP[app:1.0]DT[2026-01-01]PB[a]PW[b]BR[1d]WR[2d]\
RE[W+R]C[hello]LB[dd:A]TR[cc];B[cc];W[gg])";
        let g = parse(src).unwrap();
        assert!(
            g.warnings.is_empty(),
            "常见元数据不该产生 warning：{:?}",
            g.warnings
        );
        assert_eq!(g.result.as_deref(), Some("W+R"));
    }

    #[test]
    fn first_to_move_infers_from_setup() {
        let ab = parse("(;SZ[9]AB[dd])").unwrap();
        assert_eq!(ab.first, None, "没有落子也没有 PL 时应交给推断");
        assert_eq!(ab.first_to_move(), Color::White);

        let aw = parse("(;SZ[9]AW[dd])").unwrap();
        assert_eq!(aw.first_to_move(), Color::Black);

        let empty = SgfGame {
            size: 9,
            first: None,
            ..Default::default()
        };
        assert_eq!(empty.first_to_move(), Color::Black);
    }

    #[test]
    fn write_game_preserves_first_player() {
        // AB + PL[B]：写回时不能按"有摆子就白先"翻转颜色
        let setup = vec![(Color::Black, Point::new(3, 3))];
        let moves = vec![Move::Play(Point::new(2, 2))];
        let text = write_game(9, 7.5, &setup, &moves, RuleSet::default(), Color::Black);
        let back = parse(&text).unwrap();
        assert_eq!(back.first, Some(Color::Black));
        assert_eq!(back.first_to_move(), Color::Black);
        assert_eq!(back.moves, moves);

        // 只有摆子时写 PL，重载后先手不丢
        let text = write_game(9, 7.5, &setup, &[], RuleSet::default(), Color::White);
        assert!(text.contains("PL[W]"), "{text}");
        let back = parse(&text).unwrap();
        assert_eq!(back.first_to_move(), Color::White);
    }

    #[test]
    fn pl_invalid_or_multi_warns() {
        let g = parse("(;SZ[9]PL[b];B[cc])").unwrap();
        assert!(
            g.warnings
                .iter()
                .any(|w| w.contains("PL") && w.contains("第 1")),
            "{:?}",
            g.warnings
        );

        let g = parse("(;SZ[9]PL[B][W];B[cc])").unwrap();
        assert!(
            g.warnings.iter().any(|w| w.contains("多个值")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn komi_edge_cases_warn() {
        let g = parse("(;SZ[9]KM[];B[cc])").unwrap();
        assert!(
            g.warnings.iter().any(|w| w.contains("为空")),
            "{:?}",
            g.warnings
        );

        // 逗号小数点按 7.5 处理，不报警
        let g = parse("(;SZ[9]KM[7,5];B[cc])").unwrap();
        assert_eq!(g.komi, Some(7.5));
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);

        // NaN / inf 不是有限数
        for v in ["NaN", "inf", "-inf"] {
            let src = format!("(;SZ[9]KM[{v}];B[cc])");
            let g = parse(&src).unwrap();
            assert_eq!(g.komi, None, "{v}");
            assert!(
                g.warnings.iter().any(|w| w.contains("有限数")),
                "{v}: {:?}",
                g.warnings
            );
        }
    }

    #[test]
    fn empty_sgf_is_an_error() {
        let err = parse("(;SZ[9])").unwrap_err();
        assert!(err.contains("没有找到任何落子"), "{err}");
    }

    #[test]
    fn strict_mode_rejects_warnings() {
        assert!(parse_strict("(;SZ[9]KM[abc];B[cc])").is_err());
        assert!(parse_strict("(;SZ[9]KM[7.5];B[cc])").is_ok());
    }

    #[test]
    fn pl_sets_first_player() {
        // 没有落子时 PL 决定先手（AB 摆子按惯例是白先，PL[B] 覆盖它）
        let g = parse("(;SZ[9]AB[dd]PL[B])").unwrap();
        assert_eq!(g.first_to_move(), Color::Black);
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);

        let g = parse("(;SZ[9]PL[W]AW[dd])").unwrap();
        assert_eq!(g.first_to_move(), Color::White);
    }

    #[test]
    fn pl_conflicting_with_first_move_warns() {
        let g = parse("(;SZ[9]PL[W];B[cc];W[gg])").unwrap();
        assert_eq!(g.first_to_move(), Color::Black, "以文件里的首手为准");
        assert!(
            g.warnings.iter().any(|w| w.contains("PL")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn invalid_komi_warns() {
        let g = parse("(;SZ[9]KM[abc];B[cc])").unwrap();
        assert_eq!(g.komi, None);
        assert!(
            g.warnings.iter().any(|w| w.contains("KM")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn out_of_turn_move_warns() {
        let g = parse("(;SZ[9];B[cc];B[dd];W[ee])").unwrap();
        assert_eq!(g.moves.len(), 2);
        assert!(
            g.warnings.iter().any(|w| w.contains("轮次不符")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn split_root_pl_is_allowed() {
        let g = parse("(;SZ[9];PL[B];B[cc])").unwrap();
        assert!(
            !g.warnings.iter().any(|w| w.contains("PL")),
            "{:?}",
            g.warnings
        );
        assert_eq!(g.first_to_move(), Color::Black);
    }

    #[test]
    fn non_root_pl_warns() {
        let g = parse("(;SZ[9];B[cc];W[gg]PL[B])").unwrap();
        assert!(
            g.warnings.iter().any(|w| w.contains("PL")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn declared_colors_are_preserved_with_setup() {
        // 根节点被拆成两个节点、摆子仍在前：允许
        let g = parse("(;SZ[9];AB[ee];B[dd])").unwrap();
        assert_eq!(g.setup.len(), 1);
        assert_eq!(g.moves.len(), 1);

        // 让子棋：AW 摆白子，黑先
        let g = parse("(;SZ[9]AW[dd];B[cc];W[gg])").unwrap();
        assert_eq!(g.moves.len(), 2, "AW + 黑先不能丢第一手");
        assert_eq!(g.first, Some(Color::Black));
        assert_eq!(g.first_to_move(), Color::Black);

        // 让子棋：AB + 白先
        let g = parse("(;SZ[9]AB[dd];W[cc])").unwrap();
        assert_eq!(g.first, Some(Color::White));
        assert_eq!(g.first_to_move(), Color::White);
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
