//! 变化预览：临时沿引擎 PV 走棋，确认前不污染主线。
//!
//! 只改 `App::preview`；主线 `moves/pos/game` 在 `commit_preview` 之前不动。

use goban::{Color, Game, Move};

use crate::i18n::{self, strings};
use crate::App;

/// 从 `base` 开始预览 `pv` 的前 `step` 手；`game` 是预览到当前步的棋盘。
pub(crate) struct Preview {
    base: usize,
    pv: Vec<Move>,
    step: usize,
    game: Game,
}

impl App {
    pub fn display_game(&self) -> &Game {
        self.preview.as_ref().map(|p| &p.game).unwrap_or(&self.game)
    }

    pub fn display_pos(&self) -> usize {
        self.preview
            .as_ref()
            .map(|p| p.base + p.step)
            .unwrap_or(self.pos)
    }

    pub fn display_to_move(&self) -> Color {
        self.display_game().position().to_move
    }

    pub fn preview_active(&self) -> bool {
        self.preview.is_some()
    }

    /// (已走步数, 变化总手数)
    pub fn preview_progress(&self) -> Option<(usize, usize)> {
        self.preview.as_ref().map(|p| (p.step, p.pv.len()))
    }

    /// 用当前分析的第一候选 PV 进入变化预览，不修改主线。
    pub fn start_preview(&mut self) {
        let Some(analysis) = self.analysis() else {
            self.status = strings(self.lang).no_variation.to_string();
            return;
        };
        let pv: Vec<Move> = analysis
            .candidates
            .first()
            .map(|c| c.pv.iter().take(12).copied().collect())
            .unwrap_or_default();
        if pv.is_empty() {
            self.status = strings(self.lang).no_variation.to_string();
            return;
        }
        let base = self.pos;
        self.preview = Some(Preview {
            base,
            step: 0,
            pv,
            game: self.game.clone(),
        });
        self.status = i18n::fill(strings(self.lang).variation_start, self.preview_len());
    }

    fn preview_len(&self) -> usize {
        self.preview.as_ref().map(|p| p.pv.len()).unwrap_or(0)
    }

    fn preview_status(&self) -> String {
        match self.preview_progress() {
            Some((step, total)) => {
                format!("{} {step}/{total}", strings(self.lang).variation)
            }
            None => String::new(),
        }
    }

    pub fn preview_next(&mut self) {
        let Some(p) = self.preview.as_mut() else {
            return;
        };
        if p.step >= p.pv.len() {
            return;
        }
        let mv = p.pv[p.step];
        let mut game = p.game.clone();
        match game.play(mv) {
            Ok(()) => {
                p.step += 1;
                p.game = game;
                self.status = self.preview_status();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    pub fn preview_prev(&mut self) {
        let Some(p) = self.preview.as_mut() else {
            return;
        };
        if p.step == 0 {
            return;
        }
        let step = p.step - 1;
        let mut game = self.game.clone();
        for mv in p.pv.iter().take(step) {
            if game.play(*mv).is_err() {
                return;
            }
        }
        p.step = step;
        p.game = game;
        self.status = self.preview_status();
    }

    /// 采用预览到的变化：截断主线并落子，之后照常自动保存。
    pub fn commit_preview(&mut self) {
        let Some(p) = self.preview.take() else {
            return;
        };
        if p.step == 0 {
            return;
        }
        let step = p.step;
        let base = p.base;
        self.next_generation();
        self.moves.truncate(base);
        self.moves.extend(p.pv.iter().take(step).copied());
        self.pos = self.moves.len();
        if let Err(e) = self.rebuild() {
            self.status = e;
            return;
        }
        // 采用变化也换分支：base 之后属于旧变化，全部作废
        self.evals.retain(|&k, _| k <= base);
        self.failed_pos = None;
        self.review = None;
        self.autosave();
        self.status = i18n::fill(strings(self.lang).variation_commit, step);
    }

    pub fn exit_preview(&mut self) {
        self.preview = None;
    }
}
