//! 连接过程中的问答：主机密钥确认与认证输入。
//!
//! 登录在工作线程上进行，问题经 [`SshPrompts`] 发回界面；这里负责弹窗，
//! 答案通过 [`TerminalView::reply_to_prompt`] 送回。

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, WindowExt as _, h_flex,
    input::{InputContentType, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{button, dialog, field};
use crate::connection::{
    AuthenticationPrompt, ConnectionPrompt, ConnectionPromptKind, ConnectionPromptReply,
    ConnectionSecret,
};
use crate::theme;

/// 把弹窗里的答案送回提问方：终端的传输线程，或 SFTP 的会话线程。
pub type PromptResponder = std::sync::Arc<dyn Fn(u64, ConnectionPromptReply, &mut App) + 'static>;

/// 按问题的种类交给对应的弹窗。
pub fn open(
    prompt: ConnectionPrompt,
    responder: PromptResponder,
    workspace: Entity<crate::workspace::Workspace>,
    modal_alpha: f32,
    window: &mut Window,
    cx: &mut App,
) {
    match prompt.kind().clone() {
        ConnectionPromptKind::UnknownHost(question) => {
            open_host_key(
                HostKeyRequest {
                    request_id: prompt.request_id,
                    title: "确认主机密钥",
                    question: HostKeyQuestion {
                        host: question.host,
                        port: question.port,
                        algorithm: question.algorithm,
                        old_fingerprints: Vec::new(),
                        fingerprint: question.fingerprint,
                    },
                },
                responder.clone(),
                workspace,
                modal_alpha,
                window,
                cx,
            );
        }
        ConnectionPromptKind::HostKeyChanged(question) => {
            open_host_key(
                HostKeyRequest {
                    request_id: prompt.request_id,
                    title: "主机密钥已变化",
                    question: HostKeyQuestion {
                        host: question.host,
                        port: question.port,
                        algorithm: question.algorithm,
                        old_fingerprints: question.old_fingerprints,
                        fingerprint: question.fingerprint,
                    },
                },
                responder.clone(),
                workspace,
                modal_alpha,
                window,
                cx,
            );
        }
        ConnectionPromptKind::Authentication(question) => {
            open_authentication(
                prompt.request_id,
                question,
                responder,
                modal_alpha,
                window,
                cx,
            );
        }
    }
}

/// 主机密钥弹窗要展示的内容。
struct HostKeyQuestion {
    host: String,
    port: u16,
    algorithm: String,
    old_fingerprints: Vec<String>,
    fingerprint: String,
}

impl HostKeyQuestion {
    fn changed(&self) -> bool {
        !self.old_fingerprints.is_empty()
    }

    fn body(&self) -> String {
        if self.changed() {
            format!(
                "{}:{} 的主机密钥与记录不符。可能是服务器换了密钥，也可能是有人在中间。",
                self.host, self.port
            )
        } else {
            format!(
                "首次连接到 {}:{}。请确认下面的指纹与服务器管理员提供的一致。",
                self.host, self.port
            )
        }
    }
}

/// 一次主机密钥问答的全部内容。
struct HostKeyRequest {
    request_id: u64,
    title: &'static str,
    question: HostKeyQuestion,
}

/// 信任主机密钥，或者取消这次连接。
fn open_host_key(
    request: HostKeyRequest,
    responder: PromptResponder,
    workspace: Entity<crate::workspace::Workspace>,
    modal_alpha: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let HostKeyRequest {
        request_id,
        title,
        question,
    } = request;
    window.open_dialog(cx, move |dialog, window, cx| {
        let responder_cancel = responder.clone();
        let responder_ok = responder.clone();
        let responder_enter = responder.clone();
        // 比普通弹窗略宽：完整指纹与图标按钮要放得下一行。
        dialog::surface(dialog, px(500.), modal_alpha)
            .title(dialog::title(title))
            .margin_top(window.viewport_size().height * 0.24)
            .overlay_closable(false)
            .child(dialog::caption(
                "首次连接或服务器换过密钥时，需要确认指纹。",
            ))
            .child(dialog::divider())
            .child(
                div()
                    .mt(px(12.))
                    .text_size(theme::TYPE_BODY)
                    .line_height(px(20.))
                    .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                    .child(question.body()),
            )
            .child(fingerprint_box(&question, workspace.clone(), cx))
            .child(
                div()
                    .mt(px(10.))
                    .text_size(theme::TYPE_META)
                    .line_height(px(19.))
                    .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                    .child("信任后，该密钥将被记录。若之后指纹发生变化，连接会被阻止。"),
            )
            .footer(
                dialog::footer()
                    .child(button::form_secondary("cancel-host-key", "取消").on_click(
                        move |_, window, cx| {
                            responder_cancel(request_id, ConnectionPromptReply::Cancel, cx);
                            window.close_dialog(cx);
                        },
                    ))
                    .child(button::form_primary("trust", "信任并连接").on_click(
                        move |_, window, cx| {
                            responder_ok(request_id, ConnectionPromptReply::TrustAndSave, cx);
                            window.close_dialog(cx);
                        },
                    )),
            )
            .on_ok(move |_, window, cx| {
                responder_enter(request_id, ConnectionPromptReply::TrustAndSave, cx);
                window.close_dialog(cx);
                true
            })
    });
}

/// 指纹块：算法、按 4 字符分组的指纹、复制按钮。
///
/// 分组只影响显示；复制出去的是原始 `SHA256:...`，方便直接粘到校验工具或
/// 发给管理员比对。
fn fingerprint_box(
    question: &HostKeyQuestion,
    workspace: Entity<crate::workspace::Workspace>,
    cx: &App,
) -> AnyElement {
    let mut block = v_flex()
        .mt(px(8.))
        .gap(px(5.))
        .pl(px(12.))
        .pr(px(6.))
        .py(px(10.))
        .rounded(theme::RADIUS_MD)
        .border_1()
        .border_color(theme::line())
        .bg(theme::colored(theme::BASE_RGB, 0.6));

    if question.changed() {
        block = block.child(
            div()
                .text_size(theme::TYPE_MICRO)
                .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                .child("旧指纹"),
        );
        for fingerprint in &question.old_fingerprints {
            block = block.child(
                div()
                    .font_family(cx.theme().font_family.clone())
                    .text_size(theme::TYPE_META)
                    .line_height(px(19.))
                    .text_color(theme::solid(theme::OVERLAY_TEXT))
                    .child(crate::connection::grouped_fingerprint(fingerprint)),
            );
        }
        block = block.child(
            div()
                .mt(px(4.))
                .text_size(theme::TYPE_MICRO)
                .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                .child("新指纹"),
        );
    }

    let raw = question.fingerprint.clone();
    block
        .child(
            div()
                .font_family(cx.theme().font_family.clone())
                .text_size(theme::TYPE_MICRO)
                .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                .child(question.algorithm.clone()),
        )
        // 指纹与复制按钮同一行：弹窗宽度按完整指纹预留，不换行。
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap(px(2.))
                .child(
                    div()
                        .id("fingerprint-value")
                        .test_support()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .font_family(cx.theme().font_family.clone())
                        .text_size(theme::TYPE_META)
                        .line_height(px(20.))
                        .text_color(theme::solid(theme::OVERLAY_TEXT))
                        .child(crate::connection::grouped_fingerprint(
                            &question.fingerprint,
                        )),
                )
                .child(crate::components::icons::with_tooltip(
                    "copy-fingerprint-tip",
                    "复制指纹",
                    button::icon("copy-fingerprint")
                        .accessibility_label("复制指纹")
                        .child(
                            Icon::new(IconName::Copy)
                                .size(theme::ICON_SM)
                                .text_color(theme::solid(theme::OVERLAY_SUBTLE)),
                        )
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(raw.clone()));
                            workspace
                                .update(cx, |workspace, cx| workspace.show_toast("已复制指纹", cx));
                        }),
                )),
        )
        .into_any_element()
}

/// 一个认证问题一条输入框。
pub struct AuthenticationForm {
    responder: PromptResponder,
    request_id: u64,
    prompt: AuthenticationPrompt,
    inputs: Vec<Entity<InputState>>,
    error: bool,
    /// 已经回答过一次，不再受理第二次。
    answered: bool,
}

fn open_authentication(
    request_id: u64,
    prompt: AuthenticationPrompt,
    responder: PromptResponder,
    modal_alpha: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let inputs: Vec<Entity<InputState>> = prompt
        .prompts
        .iter()
        .map(|question| {
            let placeholder = question.text.clone();
            let masked = !question.echo;
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(masked)
                    .placeholder(placeholder)
            })
        })
        .collect();
    let form = cx.new(|_| AuthenticationForm {
        responder,
        request_id,
        prompt: prompt.clone(),
        inputs,
        error: false,
        answered: false,
    });

    let title = if prompt.title.is_empty() {
        "输入密码".to_string()
    } else {
        prompt.title.clone()
    };
    let dialog_form = form.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let form_for_ok = dialog_form.clone();
        let form_cancel = dialog_form.clone();
        dialog::surface(dialog, theme::DIALOG_WIDTH, modal_alpha)
            .title(dialog::title(title.clone()))
            .margin_top(window.viewport_size().height * 0.32)
            .overlay_closable(false)
            .child(dialog_form.clone())
            .footer(
                dialog::footer()
                    .child(button::form_secondary("cancel-password", "取消").on_click(
                        move |_, window, cx| {
                            form_cancel.update(cx, |form, cx| form.cancel(cx));
                            window.close_dialog(cx);
                        },
                    ))
                    .child({
                        let form = dialog_form.clone();
                        button::form_primary("connect", "连接").on_click(move |_, window, cx| {
                            let answered = form.update(cx, |form, cx| form.commit(cx));
                            if answered {
                                window.close_dialog(cx);
                            }
                        })
                    }),
            )
            .on_ok(move |_, window, cx| {
                let answered = form_for_ok.update(cx, |form, cx| form.commit(cx));
                if answered {
                    window.close_dialog(cx);
                }
                answered
            })
    });
    form.update(cx, |form, cx| form.focus_first(window, cx));
}

impl AuthenticationForm {
    fn focus_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.inputs.first() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        (self.responder)(self.request_id, ConnectionPromptReply::Cancel, cx);
    }

    fn commit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.answered {
            return false;
        }
        let answers: Vec<ConnectionSecret> = self
            .inputs
            .iter()
            .map(|input| ConnectionSecret::new(input.read(cx).value().to_string()))
            .collect();
        // 遮起来的字段（密码、口令）不能空着提交。
        if self
            .prompt
            .prompts
            .iter()
            .zip(&answers)
            .any(|(question, answer)| !question.echo && answer.expose().is_empty())
        {
            self.error = true;
            cx.notify();
            return false;
        }
        self.answered = true;
        (self.responder)(self.request_id, ConnectionPromptReply::Answers(answers), cx);
        true
    }
}

impl Render for AuthenticationForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut fields = v_flex().gap(px(12.));
        for (index, question) in self.prompt.prompts.iter().enumerate() {
            let Some(input) = self.inputs.get(index) else {
                continue;
            };
            let label = question.text.clone();
            let input = if question.echo {
                field::text_field_with(
                    ElementId::Name(format!("prompt-input-shell-{index}").into()),
                    input,
                    window,
                    cx,
                    |input| input.aria_label(label.clone()).w_full(),
                )
                .into_any_element()
            } else {
                field::text_field_with(
                    ElementId::Name(format!("prompt-input-shell-{index}").into()),
                    input,
                    window,
                    cx,
                    |input| {
                        input
                            .aria_label(label.clone())
                            .content_type(InputContentType::Password)
                            .w_full()
                    },
                )
                .into_any_element()
            };
            fields = fields.child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(theme::TYPE_LABEL)
                            .text_color(theme::solid(theme::OVERLAY_TEXT))
                            .child(question.text.clone()),
                    )
                    .child(input),
            );
        }

        v_flex()
            .gap(px(14.))
            .child(
                div()
                    .font_family(cx.theme().font_family.clone())
                    .text_size(theme::TYPE_MICRO)
                    .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                    .child(format!(
                        "{}@{}:{}",
                        self.prompt.username, self.prompt.host, self.prompt.port
                    )),
            )
            .child(div().h(px(1.)).w_full().bg(theme::line()))
            .when(!self.prompt.instructions.is_empty(), |form| {
                form.child(
                    div()
                        .text_size(theme::TYPE_BODY)
                        .line_height(px(20.))
                        .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                        .child(self.prompt.instructions.clone()),
                )
            })
            .child(fields)
            .when(self.error, |form| {
                form.child(
                    div()
                        .text_size(theme::TYPE_MICRO)
                        .text_color(theme::solid(theme::LOVE))
                        .child("请填写密码"),
                )
            })
            .child(
                h_flex()
                    .gap(px(6.))
                    .text_size(theme::TYPE_META)
                    .line_height(px(19.))
                    .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                    .child("密码只保存在本次会话中。"),
            )
    }
}
