//! 传输目标已有同名文件时的覆盖 / 跳过询问。

use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use crate::components::{button, dialog};
use crate::sftp::{ConflictChoice, ConflictInfo, SftpView, format_bytes};
use crate::theme;

struct ConflictBody {
    apply_all: bool,
}

impl Render for ConflictBody {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.apply_all;
        div().mt(px(10.)).child(
            crate::components::AppCheckbox::new("sftp-conflict-apply-all")
                .checked(checked)
                .label("应用于本批剩余冲突")
                .on_change(cx.listener(|this, checked: &bool, _, cx| {
                    this.apply_all = *checked;
                    cx.notify();
                })),
        )
    }
}

pub fn open(
    info: ConflictInfo,
    view: Entity<SftpView>,
    modal_alpha: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let state = cx.new(|_| ConflictBody { apply_all: false });
    let conflict_id = info.conflict_id;
    window.open_dialog(cx, move |dialog, window, cx| {
        let cancel_view = view.clone();
        let skip_view = view.clone();
        let skip_state = state.clone();
        let overwrite_view = view.clone();
        let overwrite_state = state.clone();
        let ok_view = view.clone();
        let ok_state = state.clone();
        let close_view = view.clone();

        dialog::surface(dialog, px(520.), modal_alpha)
            .title(dialog::title("文件已存在"))
            .margin_top(window.viewport_size().height * 0.30)
            .overlay_closable(false)
            .child(dialog::caption(
                "目标位置已有同名文件。请选择覆盖、跳过，或取消本次传输。",
            ))
            .child(
                div()
                    .mt(px(10.))
                    .min_w_0()
                    .truncate()
                    .text_size(theme::TYPE_BODY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(info.name.clone()),
            )
            .child(
                h_flex()
                    .mt(px(10.))
                    .gap(px(8.))
                    .child(fact(
                        "来源",
                        info.source_size,
                        info.source_modified.clone(),
                        cx,
                    ))
                    .child(fact(
                        "目标",
                        info.target_size,
                        info.target_modified.clone(),
                        cx,
                    )),
            )
            .child(state.clone())
            .footer(
                dialog::footer()
                    .child(
                        button::form_secondary("sftp-conflict-cancel", "取消").on_click(
                            move |_, window, cx| {
                                cancel_view.update(cx, |view, cx| {
                                    view.reply_conflict(
                                        conflict_id,
                                        ConflictChoice::Cancel,
                                        false,
                                        cx,
                                    );
                                });
                                window.close_dialog(cx);
                            },
                        ),
                    )
                    .child(
                        button::form_secondary("sftp-conflict-skip", "跳过").on_click(
                            move |_, window, cx| {
                                let apply_all = skip_state.read(cx).apply_all;
                                skip_view.update(cx, |view, cx| {
                                    view.reply_conflict(
                                        conflict_id,
                                        ConflictChoice::Skip,
                                        apply_all,
                                        cx,
                                    );
                                });
                                window.close_dialog(cx);
                            },
                        ),
                    )
                    .child(
                        button::form_primary("sftp-conflict-overwrite", "覆盖").on_click(
                            move |_, window, cx| {
                                let apply_all = overwrite_state.read(cx).apply_all;
                                overwrite_view.update(cx, |view, cx| {
                                    view.reply_conflict(
                                        conflict_id,
                                        ConflictChoice::Overwrite,
                                        apply_all,
                                        cx,
                                    );
                                });
                                window.close_dialog(cx);
                            },
                        ),
                    ),
            )
            // 对话框右上角的 × 也按「取消本次传输」处理，避免工作线程一直等待。
            .on_close(move |_, _, cx| {
                close_view.update(cx, |view, cx| {
                    view.reply_conflict(conflict_id, ConflictChoice::Cancel, false, cx);
                });
            })
            .on_ok(move |_, window, cx| {
                let apply_all = ok_state.read(cx).apply_all;
                ok_view.update(cx, |view, cx| {
                    view.reply_conflict(conflict_id, ConflictChoice::Overwrite, apply_all, cx);
                });
                window.close_dialog(cx);
                true
            })
    });
}

fn fact(label: &'static str, size: u64, modified: String, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .gap(px(3.))
        .p(px(10.))
        .rounded(theme::RADIUS_MD)
        .border_1()
        .border_color(theme::line())
        .bg(theme::colored(theme::BASE_RGB, 0.6))
        .child(
            div()
                .text_size(theme::TYPE_MICRO)
                .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                .child(label),
        )
        .child(
            div()
                .text_size(theme::TYPE_SECTION)
                .font_weight(FontWeight::SEMIBOLD)
                .child(format_bytes(size)),
        )
        .child(
            div()
                .font_family(cx.theme().font_family.clone())
                .text_size(theme::TYPE_MICRO)
                .text_color(theme::solid(theme::OVERLAY_SUBTLE))
                .child(modified),
        )
        .into_any_element()
}
