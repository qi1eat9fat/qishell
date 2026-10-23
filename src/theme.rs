//! 设计令牌：Rosé Pine Moon。所有颜色、尺寸都从这里取，视图里不写死色值。

use std::sync::Arc;

use gpui_kit::component::Theme;
use gpui_kit::*;

use crate::assets;

// —— 原型配色 ——
pub const BASE_RGB: u32 = 0x232136;
/// 更深的底：主按钮文字、关闭态页面、取景框。
pub const BASE_DEEP_RGB: u32 = 0x1b1928;
/// 全屏遮罩的底色。
pub const BACKDROP_RGB: u32 = 0x0f0d18;
/// 弹窗内容的底色：比背景更深。
///
/// 弹窗用的是低 alpha 的半透明罩（默认 52%），壁纸会透上来。底色越深，
/// 同样的不透明度下正文越清楚——所以要让弹窗更透，靠的不是抬高 alpha，
/// 而是把这一层压暗。
pub const MODAL_RGB: u32 = 0x1a1728;
pub const SURFACE_RGB: u32 = 0x2a273f;
/// 玻璃层的色调。比 Surface 再亮一档：底色与 Surface 只差 7 级，若玻璃也
/// 用 Surface，30% 以下的滑杆值肉眼看不出来。这一档保证 5%–90% 全程都有
/// 可见变化，5% 时仍几乎与纯色背景一致。
pub const GLASS_RGB: u32 = 0x35314e;
/// 浮层的色调：比玻璃深得多。
///
/// 浮层只盖一小块，壁纸从下面透上来时更要压得住，否则菜单项会和背景糊在
/// 一起。同样的不透明度下，深底让白字更亮，浅底反而更灰。
pub const OVERLAY_RGB: u32 = 0x1c1930;

pub const TEXT: u32 = 0xe0def4;
pub const SUBTLE: u32 = 0x908caa;
pub const MUTED_STRONG: u32 = 0xb8b4d0;
/// 浮层与弹窗内的次要文字：比 `SUBTLE` 亮两档。
///
/// 这两类界面都叠在壁纸上，背景随壁纸明暗起伏；沿用常驻面板的灰会在
/// 亮色壁纸（云、雪）上糊成一片。常驻结构面底色稳定，仍用 `SUBTLE`。
pub const OVERLAY_SUBTLE: u32 = 0xbdb8d6;
/// 浮层与弹窗内的常规文字：比 `MUTED_STRONG` 更接近白色。
pub const OVERLAY_TEXT: u32 = 0xd2cee4;
pub const FOAM: u32 = 0x9ccfd8;
pub const GOLD: u32 = 0xf6c177;
pub const LOVE: u32 = 0xeb6f92;
pub const IRIS: u32 = 0xc4a7e7;
pub const HIGHLIGHT_HIGH: u32 = 0x56526e;

/// 不透明的原型色。
pub fn solid(rgb: u32) -> Hsla {
    Hsla::from(gpui_kit::rgb(rgb))
}

/// 带透明度的原型色。
pub fn colored(rgb: u32, alpha: f32) -> Hsla {
    solid(rgb).alpha(alpha)
}

/// 等宽数字：开启 OpenType `tnum`，让 0–9 在比例字体里也同宽。
///
/// 数据字段（地址、指纹、文件大小、时间、计数）改用鸿蒙 Sans 后靠它保持
/// 列对齐，不必再为数字退回等宽字体。
pub fn tabular_numbers() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// 状态点的柔和光晕，模拟原型的 `box-shadow`。
///
/// DESIGN.md 的 The No-Glow Rule 要求状态点用「实心点 + 2px 半透明细环」
/// 表达，所以这里给出细环，而不是零偏移的彩色发光。
/// 键盘焦点环：2px 荧光青，画在元素内侧，避免焦点跳转时挤动布局。
pub fn focus_ring() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: solid(FOAM),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: true,
    }]
}

pub fn ring(color: Hsla) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: color.alpha(0.35),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    }]
}

/// 面板/标题栏等玻璃层。`alpha` 取外观设置里的百分比。
pub fn glass(alpha: f32) -> Hsla {
    colored(GLASS_RGB, alpha)
}

/// 浮层表面：菜单、下拉、Tooltip、Toast、查找栏、拖拽幽灵共用。
pub fn overlay(alpha: f32) -> Hsla {
    colored(OVERLAY_RGB, alpha.clamp(0.0, 1.0))
}

/// 弹窗内容表面：滑杆上的值就是实际值，不再偷偷抬高。
pub fn modal_surface(alpha: f32) -> Hsla {
    colored(MODAL_RGB, alpha.clamp(0.0, 1.0))
}

// —— 叠在玻璃之上的内容层 ——
//
// 这些是面板内部的局部填充，不跟随外观滑杆：它们已经是「玻璃之上」的一层，
// 再乘一次玻璃会形成双重透明。数值沿用原型，集中在这里，视图层不再写裸值。

/// 设置分类栏、SFTP 文件面板。
pub const PANEL_TINT: f32 = 0.16;
/// SFTP 根层。
pub const ROOT_TINT: f32 = 0.18;
/// SFTP 传输队列。
pub const QUEUE_TINT: f32 = 0.28;
/// 输入框、文本域、分段控件的固定深色底。
pub const FIELD_TINT: f32 = 0.6;
/// 预览块。
pub const PREVIEW_TINT: f32 = 0.55;
/// 壁纸卡片、规则表等实心卡片。
pub const CARD_TINT: f32 = 0.6;

/// 分割线与边框。全应用只有两档：
/// - [`line`]：常规分割线、面板边框
/// - [`line_subtle`]：同一栏内部的细分隔
pub fn line() -> Hsla {
    colored(HIGHLIGHT_HIGH, 0.55)
}

pub fn line_subtle() -> Hsla {
    colored(HIGHLIGHT_HIGH, 0.10)
}

/// 进度条 / 滑杆的轨道底色。
pub fn track() -> Hsla {
    colored(HIGHLIGHT_HIGH, 0.35)
}

/// 三档白色悬停叠加，见 DESIGN.md「Neutral」。
pub fn hover_subtle() -> Hsla {
    colored(0xffffff, HOVER_SUBTLE)
}

pub fn hover_tint() -> Hsla {
    colored(0xffffff, HOVER_TINT)
}

pub fn hover_tint_strong() -> Hsla {
    colored(0xffffff, HOVER_TINT_STRONG)
}

/// 壁纸之上的暗色遮罩；清晰度 65% 对应 0.35。
pub fn veil(clarity: u8) -> Hsla {
    colored(BASE_RGB, (100.0 - clarity as f32) / 100.0)
}

/// 工作区与标签栏的玻璃底色。
///
/// 与 `glass()` 同色是必须的：工作区根容器铺的就是 BASE_RGB，若这里也用
/// BASE_RGB，同色叠同色，没有壁纸时滑杆拉到底也看不出变化。
pub fn workspace_glass(alpha: f32) -> Hsla {
    colored(GLASS_RGB, alpha)
}

// —— 尺寸 ——
/// 顶栏高度：32px 控件 + 上下各 2px 留白。
pub const TITLE_BAR_HEIGHT: Pixels = px(36.0);
pub const SIDEBAR_WIDTH: Pixels = px(264.0);
/// 左右窄栏宽度：与顶栏、第二层同为 36px，形成统一的 36px 栅格。
pub const RAIL_WIDTH: Pixels = px(36.0);
/// 收起后的主机栏宽度：与右侧栏同宽，作为一个可展开的窄栏保留切换入口。
pub const SIDEBAR_COLLAPSED_WIDTH: Pixels = RAIL_WIDTH;
/// 标签固定宽度：连接与设置页签统一 180px，避免长短不一。
pub const TAB_WIDTH: Pixels = px(180.0);
/// 第二层栏高度：侧栏头部、标签栏与 SFTP 工具栏，与顶栏同高。
pub const BAR_HEIGHT: Pixels = px(36.0);
pub const MIN_WINDOW_WIDTH: f32 = 960.0;
pub const MIN_WINDOW_HEIGHT: f32 = 600.0;
/// 所有可点击目标的最小命中区（DESIGN.md 硬性下限）。
pub const MIN_HIT_TARGET: Pixels = px(32.0);
/// 紧凑图标按钮的命中区与悬停底：图标 14px。
pub const ICON_BUTTON_SIZE: Pixels = px(28.0);

// —— 控件尺寸 ——
/// 弹窗底部按钮（原型的表单按钮）。
pub const BUTTON_HEIGHT: Pixels = px(33.0);
/// 普通控件与图标按钮的统一高度。
pub const CONTROL_HEIGHT: Pixels = px(32.0);
/// 输入框、下拉框的统一高度。
pub const INPUT_HEIGHT: Pixels = px(34.0);
/// 设置内容列的最大宽度。
pub const CONTENT_MAX_WIDTH: Pixels = px(760.0);
/// 设置分类栏的默认与可拖范围宽度。
///
/// 默认宽度与标签同一档（`TAB_WIDTH`）：分类栏右侧的分割线与标签栏的分隔线
/// 落在同一条竖线上，上下两块结构面才对得齐。
pub const SETTINGS_NAV_WIDTH: Pixels = TAB_WIDTH;
pub const SETTINGS_NAV_MIN_WIDTH: Pixels = px(150.0);
pub const SETTINGS_NAV_MAX_WIDTH: Pixels = px(300.0);
/// 设置内容区的内边距。
pub const SETTINGS_CONTENT_PX: Pixels = px(32.0);
pub const SETTINGS_CONTENT_PT: Pixels = px(24.0);
pub const SETTINGS_CONTENT_PB: Pixels = px(36.0);
/// 确认类弹窗与表单类弹窗的宽度。
pub const DIALOG_WIDTH: Pixels = px(440.0);
pub const DIALOG_WIDTH_FORM: Pixels = px(560.0);
/// 禁用态的统一透明度。
pub const DISABLED_OPACITY: f32 = 0.5;

// —— 图标尺寸 ——
pub const ICON_XS: Pixels = px(12.0);
pub const ICON_SM: Pixels = px(14.0);
pub const ICON_LG: Pixels = px(18.0);

// —— 间距节奏（4px 基准）——
pub const SPACE_MD: Pixels = px(12.0);
pub const SPACE_XL: Pixels = px(22.0);
/// 普通控件里「图标 + 文字」的统一间距。
pub const ICON_TEXT_GAP: Pixels = px(8.0);

// —— 字号令牌（DESIGN.md「Typography」）——
//
// 全应用只有 11 / 12 / 13px 三档正文级字号，加上 16px 弹窗标题。不要再引
// 入 10.5px、11.5px、12.5px 这类半像素值：它们在 DPI 缩放与 CJK 回退下会
// 被取整，层级收益为零。

/// 弹窗标题。
pub const TYPE_TITLE: Pixels = px(16.0);
/// 分区标题、主机名、连接栏主机名。
pub const TYPE_SECTION: Pixels = px(13.0);
/// 正文、说明、表单控件。
pub const TYPE_BODY: Pixels = px(13.0);
/// 字段标签、设置项标签、侧栏分组标题。
pub const TYPE_LABEL: Pixels = px(12.0);
/// 元信息、注释、按钮、菜单、标签页、规则表输入。
pub const TYPE_META: Pixels = px(12.0);
/// 状态栏、关键字表头 —— 全应用下限，不再往下走。
pub const TYPE_MICRO: Pixels = px(11.0);

// —— 圆角六档（DESIGN.md「Shapes」）——
/// 小控件内部。
pub const RADIUS_XS: Pixels = px(4.0);
/// 徽标、色块。
pub const RADIUS_SM: Pixels = px(5.0);
/// 按钮、输入框、图标按钮、菜单项。
pub const RADIUS_MD: Pixels = px(6.0);
/// 主机卡片、右键菜单。
pub const RADIUS_LG: Pixels = px(8.0);
/// 弹窗。
pub const RADIUS_XL: Pixels = px(10.0);

// —— 白色悬停三档（DESIGN.md「Neutral」）——
/// 极轻的悬停底。
pub const HOVER_SUBTLE: f32 = 0.035;
/// 常规悬停。
pub const HOVER_TINT: f32 = 0.05;
/// 较强的悬停与按下。
pub const HOVER_TINT_STRONG: f32 = 0.08;

/// 写入 gpui-kit 的主题。启动时调用一次。
pub fn init(cx: &mut App) {
    Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    let mono = assets::mono_font_family(cx);
    {
        let theme = Theme::global_mut(cx);
        theme.font_family = assets::UI_FONT_FAMILY.into();
        theme.mono_font_family = mono;
        theme.font_size = px(16.0);
        theme.mono_font_size = px(13.0);
        theme.radius = RADIUS_MD;
        theme.radius_lg = RADIUS_XL;
        theme.shadow = true;
        // 无边框的按钮（primary / ghost / text）如果只改 `border_color`，
        // 聚焦时看不到任何东西。保留 gpui-kit 的外环，让所有可聚焦控件都有
        // 可见的焦点指示；输入框外壳仍额外画 2px Foam 边框。
        theme.focus_ring = true;

        let colors = &mut theme.colors;
        colors.background = solid(BASE_RGB);
        colors.foreground = solid(TEXT);
        colors.border = colored(0x56526e, 0.55);
        colors.input = colored(0x56526e, 0.65);
        colors.selection = colored(FOAM, 0.22);
        colors.muted = colored(SUBTLE, 0.22);
        colors.muted_foreground = solid(SUBTLE);
        colors.accent = colored(FOAM, 0.12);
        colors.accent_foreground = solid(FOAM);
        colors.primary = solid(FOAM);
        colors.primary_foreground = solid(BASE_DEEP_RGB);
        colors.primary_hover = solid(0xb3e0e6);
        colors.primary_active = solid(0x8fc3cc);
        colors.danger = solid(LOVE);
        colors.danger_foreground = solid(BASE_DEEP_RGB);
        colors.danger_hover = solid(0xf08aa6);
        colors.danger_active = solid(0xdd5f82);
        colors.warning = solid(GOLD);
        colors.info = solid(IRIS);
        colors.success = solid(FOAM);
        colors.overlay = colored(BACKDROP_RGB, 0.5);
        colors.ring = solid(FOAM);
        colors.caret = solid(FOAM);
        colors.link = solid(FOAM);
        colors.link_hover = solid(0xb3e0e6);
        colors.link_active = solid(FOAM);
        colors.popover = colored(SURFACE_RGB, 0.96);
        colors.popover_foreground = solid(TEXT);
        colors.sidebar = glass(0.05);
        colors.list = Hsla::default();
        colors.list_hover = colored(0xffffff, HOVER_TINT);
        colors.list_active = colored(FOAM, 0.10);
        colors.list_active_border = colored(FOAM, 0.22);
        colors.list_even = Hsla::default();
        colors.list_head = Hsla::default();
        colors.group_box = colored(SURFACE_RGB, 0.35);
        colors.group_box_foreground = solid(TEXT);
        colors.button = Hsla::default();
        colors.button_hover = colored(0xffffff, HOVER_TINT);
        colors.button_active = colored(0xffffff, HOVER_TINT_STRONG);
        colors.button_foreground = solid(MUTED_STRONG);
        colors.button_primary = solid(FOAM);
        colors.button_primary_hover = solid(0xb3e0e6);
        colors.button_primary_active = solid(0x8fc3cc);
        colors.button_primary_foreground = solid(BASE_DEEP_RGB);
        colors.button_danger = solid(LOVE);
        colors.button_danger_hover = solid(0xf08aa6);
        colors.button_danger_active = solid(0xdd5f82);
        colors.button_danger_foreground = solid(BASE_DEEP_RGB);
        colors.switch = colored(SUBTLE, 0.22);
        colors.switch_thumb = solid(MUTED_STRONG);
        colors.secondary = colored(0xffffff, HOVER_SUBTLE);
        colors.secondary_hover = colored(0xffffff, HOVER_TINT_STRONG);
        colors.secondary_active = colored(0xffffff, HOVER_TINT_STRONG);
        colors.secondary_foreground = solid(MUTED_STRONG);
        theme.transparent = Hsla::default();

        let colors = *colors;
        theme.tokens = colors.into();
        theme.tokens.list_hover = colors.list_hover.into();
        theme.tokens.table_hover = colors.list_hover.into();
        theme.tokens.list_active = colors.list_active.into();
        theme.tokens.list_active_border = colors.list_active_border.into();
    }
    Theme::sync_base(cx);
}

#[cfg(test)]
mod tests {
    use super::{BASE_RGB, glass, solid, workspace_glass};

    /// 设置分类栏的分割线要与标签栏的分隔线落在同一条竖线上。
    #[test]
    fn settings_nav_width_matches_tab_width() {
        assert_eq!(super::SETTINGS_NAV_WIDTH, super::TAB_WIDTH);
    }

    /// 玻璃滑杆必须真的改变像素：没有壁纸时面板下面就是 BASE_RGB，
    /// 若玻璃色与它相同，同色叠同色，整条滑杆等于装饰。
    #[test]
    fn glass_stays_visible_on_the_plain_background() {
        let backdrop = solid(BASE_RGB);
        let low = backdrop.blend(glass(0.05));
        let high = backdrop.blend(glass(0.90));
        let delta = (high.l - low.l).abs();
        assert!(
            delta > 0.05,
            "5% 与 90% 的观感差太小（Δl = {delta:.3}），滑杆形同失效"
        );
    }

    /// 工作区与侧栏共用同一个玻璃色，滑杆拉动时各面板一起变化。
    #[test]
    fn workspace_and_panels_share_one_glass_tint() {
        for alpha in [0.05, 0.5, 0.9] {
            assert_eq!(glass(alpha), workspace_glass(alpha));
        }
    }
}

#[cfg(test)]
mod overlay_contrast {
    //! 浮层与弹窗叠在壁纸上：默认不透明度下，文字必须还读得清。
    //!
    //! 用壁纸最亮的「云」作为最坏情况算对比度；调低不透明度或改了文字色，
    //! 这里会先失败。

    fn channel(value: u8) -> f32 {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(rgb: (u8, u8, u8)) -> f32 {
        0.2126 * channel(rgb.0) + 0.7152 * channel(rgb.1) + 0.0722 * channel(rgb.2)
    }

    fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        let (high, low) = if a > b { (a, b) } else { (b, a) };
        (high + 0.05) / (low + 0.05)
    }

    fn channels(value: u32) -> (u8, u8, u8) {
        ((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    fn blend(top: (u8, u8, u8), alpha: f32, bottom: (u8, u8, u8)) -> (u8, u8, u8) {
        let mix = |t: u8, b: u8| (t as f32 * alpha + b as f32 * (1.0 - alpha)).round() as u8;
        (
            mix(top.0, bottom.0),
            mix(top.1, bottom.1),
            mix(top.2, bottom.2),
        )
    }

    /// 壁纸经过清晰度遮罩后最亮的一处：白云。
    fn brightest_wallpaper() -> (u8, u8, u8) {
        blend(channels(super::BASE_RGB), 0.35, (220, 225, 230))
    }

    #[test]
    fn overlay_text_stays_readable_on_a_bright_wallpaper() {
        use crate::settings::appearance_tokens::OVERLAY_DEFAULT;
        let background = blend(
            channels(super::OVERLAY_RGB),
            OVERLAY_DEFAULT as f32 / 100.0,
            brightest_wallpaper(),
        );
        let contrast = contrast(channels(super::OVERLAY_TEXT), background);
        assert!(
            contrast >= 3.5,
            "浮层文字在亮色壁纸上只有 {contrast:.2}:1，菜单会读不清"
        );
    }

    #[test]
    fn modal_body_stays_readable_on_a_bright_wallpaper() {
        use crate::settings::appearance_tokens::MODAL_DEFAULT;
        let background = blend(
            channels(super::MODAL_RGB),
            MODAL_DEFAULT as f32 / 100.0,
            brightest_wallpaper(),
        );
        let contrast = contrast(channels(super::OVERLAY_TEXT), background);
        assert!(
            contrast >= 3.5,
            "弹窗正文在亮色壁纸上只有 {contrast:.2}:1，正文会读不清"
        );
    }
}
