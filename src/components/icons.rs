//! 图标辅助：全应用只用 gpui-kit 的 Lucide 目录，避免混用两套线宽。
//! 这里提供一个 12px 的工具提示包装，因为 gpui-kit 默认是 14px。

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

/// 工具提示的字号（`--type-meta`）；gpui-kit 的 `Button::tooltip` 固定用
/// 14px，比设计系统大一级，所以自己构造。
pub const TOOLTIP_TEXT_SIZE: f32 = 12.0;

/// 给一个元素配 12px 的工具提示。`id` 要唯一，工具提示挂在容器上。
pub fn with_tooltip(
    id: &'static str,
    text: impl Into<SharedString>,
    child: impl IntoElement,
) -> Stateful<Div> {
    let text = text.into();
    div()
        .id(ElementId::Name(id.into()))
        .tooltip(move |window, cx| {
            Tooltip::new(text.clone())
                .text_size(px(TOOLTIP_TEXT_SIZE))
                .build(window, cx)
        })
        .child(child)
}
