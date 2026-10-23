//! 界面模型：标签、日志与设置。
//!
//! 主机本身在 [`crate::host`]，这里只保留界面用的临时类型。

use std::path::PathBuf;

use gpui_kit::{Entity, Subscription};

use crate::sftp::SftpView;
use crate::terminal::TerminalView;

pub use crate::host::{HostId, HostLogin};

pub type TabId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Host(HostId),
    /// 一台主机的 SFTP 标签；同一主机只保留一个。
    Sftp(HostId),
    Settings,
}

pub struct Tab {
    pub id: TabId,
    pub kind: TabKind,
    /// 打开时的登录快照。主机被删掉后，标签还靠它显示名字与地址。
    pub login: Option<HostLogin>,
    pub connected: bool,
    /// 主机标签的真实终端；设置标签与 SFTP 标签没有。
    pub terminal: Option<Entity<TerminalView>>,
    /// SFTP 标签的双栏界面；其他标签没有。
    pub sftp: Option<Entity<SftpView>>,
    /// 终端事件与它自己的订阅，关标签时一并释放。
    pub subscriptions: Vec<Subscription>,
}

impl Tab {
    pub fn is_settings(&self) -> bool {
        matches!(self.kind, TabKind::Settings)
    }

    pub fn is_sftp(&self) -> bool {
        matches!(self.kind, TabKind::Sftp(_))
    }

    /// 标签对应的主机。终端与 SFTP 都算。
    pub fn host_id(&self) -> Option<HostId> {
        match self.kind {
            TabKind::Host(id) | TabKind::Sftp(id) => Some(id),
            TabKind::Settings => None,
        }
    }
}

// —— 外观 ——

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wallpaper {
    pub path: Option<PathBuf>,
    pub name: String,
    pub custom: bool,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            path: None,
            name: "纯色背景".into(),
            custom: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    /// 主界面玻璃不透明度，5–90（%）。
    pub glass: u8,
    /// 背景清晰度，0–100（%）。
    pub clarity: u8,
    /// 壁纸预模糊半径，0–30 px。
    pub blur: u8,
    pub wallpaper: Wallpaper,
    /// 轻量浮层不透明度（绝对值），30–92（%）。
    pub overlay_alpha: u8,
    /// 模态弹窗不透明度（绝对值），45–98（%）。
    pub modal_alpha: u8,
    /// 全屏遮罩不透明度，0–90（%）。
    pub backdrop_alpha: u8,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            glass: crate::settings::appearance_tokens::GLASS_DEFAULT,
            clarity: crate::settings::appearance_tokens::CLARITY_DEFAULT,
            blur: crate::settings::appearance_tokens::BLUR_DEFAULT,
            wallpaper: Wallpaper::default(),
            overlay_alpha: crate::settings::appearance_tokens::OVERLAY_DEFAULT,
            modal_alpha: crate::settings::appearance_tokens::MODAL_DEFAULT,
            backdrop_alpha: crate::settings::appearance_tokens::BACKDROP_DEFAULT,
        }
    }
}

impl Appearance {
    pub fn glass_alpha(&self) -> f32 {
        self.glass as f32 / 100.0
    }

    pub fn overlay_alpha_f32(&self) -> f32 {
        self.overlay_alpha as f32 / 100.0
    }

    pub fn modal_alpha_f32(&self) -> f32 {
        self.modal_alpha as f32 / 100.0
    }

    pub fn backdrop_alpha_f32(&self) -> f32 {
        self.backdrop_alpha as f32 / 100.0
    }
}

// —— 终端设置 ——

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSettings {
    pub font: String,
    pub size: u8,
    pub line_height: f32,
    /// 终端配色主题的 key；空字符串表示跟随应用外观的默认主题。
    pub theme: String,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font: "Cascadia Code".into(),
            size: 13,
            line_height: 1.54,
            theme: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SftpSettings {
    pub max_transfers: u8,
    /// SFTP 本地栏打开时所在的目录。空 = 用户主目录。
    pub local_root: String,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            max_transfers: 3,
            local_root: String::new(),
        }
    }
}

/// 外部文本编辑器的默认路径（设置 › SFTP › 外部编辑器）。
pub const DEFAULT_EDITOR_PATH: &str = r"D:\Notepad++\notepad++.exe";

#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub appearance: Appearance,
    pub terminal: TerminalSettings,
    pub sftp: SftpSettings,
    /// 打开本地/远程文本文件时使用的外部编辑器。
    pub editor_path: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            terminal: TerminalSettings::default(),
            sftp: SftpSettings::default(),
            editor_path: DEFAULT_EDITOR_PATH.to_string(),
        }
    }
}

/// 界面各处用的设置集合（沿用第一期的名字）。
pub type SettingsState = AppSettings;
