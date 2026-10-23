// 发布版在 Windows 上是 GUI 程序，避免额外弹出控制台窗口。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    qishell::app::run();
}
