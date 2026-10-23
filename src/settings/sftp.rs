//! 设置 › SFTP：最大同时传输数量与外部编辑器。

use gpui_kit::component::h_flex;
use gpui_kit::*;

use crate::components::{button, field};
use crate::model::SettingsState;

use super::SettingsPanel;
use gpui_kit::assets::IconName;

impl SettingsPanel {
    pub(super) fn render_sftp(
        &mut self,
        _settings: &SettingsState,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let stepper = h_flex()
            .items_center()
            .gap(px(4.))
            .child(
                button::icon("sftp-minus")
                    .accessibility_label("减少并发数")
                    .icon(IconName::Minus)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let current = this.store.read(cx).settings().sftp.max_transfers;
                        let next = current.saturating_sub(1).max(1);
                        this.set_sftp_max(next, window, cx);
                    })),
            )
            .child(field::text_field_with(
                "sftp-max-transfers",
                &self.sftp_max,
                window,
                cx,
                // 纯数字字段居中：数字在固定宽度里居中，步进时不会左右跳。
                |input| {
                    input
                        .aria_label("最大同时传输数量")
                        .w(px(48.))
                        .text_align(TextAlign::Center)
                },
            ))
            .child(
                button::icon("sftp-plus")
                    .accessibility_label("增加并发数")
                    .icon(IconName::Plus)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let current = this.store.read(cx).settings().sftp.max_transfers;
                        let next = (current + 1).min(10);
                        this.set_sftp_max(next, window, cx);
                    })),
            );

        let local_root = h_flex()
            .gap(px(8.))
            .child(field::text_field_with(
                "sftp-local-root-shell",
                &self.sftp_local_root,
                window,
                cx,
                |input| {
                    input
                        .id("sftp-local-root")
                        .aria_label("本机默认打开路径")
                        .w(px(230.))
                },
            ))
            .child(
                button::secondary("choose-local-root", "选择文件夹…").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.choose_local_root(window, cx);
                    },
                )),
            );

        let editor = h_flex()
            .gap(px(8.))
            .child(field::text_field_with(
                "editor-path",
                &self.editor_path,
                window,
                cx,
                |input| input.aria_label("外部编辑器路径").w(px(230.)),
            ))
            .child(
                button::secondary("choose-editor", "选择文件…").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.choose_editor(window, cx);
                    },
                )),
            );

        vec![
            self.leading_section(
                "本机",
                None,
                vec![self.row("默认打开路径", "本地栏从这里开始，留空用主目录", local_root)],
            )
            .into_any_element(),
            self.section(
                "传输",
                None,
                vec![self.row("最大同时传输数量", "1–10，默认 3", stepper)],
            )
            .into_any_element(),
            self.section(
                "外部编辑器",
                None,
                vec![self.row("编辑器路径", "用于「编辑」动作", editor)],
            )
            .into_any_element(),
        ]
    }

    fn choose_local_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择本地默认打开的文件夹".into()),
        });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path = path.to_string_lossy().to_string();
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.sftp_local_root
                        .update(cx, |input, cx| input.set_value(path.clone(), window, cx));
                    this.update_settings(cx, |settings| settings.sftp.local_root = path);
                });
            });
        })
        .detach();
    }

    fn choose_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择文本编辑器".into()),
        });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path = path.to_string_lossy().to_string();
            // 文件对话框在另一个窗口上返回，输入框要借主窗口才能改文字。
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.editor_path
                        .update(cx, |input, cx| input.set_value(path.clone(), window, cx));
                    this.update_settings(cx, |settings| settings.editor_path = path);
                });
            });
        })
        .detach();
    }

    fn set_sftp_max(&mut self, value: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.sftp_max.update(cx, |input, cx| {
            input.set_value(value.to_string(), window, cx)
        });
        self.update_settings(cx, |settings| settings.sftp.max_transfers = value);
    }
}
