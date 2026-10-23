//! 主工作区顶栏：品牌、居中的连接状态/信息与原生窗口按钮。
//!
//! 标题栏与连接栏合并为一条 36px 顶栏，主区因此只剩「顶栏 + 页签栏」两层。
//!
//! 不使用 gpui-kit 的 `TitleBar`：它把整条标题栏标成拖动区，Windows 的命中
//! 测试会把标题栏里的按钮一并当成标题栏（HTCAPTION），按钮点不动。这里把左右
//! 两段非交互区域标成拖动区，中间信息与右侧按钮各自处理命中。

use gpui_kit::component::{Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use gpui_kit::assets::IconName;

use crate::settings::appearance_tokens::AppearanceTokens;
use crate::theme;

use super::Workspace;

impl Workspace {
    pub(super) fn render_top_bar(
        &self,
        tokens: &AppearanceTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let info = self.render_connection_info(cx);

        h_flex()
            .relative()
            .h(theme::TITLE_BAR_HEIGHT)
            .w_full()
            .flex_shrink_0()
            .items_center()
            .pl(theme::SPACE_MD)
            // 与按钮距上/下边缘的 4px 保持一致。
            .pr(px(4.))
            .border_b_1()
            .border_color(theme::line())
            .bg(theme::glass(tokens.title_bar))
            // 中间信息用绝对定位铺满整条顶栏再居中，不受左右内容宽度影响。
            // 这一层自己不接命中：它铺满整条顶栏，一标拖动区就会盖住右侧的
            // 窗口按钮，按钮再也点不动。真正可拖的是里面那段连接信息自身。
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(info, |layer, info| layer.child(info)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(theme::ICON_TEXT_GAP)
                    .window_control_area(WindowControlArea::Drag)
                    .child(img(crate::assets::app_icon()).size(px(18.)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(theme::TYPE_SECTION)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::APP_NAME),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    // 右侧窗口按钮之前的那段空白：它只占空位，不会盖住按钮，
                    // 所以能安全地标成拖动区。连接信息只占内容宽度，右边这段
                    // 以前是死区，按下拖不动窗口。
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    )
                    .child(window_control(
                        "minimize-window",
                        IconName::WindowMinimize,
                        false,
                        |window, _| window.minimize_window(),
                    ))
                    .child(window_control(
                        "maximize-window",
                        if window.is_maximized() {
                            IconName::WindowRestore
                        } else {
                            IconName::WindowMaximize
                        },
                        false,
                        |window, _| window.zoom_window(),
                    ))
                    .child(window_control(
                        "close-window",
                        IconName::WindowClose,
                        true,
                        |window, _| window.remove_window(),
                    )),
            )
    }
}

fn window_control(
    id: &'static str,
    icon: IconName,
    danger: bool,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = match id {
        "minimize-window" => "最小化",
        "maximize-window" => "最大化或还原",
        _ => "关闭窗口",
    };
    let action = std::rc::Rc::new(action);
    let click_action = action.clone();
    div()
        .id(id)
        .test_support()
        .aria_label(label)
        .tab_index(0)
        .flex()
        .items_center()
        .justify_center()
        .w(theme::ICON_BUTTON_SIZE)
        .h(theme::ICON_BUTTON_SIZE)
        .rounded(theme::RADIUS_MD)
        .cursor_pointer()
        .focus_visible(|style| style.shadow(theme::focus_ring()))
        .on_click(move |_, window, cx| click_action(window, cx))
        .child(
            // 悬停底 28×28，命中区仍是外层的 32×32。
            div()
                .size(theme::ICON_BUTTON_SIZE)
                .rounded(theme::RADIUS_MD)
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::solid(theme::MUTED_STRONG))
                .hover(move |style| {
                    if danger {
                        style
                            .bg(theme::solid(theme::LOVE))
                            .text_color(theme::solid(theme::BASE_RGB))
                    } else {
                        style.bg(theme::hover_tint())
                    }
                })
                .child(Icon::new(icon).size(theme::ICON_SM)),
        )
}
