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
    write_game_with_result(size, komi, setup, moves, rule, first, None)
}

/// 写 SGF，可带上已确认的 `RE`。
///
/// `result` 只应在用户确认终局后传入；没有确认就不写 `RE`，避免伪造胜负。
pub fn write_game_with_result(
    size: u8,
    komi: f32,
    setup: &[(Color, Point)],
    moves: &[Move],
    rule: RuleSet,
    first: Color,
    result: Option<&str>,
) -> String {
    fn coord(p: Point) -> String {
        let mut s = String::new();
        s.push((b'a' + p.x) as char);
        s.push((b'a' + p.y) as char);
        s
    }
    let mut out = String::from("(;GM[1]FF[4]CA[UTF-8]");
    // 用最短往返表示（f32 Display），不要 `{:.1}`：6.25/5.75 会被静默舍成 6.2/5.8，
    // 反复保存会让贴目持续漂移。调用方负责保证 komi 有限。
    out.push_str(&format!("SZ[{size}]KM[{komi}]RU[{}]", rule.name));
    if let Some(result) = result.filter(|r| !r.is_empty()) {
        // SGF 属性值里的 `]` 和 `\` 必须转义，否则整份棋谱解析失败
        let escaped = result.replace('\\', "\\\\").replace(']', "\\]");
        out.push_str(&format!("RE[{escaped}]"));
    }
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
mod tests;
