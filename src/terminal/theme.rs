//! 终端主题：终端绘制用的文字、背景、光标、选区与 16 色 ANSI。设置选中的
//! 那一套写成全局量，每个终端视图都读它，所以改动立刻作用到已开的标签；
//! 程序通过 OSC 4 / 10 / 11 / 12 问颜色时也由它回答。

use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
use gpui_kit::*;

/// 一套内置主题。颜色是 `0xrrggbb` 数据，像一张图那样，不是主题角色，
/// 所以与应用的明暗外观无关。
#[derive(Debug, PartialEq, Eq)]
pub struct TerminalTheme {
    /// What the settings file stores. Never changes once released.
    key: &'static str,
    name: &'static str,
    foreground: u32,
    background: u32,
    cursor: u32,
    /// Opaque: drawn under selected text in place of its background.
    selection: u32,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then their
    /// bright versions.
    ansi: [u32; 16],
}

impl TerminalTheme {
    /// 可在设置里另选的主题；「默认」不在其中，它由 [`Self::default`]
    /// 给出，卡片由设置页单独画一张。
    pub fn all() -> &'static [TerminalTheme] {
        &THEMES
    }

    pub fn find(key: &str) -> Option<&'static TerminalTheme> {
        THEMES.iter().find(|theme| theme.key == key)
    }

    /// 没另选主题时用的那一套：应用自己的 Rosé Pine Moon，与界面的
    /// `BASE_RGB` / `TEXT` 同源。设置里空 key 表示的就是它。
    pub fn fallback() -> &'static TerminalTheme {
        &ROSE_PINE_MOON
    }

    /// 按设置里的 key 选出主题：空 key 表示「默认」，与未知 key 一并
    /// 退回 [`Self::fallback`]。
    pub fn selected(key: &str) -> &'static TerminalTheme {
        Self::find(key).unwrap_or_else(Self::fallback)
    }

    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn foreground(&self) -> Hsla {
        rgb(self.foreground).into()
    }

    pub fn background(&self) -> Hsla {
        rgb(self.background).into()
    }

    pub fn cursor(&self) -> Hsla {
        rgb(self.cursor).into()
    }

    pub fn selection(&self) -> Hsla {
        rgb(self.selection).into()
    }

    /// One of the 256 colors a program picks by number: the 16 ANSI colors
    /// are the theme's, the 6×6×6 cube and the gray ramp above them xterm's.
    pub fn indexed(&self, index: u8) -> Hsla {
        rgb(self.indexed_rgb(index)).into()
    }

    /// The answer to a program asking for color `index` it has not set
    /// itself, numbered as alacritty numbers its colors: 0–255 by number,
    /// then the named ones (foreground, background, cursor …).
    pub fn query_color(&self, index: usize) -> Rgb {
        let value = match index {
            0..=255 => self.indexed_rgb(index as u8),
            _ if index == NamedColor::Background as usize => self.background,
            _ if index == NamedColor::Cursor as usize => self.cursor,
            // The foreground, and the dim and bright versions of it and of
            // the ANSI colors, which programs cannot ask for by OSC.
            _ => self.foreground,
        };
        Rgb {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
        }
    }

    fn indexed_rgb(&self, index: u8) -> u32 {
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let index = index - 16;
                let component = |value: u8| {
                    if value == 0 {
                        0
                    } else {
                        55 + value as u32 * 40
                    }
                };
                (component(index / 36) << 16)
                    | (component((index % 36) / 6) << 8)
                    | component(index % 6)
            }
            232..=255 => {
                let value = 8 + (index as u32 - 232) * 10;
                (value << 16) | (value << 8) | value
            }
        }
    }
}

/// The theme terminals draw with now, set by the settings for the app's
/// current appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalColors(&'static TerminalTheme);

impl Global for TerminalColors {}

impl TerminalColors {
    pub fn new(theme: &'static TerminalTheme) -> Self {
        Self(theme)
    }

    /// The theme in effect; before the settings set one, the default for
    /// the app's appearance.
    pub fn current(cx: &App) -> &'static TerminalTheme {
        cx.try_global::<Self>()
            .map(|colors| colors.0)
            .unwrap_or_else(TerminalTheme::fallback)
    }

    pub fn theme(&self) -> &'static TerminalTheme {
        self.0
    }
}

/// 终端「外壳」的颜色：默认背景、不透明背景、容器与查找栏。
///
/// 终端与其余界面一致，默认背景始终透出壁纸；主题的不透明底色保留下来，
/// 给反色单元格、搜索高亮和显式背景兜底。`overlay` 是轻量浮层的最终
/// alpha，查找栏直接使用。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalChrome {
    /// 默认单元格背景：透明，壁纸与工作区玻璃透出来。
    pub default_background: Hsla,
    /// 主题实底：反色文字与搜索高亮的兜底。
    pub opaque_background: Hsla,
    /// 终端容器（含内边距）的底色：同样透明，避免叠出第二层。
    pub container: Hsla,
    /// 查找栏的浮层不透明度（0–1）。
    pub overlay: f32,
}

impl Global for TerminalChrome {}

impl TerminalChrome {
    pub fn new(theme: &'static TerminalTheme, overlay: f32) -> Self {
        Self {
            default_background: Hsla::default(),
            opaque_background: theme.background(),
            container: Hsla::default(),
            overlay,
        }
    }

    /// 当前外壳颜色；设置还没写进来时按默认主题算。
    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_else(|| {
            Self::new(
                TerminalColors::current(cx),
                crate::settings::appearance_tokens::UiTokens::current(cx).overlay,
            )
        })
    }
}

/// 「默认（跟随应用）」用的就是界面自己的调色板：Rosé Pine Moon。界面的
/// `BASE_RGB` / `TEXT` / `FOAM` / `GOLD` / `LOVE` 取的都是这一套，终端沿用
/// 同一组值，文字与界面的灰阶、ANSI 色与强调色才是同源的。
///
/// https://raw.githubusercontent.com/rose-pine/ghostty/main/dist/rose-pine-moon (MIT).
const ROSE_PINE_MOON: TerminalTheme = TerminalTheme {
    key: "rose-pine-moon",
    name: "Rosé Pine Moon",
    foreground: 0xe0def4,
    background: 0x232136,
    cursor: 0xe0def4,
    selection: 0x44415a,
    ansi: [
        0x393552, 0xeb6f92, 0x3e8fb0, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xea9a97, 0xe0def4, 0x6e6a86,
        0xeb6f92, 0x3e8fb0, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xea9a97, 0xe0def4,
    ],
};

/// 可另选的主题：只剩两个常用的深色方案，加上单独一张的「默认」（Rosé
/// Pine Moon），一行卡片就排得下，用户不用在二十个近似选项里挑。
static THEMES: [TerminalTheme; 2] = [TOKYO_NIGHT, CATPPUCCIN_MOCHA];

/// https://raw.githubusercontent.com/folke/tokyonight.nvim/main/extras/ghostty/tokyonight_night (Apache-2.0).
const TOKYO_NIGHT: TerminalTheme = TerminalTheme {
    key: "tokyo-night",
    name: "Tokyo Night",
    foreground: 0xc0caf5,
    background: 0x1a1b26,
    cursor: 0xc0caf5,
    selection: 0x283457,
    ansi: [
        0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868,
        0xff899d, 0x9fe044, 0xfaba4a, 0x8db0ff, 0xc7a9ff, 0xa4daff, 0xc0caf5,
    ],
};

/// https://raw.githubusercontent.com/catppuccin/ghostty/main/themes/catppuccin-mocha.conf (MIT).
const CATPPUCCIN_MOCHA: TerminalTheme = TerminalTheme {
    key: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    foreground: 0xcdd6f4,
    background: 0x1e1e2e,
    cursor: 0xf5e0dc,
    selection: 0x353749,
    ansi: [
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8, 0x585b70,
        0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
    ],
};
