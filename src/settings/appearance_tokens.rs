//! 外观分类的最终不透明度令牌。
//!
//! 界面按层归类：玻璃（常驻结构面）、轻量浮层（临时浮起）、模态弹窗（阻断
//! 式）、背景（壁纸与遮罩）、终端配色。滑杆上的值就是这里使用的值；结构面
//! 内部的派生比例（顶栏 ×0.92、标签栏 +0.06 等）只在这里定义，不再散落到
//! 视图里，也不再作为用户参数。
//!
//! 所有外观范围常量都定义在本模块：`settings/model.rs` 的夹取、设置页滑杆
//! 的范围与文案都引用它，避免三处各写一份。

use gpui_kit::component::Theme;
use gpui_kit::*;

use crate::model::Appearance;

// —— 范围与默认值 ——

/// 玻璃不透明度（%）。
pub const GLASS_MIN: u8 = 5;
pub const GLASS_MAX: u8 = 90;
pub const GLASS_DEFAULT: u8 = 5;

/// 轻量浮层不透明度（%）。
///
/// 壁纸是界面底色，菜单也要透出来。默认 45% 时菜单仍是一层可辨认的浮起
/// 表面，壁纸的纹理与明暗都能透过来；30% 是保底，再低菜单项就开始和背景
/// 混在一起。
pub const OVERLAY_MIN: u8 = 30;
pub const OVERLAY_MAX: u8 = 92;
pub const OVERLAY_DEFAULT: u8 = 45;

/// 模态弹窗不透明度（%）。
///
/// 默认 52：弹窗像一层压暗的玻璃罩，壁纸的云和山坡都看得见；38% 是保底，
/// 再低正文会随壁纸明暗起伏。低于 50% 时设置页提示对比度风险。
pub const MODAL_MIN: u8 = 38;
pub const MODAL_MAX: u8 = 98;
pub const MODAL_DEFAULT: u8 = 52;
/// 弹窗可读性提示的阈值。
pub const MODAL_READABLE_MIN: u8 = 50;

/// 全屏遮罩不透明度（%）。
pub const BACKDROP_MIN: u8 = 0;
pub const BACKDROP_MAX: u8 = 90;
pub const BACKDROP_DEFAULT: u8 = 50;

/// 背景清晰度（%）。
pub const CLARITY_MIN: u8 = 0;
pub const CLARITY_MAX: u8 = 100;
pub const CLARITY_DEFAULT: u8 = 65;

/// 壁纸预模糊（px）。
pub const BLUR_MIN: u8 = 0;
pub const BLUR_MAX: u8 = 30;
pub const BLUR_DEFAULT: u8 = 10;

// —— 结构面的内部派生比例 ——

/// 顶栏比主玻璃略淡，避免压过内容。
pub const TITLE_BAR_FACTOR: f32 = 0.92;
/// 标签栏比工作区玻璃略实。
pub const TAB_BAR_STEP: f32 = 0.06;
/// 键盘焦点落在标签上时。
pub const TAB_FOCUS_STEP: f32 = 0.16;
/// 当前活动标签。
pub const TAB_ACTIVE_STEP: f32 = 0.30;

/// 一帧里各层实际使用的 alpha。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppearanceTokens {
    /// 主机侧栏、右侧栏、命令片段侧栏、工作区容器。
    pub glass: f32,
    /// 顶栏。
    pub title_bar: f32,
    /// 标签栏。
    pub tab_bar: f32,
    /// 获得键盘焦点的标签。
    pub tab_focus: f32,
    /// 当前活动标签。
    pub tab_active: f32,
    /// 右键菜单、下拉、Tooltip、Toast、查找栏、拖拽幽灵。
    pub overlay: f32,
    /// 所有模态弹窗。
    pub modal: f32,
    /// 全屏遮罩。
    pub backdrop: f32,
    /// 壁纸清晰度（0–100）。
    pub clarity: u8,
    /// 壁纸预模糊半径（px）。
    pub blur: u8,
}

impl Default for AppearanceTokens {
    fn default() -> Self {
        Self::new(&Appearance::default())
    }
}

impl AppearanceTokens {
    pub fn new(appearance: &Appearance) -> Self {
        let glass = ratio(appearance.glass, GLASS_MIN, GLASS_MAX);
        Self {
            glass,
            title_bar: glass * TITLE_BAR_FACTOR,
            tab_bar: (glass + TAB_BAR_STEP).min(1.0),
            tab_focus: (glass + TAB_FOCUS_STEP).min(1.0),
            tab_active: (glass + TAB_ACTIVE_STEP).min(1.0),
            overlay: ratio(appearance.overlay_alpha, OVERLAY_MIN, OVERLAY_MAX),
            modal: ratio(appearance.modal_alpha, MODAL_MIN, MODAL_MAX),
            backdrop: ratio(appearance.backdrop_alpha, BACKDROP_MIN, BACKDROP_MAX),
            clarity: appearance.clarity.min(CLARITY_MAX),
            blur: appearance.blur.min(BLUR_MAX),
        }
    }
}

fn ratio(value: u8, min: u8, max: u8) -> f32 {
    value.clamp(min, max) as f32 / 100.0
}

/// GPUI 全局量：视图层统一读它，避免每个 Entity 缓存一份过期快照。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiTokens(pub AppearanceTokens);

impl Global for UiTokens {}

impl UiTokens {
    /// 当前令牌；设置还没写进来时用默认值。
    pub fn current(cx: &App) -> AppearanceTokens {
        cx.try_global::<Self>()
            .map(|tokens| tokens.0)
            .unwrap_or_default()
    }
}

/// 把最终值写进 gpui-kit 主题：遮罩、菜单/下拉、Tooltip、侧栏。
///
/// `Theme::sync_base` 只同步 `colors`，不会重建 `tokens`；每次改了颜色都要
/// 显式重建，Tooltip 与 Select 才会跟随浮层滑杆。
pub fn apply_to_theme(theme: &mut Theme, tokens: AppearanceTokens) {
    let colors = &mut theme.colors;
    colors.popover = crate::theme::overlay(tokens.overlay);
    colors.overlay = crate::theme::colored(crate::theme::BACKDROP_RGB, tokens.backdrop);
    colors.sidebar = crate::theme::glass(tokens.glass);

    let colors = *colors;
    theme.tokens = colors.into();
    // 重建后补回派生项。
    theme.tokens.list_hover = colors.list_hover.into();
    theme.tokens.table_hover = colors.list_hover.into();
    theme.tokens.list_active = colors.list_active.into();
    theme.tokens.list_active_border = colors.list_active_border.into();
}

#[cfg(test)]
mod tests {
    #[test]
    fn menu_popover_and_backdrop_follow_their_own_slider() {
        let tokens = crate::settings::appearance_tokens::AppearanceTokens::new(
            &crate::model::Appearance::default(),
        );
        let mut theme = gpui_kit::component::Theme::default();
        crate::settings::appearance_tokens::apply_to_theme(&mut theme, tokens);
        // 菜单/下拉/Tooltip 走浮层值，遮罩走弹窗分类下的独立值。
        assert!((theme.colors.popover.a - tokens.overlay).abs() < 0.001);
        assert!((theme.colors.overlay.a - tokens.backdrop).abs() < 0.001);
        assert!((theme.colors.sidebar.a - tokens.glass).abs() < 0.001);
        assert!((theme.tokens.popover.a - tokens.overlay).abs() < 0.001);
    }

    #[test]
    fn modal_surface_uses_the_slider_value_exactly() {
        // 旧实现固定 +0.37，用户看到的数字不是实际值；这里守住不再偏移。
        for value in [0.75, 0.92, 0.98] {
            let color = crate::theme::modal_surface(value);
            assert!((color.a - value).abs() < 0.001);
        }
    }
}
