//! 围棋规则核心：棋盘、提子、劫争规则、面积数子。
//!
//! 本 crate 不做 IO，也不认识 SGF / GTP 字符串——坐标转换只在 `sgf` / `engine` 里。

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Color {
    Black,
    White,
}

impl Color {
    pub fn opposite(self) -> Color {
        match self {
            Color::Black => Color::White,
            Color::White => Color::Black,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Color::Black => 0,
            Color::White => 1,
        }
    }

    pub fn name_zh(self) -> &'static str {
        match self {
            Color::Black => "黑",
            Color::White => "白",
        }
    }

    pub fn sgf_char(self) -> char {
        match self {
            Color::Black => 'B',
            Color::White => 'W',
        }
    }
}

/// 0-based 坐标，左上角为 (0, 0)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Point {
    pub x: u8,
    pub y: u8,
}

impl Point {
    pub const fn new(x: u8, y: u8) -> Self {
        Point { x, y }
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Move {
    Play(Point),
    Pass,
}

/// 劫争规则。中国规则默认用位置超级劫（PSK），比较盘面、不比较行棋方。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KoRule {
    /// 只禁止立即提回（单劫）。
    Simple,
    /// 盘面重复即禁止；不把下一手方纳入比较。
    #[default]
    PositionalSuperko,
    /// 盘面 + 下一手方重复才禁止。
    SituationalSuperko,
}

impl KoRule {
    pub fn name_zh(self) -> &'static str {
        match self {
            KoRule::Simple => "单劫",
            KoRule::PositionalSuperko => "位置超级劫",
            KoRule::SituationalSuperko => "情境超级劫",
        }
    }
}

/// 计分方式。只影响对外呈现出与 KataGo 的规则预设，不改变死活判定。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scoring {
    /// 数子：子 + 围空（中国 / AGA / Ing）。
    #[default]
    Area,
    /// 数目：围空 + 提子（日本 / 韩国）。本工具不做死活标记，只用于选择规则。
    Territory,
}

impl Scoring {
    pub fn name_zh(self) -> &'static str {
        match self {
            Scoring::Area => "数子",
            Scoring::Territory => "数目",
        }
    }
}

/// 一套完整规则：SGF `RU` 的语义载体，同时喂给 Goban 与 KataGo。
///
/// 只覆盖项目声明支持的规则；未识别的 `RU` 由调用方警告后回退默认，
/// 不允许静默当成某一套“差不多”的规则。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RuleSet {
    pub ko: KoRule,
    pub scoring: Scoring,
    /// 规范名（SGF `RU` 文本，同时也是 KataGo `rules` 预设名的小写形式）。
    pub name: &'static str,
}

impl RuleSet {
    pub const CHINESE: RuleSet = RuleSet {
        ko: KoRule::PositionalSuperko,
        scoring: Scoring::Area,
        name: "Chinese",
    };
    pub const JAPANESE: RuleSet = RuleSet {
        ko: KoRule::Simple,
        scoring: Scoring::Territory,
        name: "Japanese",
    };
    pub const KOREAN: RuleSet = RuleSet {
        ko: KoRule::SituationalSuperko,
        scoring: Scoring::Territory,
        name: "Korean",
    };
    pub const AGA: RuleSet = RuleSet {
        ko: KoRule::SituationalSuperko,
        scoring: Scoring::Area,
        name: "AGA",
    };
    pub const ING: RuleSet = RuleSet {
        ko: KoRule::SituationalSuperko,
        scoring: Scoring::Area,
        name: "Ing",
    };

    /// 从 `KoRule` 近似还原（旧接口/无 `RU` 时的默认）。
    pub fn from_ko(ko: KoRule) -> Self {
        match ko {
            KoRule::Simple => RuleSet::JAPANESE,
            KoRule::PositionalSuperko => RuleSet::CHINESE,
            KoRule::SituationalSuperko => RuleSet::KOREAN,
        }
    }

    /// KataGo `rules` 字段用的预设名。
    pub fn katago_name(self) -> &'static str {
        match self.name {
            "Chinese" => "chinese",
            "Japanese" => "japanese",
            "Korean" => "korean",
            "AGA" => "aga",
            _ => "ing",
        }
    }
}

impl Default for RuleSet {
    fn default() -> Self {
        RuleSet::CHINESE
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IllegalMove {
    OutOfBounds,
    Occupied,
    Suicide,
    Superko,
    /// 一盘棋最多 `MAX_MOVES` 手（回合号是 u16）
    TooManyMoves,
}

/// 一盘棋允许的最大手数。回合编号（`TurnEval.turn`、KataGo `turnNumber`）
/// 是 `u16`，超过就无法表达；在核心层直接拦住，避免后续转换溢出。
pub const MAX_MOVES: usize = u16::MAX as usize;

impl fmt::Display for IllegalMove {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            IllegalMove::OutOfBounds => "越界",
            IllegalMove::Occupied => "该点已有子",
            IllegalMove::Suicide => "自杀",
            IllegalMove::Superko => "重复局面（劫争规则）",
            IllegalMove::TooManyMoves => "手数超过 65535（回合号是 u16）",
        };
        f.write_str(s)
    }
}

/// 静态盘面。`set` 是给摆子用的，调用方自己保证不破坏不变量。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Board {
    size: u8,
    cells: Vec<Option<Color>>,
}

impl Board {
    pub fn new(size: u8) -> Self {
        assert!(size > 1 && size <= 25, "盘面大小必须在 2..=25");
        Board {
            size,
            cells: vec![None; size as usize * size as usize],
        }
    }

    pub fn size(&self) -> u8 {
        self.size
    }

    pub fn in_bounds(&self, p: Point) -> bool {
        p.x < self.size && p.y < self.size
    }

    pub fn at(&self, p: Point) -> Option<Color> {
        if self.in_bounds(p) {
            self.cells[self.idx(p)]
        } else {
            None
        }
    }

    pub fn set(&mut self, p: Point, value: Option<Color>) {
        if self.in_bounds(p) {
            let i = self.idx(p);
            self.cells[i] = value;
        }
    }

    pub fn stones(&self) -> impl Iterator<Item = (Point, Color)> + '_ {
        let size = self.size;
        self.cells.iter().enumerate().filter_map(move |(i, cell)| {
            cell.map(|c| {
                (
                    Point::new((i % size as usize) as u8, (i / size as usize) as u8),
                    c,
                )
            })
        })
    }

    fn idx(&self, p: Point) -> usize {
        p.y as usize * self.size as usize + p.x as usize
    }

    pub fn neighbors(&self, p: Point) -> Vec<Point> {
        let mut out = Vec::with_capacity(4);
        if p.x > 0 {
            out.push(Point::new(p.x - 1, p.y));
        }
        if p.y > 0 {
            out.push(Point::new(p.x, p.y - 1));
        }
        if p.x + 1 < self.size {
            out.push(Point::new(p.x + 1, p.y));
        }
        if p.y + 1 < self.size {
            out.push(Point::new(p.x, p.y + 1));
        }
        out
    }

    /// 返回 (同色连通块, 气)。起点为空则返回两个空 vec。
    pub fn group_and_liberties(&self, start: Point) -> (Vec<Point>, Vec<Point>) {
        let color = match self.at(start) {
            Some(c) => c,
            None => return (Vec::new(), Vec::new()),
        };
        let mut stones = Vec::new();
        let mut liberties = Vec::new();
        let mut seen = vec![false; self.cells.len()];
        let mut stack = vec![start];
        seen[self.idx(start)] = true;
        while let Some(p) = stack.pop() {
            stones.push(p);
            for n in self.neighbors(p) {
                let i = self.idx(n);
                if seen[i] {
                    continue;
                }
                seen[i] = true;
                match self.cells[i] {
                    Some(c) if c == color => stack.push(n),
                    Some(_) => {}
                    None => liberties.push(n),
                }
            }
        }
        (stones, liberties)
    }
}

/// 可落子的局面：盘面 + 轮到谁。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Position {
    pub board: Board,
    pub to_move: Color,
    /// 双方累计提子数。
    ///
    /// 用 `u32` 而不是 `u16`：每颗落下的子最多被提一次，所以总提子数
    /// ≤ 手数 + 开局摆子 ≤ `MAX_MOVES(65535) + 25×25(625)`。这个上界只比
    /// `u16::MAX` 大 625，极端棋谱（几乎每手都提子）会让 `u16` 溢出。
    pub captures: [u32; 2],
    pub last_move: Option<Move>,
}

impl Position {
    pub fn new(size: u8) -> Self {
        Position {
            board: Board::new(size),
            to_move: Color::Black,
            captures: [0, 0],
            last_move: None,
        }
    }

    pub fn try_play(&self, mv: Move) -> Result<Position, IllegalMove> {
        match mv {
            Move::Pass => Ok(Position {
                board: self.board.clone(),
                to_move: self.to_move.opposite(),
                captures: self.captures,
                last_move: Some(Move::Pass),
            }),
            Move::Play(p) => {
                if !self.board.in_bounds(p) {
                    return Err(IllegalMove::OutOfBounds);
                }
                if self.board.at(p).is_some() {
                    return Err(IllegalMove::Occupied);
                }
                let color = self.to_move;
                let mut board = self.board.clone();
                board.set(p, Some(color));

                let mut captured = 0u32;
                for n in board.neighbors(p) {
                    if board.at(n) == Some(color.opposite()) {
                        let (stones, liberties) = board.group_and_liberties(n);
                        if liberties.is_empty() {
                            for s in stones {
                                board.set(s, None);
                                captured += 1;
                            }
                        }
                    }
                }

                let (_, liberties) = board.group_and_liberties(p);
                if liberties.is_empty() {
                    return Err(IllegalMove::Suicide);
                }

                let mut captures = self.captures;
                captures[color.index()] += captured;
                Ok(Position {
                    board,
                    to_move: color.opposite(),
                    captures,
                    last_move: Some(mv),
                })
            }
        }
    }
}

/// 确定性 Zobrist：不依赖随机源，保证同样局面得到同样 hash。
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 只哈希盘面（位置超级劫用）。
pub fn hash_board(pos: &Position) -> u64 {
    let size = pos.board.size() as u64;
    let mut h = 0u64;
    for (p, c) in pos.board.stones() {
        let key = (p.y as u64 * size + p.x as u64) * 2 + c.index() as u64 + 1;
        h ^= splitmix64(key);
    }
    h
}

/// 哈希盘面 + 下一手方（情境超级劫用）。
pub fn hash_position(pos: &Position) -> u64 {
    let mut h = hash_board(pos);
    if pos.to_move == Color::White {
        h ^= splitmix64(0x51ED_2701);
    }
    h
}

/// 整盘棋：带历史的局面栈，支持撤销和可配置的劫争规则。
#[derive(Clone, Debug)]
pub struct Game {
    size: u8,
    komi: f32,
    setup: Vec<(Color, Point)>,
    rule: RuleSet,
    /// 开局第一手由谁下。SGF 的 `PL`/首手颜色是文件事实，不能事后靠摆子猜。
    first: Color,
    positions: Vec<Position>,
    board_hashes: Vec<u64>,
    situational_hashes: Vec<u64>,
    moves: Vec<Move>,
}

impl Game {
    pub fn new(size: u8, komi: f32) -> Self {
        Self::with_setup(size, komi, Vec::new())
    }

    pub fn with_setup(size: u8, komi: f32, setup: Vec<(Color, Point)>) -> Self {
        Self::with_setup_and_rule(size, komi, setup, KoRule::default())
    }

    pub fn with_rule(size: u8, komi: f32, ko: KoRule) -> Self {
        Self::with_setup_and_rule(size, komi, Vec::new(), ko)
    }

    pub fn with_setup_and_rule(
        size: u8,
        komi: f32,
        setup: Vec<(Color, Point)>,
        ko: KoRule,
    ) -> Self {
        // 让子棋（黑摆子）白先；只摆白子或空盘都是黑先
        let first = if setup.iter().any(|(c, _)| *c == Color::Black) {
            Color::White
        } else {
            Color::Black
        };
        Self::with_setup_rule_first(size, komi, setup, ko, first)
    }

    /// 明确指定先手方。SGF 里的 `B`/`W` 是文件事实，不能靠摆子猜。
    pub fn with_setup_rule_first(
        size: u8,
        komi: f32,
        setup: Vec<(Color, Point)>,
        ko: KoRule,
        first: Color,
    ) -> Self {
        Self::with_setup_rules_first(size, komi, setup, RuleSet::from_ko(ko), first)
    }

    /// 完整规则 + 明确先手（SGF `RU` + `PL` 的组合，唯一不会丢信息的构造方式）。
    pub fn with_setup_rules_first(
        size: u8,
        komi: f32,
        setup: Vec<(Color, Point)>,
        rule: RuleSet,
        first: Color,
    ) -> Self {
        let mut pos = Position::new(size);
        for (c, p) in &setup {
            pos.board.set(*p, Some(*c));
        }
        pos.to_move = first;
        let board_hash = hash_board(&pos);
        let situational = hash_position(&pos);
        Game {
            size,
            komi,
            setup,
            rule,
            first,
            positions: vec![pos],
            board_hashes: vec![board_hash],
            situational_hashes: vec![situational],
            moves: Vec::new(),
        }
    }

    pub fn ko_rule(&self) -> KoRule {
        self.rule.ko
    }

    /// 当前棋局的完整规则集。
    pub fn rules(&self) -> RuleSet {
        self.rule
    }

    pub fn play(&mut self, mv: Move) -> Result<(), IllegalMove> {
        if self.moves.len() >= MAX_MOVES {
            return Err(IllegalMove::TooManyMoves);
        }
        let next = self.position().try_play(mv)?;
        let board_hash = hash_board(&next);
        let situational = hash_position(&next);
        if mv != Move::Pass {
            let repeated = match self.rule.ko {
                KoRule::Simple => {
                    // 只禁止立即提回：和上一手之前的盘面比较
                    self.positions
                        .len()
                        .checked_sub(2)
                        .is_some_and(|i| self.board_hashes[i] == board_hash)
                }
                KoRule::PositionalSuperko => self.board_hashes.contains(&board_hash),
                KoRule::SituationalSuperko => self.situational_hashes.contains(&situational),
            };
            if repeated {
                return Err(IllegalMove::Superko);
            }
        }
        self.positions.push(next);
        self.board_hashes.push(board_hash);
        self.situational_hashes.push(situational);
        self.moves.push(mv);
        Ok(())
    }

    pub fn undo(&mut self) -> Option<Move> {
        if self.moves.is_empty() {
            None
        } else {
            self.positions.pop();
            self.board_hashes.pop();
            self.situational_hashes.pop();
            self.moves.pop()
        }
    }

    pub fn position(&self) -> &Position {
        self.positions.last().expect("局面栈不会为空")
    }

    pub fn position_at(&self, turn: u16) -> Option<&Position> {
        self.positions.get(turn as usize)
    }

    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    pub fn size(&self) -> u8 {
        self.size
    }

    pub fn komi(&self) -> f32 {
        self.komi
    }

    pub fn setup(&self) -> &[(Color, Point)] {
        &self.setup
    }

    /// 开局第一手由谁下（SGF `PL` 或首手颜色）。
    pub fn first_to_move(&self) -> Color {
        self.first
    }

    /// 第 `turn` 手（1-based）由谁下。
    ///
    /// `turn` 从 1 开始；传 0 属于调用方错误，用 [`Self::checked_color_of_move`] 表达。
    pub fn color_of_move(&self, turn: u16) -> Color {
        self.checked_color_of_move(turn)
            .expect("color_of_move 的 turn 从 1 开始")
    }

    /// 第 `turn` 手（1-based）由谁下；`turn == 0` 不是合法手，返回 `None`。
    pub fn checked_color_of_move(&self, turn: u16) -> Option<Color> {
        turn.checked_sub(1).map(|t| self.to_move_at(t))
    }

    /// 第 `turn` 手开始前轮到谁（0 = 开局）。
    ///
    /// 历史范围内的局面直接读 `Position::to_move`（唯一事实来源）；
    /// 超出历史的回合按奇偶从开局先手推，绝不重新从摆子猜测。
    pub fn to_move_at(&self, turn: u16) -> Color {
        if let Some(pos) = self.positions.get(turn as usize) {
            return pos.to_move;
        }
        if turn.is_multiple_of(2) {
            self.first
        } else {
            self.first.opposite()
        }
    }

    /// 面积计子（中国 / AGA / Ing 规则），不做死活判定：把盘上所有子都当活棋。
    ///
    /// 只适用于 `Scoring::Area`；数目规则（日本 / 韩国）需要先标死子再算提子，
    /// 本函数不适用。界面上的目差来自引擎 `scoreLead`，不依赖这个函数。
    pub fn area_score(&self) -> AreaScore {
        let board = &self.position().board;
        let mut black = 0u16;
        let mut white = 0u16;
        let mut visited_empty = vec![false; board.cells.len()];
        for (_, c) in board.stones() {
            match c {
                Color::Black => black += 1,
                Color::White => white += 1,
            }
        }
        for y in 0..board.size() {
            for x in 0..board.size() {
                let p = Point::new(x, y);
                let i = board.idx(p);
                if board.at(p).is_some() || visited_empty[i] {
                    continue;
                }
                // 只遍历空点；边界棋子只用来判断归属
                let mut region = 0u16;
                let mut touches_black = false;
                let mut touches_white = false;
                let mut stack = vec![p];
                visited_empty[i] = true;
                while let Some(q) = stack.pop() {
                    region += 1;
                    for n in board.neighbors(q) {
                        let ni = board.idx(n);
                        match board.cells[ni] {
                            None => {
                                if !visited_empty[ni] {
                                    visited_empty[ni] = true;
                                    stack.push(n);
                                }
                            }
                            Some(Color::Black) => touches_black = true,
                            Some(Color::White) => touches_white = true,
                        }
                    }
                }
                match (touches_black, touches_white) {
                    (true, false) => black += region,
                    (false, true) => white += region,
                    _ => {}
                }
            }
        }
        AreaScore {
            black,
            white,
            komi: self.komi,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaScore {
    pub black: u16,
    pub white: u16,
    pub komi: f32,
}

impl AreaScore {
    pub fn black_total(&self) -> f32 {
        self.black as f32
    }

    pub fn white_total(&self) -> f32 {
        self.white as f32 + self.komi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: u8, y: u8) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn capture_single_stone() {
        let mut g = Game::new(9, 7.5);
        g.play(Move::Play(p(1, 0))).unwrap(); // B
        g.play(Move::Play(p(0, 0))).unwrap(); // W
        g.play(Move::Play(p(0, 1))).unwrap(); // B：提掉 (0,0)
        assert_eq!(g.position().board.at(p(0, 0)), None);
        assert_eq!(g.position().captures[Color::Black.index()], 1);
    }

    #[test]
    fn captures_do_not_overflow_u16_range() {
        // 总提子上界是 手数 + 摆子 ≤ 66160，比 u16::MAX 多 625：
        // 极端棋谱（几乎每手都提子）会踩到。这里把计数推到 u16::MAX 再提一子，
        // 旧的 u16 实现会 debug panic / release 回绕。
        let mut pos = Position::new(9);
        pos.captures[Color::Black.index()] = u32::from(u16::MAX);
        // 角上的白子只剩 (1,0) 一口气
        pos.board.set(p(0, 0), Some(Color::White));
        pos.board.set(p(0, 1), Some(Color::Black));
        pos.to_move = Color::Black;

        let next = pos.try_play(Move::Play(p(1, 0))).unwrap();
        assert_eq!(next.board.at(p(0, 0)), None, "白子应被提掉");
        assert_eq!(
            next.captures[Color::Black.index()],
            u32::from(u16::MAX) + 1,
            "累计提子不能停在或回绕到 u16 边界"
        );
    }

    #[test]
    fn suicide_is_illegal() {
        let mut g = Game::new(9, 7.5);
        // 白先把 (0,0) 围成只有一口气的点，黑下进去就是自杀
        for mv in [
            Move::Play(p(1, 1)), // B
            Move::Play(p(0, 0)), // W  (角上)
            Move::Play(p(2, 2)), // B
            Move::Play(p(0, 1)), // W
            Move::Play(p(3, 3)), // B
            Move::Play(p(1, 0)), // W：此时角上白子被围死？不，先构造
        ] {
            let _ = g.play(mv);
        }
        // 直接测一个干净的：9 路盘，白占 (0,1) 和 (1,0)，黑下 (0,0) 自杀
        let mut g = Game::new(9, 7.5);
        g.play(Move::Play(p(5, 5))).unwrap(); // B
        g.play(Move::Play(p(0, 1))).unwrap(); // W
        g.play(Move::Play(p(6, 5))).unwrap(); // B
        g.play(Move::Play(p(1, 0))).unwrap(); // W
        assert_eq!(g.play(Move::Play(p(0, 0))), Err(IllegalMove::Suicide));
    }

    #[test]
    fn superko_forbids_repeat() {
        let mut g = Game::new(9, 7.5);
        //   . . . . .       黑： (1,2)(2,3)(3,2)
        //   . W W W .       白： (2,2)(1,1)(3,1)(2,0)
        //   . W . W .
        //   . B W B .
        //   . . B . .
        let seq = [
            Move::Play(p(1, 2)), // B
            Move::Play(p(2, 2)), // W
            Move::Play(p(2, 3)), // B
            Move::Play(p(1, 1)), // W
            Move::Play(p(3, 2)), // B
            Move::Play(p(3, 1)), // W
            Move::Play(p(5, 5)), // B 脱先
            Move::Play(p(2, 0)), // W
        ];
        for mv in seq {
            g.play(mv).unwrap();
        }
        // 黑提劫
        assert!(g.play(Move::Play(p(2, 1))).is_ok());
        assert_eq!(g.position().board.at(p(2, 2)), None);
        // 白立即提回 -> 局面与第 8 手后完全一致，判位置超级劫
        assert_eq!(g.play(Move::Play(p(2, 2))), Err(IllegalMove::Superko));
    }

    #[test]
    fn two_passes_are_legal() {
        let mut g = Game::new(9, 7.5);
        assert!(g.play(Move::Pass).is_ok());
        assert!(g.play(Move::Pass).is_ok());
        assert!(g.play(Move::Pass).is_ok());
        assert_eq!(g.moves(), &[Move::Pass, Move::Pass, Move::Pass]);
    }

    #[test]
    fn hash_board_ignores_to_move() {
        let mut a = Position::new(9);
        a.board.set(p(2, 2), Some(Color::Black));
        let mut b = a.clone();
        b.to_move = Color::White;
        assert_eq!(hash_board(&a), hash_board(&b));
        assert_ne!(hash_position(&a), hash_position(&b));
    }

    #[test]
    fn simple_ko_still_rejects_immediate_recapture() {
        let mut g = Game::with_rule(9, 7.5, KoRule::Simple);
        let seq = [
            Move::Play(p(1, 2)),
            Move::Play(p(2, 2)),
            Move::Play(p(2, 3)),
            Move::Play(p(1, 1)),
            Move::Play(p(3, 2)),
            Move::Play(p(3, 1)),
            Move::Play(p(5, 5)),
            Move::Play(p(2, 0)),
        ];
        for mv in seq {
            g.play(mv).unwrap();
        }
        assert!(g.play(Move::Play(p(2, 1))).is_ok());
        assert_eq!(g.play(Move::Play(p(2, 2))), Err(IllegalMove::Superko));
    }

    #[test]
    fn area_score_counts_black_and_white_territory() {
        let g = Game::with_setup(
            5,
            0.0,
            vec![
                (Color::Black, p(0, 1)),
                (Color::Black, p(1, 0)),
                (Color::White, p(4, 3)),
                (Color::White, p(3, 4)),
            ],
        );
        let s = g.area_score();
        // 两个角各 1 目 + 2 子；中间空区同时接触黑白，中立
        assert_eq!(s.black, 3);
        assert_eq!(s.white, 3);
    }

    #[test]
    fn area_score_empty_board_is_zero() {
        let g = Game::new(9, 0.0);
        let s = g.area_score();
        assert_eq!((s.black, s.white), (0, 0));
    }

    #[test]
    fn area_score_one_stone_owns_the_empty_board() {
        let mut g = Game::new(9, 0.0);
        g.play(Move::Play(p(0, 0))).unwrap();
        let s = g.area_score();
        assert_eq!(s.black, 81, "1 子 + 80 空点（没有白棋）");
        assert_eq!(s.white, 0);
    }

    #[test]
    fn setup_determines_first_player() {
        // 只摆白子：黑先
        let g = Game::with_setup(9, 0.0, vec![(Color::White, p(2, 2))]);
        assert_eq!(g.position().to_move, Color::Black);
        // 让子（黑摆子）：白先
        let g = Game::with_setup(9, 0.0, vec![(Color::Black, p(2, 2))]);
        assert_eq!(g.position().to_move, Color::White);
        // 显式指定优先
        let g = Game::with_setup_rule_first(
            9,
            0.0,
            vec![(Color::White, p(2, 2))],
            KoRule::default(),
            Color::White,
        );
        assert_eq!(g.position().to_move, Color::White);
    }

    #[test]
    fn explicit_first_player_drives_to_move_at() {
        // PL[W] 且没有摆子：整盘棋都必须认白先，不能因为 setup 为空就退回黑先
        let g = Game::with_setup_rule_first(9, 7.5, Vec::new(), KoRule::default(), Color::White);
        assert_eq!(g.position().to_move, Color::White);
        assert_eq!(g.to_move_at(0), Color::White);
        assert_eq!(g.color_of_move(1), Color::White);
        assert_eq!(g.to_move_at(1), Color::Black);
        assert_eq!(g.color_of_move(2), Color::Black);

        // PL[W] + AW 摆白子：也不能翻成黑先
        let g = Game::with_setup_rule_first(
            9,
            7.5,
            vec![(Color::White, p(2, 2))],
            KoRule::default(),
            Color::White,
        );
        assert_eq!(g.color_of_move(1), Color::White);

        // PL[B] + AB 摆黑子：让子惯例是白先，但显式 PL 优先
        let g = Game::with_setup_rule_first(
            9,
            7.5,
            vec![(Color::Black, p(2, 2))],
            KoRule::default(),
            Color::Black,
        );
        assert_eq!(g.color_of_move(1), Color::Black);
    }

    #[test]
    fn to_move_at_matches_recorded_positions() {
        let mut g =
            Game::with_setup_rule_first(9, 7.5, Vec::new(), KoRule::default(), Color::White);
        for _ in 0..5 {
            g.play(Move::Pass).unwrap();
        }
        for turn in 0..=5u16 {
            assert_eq!(
                g.to_move_at(turn),
                g.position_at(turn).unwrap().to_move,
                "turn {turn} 与历史记录不一致"
            );
            if turn > 0 {
                assert_eq!(
                    g.color_of_move(turn),
                    g.position_at(turn - 1).unwrap().to_move
                );
            }
        }
    }

    #[test]
    fn moves_are_capped_at_u16_range() {
        let mut g = Game::new(9, 0.0);
        for _ in 0..MAX_MOVES {
            g.play(Move::Pass).expect("未到上限前 Pass 必须合法");
        }
        assert_eq!(g.moves().len(), MAX_MOVES);
        assert_eq!(g.play(Move::Pass), Err(IllegalMove::TooManyMoves));
        assert_eq!(g.moves().len(), MAX_MOVES, "失败不能改变历史");
    }

    #[test]
    fn undo_restores_position() {
        let mut g = Game::new(9, 7.5);
        g.play(Move::Play(p(2, 2))).unwrap();
        let before = g.position().clone();
        g.play(Move::Play(p(3, 3))).unwrap();
        g.undo();
        assert_eq!(&before, g.position());
    }

    // ---- 属性测试：不引第三方依赖，用确定性 PRNG 跑随机对局 ----

    /// xorshift64*：同样种子得到同样序列，失败可复现。
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

    /// 种子扩散：`seed | 1` 会把 2 和 3 折叠成同一个状态。
    fn seeded(seed: u64) -> u64 {
        splitmix64(seed) | 1
    }

    /// 随机下一盘合法棋；非法手直接跳过（不当作失败）。
    fn random_game(seed: u64, size: u8, plies: usize) -> Game {
        let mut rng = Rng(seeded(seed));
        let mut g = Game::new(size, 7.5);
        let mut played = 0;
        while played < plies {
            if rng.below(16) == 0 {
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
    fn random_games_keep_stone_accounting() {
        for seed in 1..=8 {
            let size = 9;
            let mut g = Game::new(size, 7.5);
            let mut rng = Rng(seeded(seed * 7919));
            let mut plays = 0usize;
            let mut rejected = 0usize;
            for _ in 0..150 {
                let mv = if rng.below(20) == 0 {
                    Move::Pass
                } else {
                    Move::Play(Point::new(
                        rng.below(size as u64) as u8,
                        rng.below(size as u64) as u8,
                    ))
                };
                if g.play(mv).is_err() {
                    rejected += 1;
                    continue;
                }
                if let Move::Play(_) = mv {
                    plays += 1;
                }
                let pos = g.position();
                let stones = pos.board.stones().count();
                let captured = pos.captures[0] as usize + pos.captures[1] as usize;
                assert_eq!(
                    stones,
                    plays - captured,
                    "seed {seed}: 盘上子数必须等于落子数减提子数"
                );
                assert!(stones <= size as usize * size as usize);
                let area = g.area_score();
                assert!(
                    area.black + area.white <= size as u16 * size as u16,
                    "seed {seed}: 黑白总面积不能超过棋盘"
                );
            }
            // 防假绿：如果所有随机手都被拒，上面的断言等于没跑
            assert!(plays > 20, "seed {seed}: 合法手只有 {plays} 手");
            assert!(
                rejected < 150 * 9 / 10,
                "seed {seed}: 拒手率过高（{rejected}/150）"
            );
        }
    }

    #[test]
    fn random_games_replay_identically() {
        for seed in 1..=4 {
            let original = random_game(seed, 9, 80);
            let mut replay = Game::new(9, 7.5);
            for (i, mv) in original.moves().iter().enumerate() {
                replay
                    .play(*mv)
                    .unwrap_or_else(|e| panic!("seed {seed}: 重放第 {i} 手失败：{e}"));
            }
            assert_eq!(replay.moves(), original.moves(), "seed {seed}: 手顺不一致");
            assert_eq!(
                replay.position(),
                original.position(),
                "seed {seed}: 终局不一致"
            );
            for turn in 0..=original.moves().len() {
                assert_eq!(
                    replay.position_at(turn as u16),
                    original.position_at(turn as u16),
                    "seed {seed}: 第 {turn} 手局面不一致"
                );
            }
        }
    }

    #[test]
    fn undo_all_restores_start() {
        let mut g = random_game(42, 9, 60);
        while g.undo().is_some() {}
        assert!(g.moves().is_empty());
        assert_eq!(g.position(), &Position::new(9));
        assert_eq!(g.area_score().black + g.area_score().white, 0);
        // 撤空后还能继续下
        assert!(g.play(Move::Play(p(4, 4))).is_ok());
    }

    #[test]
    fn only_black_stones_own_the_whole_board() {
        for size in [5u8, 9, 13] {
            let mut rng = Rng(seeded(size as u64 * 104_729));
            let setup: Vec<(Color, Point)> = (0..6)
                .map(|_| {
                    (
                        Color::Black,
                        Point::new(rng.below(size as u64) as u8, rng.below(size as u64) as u8),
                    )
                })
                .collect();
            let g = Game::with_setup(size, 0.0, setup);
            let s = g.area_score();
            assert_eq!(
                s.black,
                size as u16 * size as u16,
                "{size} 路盘只有黑子时，全盘都算黑"
            );
            assert_eq!(s.white, 0);
        }
    }

    /// 换色 + 水平镜像后，黑白得分必须对调。
    #[test]
    fn area_score_is_symmetric_under_color_swap() {
        for seed in 1..=4 {
            let size = 9u8;
            let g = random_game(seed, size, 40);
            let mirror: Vec<(Color, Point)> = g
                .position()
                .board
                .stones()
                .map(|(pt, c)| (c.opposite(), Point::new(size - 1 - pt.x, pt.y)))
                .collect();
            let mirrored = Game::with_setup(size, 0.0, mirror);
            let a = g.area_score();
            let b = mirrored.area_score();
            assert_eq!(a.black, b.white, "seed {seed}: 黑得分应等于镜像白得分");
            assert_eq!(a.white, b.black, "seed {seed}: 白得分应等于镜像黑得分");
        }
    }

    #[test]
    fn out_of_bounds_move_is_rejected() {
        let mut g = Game::new(9, 7.5);
        assert_eq!(g.play(Move::Play(p(9, 0))), Err(IllegalMove::OutOfBounds));
        assert_eq!(g.play(Move::Play(p(0, 9))), Err(IllegalMove::OutOfBounds));
        assert_eq!(g.moves().len(), 0);
    }
}
