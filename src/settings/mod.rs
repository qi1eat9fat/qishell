//! 设置：模型、落盘存储，以及外观/终端落到全局量的那一层。

mod appearance;
pub mod appearance_tokens;
mod connection;
pub mod model;
pub mod panel;
mod sftp;
pub mod store;
mod terminal;

pub use panel::SettingsPanel;
pub use store::{SettingsStore, SettingsStoreEvent};

use gpui_kit::assets::IconName;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    Appearance,
    Terminal,
    Connection,
    Sftp,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 4] = [
        SettingsPage::Appearance,
        SettingsPage::Terminal,
        SettingsPage::Connection,
        SettingsPage::Sftp,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsPage::Appearance => "外观",
            SettingsPage::Terminal => "终端",
            SettingsPage::Connection => "连接",
            SettingsPage::Sftp => "SFTP",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            SettingsPage::Appearance => IconName::SlidersHorizontal,
            SettingsPage::Terminal => IconName::SquareTerminal,
            SettingsPage::Connection => IconName::ShieldCheck,
            SettingsPage::Sftp => IconName::FolderClosed,
        }
    }
}
