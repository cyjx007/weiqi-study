//! 界面文案（中/英）。TUI 和后续 GTK 界面共用。

use goban::Color;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// `WEIQI_LANG=zh|en` 优先；否则看 `LANG` 是否以 `zh` 开头。
    pub fn from_env() -> Self {
        if let Ok(v) = std::env::var("WEIQI_LANG") {
            if v.to_ascii_lowercase().starts_with("en") {
                return Lang::En;
            }
            if v.to_ascii_lowercase().starts_with("zh") {
                return Lang::Zh;
            }
        }
        match std::env::var("LANG") {
            Ok(v) if v.to_ascii_lowercase().starts_with("zh") => Lang::Zh,
            _ => Lang::En,
        }
    }

    pub fn is_zh(self) -> bool {
        self == Lang::Zh
    }
}

/// 把模板里的第一个 `{}` 替换成参数（Rust 的 `format!` 只接受字面量）。
pub fn fill(template: &str, arg: impl std::fmt::Display) -> String {
    template.replacen("{}", &arg.to_string(), 1)
}

pub fn color_name(c: Color, lang: Lang) -> &'static str {
    match (c, lang) {
        (Color::Black, Lang::Zh) => "黑",
        (Color::White, Lang::Zh) => "白",
        (Color::Black, Lang::En) => "Black",
        (Color::White, Lang::En) => "White",
    }
}

pub struct Strings {
    pub game: &'static str,
    pub winrate: &'static str,
    pub position: &'static str,
    pub score: &'static str,
    pub moves: &'static str,
    pub turn: &'static str,
    pub you: &'static str,
    pub ai: &'static str,
    pub review: &'static str,
    pub predict: &'static str,
    pub line: &'static str,
    pub commentary: &'static str,
    pub status: &'static str,
    pub games: &'static str,
    pub no_games: &'static str,
    pub press_a: &'static str,
    pub fallback_comment: &'static str,
    pub suggestion: &'static str,
    pub help: &'static str,
    pub new_game_btn: &'static str,
    pub analyse_btn: &'static str,
    pub review_btn: &'static str,
    pub undo_btn: &'static str,
    pub saved: &'static str,
    pub ready: &'static str,
    pub image_fallback: &'static str,
    pub image_board: &'static str,
    pub analyzing: &'static str,
    pub busy: &'static str,
    pub analysis_done: &'static str,
    pub analysis_failed: &'static str,
    pub reviewing: &'static str,
    pub review_done: &'static str,
    pub review_failed: &'static str,
    pub move_played: &'static str,
    pub ai_played: &'static str,
    pub undone: &'static str,
    pub ai_on: &'static str,
    pub ai_off: &'static str,
    pub play_started: &'static str,
    pub jump_turn: &'static str,
    pub need_review: &'static str,
    pub no_blunders: &'static str,
    pub turn_ai: &'static str,
    pub game_loaded: &'static str,
    pub new_game: &'static str,
    pub games_hint: &'static str,
    pub footer: &'static str,
    pub help_lines: &'static [(&'static str, &'static str)],
}

const ZH: Strings = Strings {
    game: "对局",
    winrate: "胜率（黑）",
    position: "局面",
    score: "目差",
    moves: "手数",
    turn: "轮到",
    you: "你",
    ai: "AI",
    review: "复盘",
    predict: "预测 / 候选",
    line: "变化",
    commentary: "讲解",
    status: "状态",
    games: "棋谱",
    no_games: "还没有保存的棋谱（按 t 开一局）",
    press_a: "按 a 分析当前局面",
    fallback_comment: "按 t 开始人机对局；你落子后这里会写这手亏了多少、该下哪。",
    suggestion: "建议",
    help: "帮助",
    new_game_btn: "新局",
    analyse_btn: "分析",
    review_btn: "复盘",
    undo_btn: "撤回",
    saved: "已保存",
    ready: "就绪",
    image_fallback: "图片协议不可用，已回退文本棋盘",
    image_board: "图片棋盘：{}",
    analyzing: "分析中（{} visits）…",
    busy: "上一个分析还没结束",
    analysis_done: "分析完成",
    analysis_failed: "分析失败：{}",
    reviewing: "全盘复盘中…",
    review_done: "全盘复盘完成（b 跳到下一个失误）",
    review_failed: "复盘失败：{}",
    move_played: "落子",
    ai_played: "AI 落子",
    undone: "撤回一手",
    ai_on: "AI 应手：开（你执{}）",
    ai_off: "AI 应手：关（自由摆子）",
    play_started: "人机对局：你执黑，AI 执白。Enter 落子",
    jump_turn: "跳到第 {} 手",
    need_review: "还没有全盘复盘（按 A）",
    no_blunders: "没有超过阈值的失误",
    turn_ai: "现在轮到 AI",
    game_loaded: "已载入 {}",
    new_game: "新棋谱：{}",
    games_hint: "↑↓ 选择 · Enter 打开 · Esc 关闭",
    footer: "hjkl 移动 · Enter 落子 · t 对局 · v AI · u 撤回 · n/p 翻手 · a 分析 · A 复盘 · b 失误 · L 棋谱 · i 信息 · q 退出",
    help_lines: &[
        ("hjkl / 方向键", "移动光标"),
        ("Enter", "在光标处落子"),
        ("t", "开始人机对局（你执黑）"),
        ("v", "开关 AI 应手"),
        ("u", "撤回一手"),
        ("n / p", "下一手 / 上一手"),
        ("g / G", "回到开局 / 跳到末尾"),
        ("a", "分析当前局面"),
        ("A", "全盘复盘（生成失误清单）"),
        ("b", "跳到下一个失误"),
        ("L", "棋谱列表（打开/继续）"),
        ("?", "关闭帮助"),
        ("q / Esc", "退出"),
    ],
};

const EN: Strings = Strings {
    game: "Game",
    winrate: "Winrate (Black)",
    position: "Position",
    score: "Score",
    moves: "Moves",
    turn: "Turn",
    you: "you",
    ai: "AI",
    review: "review",
    predict: "Predictions",
    line: "Line",
    commentary: "Commentary",
    status: "Status",
    games: "Games",
    no_games: "No saved games yet — press t to start one",
    press_a: "Press a to analyse this position",
    fallback_comment: "Press t to play the engine; after each move this panel explains the loss and the better move.",
    suggestion: "Suggestion",
    help: "Help",
    new_game_btn: "New",
    analyse_btn: "Analyse",
    review_btn: "Review",
    undo_btn: "Undo",
    saved: "saved",
    ready: "Ready",
    image_fallback: "No image protocol detected; using the text board",
    image_board: "Image board: {}",
    analyzing: "Analysing ({} visits)…",
    busy: "Previous analysis still running",
    analysis_done: "Analysis done",
    analysis_failed: "Analysis failed: {}",
    reviewing: "Reviewing the whole game…",
    review_done: "Review complete — b jumps to the next blunder",
    review_failed: "Review failed: {}",
    move_played: "Move played",
    ai_played: "Engine played",
    undone: "Undone",
    ai_on: "Engine replies: on (you play {})",
    ai_off: "Engine replies: off (free placement)",
    play_started: "Human vs engine: you play Black. Enter to move",
    jump_turn: "Jumped to move {}",
    need_review: "No full review yet — press A",
    no_blunders: "No moves above the blunder threshold",
    turn_ai: "It is the engine's turn",
    game_loaded: "Loaded {}",
    new_game: "New game: {}",
    games_hint: "↑↓ select · Enter open · Esc close",
    footer: "hjkl move · Enter play · t game · v AI · u undo · n/p prev/next · a analyse · A review · b blunder · L games · i info · q quit",
    help_lines: &[
        ("hjkl / arrows", "move the cursor"),
        ("Enter", "play at the cursor"),
        ("t", "start a human-vs-engine game (you are Black)"),
        ("v", "toggle automatic engine replies"),
        ("u", "undo one move"),
        ("n / p", "next / previous move"),
        ("g / G", "jump to start / end"),
        ("a", "analyse the current position"),
        ("A", "full-game review (blunder list)"),
        ("b", "jump to the next blunder"),
        ("L", "game list (open / continue)"),
        ("?", "close help"),
        ("q / Esc", "quit"),
    ],
};

pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::Zh => &ZH,
        Lang::En => &EN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_and_chinese_have_same_help_length() {
        assert_eq!(ZH.help_lines.len(), EN.help_lines.len());
    }
}
