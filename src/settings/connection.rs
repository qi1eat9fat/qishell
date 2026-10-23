//! 设置 › 连接：应用目录里的私钥与主机信任列表。
//!
//! 两样都跟着数据目录走，拷贝整个文件夹就能迁移。私钥在 `keys/`，
//! 信任记录在 `known_hosts`；这里只负责列出、生成、导入与删除。

use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Icon, WindowExt as _, h_flex, input::InputState, v_flex};
use gpui_kit::*;

use crate::components::{button, dialog, field};
use crate::model::SettingsState;
use crate::ssh::vault::{self, KnownHost, StoredKey};
use crate::theme;

use super::SettingsPanel;

impl SettingsPanel {
    pub(super) fn render_connection(
        &mut self,
        _settings: &SettingsState,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut body = Vec::new();

        let keys = self.keys.clone();
        let key_notice = self.key_notice.clone();
        let key_rows: Vec<AnyElement> = keys
            .iter()
            .map(|key| self.render_key_row(key, cx))
            .collect();
        body.push(
            self.leading_section(
                "私钥",
                None,
                vec![
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap(px(8.))
                        .py(px(9.))
                        .child(div().flex_1())
                        .child(
                            button::secondary("key-generate", "新建密钥").on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.open_generate_key(window, cx);
                                },
                            )),
                        )
                        .child(
                            button::secondary("key-import", "导入…").on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.import_key(window, cx);
                                },
                            )),
                        )
                        .into_any_element(),
                    v_flex().w_full().children(key_rows).into_any_element(),
                ]
                .into_iter()
                .chain(key_notice.map(|notice| {
                    div()
                        .py(px(6.))
                        .text_size(theme::TYPE_META)
                        .text_color(theme::solid(theme::FOAM))
                        .child(notice)
                        .into_any_element()
                }))
                .collect(),
            )
            .into_any_element(),
        );

        let known = self.known_hosts.clone();
        let host_notice = self.host_notice.clone();
        let host_rows: Vec<AnyElement> = known
            .iter()
            .map(|host| self.render_known_host_row(host, cx))
            .collect();
        body.push(
            self.section(
                "信任的主机",
                None,
                vec![v_flex().w_full().children(host_rows).into_any_element()]
                    .into_iter()
                    .chain(host_notice.map(|notice| {
                        div()
                            .py(px(6.))
                            .text_size(theme::TYPE_META)
                            .text_color(theme::solid(theme::FOAM))
                            .child(notice)
                            .into_any_element()
                    }))
                    .collect(),
            )
            .into_any_element(),
        );

        body
    }

    fn render_key_row(&self, key: &StoredKey, cx: &mut Context<Self>) -> AnyElement {
        let name = key.name.clone();
        let delete_name = name.clone();
        let path = key.path.display().to_string();
        h_flex()
            .w_full()
            .items_center()
            .gap(px(10.))
            .py(px(9.))
            .child(
                Icon::new(IconName::KeyRound)
                    .size(theme::ICON_SM)
                    .text_color(theme::solid(theme::FOAM)),
            )
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
                            .text_color(theme::solid(theme::MUTED_STRONG))
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child(format!("{} · {} · {path}", key.algorithm, key.fingerprint)),
                    ),
            )
            .child(
                button::icon(ElementId::Name(format!("key-delete-{}", key.name).into()))
                    .icon(IconName::Trash)
                    .accessibility_label("删除私钥")
                    .tooltip("删除私钥")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_key(&delete_name, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_known_host_row(&self, host: &KnownHost, cx: &mut Context<Self>) -> AnyElement {
        let entry = host.entry.clone();
        let forget_entry = entry.clone();
        h_flex()
            .w_full()
            .items_center()
            .gap(px(10.))
            .py(px(9.))
            .child(
                Icon::new(IconName::ShieldCheck)
                    .size(theme::ICON_SM)
                    .text_color(theme::solid(theme::FOAM)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .truncate()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_META)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::solid(theme::MUTED_STRONG))
                            .child(entry),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child(format!("{} · {}", host.algorithm, host.fingerprint)),
                    ),
            )
            .child(
                button::icon(ElementId::Name(
                    format!("known-host-forget-{}", host.entry).into(),
                ))
                .icon(IconName::Trash)
                .accessibility_label("不再信任这台主机")
                .tooltip("不再信任这台主机")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.forget_host(&forget_entry, cx);
                })),
            )
            .into_any_element()
    }

    /// 读一遍应用目录里的私钥与信任记录。打开连接页、增删之后都调它。
    pub(super) fn reload_connection_files(&mut self) {
        self.keys = vault::list_keys(&crate::app::paths::keys_dir());
        self.known_hosts = vault::list_known_hosts(&crate::app::paths::known_hosts_path());
    }

    fn remove_key(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(key) = self.keys.iter().find(|key| key.name == name).cloned() else {
            return;
        };
        match vault::delete_key(&key) {
            Ok(()) => self.set_key_notice(format!("已删除 {}", key.name), cx),
            Err(error) => self.set_key_notice(format!("{error:#}"), cx),
        }
        self.reload_connection_files();
        cx.notify();
    }

    fn forget_host(&mut self, entry: &str, cx: &mut Context<Self>) {
        match vault::forget_known_host(&crate::app::paths::known_hosts_path(), entry) {
            Ok(()) => self.set_host_notice(format!("已不再信任 {entry}"), cx),
            Err(error) => self.set_host_notice(format!("{error:#}"), cx),
        }
        self.reload_connection_files();
        cx.notify();
    }

    /// 新建密钥：填个名字，生成一把 Ed25519。
    fn open_generate_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("id_ed25519")
                .placeholder("密钥文件名")
        });
        let alpha = self.store.read(cx).appearance().modal_alpha_f32();
        let panel = cx.entity();
        let field_state = name.clone();
        let save_state = name.clone();
        window.open_dialog(cx, move |dialog, window, _cx| {
            let field_state = field_state.clone();
            dialog::surface(dialog, px(480.), alpha)
                .title(dialog::title("新建密钥"))
                .margin_top(window.viewport_size().height * 0.3)
                .overlay_closable(true)
                .child(v_flex().gap(px(10.)).child(field::text_field_with(
                    "key-name",
                    &field_state,
                    window,
                    _cx,
                    |input| input.aria_label("密钥文件名").w_full(),
                )))
                .footer(dialog::footer().child(
                    button::form_primary("key-generate-confirm", "生成").on_click({
                        let save_state = save_state.clone();
                        let panel = panel.clone();
                        move |_, window, cx| {
                            let name = save_state.read(cx).value().trim().to_string();
                            panel.update(cx, |panel, cx| {
                                match vault::generate_key(&crate::app::paths::keys_dir(), &name) {
                                    Ok(key) => {
                                        panel.set_key_notice(format!("已生成 {}", key.name), cx);
                                        panel.reload_connection_files();
                                    }
                                    Err(error) => panel.set_key_notice(format!("{error:#}"), cx),
                                }
                                cx.notify();
                            });
                            window.close_dialog(cx);
                        }
                    }),
                ))
        });
    }

    /// 导入：选一个私钥文件，复制进应用目录。
    fn import_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择要导入的私钥".into()),
        });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = handle.update(cx, |_, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    match vault::import_key(&crate::app::paths::keys_dir(), &path) {
                        Ok(key) => this.set_key_notice(format!("已导入 {}", key.name), cx),
                        Err(error) => this.set_key_notice(format!("{error:#}"), cx),
                    }
                    this.reload_connection_files();
                    cx.notify();
                });
            });
        })
        .detach();
    }
}
