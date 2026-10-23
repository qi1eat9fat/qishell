#![recursion_limit = "512"]
//! Qi Shell：SSH 工作区。主机、设置、信任主机与秘密都落盘，终端与 SFTP 是真实连接。

/// 应用显示名：窗口标题、标题栏品牌与提示文案都取这里。
pub const APP_NAME: &str = "Qi Shell";

pub mod actions;
pub mod app;
pub mod assets;
pub mod components;
pub mod connection;
pub mod dialogs;
pub mod host;
pub mod model;
pub mod secrets;
pub mod settings;
pub mod sftp;
pub mod snippets;
pub mod ssh;
pub mod terminal;
pub mod theme;
pub mod workspace;
