//! 左侧主机列表。平铺显示，没有分组与搜索（产品范围内的明确取舍）。

use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, scroll::ScrollableElement as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::NewHost;
use crate::host::{Host, HostId};
use crate::settings::appearance_tokens::{AppearanceTokens, UiTokens};
use crate::theme;

use super::Workspace;
use gpui_kit::assets::IconName;

/// 正在拖动的主机。拖动期间跟着指针走的预览读它画名字。
#[derive(Clone, Debug)]
struct HostDrag {
    id: HostId,
    name: SharedString,
}

/// 拖动时跟在指针边上的小块，与 SFTP 的拖拽幽灵同一套外观。
struct HostDragPreview {
    name: SharedString,
}

impl Render for HostDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = UiTokens::current(cx).overlay;
        h_flex()
            .gap(theme::ICON_TEXT_GAP)
            .px(px(10.))
            .py(px(6.))
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(theme::colored(theme::FOAM, 0.32))
            .bg(theme::overlay(overlay))
            .shadow_lg()
            .text_size(theme::TYPE_META)
            .text_color(theme::solid(theme::TEXT))
            .child(
                Icon::new(IconName::Server)
                    .size(theme::ICON_SM)
                    .text_color(theme::solid(theme::FOAM)),
            )
            .child(self.name.clone())
    }
}

impl Workspace {
    pub(super) fn render_host_panel(
        &self,
        tokens: &AppearanceTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let target = self.focus_handle.clone();
        let current = self.current_host_id();
        let hosts: Vec<Host> = self.visible_hosts(cx).cloned().collect();
        let rows: Vec<AnyElement> = hosts
            .iter()
            .enumerate()
            .map(|(index, host)| {
                self.render_host_row(host, index, current, cx)
                    .into_any_element()
            })
            .collect();
        let collapsed = self.host_panel_collapsed;

        let toggle = crate::components::icons::with_tooltip(
            "host-panel-toggle-tip",
            if collapsed {
                "展开主机列表"
            } else {
                "收起主机列表"
            },
            crate::components::button::icon("host-panel-toggle")
                .accessibility_label(if collapsed {
                    "展开主机列表"
                } else {
                    "收起主机列表"
                })
                // 与传输队列的收起按钮同一套箭头样式：收起时指回展开方向。
                .icon(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronLeft
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.host_panel_collapsed = !this.host_panel_collapsed;
                    cx.notify();
                })),
        )
        .into_any_element();

        // 收起后保留一条与右侧栏同宽的窄栏，切换按钮始终在原位，
        // 不需要在别处再放第二个「展开」入口。
        let header: AnyElement = if collapsed {
            h_flex()
                .h(theme::BAR_HEIGHT)
                .w_full()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .border_b_1()
                .border_color(theme::line())
                .child(toggle)
                .into_any_element()
        } else {
            let actions = h_flex()
                .items_center()
                .child(crate::components::icons::with_tooltip(
                    "new-host-tip",
                    "新建主机（Ctrl+N）",
                    crate::components::button::icon("new-host")
                        .accessibility_label("新建主机")
                        .icon(IconName::Plus)
                        .on_click(move |_, window, cx| {
                            target.dispatch_action(&NewHost, window, cx);
                        }),
                ))
                .child(toggle)
                .into_any_element();
            crate::components::layout::panel_header("主机", Some(actions)).into_any_element()
        };

        v_flex()
            .w(if collapsed {
                theme::SIDEBAR_COLLAPSED_WIDTH
            } else {
                theme::SIDEBAR_WIDTH
            })
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .border_r_1()
            .border_color(theme::line())
            .bg(theme::glass(tokens.glass))
            .child(header)
            .when(!collapsed, |panel| {
                panel.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .id("host-list-scroll")
                        .overflow_y_scrollbar()
                        .p(px(10.))
                        .child(if rows.is_empty() {
                            crate::components::layout::empty_state(
                                "还没有主机，点击右上角 + 新建。",
                            )
                            .into_any_element()
                        } else {
                            v_flex().gap(px(2.)).children(rows).into_any_element()
                        }),
                )
            })
    }

    fn render_host_row(
        &self,
        host: &Host,
        index: usize,
        current: Option<HostId>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = host.id;
        let selected = self.selected_host == Some(id);
        let is_current = current == Some(id);
        let connected = self.host_connected(id);
        let dot_color = if connected {
            theme::solid(theme::FOAM)
        } else {
            theme::solid(theme::SUBTLE)
        };
        let group = format!("host-{id}");
        let row_bg = if is_current {
            theme::colored(theme::FOAM, 0.08)
        } else if selected {
            theme::hover_subtle()
        } else {
            Hsla::default()
        };
        let row_border = if is_current {
            theme::colored(theme::FOAM, 0.20)
        } else if selected {
            theme::line()
        } else {
            Hsla::default()
        };

        let menu_group = group.clone();
        div()
            .id(ElementId::Name(format!("host-{id}").into()))
            .test_support()
            .tab_index(0)
            .aria_label(format!(
                "{}，{}，{}",
                host.name,
                host.endpoint(),
                if connected { "已连接" } else { "未连接" }
            ))
            .group(group)
            .relative()
            .flex()
            .items_center()
            .gap(theme::ICON_TEXT_GAP)
            .w_full()
            .py(px(7.))
            .pl(px(10.))
            .pr(px(8.))
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(row_border)
            .bg(row_bg)
            .cursor_pointer()
            .focus_visible(|style| {
                style
                    .border_color(theme::colored(theme::FOAM, 0.55))
                    .bg(theme::hover_tint())
            })
            .hover(|style| style.border_color(theme::line()).bg(theme::hover_subtle()))
            .when(is_current, |row| {
                row.child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(px(2.))
                        .rounded(theme::RADIUS_XS)
                        .bg(theme::solid(theme::FOAM)),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_host = Some(id);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" | "space" | " " => {
                        cx.stop_propagation();
                        this.selected_host = Some(id);
                        this.connect_host(id, window, cx);
                    }
                    "f2" => {
                        cx.stop_propagation();
                        this.selected_host = Some(id);
                        crate::dialogs::host::open(this, Some(id), window, cx);
                    }
                    "delete" => {
                        cx.stop_propagation();
                        this.selected_host = Some(id);
                        this.open_delete_host(id, window, cx);
                    }
                    _ => {}
                }
            }))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() == 2 {
                    this.connect_host(id, window, cx);
                }
            }))
            // 拖动超过阈值才开始，单击选中与双击连接不受影响。
            .on_drag(
                HostDrag {
                    id,
                    name: host.display_name().to_string().into(),
                },
                |drag: &HostDrag, _, _, cx| {
                    cx.new(|_| HostDragPreview {
                        name: drag.name.clone(),
                    })
                },
            )
            // 松手落在哪一行，就插到那一行的位置。
            .drag_over::<HostDrag>(|style, _, _, _| style.border_color(theme::solid(theme::FOAM)))
            .on_drop(cx.listener(move |this, drag: &HostDrag, _, cx| {
                this.hosts
                    .update(cx, |store, cx| store.move_host(drag.id, index, cx));
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_host_menu(id, event.position.x, event.position.y, window, cx);
                }),
            )
            .child(
                div()
                    .size(px(6.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(dot_color)
                    .when(connected, |dot| {
                        dot.shadow(theme::ring(theme::solid(theme::FOAM)))
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(theme::TYPE_SECTION)
                            .font_weight(FontWeight::SEMIBOLD)
                            // 名字里可以有 emoji（国旗）：emoji 段换 Twemoji 画。
                            .child(crate::components::emoji::with_flags(
                                host.name.clone(),
                                theme::TYPE_SECTION,
                            )),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_META)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child(host.endpoint()),
                    ),
            )
            .child(
                // 视觉 22×22，命中区 32×32（DESIGN.md 下限）：外层撑大、
                // 用负边距收回视觉占位，内层画出实际方块。
                div()
                    .id(("host-menu", id.0))
                    .aria_label("主机菜单")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(theme::ICON_BUTTON_SIZE)
                    .flex_shrink_0()
                    .rounded(theme::RADIUS_MD)
                    .opacity(if selected || is_current { 1.0 } else { 0.0 })
                    .group_hover(menu_group, |style| style.opacity(1.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_host_menu(id, event.position.x, event.position.y, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .size(theme::ICON_BUTTON_SIZE)
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(theme::RADIUS_MD)
                            .hover(|style| style.bg(theme::hover_tint()))
                            .child(
                                gpui_kit::component::Icon::new(IconName::Ellipsis)
                                    .size(theme::ICON_SM)
                                    .text_color(theme::solid(theme::SUBTLE)),
                            ),
                    ),
            )
    }
}
