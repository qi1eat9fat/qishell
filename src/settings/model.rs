//! 设置的模型与落盘格式（`settings.json`）。
//!
//! 外观、终端、SFTP 三组；`SettingsStore` 负责读写。

use serde::{Deserialize, Serialize};

use crate::model::{AppSettings, Appearance, SftpSettings, TerminalSettings, Wallpaper};

use super::appearance_tokens::{
    BACKDROP_DEFAULT, BACKDROP_MAX, BACKDROP_MIN, BLUR_DEFAULT, BLUR_MAX, CLARITY_DEFAULT,
    CLARITY_MAX, GLASS_DEFAULT, GLASS_MAX, GLASS_MIN, MODAL_DEFAULT, MODAL_MAX, MODAL_MIN,
    OVERLAY_DEFAULT, OVERLAY_MAX, OVERLAY_MIN,
};

/// `settings.json` 的格式版本。0 表示引入版本号之前的旧文件（没有
/// `version` 字段）。
pub const SETTINGS_FILE_VERSION: u32 = 3;

/// 历代浮层与弹窗的默认值，迁移时用来辨认「用户没改过」。
const OVERLAY_V1_DEFAULT: u8 = 70;
const MODAL_V1_DEFAULT: u8 = 92;
const OVERLAY_V2_DEFAULT: u8 = 55;
const MODAL_V2_DEFAULT: u8 = 62;

// —— 落盘用的镜像类型：字段都带默认值，读不出或缺失就回到默认。 ——

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperSettings {
    pub path: Option<std::path::PathBuf>,
    pub name: String,
    pub custom: bool,
}

impl Default for WallpaperSettings {
    fn default() -> Self {
        Self {
            path: None,
            name: "纯色背景".into(),
            custom: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub glass: u8,
    pub clarity: u8,
    pub blur: u8,
    pub wallpaper: WallpaperSettings,
    /// 轻量浮层不透明度（绝对值）。缺失时由 `normalized` 补 70。
    pub overlay_alpha: Option<u8>,
    /// v0 的旧字段：只用于读旧文件，不再写回。
    #[serde(default, skip_serializing)]
    pub overlay_alpha_delta: Option<u8>,
    /// 模态弹窗不透明度。v0 文件里是增量语义，迁移时折算一次。
    pub modal_alpha: Option<u8>,
    pub backdrop_alpha: u8,
}

impl Default for AppearanceSettings {
    /// 缺字段时的形状：`None` 表示「文件里没写」，与用户明确写的值区分开。
    fn default() -> Self {
        Self {
            glass: GLASS_DEFAULT,
            clarity: CLARITY_DEFAULT,
            blur: BLUR_DEFAULT,
            wallpaper: WallpaperSettings::default(),
            overlay_alpha: None,
            overlay_alpha_delta: None,
            modal_alpha: None,
            backdrop_alpha: BACKDROP_DEFAULT,
        }
    }
}

impl AppearanceSettings {
    /// 新建文件的默认值。与 `default()` 分开：旧文件缺字段时不能被当成
    /// 「用户明确写了新值」。
    pub fn current_defaults() -> Self {
        Self {
            overlay_alpha: Some(OVERLAY_DEFAULT),
            modal_alpha: Some(MODAL_DEFAULT),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontSettings {
    /// `None` 表示跟随主题的等宽字体。
    pub family: Option<String>,
    pub size: f32,
    pub line_height: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            family: None,
            size: 13.0,
            line_height: 1.54,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettingsFile {
    pub font: FontSettings,
    /// 终端配色主题的 key；空 = 跟随应用外观的默认主题。
    pub theme: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SftpSettingsFile {
    pub max_transfers: u8,
    /// SFTP 本地栏打开时的目录；空 = 用户主目录。
    ///
    /// 跟 `editor_path` 一样是本机的东西（这台机器上的哪个文件夹），不按
    /// 数据目录转相对路径：换台机器这个路径本来就该另设。
    pub local_root: String,
}

impl Default for SftpSettingsFile {
    fn default() -> Self {
        Self {
            max_transfers: 3,
            local_root: String::new(),
        }
    }
}

/// `settings.json` 的全部内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsFile {
    /// 格式版本；没有这一字段的旧文件按 0 处理。
    #[serde(default)]
    pub version: u32,
    /// 缺字段时用「未写」的形状（`None`），而不是新建文件的默认值，
    /// 否则旧文件会被误判成已经写过新的绝对值。
    #[serde(default)]
    pub appearance: AppearanceSettings,
    pub terminal: TerminalSettingsFile,
    pub sftp: SftpSettingsFile,
    /// 打开文本文件使用的外部编辑器。
    pub editor_path: String,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            version: SETTINGS_FILE_VERSION,
            appearance: AppearanceSettings::current_defaults(),
            terminal: TerminalSettingsFile::default(),
            sftp: SftpSettingsFile::default(),
            editor_path: crate::model::DEFAULT_EDITOR_PATH.to_string(),
        }
    }
}

impl SettingsFile {
    /// 把数值夹进合法范围，读到手写的文件也安全。
    pub fn normalized(mut self) -> Self {
        // v0 → v1：增量语义折算成绝对值，只做一次。迁移后写回就带版本号，
        // 下次启动不会再走这里。
        if self.version < 1 {
            let old_modal = self.appearance.modal_alpha.unwrap_or(55);
            // v0 的弹窗值是增量语义：旧公式弹窗表面 = 滑杆值 + 0.37。
            self.appearance.modal_alpha = Some(old_modal.saturating_add(37));
            // 旧的浮层增量只影响过 Toast（glass + delta + 25），没有对应的
            // 绝对默认值，直接换成上浮层的旧默认。
            self.appearance.overlay_alpha = Some(OVERLAY_V1_DEFAULT);
            self.appearance.overlay_alpha_delta = None;
        }
        if self.version < SETTINGS_FILE_VERSION {
            // v1 → v3：浮层与弹窗的默认值一路下调，让壁纸在所有界面上都透出来。
            // 只把「恰好停在旧默认值」的项换成新默认：那多半是没调过；用户
            // 自己拖出来的其他数值（含调到 98% 这类极端值）保持原样。
            let overlay = self.appearance.overlay_alpha.unwrap_or(OVERLAY_DEFAULT);
            self.appearance.overlay_alpha = Some(
                if matches!(overlay, OVERLAY_V1_DEFAULT | OVERLAY_V2_DEFAULT) {
                    OVERLAY_DEFAULT
                } else {
                    overlay
                },
            );
            let modal = self.appearance.modal_alpha.unwrap_or(MODAL_DEFAULT);
            self.appearance.modal_alpha =
                Some(if matches!(modal, MODAL_V1_DEFAULT | MODAL_V2_DEFAULT) {
                    MODAL_DEFAULT
                } else {
                    modal
                });
            self.version = SETTINGS_FILE_VERSION;
        }

        self.appearance.glass = self.appearance.glass.clamp(GLASS_MIN, GLASS_MAX);
        self.appearance.clarity = self.appearance.clarity.min(CLARITY_MAX);
        self.appearance.blur = self.appearance.blur.min(BLUR_MAX);
        self.appearance.overlay_alpha = Some(
            self.appearance
                .overlay_alpha
                .unwrap_or(OVERLAY_DEFAULT)
                .clamp(OVERLAY_MIN, OVERLAY_MAX),
        );
        self.appearance.modal_alpha = Some(
            self.appearance
                .modal_alpha
                .unwrap_or(MODAL_DEFAULT)
                .clamp(MODAL_MIN, MODAL_MAX),
        );
        self.appearance.backdrop_alpha = self
            .appearance
            .backdrop_alpha
            .clamp(BACKDROP_MIN, BACKDROP_MAX);

        self.terminal.font.size = self.terminal.font.size.clamp(10.0, 28.0);
        self.terminal.font.line_height =
            (self.terminal.font.line_height.clamp(1.0, 2.2) * 100.0).round() / 100.0;

        self.sftp.max_transfers = self.sftp.max_transfers.clamp(1, 10);
        self.sftp.local_root = self.sftp.local_root.trim().to_string();
        self
    }

    /// 转成界面用的 `AppSettings`。
    pub fn to_app(&self) -> AppSettings {
        AppSettings {
            appearance: Appearance {
                glass: self.appearance.glass,
                clarity: self.appearance.clarity,
                blur: self.appearance.blur,
                wallpaper: Wallpaper {
                    path: self.appearance.wallpaper.path.clone(),
                    name: self.appearance.wallpaper.name.clone(),
                    custom: self.appearance.wallpaper.custom,
                },
                overlay_alpha: self.appearance.overlay_alpha.unwrap_or(OVERLAY_DEFAULT),
                modal_alpha: self.appearance.modal_alpha.unwrap_or(MODAL_DEFAULT),
                backdrop_alpha: self.appearance.backdrop_alpha,
            },
            terminal: TerminalSettings {
                font: self
                    .terminal
                    .font
                    .family
                    .clone()
                    .unwrap_or_else(|| "Cascadia Code".into()),
                size: self.terminal.font.size as u8,
                line_height: self.terminal.font.line_height,
                theme: self.terminal.theme.clone(),
            },
            sftp: SftpSettings {
                max_transfers: self.sftp.max_transfers,
                local_root: self.sftp.local_root.clone(),
            },
            editor_path: self.editor_path.clone(),
        }
    }

    /// 由界面状态写回落盘格式。
    pub fn from_app(settings: &AppSettings) -> Self {
        Self {
            version: SETTINGS_FILE_VERSION,
            appearance: AppearanceSettings {
                glass: settings.appearance.glass,
                clarity: settings.appearance.clarity,
                blur: settings.appearance.blur,
                wallpaper: WallpaperSettings {
                    path: settings.appearance.wallpaper.path.clone(),
                    name: settings.appearance.wallpaper.name.clone(),
                    custom: settings.appearance.wallpaper.custom,
                },
                overlay_alpha: Some(settings.appearance.overlay_alpha),
                overlay_alpha_delta: None,
                modal_alpha: Some(settings.appearance.modal_alpha),
                backdrop_alpha: settings.appearance.backdrop_alpha,
            },
            terminal: TerminalSettingsFile {
                font: FontSettings {
                    family: Some(settings.terminal.font.clone()),
                    size: settings.terminal.size as f32,
                    line_height: settings.terminal.line_height,
                },
                theme: settings.terminal.theme.clone(),
            },
            sftp: SftpSettingsFile {
                max_transfers: settings.sftp.max_transfers,
                local_root: settings.sftp.local_root.clone(),
            },
            editor_path: settings.editor_path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::appearance_tokens::AppearanceTokens;

    fn parse(json: &str) -> SettingsFile {
        serde_json::from_str::<SettingsFile>(json)
            .expect("测试 fixture 应当能读")
            .normalized()
    }

    /// v0：没有 version，外观是增量语义。
    #[test]
    fn migrates_v0_delta_semantics_once() {
        let file = parse(
            r#"{
                "appearance": {
                    "glass": 20,
                    "clarity": 40,
                    "blur": 12,
                    "overlay_alpha_delta": 48,
                    "overlay_blur_delta": 18,
                    "modal_alpha": 55,
                    "modal_blur": 22,
                    "backdrop_alpha": 30,
                    "backdrop_blur": 3
                }
            }"#,
        );
        assert_eq!(file.version, SETTINGS_FILE_VERSION);
        // v0 的 55 + 37 = 92 是 v1 的默认值；v1 → v2 再压到新默认 62。
        assert_eq!(file.appearance.overlay_alpha, Some(OVERLAY_DEFAULT));
        assert_eq!(file.appearance.overlay_alpha_delta, None);
        assert_eq!(file.appearance.modal_alpha, Some(MODAL_DEFAULT));
        assert_eq!(file.appearance.backdrop_alpha, 30);

        let app = file.to_app();
        assert_eq!(app.appearance.glass, 20);
        assert_eq!(app.appearance.overlay_alpha, OVERLAY_DEFAULT);
        assert_eq!(app.appearance.modal_alpha, MODAL_DEFAULT);
        assert_eq!(app.appearance.backdrop_alpha, 30);
    }

    /// v0 缺字段：旧默认值（glass 5、modal 55）也要折成新默认。
    #[test]
    fn migrates_v0_missing_fields_to_current_defaults() {
        let file = parse("{}");
        let app = file.to_app();
        assert_eq!(app.appearance.glass, GLASS_DEFAULT);
        assert_eq!(app.appearance.overlay_alpha, OVERLAY_DEFAULT);
        assert_eq!(app.appearance.modal_alpha, MODAL_DEFAULT);
        assert_eq!(app.appearance.backdrop_alpha, BACKDROP_DEFAULT);
    }

    /// v0 极端值：折算后夹进新范围，不会把弹窗写到 98 以上。
    #[test]
    fn migrates_v0_extreme_values_into_range() {
        // 95 在 v0 是「几乎不透」的增量语义，折成绝对值（132）后夹到上限。
        let file = parse(r#"{"appearance": {"modal_alpha": 95}}"#);
        assert_eq!(file.appearance.modal_alpha, Some(MODAL_MAX));

        // 30 + 37 = 67：自己调过的值，不跟着新默认回退。
        let file = parse(r#"{"appearance": {"glass": 0, "modal_alpha": 30}}"#);
        assert_eq!(file.appearance.glass, GLASS_MIN);
        assert_eq!(file.appearance.modal_alpha, Some(67));
    }

    /// v1 → v2：旧默认值（70 / 92）换成新默认，用户拖过的数值保持不变。
    #[test]
    fn replaces_v1_defaults_but_keeps_user_values() {
        let file = parse(
            r#"{
                "version": 1,
                "appearance": {
                    "overlay_alpha": 70,
                    "modal_alpha": 92,
                    "backdrop_alpha": 40
                }
            }"#,
        );
        let app = file.to_app();
        assert_eq!(app.appearance.overlay_alpha, OVERLAY_DEFAULT);
        assert_eq!(app.appearance.modal_alpha, MODAL_DEFAULT);
        assert_eq!(app.appearance.backdrop_alpha, 40);

        let file = parse(
            r#"{
                "version": 1,
                "appearance": {"overlay_alpha": 35, "modal_alpha": 50}
            }"#,
        );
        let app = file.to_app();
        assert_eq!(app.appearance.overlay_alpha, 35, "调低过的值不跟着回退");
        assert_eq!(app.appearance.modal_alpha, 50);

        // 调到极端值也是用户的明确选择，迁移不能悄悄改掉。
        let file = parse(
            r#"{
                "version": 1,
                "appearance": {"overlay_alpha": 90, "modal_alpha": 98}
            }"#,
        );
        let app = file.to_app();
        assert_eq!(app.appearance.overlay_alpha, 90);
        assert_eq!(app.appearance.modal_alpha, 98);
    }

    /// 新文件与越界值：迁移只管旧默认，之后一律夹进范围。
    #[test]
    fn keeps_current_absolute_values_and_clamps() {
        let file = parse(
            r#"{
                "version": 3,
                "appearance": {"overlay_alpha": 80, "modal_alpha": 90, "backdrop_alpha": 40}
            }"#,
        );
        let app = file.to_app();
        assert_eq!(app.appearance.overlay_alpha, 80);
        assert_eq!(app.appearance.modal_alpha, 90);

        let file = parse(
            r#"{"version": 3, "appearance": {"overlay_alpha": 10, "modal_alpha": 100, "backdrop_alpha": 200}}"#,
        );
        let app = file.to_app();
        assert_eq!(app.appearance.overlay_alpha, OVERLAY_MIN);
        assert_eq!(app.appearance.modal_alpha, MODAL_MAX);
        assert_eq!(app.appearance.backdrop_alpha, BACKDROP_MAX);
    }

    /// 写回时带版本号，旧字段彻底消失。
    #[test]
    fn writes_version_and_drops_legacy_fields() {
        let file = SettingsFile::from_app(&AppSettings::default());
        assert_eq!(file.version, SETTINGS_FILE_VERSION);
        let json = serde_json::to_string(&file).expect("能序列化");
        assert!(!json.contains("overlay_alpha_delta"));
        assert!(json.contains("\"version\":3"));
    }

    /// 默认设置读回后不变。
    #[test]
    fn defaults_roundtrip() {
        let settings = AppSettings::default();
        let file = SettingsFile::from_app(&settings);
        let restored = file.normalized().to_app();
        assert_eq!(restored.appearance, settings.appearance);
        assert_eq!(restored.sftp, settings.sftp);
        assert_eq!(restored.sftp.local_root, "", "默认留给主目录");
    }

    /// 本机默认打开路径是本机的事：读写都原样跟着走，只清掉首尾空白。
    #[test]
    fn the_sftp_local_root_roundtrips() {
        let mut settings = AppSettings::default();
        settings.sftp.local_root = r"  D:\work\deploys  ".into();
        let restored = SettingsFile::from_app(&settings).normalized().to_app();
        assert_eq!(restored.sftp.local_root, r"D:\work\deploys");

        // 旧文件没有这个字段：读出来是空，等于「用主目录」。
        let file = parse(r#"{"version": 3, "sftp": {"max_transfers": 5}}"#);
        assert_eq!(file.to_app().sftp.local_root, "");
        assert_eq!(file.to_app().sftp.max_transfers, 5);
    }

    // —— 外观令牌 ——

    #[test]
    fn tokens_derive_structural_levels_from_glass() {
        let appearance = Appearance {
            glass: 40,
            ..Appearance::default()
        };
        let tokens = AppearanceTokens::new(&appearance);
        assert!((tokens.glass - 0.40).abs() < 0.001);
        assert!((tokens.title_bar - 0.368).abs() < 0.001);
        assert!((tokens.tab_bar - 0.46).abs() < 0.001);
        assert!((tokens.tab_focus - 0.56).abs() < 0.001);
        assert!((tokens.tab_active - 0.70).abs() < 0.001);
    }

    #[test]
    fn tokens_stay_within_one() {
        let appearance = Appearance {
            glass: GLASS_MAX,
            ..Appearance::default()
        };
        let tokens = AppearanceTokens::new(&appearance);
        assert!(tokens.tab_active <= 1.0);
        assert!(tokens.tab_focus <= 1.0);
        assert!(tokens.tab_bar <= 1.0);
    }

    #[test]
    fn tokens_clamp_each_layer_to_its_own_range() {
        let appearance = Appearance {
            glass: 0,
            overlay_alpha: 100,
            modal_alpha: 10,
            backdrop_alpha: 100,
            clarity: 200,
            blur: 99,
            ..Appearance::default()
        };
        let tokens = AppearanceTokens::new(&appearance);
        assert!((tokens.glass - GLASS_MIN as f32 / 100.0).abs() < 0.001);
        assert!((tokens.overlay - OVERLAY_MAX as f32 / 100.0).abs() < 0.001);
        assert!((tokens.modal - MODAL_MIN as f32 / 100.0).abs() < 0.001);
        assert!((tokens.backdrop - BACKDROP_MAX as f32 / 100.0).abs() < 0.001);
        assert_eq!(tokens.clarity, CLARITY_MAX);
        assert_eq!(tokens.blur, BLUR_MAX);
    }

    #[test]
    fn tokens_defaults_match_product_decisions() {
        let tokens = AppearanceTokens::default();
        // 默认值来自常量，产品调参时只改常量，断言不用跟着改。
        assert!((tokens.glass - GLASS_DEFAULT as f32 / 100.0).abs() < 0.001);
        assert!((tokens.overlay - OVERLAY_DEFAULT as f32 / 100.0).abs() < 0.001);
        assert!((tokens.modal - MODAL_DEFAULT as f32 / 100.0).abs() < 0.001);
        assert!((tokens.backdrop - BACKDROP_DEFAULT as f32 / 100.0).abs() < 0.001);
        assert_eq!(tokens.blur, 10);
    }

    // —— 终端配色 ——

    #[test]
    fn terminal_theme_key_roundtrips_through_the_file() {
        let mut settings = AppSettings::default();
        settings.terminal.theme = "tokyo-night".into();
        let restored = SettingsFile::from_app(&settings).normalized().to_app();
        assert_eq!(restored.terminal.theme, "tokyo-night");
    }

    /// 空 key、未知 key 与已下线的旧 key 都退回默认。
    #[test]
    fn terminal_theme_selection_falls_back_to_default() {
        use crate::terminal::TerminalTheme;

        let default = TerminalTheme::fallback();
        assert_eq!(default.key(), "rose-pine-moon");
        assert_eq!(TerminalTheme::selected("tokyo-night").key(), "tokyo-night");
        assert_eq!(TerminalTheme::selected("").key(), default.key());
        assert_eq!(
            TerminalTheme::selected("no-such-theme").key(),
            default.key()
        );
        // 上一版列过、这一版删掉的主题：老设置里可能还留着。
        assert_eq!(TerminalTheme::selected("dracula").key(), default.key());
        assert_eq!(
            TerminalTheme::selected("solarized-light").key(),
            default.key()
        );
    }
}
