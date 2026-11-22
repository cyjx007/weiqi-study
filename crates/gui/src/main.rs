//! weiqi-gui：GTK4 + libadwaita 原生窗口。
//!
//! 棋盘用 Cairo 画，状态与棋谱库复用 `app` crate，和 TUI 完全同一份数据。
//! `board` 只管画，`panels` 只管控件，`controller` 负责事件绑定。

mod board;
mod controller;
mod panels;

use adw::prelude::*;
use gtk::glib;

fn main() -> glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("io.github.cyjx007.WeiqiStudy")
        .build();
    application.connect_activate(controller::build_ui);
    application.run()
}
