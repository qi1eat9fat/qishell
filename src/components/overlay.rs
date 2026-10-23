//! 浮层：右键菜单、下拉菜单与 Toast 的共用表面。

use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;

/// 菜单表面：浮层玻璃、1px 线条、10px 圆角、6px 内边距。
///
/// 调用方在返回的 `Div` 上继续设置 `id`、绝对定位与宽度。
pub fn menu_surface(overlay_alpha: f32) -> Div {
    div()
        .p(px(6.))
        .rounded(theme::RADIUS_XL)
        .border_1()
        .border_color(theme::line())
        .bg(theme::overlay(overlay_alpha))
        .shadow_lg()
        .occlude()
}

/// 菜单项：26px 高、12px 文字、纯文本；不显示图标与快捷键。
///
/// 选中/键盘高亮由调用方根据 active 状态补背景。
pub fn menu_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    danger: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .w_full()
        .h(px(26.))
        .px(px(8.))
        .rounded(theme::RADIUS_MD)
        .text_size(theme::TYPE_META)
        .text_color(theme::solid(if danger {
            theme::LOVE
        } else {
            theme::OVERLAY_TEXT
        }))
        .when(enabled, |item| {
            item.cursor_pointer().hover(move |style| {
                style.bg(if danger {
                    theme::colored(theme::LOVE, 0.10)
                } else {
                    theme::hover_tint()
                })
            })
        })
        .when(!enabled, |item| {
            item.cursor_default().opacity(theme::DISABLED_OPACITY)
        })
        .child(label.into())
}

/// 菜单里的 1px 分隔线。
pub fn menu_divider() -> Div {
    div().h(px(1.)).mx(px(4.)).my(px(4.)).bg(theme::line())
}

/// 标准 Toast：右下角、最多 320px、12px 文字、浮层玻璃。
///
/// Toast 同时承载成功、提示与错误消息，调用方没有统一的类型信息，因此
/// 不使用勾选图标或成功色，避免把失败消息画成成功。
pub fn toast(message: impl Into<SharedString>, overlay_alpha: f32) -> Div {
    h_flex()
        .items_center()
        .gap(px(8.))
        .px(px(11.))
        .py(px(9.))
        .rounded(theme::RADIUS_LG)
        .border_1()
        .border_color(theme::line())
        .bg(theme::overlay(overlay_alpha))
        .text_size(theme::TYPE_META)
        .text_color(theme::solid(theme::TEXT))
        .child(message.into())
}
