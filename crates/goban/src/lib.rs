//! 围棋规则核心：棋盘、提子、位置超级劫、面积数子。
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IllegalMove {
    OutOfBounds,
    Occupied,
    Suicide,
    Superko,
}

impl fmt::Display for IllegalMove {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            IllegalMove::OutOfBounds => "越界",
            IllegalMove::Occupied => "该点已有子",
            IllegalMove::Suicide => "自杀",
            IllegalMove::Superko => "违规（位置超级劫）",
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
        self.cells
            .iter()
            .enumerate()
            .filter_map(move |(i, cell)| {
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
    pub captures: [u16; 2],
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

                let mut captured = 0u16;
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

pub fn hash_position(pos: &Position) -> u64 {
    let size = pos.board.size() as u64;
    let mut h = 0u64;
    for (p, c) in pos.board.stones() {
        let key = (p.y as u64 * size + p.x as u64) * 2 + c.index() as u64 + 1;
        h ^= splitmix64(key);
    }
    if pos.to_move == Color::White {
        h ^= splitmix64(0x51ED_2701);
    }
    h
}

/// 整盘棋：带历史的局面栈，支持撤销和位置超级劫。
#[derive(Clone, Debug)]
pub struct Game {
    size: u8,
    komi: f32,
    setup: Vec<(Color, Point)>,
    positions: Vec<Position>,
    hashes: Vec<u64>,
    moves: Vec<Move>,
}

impl Game {
    pub fn new(size: u8, komi: f32) -> Self {
        Self::with_setup(size, komi, Vec::new())
    }

    pub fn with_setup(size: u8, komi: f32, setup: Vec<(Color, Point)>) -> Self {
        let mut pos = Position::new(size);
        for (c, p) in &setup {
            pos.board.set(*p, Some(*c));
        }
        // 有摆子（让子）时白先
        pos.to_move = if setup.is_empty() {
            Color::Black
        } else {
            Color::White
        };
        let h = hash_position(&pos);
        Game {
            size,
            komi,
            setup,
            positions: vec![pos],
            hashes: vec![h],
            moves: Vec::new(),
        }
    }

    pub fn play(&mut self, mv: Move) -> Result<(), IllegalMove> {
        let next = self.position().try_play(mv)?;
        let h = hash_position(&next);
        if self.hashes.contains(&h) {
            return Err(IllegalMove::Superko);
        }
        self.positions.push(next);
        self.hashes.push(h);
        self.moves.push(mv);
        Ok(())
    }

    pub fn undo(&mut self) -> Option<Move> {
        if self.moves.is_empty() {
            None
        } else {
            self.positions.pop();
            self.hashes.pop();
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

    /// 第 `turn` 手（1-based）由谁下。
    pub fn color_of_move(&self, turn: u16) -> Color {
        self.to_move_at(turn - 1)
    }

    /// 第 `turn` 手开始前轮到谁（0 = 开局）。
    pub fn to_move_at(&self, turn: u16) -> Color {
        let first = if self.setup.is_empty() {
            Color::Black
        } else {
            Color::White
        };
        if turn % 2 == 0 {
            first
        } else {
            first.opposite()
        }
    }

    /// 面积计子（中国规则），不做死活判定：把盘上所有子都当活棋。
    pub fn area_score(&self) -> AreaScore {
        let board = &self.position().board;
        let mut black = 0u16;
        let mut white = 0u16;
        let mut seen = vec![false; board.cells.len()];
        for (p, c) in board.stones() {
            match c {
                Color::Black => black += 1,
                Color::White => white += 1,
            }
            seen[board.idx(p)] = true;
        }
        for y in 0..board.size() {
            for x in 0..board.size() {
                let p = Point::new(x, y);
                let i = board.idx(p);
                if seen[i] {
                    continue;
                }
                // 洪水填充空区域，看边界颜色
                let mut region = Vec::new();
                let mut border_black = false;
                let mut border_white = false;
                let mut stack = vec![p];
                seen[i] = true;
                while let Some(q) = stack.pop() {
                    region.push(q);
                    for n in board.neighbors(q) {
                        let ni = board.idx(n);
                        if seen[ni] {
                            continue;
                        }
                        match board.cells[ni] {
                            None => {
                                seen[ni] = true;
                                stack.push(n);
                            }
                            Some(Color::Black) => {
                                seen[ni] = true;
                                border_black = true;
                            }
                            Some(Color::White) => {
                                seen[ni] = true;
                                border_white = true;
                            }
                        }
                    }
                }
                match (border_black, border_white) {
                    (true, false) => black += region.len() as u16,
                    (false, true) => white += region.len() as u16,
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
    fn area_score_corner() {
        let mut g = Game::new(9, 0.0);
        g.play(Move::Play(p(0, 0))).unwrap();
        let s = g.area_score();
        // 角上一子 + 整块空者归黑？9 路盘被这一子分割不现实，只测不会 panic
        assert!(s.black >= 1);
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
}
