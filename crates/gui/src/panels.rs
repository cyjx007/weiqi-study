//! 侧栏与底部状态栏：控件构造、文案刷新、棋谱下拉同步。

use adw::prelude::*;
use app::i18n::{color_name, strings};
use app::{move_text, App, HUMAN_RANKS};
use gtk::gdk;

pub struct Panels {
    pub winrate: gtk::ProgressBar,
    pub winrate_text: gtk::Label,
    pub turn: gtk::Label,
    pub score: gtk::Label,
    pub moves: gtk::Label,
    pub cand: gtk::Label,
    pub comment: gtk::Label,
    pub status: gtk::Label,
    pub games: gtk::DropDown,
    pub lang_btn: gtk::Button,
    pub new_btn: gtk::Button,
    pub analyse_btn: gtk::Button,
    pub review_btn: gtk::Button,
    pub undo_btn: gtk::Button,
    pub prev_btn: gtk::Button,
    pub next_btn: gtk::Button,
    pub rename_btn: gtk::Button,
    pub delete_btn: gtk::Button,
    pub visits: gtk::SpinButton,
    pub human_switch: gtk::Switch,
    pub rank: gtk::DropDown,
    pub variation_btn: gtk::Button,
    pub commit_btn: gtk::Button,
    strength_label: gtk::Label,
    human_label: gtk::Label,
    updating_controls: bool,
    sec_winrate: gtk::Label,
    sec_position: gtk::Label,
    sec_predict: gtk::Label,
    sec_comment: gtk::Label,
    game_count: usize,
    updating_games: bool,
}

impl Panels {
    /// 构造右侧栏 + 底部状态栏；返回整块侧栏和持有控件引用的 `Panels`。
    pub fn build() -> (gtk::Box, Self) {
        let winrate = gtk::ProgressBar::new();
        winrate.set_show_text(true);
        winrate.set_fraction(0.5);
        winrate.set_text(Some("50.0%"));

        let turn = label("");
        let score = label("");
        let moves = label("");
        let cand = label("");
        cand.set_wrap(true);
        let comment = label("");
        comment.set_wrap(true);
        comment.set_valign(gtk::Align::Start);
        let status = label("");
        status.add_css_class("dim-label");
        status.set_wrap(true);

        let games = gtk::DropDown::from_strings(&["—"]);
        games.set_hexpand(true);

        let lang_btn = gtk::Button::with_label("EN");
        let new_btn = gtk::Button::with_label("");
        let analyse_btn = gtk::Button::with_label("");
        let review_btn = gtk::Button::with_label("");
        let undo_btn = gtk::Button::with_label("");
        let prev_btn = gtk::Button::with_label("‹");
        let next_btn = gtk::Button::with_label("›");
        let rename_btn = gtk::Button::with_label("");
        let delete_btn = gtk::Button::with_label("");

        let side = gtk::Box::new(gtk::Orientation::Vertical, 10);
        side.set_size_request(340, -1);
        side.set_margin_top(14);
        side.set_margin_bottom(14);
        side.set_margin_start(14);
        side.set_margin_end(14);

        // 头部卡片：轮到谁 + 大号胜率
        let head = card();
        let sec_winrate = section("");
        turn.add_css_class("turn-label");
        let winrate_text = gtk::Label::new(Some("50.0%"));
        winrate_text.add_css_class("winrate-big");
        winrate_text.set_xalign(0.0);
        winrate.set_show_text(false);
        winrate.add_css_class("winrate-bar");
        head.append(&sec_winrate);
        head.append(&turn);
        head.append(&winrate_text);
        head.append(&winrate);
        side.append(&head);

        // 局面
        let stats = card();
        let sec_position = section("");
        stats.append(&sec_position);
        stats.append(&score);
        stats.append(&moves);
        side.append(&stats);

        // AI 强度：visits + 拟人网开关/段位
        let strength_card = card();
        let strength_label = section("");
        let visits = gtk::SpinButton::with_range(10.0, 5000.0, 10.0);
        visits.set_hexpand(true);
        visits.set_numeric(true);
        strength_card.append(&strength_label);
        strength_card.append(&visits);

        let human_label = section("");
        strength_card.append(&human_label);
        let human_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let human_switch = gtk::Switch::new();
        let ranks: Vec<&str> = HUMAN_RANKS
            .iter()
            .map(|r| r.strip_prefix("rank_").unwrap_or(r))
            .collect();
        let rank = gtk::DropDown::from_strings(&ranks);
        rank.set_hexpand(true);
        rank.set_sensitive(false);
        human_row.append(&human_switch);
        human_row.append(&rank);
        strength_card.append(&human_row);
        side.append(&strength_card);

        // 预测
        let predict_card = card();
        let sec_predict = section("");
        predict_card.append(&sec_predict);
        predict_card.append(&cand);
        let preview_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let variation_btn = gtk::Button::with_label("");
        let commit_btn = gtk::Button::with_label("");
        preview_row.append(&variation_btn);
        preview_row.append(&commit_btn);
        predict_card.append(&preview_row);
        side.append(&predict_card);

        // 讲解（占满剩余高度）
        let comment_card = card();
        comment_card.set_vexpand(true);
        let sec_comment = section("");
        comment_card.append(&sec_comment);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&comment));
        scroll.set_vexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        comment_card.append(&scroll);
        side.append(&comment_card);

        // 操作
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.add_css_class("actions");
        actions.append(&new_btn);
        actions.append(&analyse_btn);
        actions.append(&review_btn);
        actions.append(&undo_btn);
        side.append(&actions);

        // 棋谱 + 翻手 + 管理
        let games_card = card();
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        nav.append(&prev_btn);
        nav.append(&next_btn);
        nav.append(&games);
        games_card.append(&nav);
        let manage = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        manage.append(&rename_btn);
        manage.append(&delete_btn);
        games_card.append(&manage);
        side.append(&games_card);

        // 底部状态 + 语言
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status.add_css_class("status-label");
        status.set_hexpand(true);
        lang_btn.add_css_class("pill");
        footer.append(&status);
        footer.append(&lang_btn);
        side.append(&footer);

        let panels = Panels {
            winrate,
            winrate_text,
            turn,
            score,
            moves,
            cand,
            comment,
            status,
            games,
            lang_btn,
            new_btn,
            analyse_btn,
            review_btn,
            undo_btn,
            prev_btn,
            next_btn,
            rename_btn,
            delete_btn,
            visits,
            human_switch,
            rank,
            variation_btn,
            commit_btn,
            strength_label,
            human_label,
            updating_controls: false,
            sec_winrate,
            sec_position,
            sec_predict,
            sec_comment,
            game_count: 0,
            updating_games: false,
        };
        (side, panels)
    }

    pub fn is_updating_games(&self) -> bool {
        self.updating_games
    }

    pub fn is_updating_controls(&self) -> bool {
        self.updating_controls
    }

    /// 重新扫描棋谱目录并更新下拉框（调用方负责避免重入回调）。
    pub fn reload_games(&mut self, app: &mut App) {
        app.refresh_games();
        let names: Vec<String> = if app.games.is_empty() {
            vec![strings(app.lang).no_games.to_string()]
        } else {
            app.games.iter().map(|g| g.label.clone()).collect()
        };
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        self.updating_games = true;
        self.games.set_model(Some(&gtk::StringList::new(&refs)));
        self.updating_games = false;
        self.game_count = app.games.len();
    }

    /// 按当前 `App` 状态刷新所有文字；棋谱数量变化时顺带重建下拉框。
    pub fn refresh(&mut self, app: &mut App) {
        let s = strings(app.lang);
        self.sec_winrate
            .set_markup(&format!("<b>{}</b>", s.winrate));
        self.sec_position
            .set_markup(&format!("<b>{}</b>", s.position));
        self.sec_predict
            .set_markup(&format!("<b>{}</b>", s.predict));
        self.sec_comment
            .set_markup(&format!("<b>{}</b>", s.commentary));
        self.new_btn.set_label(s.new_game_btn);
        self.analyse_btn.set_label(s.analyse_btn);
        self.review_btn.set_label(s.review_btn);
        self.undo_btn.set_label(s.undo_btn);
        self.rename_btn.set_label(s.rename);
        self.delete_btn.set_label(s.delete);
        self.strength_label
            .set_markup(&format!("<b>{}</b>", s.strength));
        self.human_label
            .set_markup(&format!("<b>{}</b>", s.human_net));
        self.variation_btn.set_label(s.variation);
        self.commit_btn.set_label(s.use_variation);
        self.commit_btn.set_sensitive(
            app.preview_active() && app.preview_progress().is_some_and(|(s, _)| s > 0),
        );

        self.updating_controls = true;
        if self.visits.value() as u32 != app.config.engine.visits {
            self.visits.set_value(app.config.engine.visits as f64);
        }
        let has_human = app.human_model_available();
        self.human_switch.set_sensitive(has_human);
        if self.human_switch.is_active() != app.human_enabled {
            self.human_switch.set_active(app.human_enabled);
        }
        self.rank.set_sensitive(has_human);
        if self.rank.selected() as usize != app.human_rank {
            self.rank.set_selected(app.human_rank as u32);
        }
        self.updating_controls = false;
        self.lang_btn
            .set_label(if app.lang.is_zh() { "EN" } else { "中文" });

        let wr = app.analysis().map(|a| a.winrate_black).unwrap_or(0.5);
        self.winrate.set_fraction(wr.clamp(0.0, 1.0) as f64);
        self.winrate_text.set_text(&format!("{:.1}%", wr * 100.0));

        let who = if !app.ai_enabled {
            s.review.to_string()
        } else if app.display_to_move() == app.human {
            s.you.to_string()
        } else {
            s.ai.to_string()
        };
        self.turn.set_text(&format!(
            "{}: {}({})",
            s.turn,
            color_name(app.display_to_move(), app.lang),
            who
        ));
        let score = app.analysis().map(|a| a.score_black).unwrap_or(0.0);
        self.score.set_text(&format!("{}: {score:+.1}", s.score));
        self.moves.set_text(&format!(
            "{}: {} / {}",
            s.moves,
            app.display_pos(),
            app.total_moves()
        ));

        let mut cand = String::new();
        if let Some((step, total)) = app.preview_progress() {
            cand.push_str(&format!(
                "{} {step}/{total}\n{}",
                s.variation, s.variation_hint
            ));
        }
        for (i, (mv, wr)) in app.prediction().iter().enumerate() {
            cand.push_str(&format!(
                "{}. {}  {:.1}%\n",
                i + 1,
                move_text(*mv, app.size),
                wr * 100.0
            ));
        }
        if let Some(a) = app.analysis() {
            if let Some(best) = a.candidates.first() {
                let pv: Vec<String> = best
                    .pv
                    .iter()
                    .take(6)
                    .map(|m| move_text(*m, app.size))
                    .collect();
                if !pv.is_empty() {
                    cand.push_str(&format!("{}: {}", s.line, pv.join(" ")));
                }
            }
        }
        if cand.is_empty() {
            cand.push_str(s.press_a);
        }
        self.cand.set_text(&cand);

        let mut text = app.last_comment.clone().unwrap_or_default();
        if app.ai_enabled && !app.preview_active() && app.to_move() == app.human {
            if let Some(a) = app.analysis() {
                if let Some(best) = a.candidates.first() {
                    if !text.is_empty() {
                        text.push_str("\n\n");
                    }
                    text.push_str(&format!(
                        "{} {}（{:.1}%）",
                        s.suggestion,
                        move_text(best.mv, app.size),
                        best.winrate_black * 100.0
                    ));
                }
            }
        }
        if text.is_empty() {
            text.push_str(s.fallback_comment);
        }
        self.comment.set_text(&text);
        self.status.set_text(&app.status);

        if app.games.len() != self.game_count {
            self.reload_games(app);
        }
    }
}

pub fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        r#"
        .panel-card {
            background-color: alpha(currentColor, 0.055);
            border-radius: 12px;
            padding: 12px;
        }
        .section-title { font-size: 0.8em; font-weight: 700; opacity: 0.6; }
        .turn-label { font-size: 1.25em; font-weight: 700; }
        .winrate-big { font-size: 1.7em; font-weight: 800; }
        .winrate-bar { min-height: 8px; border-radius: 4px; }
        .status-label { font-size: 0.85em; opacity: 0.65; }
        "#,
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn card() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.add_css_class("panel-card");
    b
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l
}

fn section(text: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.add_css_class("section-title");
    l.set_markup(&format!("<b>{text}</b>"));
    l
}
