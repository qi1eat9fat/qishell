//! 统一弹窗表面与排版。
//!
//! 所有 `window.open_dialog` 都通过 [`surface`] 取同一套宽度、玻璃、边框与
//! 圆角；标题、说明、分隔线、底部按钮也各有固定实现，避免每个弹窗各写一遍。

use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::*;

use crate::theme;

/// 弹窗表面：宽度、内容实底、1px 线条边框。
pub fn surface(dialog: Dialog, width: Pixels, modal_alpha: f32) -> Dialog {
    dialog
        .width(width)
        .bg(theme::modal_surface(modal_alpha))
        .border_1()
        .border_color(theme::line())
}

/// 弹窗标题（16px，600）。
pub fn title(title: impl Into<SharedString>) -> Div {
    div()
        .text_size(theme::TYPE_TITLE)
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.into())
}

/// 标题下的一行说明（13px，`OVERLAY_SUBTLE`）。
pub fn caption(caption: impl Into<SharedString>) -> Div {
    div()
        .mt(px(5.))
        .text_size(theme::TYPE_BODY)
        .line_height(px(19.))
        .text_color(theme::solid(theme::OVERLAY_SUBTLE))
        .child(caption.into())
}

/// 标题与内容之间的 1px 分隔线。
pub fn divider() -> Div {
    div().h(px(1.)).w_full().bg(theme::line())
}

/// 标准底部按钮栏。
pub fn footer() -> DialogFooter {
    DialogFooter::new().gap_2()
}

/// 表单按钮的统一尺寸。保留这个入口，方便调用方继续链式设置状态。
pub fn button(button: Button) -> Button {
    button
        .h(theme::BUTTON_HEIGHT)
        .px(px(12.))
        .rounded(theme::RADIUS_MD)
}
