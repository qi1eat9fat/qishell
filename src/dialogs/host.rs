//! 新建/编辑主机对话框。

use std::path::Path;

use gpui_kit::component::{
    Disableable as _, WindowExt as _, h_flex,
    input::{InputContentType, InputState, TextareaState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{button, dialog, field};
use crate::host::{AuthKind, DEFAULT_USER, Host, HostDraft, HostId, SSH_PORT};
use crate::theme;
use crate::workspace::Workspace;

#[derive(Default)]
struct Errors {
    name: bool,
    address: bool,
    port: bool,
    key: bool,
}

pub struct HostForm {
    workspace: Entity<Workspace>,
    editing: Option<HostId>,
    name: Entity<InputState>,
    /// 名称原来开头的国旗（`🇭🇰`）。输入框里只画名字本身，保存时再接回前面。
    name_flags: String,
    address: Entity<InputState>,
    user: Entity<InputState>,
    port: Entity<InputState>,
    notes: Entity<TextareaState>,
    password: Entity<InputState>,
    key_path: Entity<InputState>,
    key_passphrase: Entity<InputState>,
    auth: AuthKind,
    errors: Errors,
    testing: bool,
    test_result: Option<(bool, String)>,
    /// 已经提交过一次，不再受理第二次。
    submitting: bool,
    /// 表单一改，「测试连接」的旧结果就不再可信，跟着清掉。
    _subscriptions: Vec<Subscription>,
}

impl HostForm {
    fn new(
        workspace: Entity<Workspace>,
        editing: Option<Host>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = editing.clone();
        // 名称输入框画不了国旗（文字走系统字体光栅化，区域指示符在 Windows
        // 上没有彩色轮廓），所以把开头的旗从文字里摘出来，另画成图片摆在
        // 输入框左侧；提交时再接回去。`flags` 记的就是摘出来的那一段。
        let (flags, name) = host
            .as_ref()
            .map(|host| crate::components::emoji::take_leading_flags(&host.name))
            .unwrap_or_default();
        let address = host
            .as_ref()
            .map(|host| host.address.clone())
            .unwrap_or_default();
        // 用户名与端口留空时按默认值处理，所以新建时不预填：输入框里只留
        // 灰色提示，用户真要用别的值再手填。
        let user = host
            .as_ref()
            .map(|host| host.username.clone())
            .unwrap_or_default();
        let port = host
            .as_ref()
            .map(|host| host.port.to_string())
            .unwrap_or_default();
        let notes = host
            .as_ref()
            .map(|host| host.notes.clone())
            .unwrap_or_default();
        // 库里存的是数据目录内的相对路径（拷走文件夹还能用），输入框里
        // 显示解析后的完整路径——相对路径没法照着去文件管理器里找。
        let key_path = host
            .as_ref()
            .and_then(|host| host.key_path.as_deref())
            .map(|path| {
                crate::app::paths::resolve(std::path::Path::new(path))
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_default();
        let key_passphrase = String::new();
        let auth = host
            .as_ref()
            .map(|host| host.auth)
            .unwrap_or(AuthKind::Password);

        let form = Self {
            workspace,
            editing: editing.map(|host| host.id),
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(name)
                    .placeholder("例如：staging api")
            }),
            name_flags: flags,
            address: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(address)
                    .placeholder("10.0.0.12")
            }),
            user: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(user)
                    .placeholder(format!("留空默认 {}", crate::host::DEFAULT_USER))
            }),
            port: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(port)
                    .placeholder(format!("留空默认 {}", crate::host::SSH_PORT))
            }),
            notes: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .default_value(notes)
                    .placeholder("例如：生产环境，变更需审批")
            }),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(if host.is_some() {
                        "留空保持不变，或连接时询问"
                    } else {
                        "留空则在连接时询问"
                    })
            }),
            key_path: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(key_path)
                    .placeholder("未选择私钥文件")
            }),
            key_passphrase: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .default_value(key_passphrase)
                    .placeholder("留空表示私钥无口令")
            }),
            auth,
            errors: Errors::default(),
            testing: false,
            test_result: None,
            submitting: false,
            _subscriptions: Vec::new(),
        };
        // InputState 在创建时会注册自己的 keymap（Tab → 缩进），晚于
        // app::bind_keys；这里在表单创建后再覆盖一次，让 Tab 回到焦点顺序。
        cx.bind_keys([
            KeyBinding::new("tab", crate::actions::FocusNext, Some("Input")),
            KeyBinding::new("shift-tab", crate::actions::FocusPrev, Some("Input")),
        ]);
        form
    }

    /// 表单有任何输入就清掉上一次的「测试连接」结果：改动之后那个结论
    /// 已经不成立了。
    fn watch_for_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fields = [
            self.name.clone(),
            self.address.clone(),
            self.user.clone(),
            self.port.clone(),
            self.password.clone(),
            self.key_path.clone(),
            self.key_passphrase.clone(),
        ];
        for field in fields {
            self._subscriptions.push(cx.subscribe_in(
                &field,
                window,
                |this, _, event: &gpui_kit::component::input::InputEvent, _, cx| {
                    if matches!(event, gpui_kit::component::input::InputEvent::Change)
                        && this.test_result.is_some()
                    {
                        this.test_result = None;
                        cx.notify();
                    }
                },
            ));
        }
    }

    fn commit(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        // 一次性守卫：回车和按钮可能在同一帧里都触发提交。
        if self.submitting {
            return false;
        }
        self.submitting = true;
        // 名字清空就是没写名字，开头的旗不单独留下来凑数。
        let name_text = self.name.read(cx).value().trim().to_string();
        let name = if name_text.is_empty() {
            String::new()
        } else {
            format!("{}{}", self.name_flags, name_text)
        };
        let address = self.address.read(cx).value().trim().to_string();
        let user = self.user.read(cx).value().trim().to_string();
        let port = match self.port.read(cx).value().trim() {
            // 空串走默认端口；其它内容必须能解析成 1–65535。
            "" => Some(SSH_PORT),
            value => value.parse::<u16>().ok(),
        };
        let notes = self.notes.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        let key_path = self.key_path.read(cx).value().trim().to_string();
        let key_passphrase = self.key_passphrase.read(cx).value().to_string();

        // 用户名留空就是「用默认」：输入框里只画提示文字，不当成错误。
        let user = if user.is_empty() {
            DEFAULT_USER.to_string()
        } else {
            user
        };

        self.errors = Errors {
            name: name.is_empty(),
            address: address.is_empty(),
            port: !matches!(port, Some(1..=65535)),
            key: self.auth == AuthKind::Key && key_path.is_empty(),
        };
        if self.errors.name || self.errors.address || self.errors.port || self.errors.key {
            self.submitting = false;
            cx.notify();
            return false;
        }
        let Some(port) = port else {
            self.submitting = false;
            return false;
        };

        // 选的私钥在数据目录外面（比如 `~/.ssh`）就先收一份进来：只有放进
        // 这个文件夹，拷贝目录才带得走它。收不进来（读不了、目录不可写）就
        // 用用户给的原路径，至少这台机器上还能连。
        let key_path = if self.auth == AuthKind::Key {
            Some(Path::new(&key_path))
                .and_then(|path| {
                    crate::ssh::vault::adopt_key(&crate::app::paths::keys_dir(), path)
                        .or(Some(path.to_path_buf()))
                })
                .map(|path| crate::app::paths::stored(&path))
        } else {
            None
        };
        let draft = HostDraft {
            name,
            address,
            port,
            username: user,
            auth: self.auth,
            key_path,
            notes,
        };
        let draft = match draft.validated() {
            Ok(draft) => draft,
            Err(error) => {
                self.test_result = Some((false, error.message().to_string()));
                self.submitting = false;
                cx.notify();
                return false;
            }
        };

        let workspace = self.workspace.clone();
        let editing = self.editing;
        let auth = self.auth;
        workspace.update(cx, |workspace, cx| {
            let saved = workspace.hosts.update(cx, |store, cx| {
                let id = match editing {
                    Some(id) => {
                        if !store.update(id, draft, cx) {
                            return None;
                        }
                        id
                    }
                    None => store.insert(draft, cx),
                };
                // 秘密写进应用目录的加密秘密库，数据库里永远没有它们。
                // 私钥口令按库里的路径归属（数据目录内是相对路径），拷走
                // 文件夹后仍然找得到。
                if let Some(login) = store.login(id) {
                    if auth == AuthKind::Password && !password.is_empty() {
                        store.save_secret(login.password_secret(), Some(password.clone()), cx);
                    }
                    if auth == AuthKind::Key
                        && !key_passphrase.is_empty()
                        && let Some(secret) = login.passphrase_secret()
                    {
                        store.save_secret(secret, Some(key_passphrase.clone()), cx);
                    }
                }
                Some(id)
            });
            match (editing, saved) {
                (Some(_), Some(_)) => workspace.show_toast("已更新主机", cx),
                (None, Some(id)) => {
                    workspace.selected_host = Some(id);
                    workspace.show_toast("已保存主机，双击左侧列表连接", cx);
                }
                (_, None) => {}
            }
            cx.notify();
        });
        true
    }

    /// 真登录一次；表单里的密码/口令优先于秘密库里保存的值。
    fn test_connection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.testing {
            return;
        }
        // 名字只用于报告，这里带上原来开头的旗，与保存后的名字一致。
        let name = format!("{}{}", self.name_flags, self.name.read(cx).value().trim());
        let address = self.address.read(cx).value().trim().to_string();
        let user = self.user.read(cx).value().trim().to_string();
        let user = if user.is_empty() {
            DEFAULT_USER.to_string()
        } else {
            user
        };
        let port = match self.port.read(cx).value().trim() {
            "" => Some(SSH_PORT),
            value => value.parse::<u16>().ok(),
        };
        if address.is_empty() || port.is_none_or(|port| port == 0) {
            self.test_result = Some((false, "请先填写主机地址".into()));
            cx.notify();
            return;
        }
        let Some(port) = port else {
            return;
        };
        let key_path = self.key_path.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        let passphrase = self.key_passphrase.read(cx).value().to_string();
        let request = crate::connection::LoginTest {
            host: self.editing.unwrap_or(HostId(0)),
            name,
            address,
            port,
            username: user,
            method: if self.auth == AuthKind::Key {
                crate::host::LoginMethod::Key
            } else {
                crate::host::LoginMethod::Password
            },
            key_path: (self.auth == AuthKind::Key && !key_path.is_empty())
                .then(|| std::path::PathBuf::from(key_path)),
            password: (!password.is_empty()).then_some(password),
            passphrase: (!passphrase.is_empty()).then_some(passphrase),
        };
        let tester = self.workspace.read(cx).tester.clone();

        self.testing = true;
        self.test_result = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { tester.test(request, Box::new(|_| false)) })
                .await;
            this.update(cx, |this, cx| {
                this.testing = false;
                this.test_result = Some(match result {
                    Ok(()) => (true, "连接成功".into()),
                    Err(reason) => (false, reason),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

pub fn open(
    workspace: &mut Workspace,
    editing: Option<HostId>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let modal_alpha = crate::settings::appearance_tokens::AppearanceTokens::new(
        &workspace.store.read(cx).settings().appearance,
    )
    .modal;
    let host = editing.and_then(|id| workspace.host(cx, id).cloned());
    let entity = cx.entity();
    let form = cx.new(|cx| HostForm::new(entity, host.clone(), window, cx));
    let is_editing = host.is_some();
    let title = if is_editing {
        "编辑主机"
    } else {
        "新建主机"
    };
    let caption = if is_editing {
        "修改连接信息；地址或端口变化后需重新确认主机密钥。"
    } else {
        "保存后，从左侧主机列表双击即可连接。"
    };

    let dialog_form = form.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        let form_for_ok = dialog_form.clone();
        let test_result = dialog_form.read(cx).test_result.clone();
        let testing = dialog_form.read(cx).testing;
        dialog::surface(dialog, theme::DIALOG_WIDTH_FORM, modal_alpha)
            .title(dialog::title(title))
            .margin_top(window.viewport_size().height * 0.18)
            // 表单有多个字段：误点遮罩不应丢失已填内容。
            .overlay_closable(false)
            .child(
                v_flex()
                    .gap(theme::SPACE_MD)
                    .child(dialog::caption(caption))
                    .child(dialog::divider())
                    .child(dialog_form.clone()),
            )
            .footer(
                dialog::footer()
                    .w_full()
                    .justify_between()
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                button::form_secondary(
                                    "test-connection",
                                    if testing {
                                        "正在测试…"
                                    } else {
                                        "测试连接"
                                    },
                                )
                                .disabled(testing)
                                .on_click({
                                    let form = dialog_form.clone();
                                    move |_, window, cx| {
                                        form.update(cx, |form, cx| form.test_connection(window, cx))
                                    }
                                }),
                            )
                            .when_some(test_result, |row, (ok, text)| {
                                row.child(
                                    h_flex()
                                        .gap(px(6.))
                                        .text_size(theme::TYPE_META)
                                        .text_color(theme::solid(if ok {
                                            theme::FOAM
                                        } else {
                                            theme::LOVE
                                        }))
                                        .child(div().size(px(6.)).rounded_full().bg(theme::solid(
                                            if ok { theme::FOAM } else { theme::LOVE },
                                        )))
                                        .child(text),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(button::form_secondary("cancel-host", "取消").on_click(
                                |_, window, cx| {
                                    window.close_dialog(cx);
                                },
                            ))
                            .child({
                                let form = dialog_form.clone();
                                button::form_primary("save-host", "保存").on_click(
                                    move |_, window, cx| {
                                        let saved =
                                            form.update(cx, |form, cx| form.commit(window, cx));
                                        if saved {
                                            window.close_dialog(cx);
                                        }
                                    },
                                )
                            }),
                    ),
            )
            .on_ok(move |_, window, cx| form_for_ok.update(cx, |form, cx| form.commit(window, cx)))
    });
    form.update(cx, |form, cx| {
        form.watch_for_changes(window, cx);
        form.focus_name(window, cx);
    });
}

impl Render for HostForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |label: &'static str,
                     error: bool,
                     description: Option<&'static str>,
                     input: AnyElement| {
            v_flex()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(theme::TYPE_LABEL)
                        .text_color(theme::solid(theme::OVERLAY_TEXT))
                        .child(label),
                )
                .child(input)
                .when(error, |field| {
                    field.child(
                        div()
                            .text_size(theme::TYPE_META)
                            .text_color(theme::solid(theme::LOVE))
                            .child(description.unwrap_or("请填写此项")),
                    )
                })
        };

        // 名称开头的国旗不在输入框的文字里（见 `HostForm::new`），这里把它
        // 画成图片摆在输入框左侧；用户看到的仍是「旗 + 名字」。用户自己在
        // 名字里又敲了国旗时，输入框旁边的这张图保持不变，下次打开时会重新
        // 摘一次开头的旗。
        let name_flags: Vec<AnyElement> = crate::components::emoji::flag_codes(&self.name_flags)
            .into_iter()
            .filter_map(|code| crate::components::emoji::flag(code, theme::TYPE_BODY))
            .collect();
        let name = field(
            "名称",
            self.errors.name,
            Some("请输入主机名称"),
            field::text_field_with("host-name-shell", &self.name, window, cx, |input| {
                let input = input.id("host-name").aria_label("名称").w_full();
                if name_flags.is_empty() {
                    input
                } else {
                    input.prefix(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .mr(px(2.))
                            .children(name_flags),
                    )
                }
            })
            .into_any_element(),
        );
        let address = field(
            "地址",
            self.errors.address,
            Some("请输入主机地址"),
            field::text_field_with("host-address-shell", &self.address, window, cx, |input| {
                input.id("host-address").aria_label("地址").w_full()
            })
            .into_any_element(),
        );
        let user = field(
            "用户名",
            false,
            None,
            field::text_field_with("host-user-shell", &self.user, window, cx, |input| {
                input.id("host-user").aria_label("用户名").w_full()
            })
            .into_any_element(),
        );
        let port = field(
            "端口",
            self.errors.port,
            Some("端口无效"),
            field::text_field_with("host-port-shell", &self.port, window, cx, |input| {
                input.id("host-port").aria_label("端口").w_full()
            })
            .into_any_element(),
        );
        let notes = field(
            "备注（可选）",
            false,
            None,
            div()
                .id("host-notes-field")
                .test_support()
                .child(field::text_area(
                    "host-notes-shell",
                    &self.notes,
                    window,
                    cx,
                    px(68.),
                    "备注",
                ))
                .into_any_element(),
        );
        let password = field(
            "密码",
            false,
            None,
            field::text_field_with("host-password-shell", &self.password, window, cx, |input| {
                input
                    .id("host-password")
                    .aria_label("密码")
                    .content_type(InputContentType::Password)
                    .w_full()
            })
            .into_any_element(),
        );
        let key = field(
            "私钥文件",
            self.errors.key,
            Some("请选择私钥文件"),
            h_flex()
                .gap(px(8.))
                .child(
                    field::text_field_with(
                        "host-key-path-shell",
                        &self.key_path,
                        window,
                        cx,
                        |input| input.id("host-key-path").aria_label("私钥文件").w_full(),
                    )
                    .flex_1()
                    .min_w_0()
                    .into_any_element(),
                )
                .child(
                    button::form_secondary("choose-key", "选择文件…").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.choose_key(window, cx);
                        },
                    )),
                )
                .into_any_element(),
        );
        let passphrase = field(
            "私钥口令（可选）",
            false,
            None,
            field::text_field_with(
                "host-key-passphrase-shell",
                &self.key_passphrase,
                window,
                cx,
                |input| {
                    input
                        .aria_label("私钥口令")
                        .id("host-key-passphrase")
                        .content_type(InputContentType::Password)
                        .w_full()
                },
            )
            .into_any_element(),
        );

        let auth = self.auth;
        let segmented = |value: AuthKind, label: &'static str, cx: &mut Context<Self>| {
            let active = auth == value;
            div()
                .id(ElementId::Name(format!("auth-method-{label}").into()))
                .test_support()
                .tab_index(0)
                .aria_label(label)
                .flex()
                .items_center()
                .justify_center()
                .h(px(28.))
                .px(px(14.))
                .rounded(theme::RADIUS_XS)
                .text_size(theme::TYPE_META)
                .text_color(theme::solid(if active {
                    theme::TEXT
                } else {
                    theme::OVERLAY_SUBTLE
                }))
                .bg(if active {
                    theme::colored(theme::FOAM, 0.12)
                } else {
                    Hsla::default()
                })
                .cursor_pointer()
                .focus_visible(|style| style.bg(theme::colored(theme::FOAM, 0.08)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.auth = value;
                    this.test_result = None;
                    cx.notify();
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space" | " ") {
                        cx.stop_propagation();
                        this.auth = value;
                        this.test_result = None;
                        cx.notify();
                    }
                }))
                .child(label)
                .into_any_element()
        };

        v_flex()
            .gap(theme::SPACE_MD)
            .child(
                v_flex()
                    .gap(theme::SPACE_MD)
                    .child(name)
                    .child(address)
                    .child(
                        h_flex()
                            .gap(px(12.))
                            .child(div().flex_1().child(user))
                            .child(div().w(px(104.)).child(port)),
                    )
                    .child(notes),
            )
            .child(
                v_flex()
                    .mt(px(14.))
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(theme::TYPE_LABEL)
                            .text_color(theme::solid(theme::OVERLAY_TEXT))
                            .child("认证方式"),
                    )
                    .child(
                        h_flex()
                            .justify_start()
                            .self_start()
                            .p(px(2.))
                            .gap_0()
                            .rounded(theme::RADIUS_MD)
                            .border_1()
                            .border_color(theme::line())
                            .bg(theme::colored(theme::BASE_RGB, 0.5))
                            .child(segmented(AuthKind::Password, "密码", cx))
                            .child(segmented(AuthKind::Key, "密钥", cx)),
                    )
                    .when(self.auth == AuthKind::Password, |panel| {
                        panel.child(password)
                    })
                    .when(self.auth == AuthKind::Key, |panel| {
                        panel.child(key).child(passphrase)
                    }),
            )
    }
}

impl HostForm {
    pub(crate) fn focus_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |state, cx| state.focus(window, cx));
    }

    fn choose_key(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择私钥文件".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path = path.to_string_lossy().to_string();
            this.update_in(cx, |this, window, cx| {
                this.key_path
                    .update(cx, |input, cx| input.set_value(path, window, cx));
            })
            .ok();
        })
        .detach();
    }
}
