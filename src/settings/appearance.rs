//! 设置 › 外观：玻璃、轻量浮层、模态弹窗与背景。
//!
//! 每个滑杆显示的就是最终使用的值，派生比例只存在于 `appearance_tokens`。

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::components::button;
use crate::model::{Appearance, SettingsState, Wallpaper};
use crate::theme;

use super::SettingsPanel;
use super::appearance_tokens::MODAL_READABLE_MIN;

/// 把用户选的壁纸复制到应用目录，返回相对路径。只接受图片扩展名。
fn store_wallpaper(source: &std::path::Path) -> Option<std::path::PathBuf> {
    let extension = source.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
    ) {
        return None;
    }
    let directory = crate::app::paths::data_dir();
    std::fs::create_dir_all(&directory).ok()?;
    // 文件名必须唯一：GPUI 按路径缓存已解码的图片，同名覆盖会继续显示旧图。
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let name = format!("wallpaper-{stamp}.{extension}");
    std::fs::copy(source, directory.join(&name)).ok()?;
    Some(std::path::PathBuf::from(name))
}

impl SettingsPanel {
    pub(super) fn render_appearance(
        &mut self,
        settings: &SettingsState,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let appearance = settings.appearance.clone();
        let mut body = Vec::new();

        body.push(
            self.leading_section(
                "玻璃",
                Some("覆盖顶栏、主机侧栏、右侧栏、标签栏与工作区容器；数值越低越通透。"),
                vec![self.slider_row(
                    "glass",
                    "不透明度",
                    "",
                    &self.glass,
                    appearance.glass,
                    "%",
                    cx,
                )],
            )
            .into_any_element(),
        );

        body.push(
            self.section(
                "轻量浮层",
                Some("右键菜单、下拉、Tooltip、Toast、终端查找栏与 SFTP 拖拽幽灵。数值即最终值。"),
                vec![self.slider_row(
                    "overlay-alpha",
                    "不透明度",
                    "低于 30% 时浮层上的文字在亮壁上可能读不清",
                    &self.overlay_alpha,
                    appearance.overlay_alpha,
                    "%",
                    cx,
                )],
            )
            .into_any_element(),
        );

        body.push(
            self.section(
                "模态弹窗",
                Some("新建/编辑主机、确认弹窗、密码提示与 SFTP 对话框。"),
                vec![
                    self.slider_row(
                        "modal-alpha",
                        "弹窗不透明度",
                        "弹窗自身的玻璃底色",
                        &self.modal_alpha,
                        appearance.modal_alpha,
                        "%",
                        cx,
                    ),
                    self.slider_row(
                        "backdrop-alpha",
                        "遮罩不透明度",
                        "弹窗背后整页的压暗程度",
                        &self.backdrop_alpha,
                        appearance.backdrop_alpha,
                        "%",
                        cx,
                    ),
                    self.note(if appearance.modal_alpha < MODAL_READABLE_MIN {
                        "当前弹窗不透明度较低，正文对比度可能不足。"
                    } else {
                        "弹窗透出壁纸，低于 50% 时正文对比度可能不足。"
                    }),
                ],
            )
            .into_any_element(),
        );

        body.push(self.render_wallpaper(settings, cx).into_any_element());
        // 恢复默认与其余设置一样占一行：左边标题与说明，右边按钮，不再是一
        // 行按钮加一行灰字挂在页面末尾。
        body.push(
            self.section(
                "恢复默认",
                None,
                vec![self.row(
                    "恢复外观默认",
                    "",
                    button::secondary("reset-appearance", "恢复默认").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.reset_appearance(window, cx);
                        },
                    )),
                )],
            )
            .into_any_element(),
        );
        body
    }

    /// 重置外观参数与终端主题；壁纸文件与终端字体保持不动。
    fn reset_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = Appearance::default();
        self.update_settings(cx, |settings| {
            let wallpaper = settings.appearance.wallpaper.clone();
            settings.appearance = defaults.clone();
            settings.appearance.wallpaper = wallpaper;
            settings.terminal.theme = String::new();
        });
        // 滑杆是独立 Entity，改了设置不会自动回位；必须逐个写回，否则拇指
        // 停在旧值上，和页面上的数字对不上。
        for (state, value) in [
            (&self.glass, defaults.glass),
            (&self.clarity, defaults.clarity),
            (&self.blur, defaults.blur),
            (&self.overlay_alpha, defaults.overlay_alpha),
            (&self.modal_alpha, defaults.modal_alpha),
            (&self.backdrop_alpha, defaults.backdrop_alpha),
        ] {
            state.update(cx, |state, cx| {
                state.set_value(value as f32, window, cx);
            });
        }
    }

    /// 背景小节：壁纸卡片、清晰度与壁纸模糊。
    fn render_wallpaper(&self, settings: &SettingsState, cx: &mut Context<Self>) -> Div {
        let appearance = &settings.appearance;
        let wallpaper = &appearance.wallpaper;
        let has_wallpaper = wallpaper.path.is_some();
        let preview: AnyElement = match &wallpaper.path {
            Some(path) => img(crate::app::paths::resolve(path))
                .w(px(240.))
                .h(px(135.))
                .rounded(theme::RADIUS_MD)
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => div()
                .w(px(240.))
                .h(px(135.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS_MD)
                .border_1()
                .border_color(theme::line())
                .bg(theme::colored(theme::BASE_RGB, theme::CARD_TINT))
                .text_size(theme::TYPE_MICRO)
                .text_color(theme::solid(theme::SUBTLE))
                .child("纯色背景")
                .into_any_element(),
        };
        let hint = if wallpaper.custom {
            "本机图片"
        } else {
            "纯色背景"
        };

        let card = h_flex()
            .w_full()
            .gap(px(16.))
            .p(px(10.))
            .rounded(theme::RADIUS_LG)
            .border_1()
            .border_color(theme::line())
            .bg(theme::colored(theme::BASE_RGB, theme::PREVIEW_TINT))
            .child(preview)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(
                        div()
                            .truncate()
                            .text_size(theme::TYPE_META)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(wallpaper.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child(hint),
                    )
                    .child(
                        h_flex()
                            .mt(px(7.))
                            .gap(px(8.))
                            .child(button::secondary("choose-wallpaper", "选择图片…").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.choose_wallpaper(window, cx);
                                }),
                            ))
                            .child(button::secondary("reset-wallpaper", "恢复默认").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.update_settings(cx, |settings| {
                                        settings.appearance.wallpaper = Wallpaper::default();
                                    });
                                }),
                            )),
                    ),
            )
            .into_any_element();

        self.section(
            "背景",
            None,
            vec![
                card,
                self.slider_row(
                    "clarity",
                    "清晰度",
                    "越高暗色遮罩越淡；浅色壁纸建议 60% 以上",
                    &self.clarity,
                    appearance.clarity,
                    "%",
                    cx,
                ),
                self.slider_row_enabled(
                    "blur",
                    "壁纸模糊",
                    if has_wallpaper {
                        ""
                    } else {
                        "设置壁纸后生效"
                    },
                    &self.blur,
                    appearance.blur,
                    "px",
                    has_wallpaper,
                    cx,
                ),
            ],
        )
    }

    fn note(&self, text: &'static str) -> AnyElement {
        div()
            .text_size(theme::TYPE_MICRO)
            .line_height(px(17.))
            .text_color(theme::solid(theme::SUBTLE))
            .child(text)
            .into_any_element()
    }

    fn choose_wallpaper(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择背景图片".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("背景")
                .to_string();
            // 复制进应用目录，设置里存相对路径：整个文件夹拷走，壁纸也跟
            // 着走。复制失败就退回原路径（本机仍然能用）。
            let portable = store_wallpaper(&path);
            this.update(cx, |this, cx| {
                this.update_settings(cx, |settings| {
                    // 只有复制成功后才删旧文件；复制失败时旧壁纸仍然可用。
                    if let Some(new_path) = portable.as_deref()
                        && let Some(old) = settings.appearance.wallpaper.path.as_ref()
                        && !old.is_absolute()
                        && new_path != old.as_path()
                    {
                        let _ = std::fs::remove_file(crate::app::paths::data_dir().join(old));
                    }
                    settings.appearance.wallpaper = Wallpaper {
                        path: portable.or(Some(path)),
                        name,
                        custom: true,
                    };
                });
            })
            .ok();
        })
        .detach();
    }
}
