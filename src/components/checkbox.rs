//! 统一复选框：16×16 视觉、32px 命中区、可用 Space 切换。

use std::rc::Rc;

use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use gpui_kit::assets::IconName;

use crate::theme;

type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct AppCheckbox {
    id: ElementId,
    checked: bool,
    disabled: bool,
    label: Option<SharedString>,
    on_change: Option<ChangeHandler>,
}

impl AppCheckbox {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            checked: false,
            disabled: false,
            label: None,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_change(mut self, on_change: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(on_change));
        self
    }
}

impl RenderOnce for AppCheckbox {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let checked = self.checked;
        let disabled = self.disabled;
        let label = self.label;
        let on_change = self.on_change;
        let click_change = on_change.clone();
        let key_change = on_change;

        div()
            .id(self.id)
            .test_support()
            .tab_index(0)
            .role(gpui_kit::Role::CheckBox)
            .aria_label({
                let state = if checked { "已选中" } else { "未选中" };
                label
                    .clone()
                    .map(|label| format!("{label}，{state}"))
                    .unwrap_or_else(|| state.to_string())
            })
            .flex()
            .items_center()
            .gap(px(8.))
            .min_h(theme::MIN_HIT_TARGET)
            .cursor_pointer()
            .opacity(if disabled {
                theme::DISABLED_OPACITY
            } else {
                1.0
            })
            .focus_visible(|style| {
                style
                    .border_1()
                    .border_color(theme::solid(theme::FOAM))
                    .rounded(theme::RADIUS_XS)
            })
            .when(!disabled, |this| {
                this.on_click(move |_, window, cx| {
                    if let Some(on_change) = &click_change {
                        on_change(&!checked, window, cx);
                    }
                    cx.stop_propagation();
                })
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space" | " ") {
                        cx.stop_propagation();
                        if let Some(on_change) = &key_change {
                            on_change(&!checked, window, cx);
                        }
                    }
                })
            })
            .child(
                div()
                    .size(px(16.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme::RADIUS_XS)
                    .border_1()
                    .border_color(if checked {
                        theme::solid(theme::FOAM)
                    } else {
                        theme::line()
                    })
                    .bg(if checked {
                        theme::solid(theme::FOAM)
                    } else {
                        Hsla::default()
                    })
                    .when(checked, |box_| {
                        box_.child(
                            Icon::new(IconName::Check)
                                .size(theme::ICON_XS)
                                .text_color(theme::solid(theme::BASE_RGB)),
                        )
                    }),
            )
            .when_some(label, |this, label| {
                this.child(
                    div()
                        .text_size(theme::TYPE_META)
                        .text_color(theme::solid(theme::MUTED_STRONG))
                        .child(label),
                )
            })
    }
}
