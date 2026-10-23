//! 标签栏：主机标签与设置单例标签。

use gpui_kit::component::{Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{Tab, TabId};
use crate::settings::appearance_tokens::AppearanceTokens;
use crate::theme;

use super::Workspace;
use gpui_kit::assets::IconName;

impl Workspace {
    pub(super) fn render_tab_bar(
        &self,
        tokens: &AppearanceTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tabs: Vec<AnyElement> = self
            .tabs
            .iter()
            .map(|tab| self.render_tab(tab, tokens, cx).into_any_element())
            .collect();

        h_flex()
            .h(theme::BAR_HEIGHT)
            .flex_shrink_0()
            .w_full()
            .items_stretch()
            .border_b_1()
            .border_color(theme::line())
            .bg(theme::workspace_glass(tokens.tab_bar))
            .child(
                div()
                    .id("tabs-scroll")
                    .flex()
                    .min_w_0()
                    .overflow_x_scroll()
                    .children(tabs),
            )
    }

    fn render_tab(
        &self,
        tab: &Tab,
        tokens: &AppearanceTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = tab.id;
        let active = self.active_tab == Some(id);
        let title = self.tab_label(tab);
        let connected = tab.connected;
        let group = format!("tab-{id}");
        let close_group = group.clone();

        div()
            .id(ElementId::Name(format!("tab-{id}").into()))
            .test_support()
            .aria_label(format!(
                "{title}，{}",
                if connected { "已连接" } else { "已断开" }
            ))
            .group(group)
            .relative()
            .flex()
            .items_center()
            .gap(theme::ICON_TEXT_GAP)
            .w(theme::TAB_WIDTH)
            .h_full()
            .pl(px(11.))
            .pr(px(0.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(theme::line_subtle())
            .text_size(theme::TYPE_META)
            .text_color(theme::solid(if active {
                theme::TEXT
            } else {
                theme::SUBTLE
            }))
            .tab_index(0)
            .cursor_pointer()
            .focus_visible(|style| {
                style
                    .text_color(theme::solid(theme::TEXT))
                    .bg(theme::workspace_glass(tokens.tab_focus))
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" | "space" | " " => {
                        cx.stop_propagation();
                        this.activate_tab(id, window, cx);
                    }
                    "delete" | "backspace" => {
                        cx.stop_propagation();
                        this.close_tab(id, cx);
                    }
                    _ => {}
                }
            }))
            .when(active, |tab| {
                tab.bg(theme::workspace_glass(tokens.tab_active)).child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .h(px(2.))
                        .bg(theme::solid(theme::FOAM)),
                )
            })
            .hover(|style| style.bg(theme::hover_subtle()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_tab(id, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.activate_tab(id, window, cx);
                }),
            )
            .child(match tab.kind {
                crate::model::TabKind::Settings => Icon::new(IconName::Cog)
                    .size(theme::ICON_SM)
                    .text_color(theme::solid(if active {
                        theme::FOAM
                    } else {
                        theme::SUBTLE
                    }))
                    .into_any_element(),
                crate::model::TabKind::Host(_) => div()
                    .size(px(5.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(if connected {
                        theme::solid(theme::FOAM)
                    } else {
                        theme::solid(theme::GOLD)
                    })
                    .into_any_element(),
                crate::model::TabKind::Sftp(_) => Icon::new(IconName::FolderOpen)
                    .size(theme::ICON_SM)
                    .text_color(theme::solid(if !connected {
                        theme::GOLD
                    } else if active {
                        theme::FOAM
                    } else {
                        theme::SUBTLE
                    }))
                    .into_any_element(),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .line_height(px(16.))
                    // 主机名里可能有 emoji（国旗），单独用 Twemoji 画。
                    .child(crate::components::emoji::with_flags(
                        title.clone(),
                        theme::TYPE_META,
                    )),
            )
            .child(tab_close_button(id, active, close_group, title.into(), cx))
    }
}

fn tab_close_button(
    id: TabId,
    active: bool,
    group: String,
    title: SharedString,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    // 视觉 18×18，命中区 32×32：外层撑到 32 并用负边距收回视觉占位，
    // 内层只负责画出 18×18 的方块。DESIGN.md：命中区不得小于 32px。
    div()
        .id(ElementId::Name(format!("tab-close-{id}").into()))
        .test_support()
        .aria_label(format!("关闭 {title}"))
        .flex()
        .items_center()
        .justify_center()
        .size(theme::ICON_BUTTON_SIZE)
        .flex_shrink_0()
        .rounded(theme::RADIUS_MD)
        .tab_index(0)
        .opacity(if active { 1.0 } else { 0.0 })
        .group_hover(group, |style| style.opacity(1.0))
        .cursor_pointer()
        .focus_visible(|style| style.opacity(1.0))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.close_tab(id, cx);
            }),
        )
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space" | " ") {
                cx.stop_propagation();
                this.close_tab(id, cx);
            }
        }))
        .child(
            div()
                .size(theme::ICON_BUTTON_SIZE)
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS_MD)
                .hover(|style| style.bg(theme::hover_tint()))
                .child(
                    Icon::new(IconName::X)
                        .size(theme::ICON_SM)
                        .text_color(theme::solid(theme::SUBTLE)),
                ),
        )
}
