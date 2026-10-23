//! 右侧命令片段面板：增删改，点击插入当前终端。

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{button, dialog, field};
use crate::settings::appearance_tokens::AppearanceTokens;
use crate::snippets::Snippet;
use crate::theme;

use super::Workspace;

impl Workspace {
    /// 右侧命令片段面板。没开就不占位；开着时任何标签都显示——看片段、增删改
    /// 都不需要终端，只有「点到终端里」才需要，那一步会自己提示。
    pub(super) fn render_snippets_panel(
        &self,
        tokens: &AppearanceTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.snippets_open {
            return div().into_any_element();
        }

        let mut list: Vec<AnyElement> = Vec::new();
        if self.snippets.snippets().is_empty() {
            list.push(
                div()
                    .p(px(12.))
                    .text_size(theme::TYPE_META)
                    .text_color(theme::solid(theme::SUBTLE))
                    .child("还没有代码片段，点击右上角 + 新建。")
                    .into_any_element(),
            );
        }
        for snippet in self.snippets.snippets() {
            list.push(self.render_snippet_row(snippet, cx).into_any_element());
        }

        v_flex()
            .id("snippets-panel")
            .test_support()
            .w(px(300.))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .border_l_1()
            .border_color(theme::line())
            .bg(theme::glass(tokens.glass))
            .child(
                h_flex()
                    .h(theme::BAR_HEIGHT)
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .pl(px(14.))
                    .pr(px(4.))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(
                        h_flex()
                            .items_center()
                            .gap(theme::ICON_TEXT_GAP)
                            .child(
                                Icon::new(IconName::ListChecks)
                                    .size(theme::ICON_SM)
                                    .text_color(theme::solid(theme::MUTED_STRONG)),
                            )
                            .child(
                                div()
                                    .text_size(theme::TYPE_LABEL)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::solid(theme::MUTED_STRONG))
                                    .child("命令片段"),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(2.))
                            .child(
                                button::icon("snippet-add")
                                    .icon(IconName::Plus)
                                    .accessibility_label("新建片段")
                                    .tooltip("新建片段")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_snippet_dialog(None, window, cx);
                                    })),
                            )
                            // 与左侧主机列表的收起按钮同一套箭头图标。
                            .child(
                                button::icon("snippets-toggle")
                                    .icon(IconName::ChevronRight)
                                    .accessibility_label("收起命令片段")
                                    .tooltip("收起命令片段")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_snippets(cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("snippets-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px(px(6.))
                    .pb(px(8.))
                    .children(list),
            )
            .into_any_element()
    }

    fn render_snippet_row(&self, snippet: &Snippet, cx: &mut Context<Self>) -> impl IntoElement {
        let id = snippet.id;
        let group = format!("snippet-{id}");
        let hover_group = group.clone();
        let edit_group = group;
        div()
            .id(ElementId::Name(format!("snippet-row-{id}").into()))
            .test_support()
            .aria_label(format!("{}，{}", snippet.name, snippet.command))
            .group(hover_group)
            .mb(px(2.))
            .px(px(8.))
            .py(px(6.))
            .rounded(theme::RADIUS_MD)
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover_subtle()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.run_snippet(id, window, cx);
            }))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::TYPE_META)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::solid(theme::TEXT))
                                    .child(snippet.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::TYPE_MICRO)
                                    .text_color(theme::solid(theme::SUBTLE))
                                    .child(snippet.command.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap(px(2.))
                            .opacity(0.)
                            .group_hover(edit_group, |style| style.opacity(1.))
                            .child(
                                button::icon(("snippet-edit", id))
                                    .icon(IconName::Pencil)
                                    .accessibility_label("编辑片段")
                                    .tooltip("编辑片段")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.open_snippet_dialog(Some(id), window, cx);
                                    })),
                            )
                            .child(
                                button::icon(("snippet-delete", id))
                                    .icon(IconName::Trash)
                                    .accessibility_label("删除片段")
                                    .tooltip("删除片段")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.remove_snippet(id, cx);
                                    })),
                            ),
                    ),
            )
    }

    fn run_snippet(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snippet) = self.snippets.get(id).cloned() else {
            return;
        };
        let Some(terminal) = self.active_tab().and_then(|tab| tab.terminal.clone()) else {
            self.show_toast("命令片段只在终端标签中可用", cx);
            return;
        };
        let result = terminal.update(cx, |terminal, cx| {
            terminal.enter_command(&snippet.command, false, true, cx)
        });
        match result {
            Ok(()) => {
                terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
            }
            Err(error) => self.show_toast(error, cx),
        }
    }

    fn commit_snippet(
        &mut self,
        id: Option<u64>,
        name: String,
        command: String,
        cx: &mut Context<Self>,
    ) {
        let message = if id.is_some() {
            "片段已更新"
        } else {
            "片段已添加"
        };
        match self.snippets.upsert(id, name, command) {
            Ok(()) => self.show_toast(message, cx),
            Err(error) => self.show_toast(error, cx),
        }
        cx.notify();
    }

    fn remove_snippet(&mut self, id: u64, cx: &mut Context<Self>) {
        match self.snippets.remove(id) {
            Ok(()) => self.show_toast("片段已删除", cx),
            Err(error) => self.show_toast(error, cx),
        }
        cx.notify();
    }

    /// 打开或收起右侧片段面板。开面板本身不挑标签：当前不是终端也能先看看、
    /// 改改片段，真正插不进去只会在点片段时提示。
    pub(super) fn toggle_snippets(&mut self, cx: &mut Context<Self>) {
        self.snippets_open = !self.snippets_open;
        cx.notify();
    }

    fn open_snippet_dialog(
        &mut self,
        editing: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snippet = editing.and_then(|id| self.snippets.get(id)).cloned();
        let form = cx.new(|cx| SnippetForm::new(window, cx, snippet.clone()));
        let alpha = self.store.read(cx).appearance().modal_alpha_f32();
        let workspace = cx.entity();
        let editing_id = editing;
        window.open_dialog(cx, move |dialog, window, _cx| {
            let form_body = form.clone();
            let form_cancel = form.clone();
            let form_ok = form.clone();
            let workspace_ok = workspace.clone();

            dialog::surface(dialog, px(520.), alpha)
                .title(dialog::title(if editing_id.is_some() {
                    "编辑代码片段"
                } else {
                    "新建代码片段"
                }))
                .margin_top(window.viewport_size().height * 0.28)
                .overlay_closable(true)
                .child(form_body)
                .footer(
                    dialog::footer()
                        .child(button::form_secondary("snippet-cancel", "取消").on_click(
                            move |_, window, cx| {
                                let _ = &form_cancel;
                                window.close_dialog(cx);
                            },
                        ))
                        .child(button::form_primary("snippet-save", "保存").on_click(
                            move |_, window, cx| {
                                let name =
                                    form_ok.read(cx).name.read(cx).value().trim().to_string();
                                let command =
                                    form_ok.read(cx).command.read(cx).value().trim().to_string();
                                if name.is_empty() || command.is_empty() {
                                    form_ok.update(cx, |form, cx| {
                                        form.error = Some("名称和命令不能为空".into());
                                        cx.notify();
                                    });
                                    return;
                                }
                                workspace_ok.update(cx, |workspace, cx| {
                                    workspace.commit_snippet(editing_id, name, command, cx);
                                });
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }
}

struct SnippetForm {
    name: Entity<InputState>,
    command: Entity<TextareaState>,
    error: Option<SharedString>,
}

impl SnippetForm {
    fn new(window: &mut Window, cx: &mut Context<Self>, snippet: Option<Snippet>) -> Self {
        let name = snippet.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        let command = snippet
            .as_ref()
            .map(|s| s.command.clone())
            .unwrap_or_default();
        Self {
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(name)
                    .placeholder("片段名称")
            }),
            command: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .default_value(command)
                    .placeholder("要插入终端的命令，可多行")
            }),
            error: None,
        }
    }
}

impl Render for SnippetForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(px(10.))
            .child(field::text_field_with(
                "snippet-name",
                &self.name,
                window,
                cx,
                |input| input.aria_label("片段名称").w_full(),
            ))
            .child(field::text_area(
                "snippet-command",
                &self.command,
                window,
                cx,
                px(110.),
                "片段命令",
            ))
            .when_some(self.error.clone(), |form, error| {
                form.child(
                    div()
                        .text_size(theme::TYPE_META)
                        .text_color(theme::solid(theme::LOVE))
                        .child(error),
                )
            })
    }
}
