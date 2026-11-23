//! weiqi-gui：GTK4 + libadwaita 原生窗口。
//!
//! 棋盘用 Cairo 画，状态与棋谱库复用 `app` crate，和 TUI 完全同一份数据。
//! `board` 只管画，`panels` 只管控件，`controller` 负责事件绑定。

mod board;
mod controller;
mod panels;

use adw::prelude::*;
use gtk::glib;

/// 在 GTK 初始化之前处理的信息类参数。
///
/// 只拦截 `-h/--help` 和 `-V/--version`，其余参数原样交给 GTK/Adw
/// （`--display`、调试开关等仍按 GTK 的方式工作）。
fn print_info_arg(arg: &str) -> Option<glib::ExitCode> {
    match arg {
        "-V" | "--version" => {
            println!("weiqi-gui {}", env!("CARGO_PKG_VERSION"));
            Some(glib::ExitCode::SUCCESS)
        }
        "-h" | "--help" => {
            println!(
                "weiqi-gui {} — GTK4 围棋学习窗口 / GTK4 Go study window\n\n\
                 用法 / usage: weiqi-gui\n\n\
                 不带参数启动，自动打开棋谱库里最近的一盘（没有就空盘）；\n\
                 GUI 不接受棋谱路径，用 `hh` 或 TUI 选谱。\n\
                 Starts with the most recent game in the store (or an empty board).\n\
                 Pass a game via `hh` or the TUI.\n\n\
                 -h, --help      显示帮助 / show this help\n\
                 -V, --version   显示版本 / show version",
                env!("CARGO_PKG_VERSION")
            );
            Some(glib::ExitCode::SUCCESS)
        }
        _ => None,
    }
}

fn main() -> glib::ExitCode {
    if let Some(code) = std::env::args().skip(1).find_map(|a| print_info_arg(&a)) {
        return code;
    }

    let application = adw::Application::builder()
        .application_id("io.github.cyjx007.WeiqiStudy")
        .build();
    application.connect_activate(controller::build_ui);
    application.run()
}
