//! 统一按钮。
//!
//! 页面不再直接给 gpui-kit 的 `Button` 写高度、内边距、圆角与字号：
//! 这里给出五种标准形态，尺寸来自 `theme`。需要额外链式配置时，直接继续
//! 调用 `Button` 的方法即可。

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon,
    button::{Button, ButtonVariants as _},
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;

/// 按钮文字。gpui-kit 会按尺寸把按钮文字设成 14–16px，设计系统统一 12px。
pub fn button_label(label: impl Into<SharedString>) -> Div {
    div()
        .text_size(theme::TYPE_META)
        .line_height(px(16.))
        .child(label.into())
}

/// 表单 / 弹窗底部按钮：高 33px、水平内边距 12px。
fn form_button(button: Button) -> Button {
    button
        .h(theme::BUTTON_HEIGHT)
        .px(px(12.))
        .rounded(theme::RADIUS_MD)
}

/// 普通页面按钮：高 32px、水平内边距 10px。
fn control_button(button: Button) -> Button {
    button
        .h(theme::CONTROL_HEIGHT)
        .px(px(10.))
        .rounded(theme::RADIUS_MD)
}

pub fn form_primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    form_button(Button::new(id).primary()).child(button_label(label))
}

pub fn form_secondary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    form_button(Button::new(id).outline()).child(button_label(label))
}

pub fn form_danger(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    form_button(Button::new(id).danger()).child(button_label(label))
}

pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    control_button(Button::new(id).primary()).child(button_label(label))
}

pub fn secondary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    control_button(Button::new(id).outline()).child(button_label(label))
}

pub fn danger(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    control_button(Button::new(id).danger()).child(button_label(label))
}

/// 低权重文字按钮（如「恢复默认」）：无边框、无底色，但仍保留 32px 命中区。
pub fn text(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id)
        .text()
        .h(theme::CONTROL_HEIGHT)
        .px(px(6.))
        .rounded(theme::RADIUS_MD)
        .child(button_label(label))
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 图标按钮：32×32 命中区，28×28 可见悬停底。
///
/// 顶栏、侧栏头部与查找栏空间紧张，32×32 的悬停方块显得过大；视觉底向内
/// 收 2px，点击区仍是规范要求的 32×32。
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: Option<IconName>,
    icon_size: Option<Pixels>,
    child: Option<AnyElement>,
    label: Option<SharedString>,
    disabled: bool,
    tooltip: Option<SharedString>,
    hover: Option<Hsla>,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            icon_size: None,
            child: None,
            label: None,
            disabled: false,
            tooltip: None,
            hover: None,
            on_click: None,
        }
    }

    pub fn icon(mut self, icon: impl Into<IconName>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn icon_size(mut self, size: Pixels) -> Self {
        self.icon_size = Some(size);
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// 覆盖默认的 5% 悬停底；仍只占 28×28。
    pub fn hover_color(mut self, color: Hsla) -> Self {
        self.hover = Some(color);
        self
    }

    pub fn on_click(
        mut self,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(on_click));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.disabled;
        let on_click = self.on_click;
        let label = self.label.unwrap_or_default();
        let focus_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let hover = self.hover.unwrap_or_else(theme::hover_tint);
        let content = self.child.unwrap_or_else(|| {
            Icon::new(self.icon.unwrap_or(IconName::Plus))
                .size(self.icon_size.unwrap_or(theme::ICON_SM))
                .text_color(theme::solid(theme::MUTED_STRONG))
                .into_any_element()
        });

        let button = div()
            .id(self.id)
            .test_support()
            .track_focus(&focus_handle)
            .role(Role::Button)
            .aria_label(label)
            .size(theme::ICON_BUTTON_SIZE)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_MD)
            .when(disabled, |button| {
                button.opacity(theme::DISABLED_OPACITY).cursor_default()
            })
            .when(!disabled, |button| {
                button
                    .cursor_pointer()
                    .tab_index(0)
                    .focus_visible(|style| style.shadow(theme::focus_ring()))
                    .when_some(on_click, |button, on_click| {
                        button.on_click(move |event, window, cx| on_click(event, window, cx))
                    })
            })
            .child(
                // 可见底收进 2px：外圈仍是 32×32 的命中区。
                div()
                    .size(theme::ICON_BUTTON_SIZE)
                    .rounded(theme::RADIUS_MD)
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!disabled, |inner| inner.hover(move |style| style.bg(hover)))
                    .child(content),
            );

        match self.tooltip {
            Some(text) => button
                .tooltip(move |window, cx| {
                    Tooltip::new(text.clone())
                        .text_size(px(crate::components::icons::TOOLTIP_TEXT_SIZE))
                        .build(window, cx)
                })
                .into_any_element(),
            None => button.into_any_element(),
        }
    }
}

/// 图标按钮的统一入口；调用方继续链式设置图标、提示与点击。
pub fn icon(id: impl Into<ElementId>) -> IconButton {
    IconButton::new(id)
}
