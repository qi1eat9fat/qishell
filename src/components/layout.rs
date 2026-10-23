//! 页面骨架：设置页、侧栏标题、内容列、分区与空状态。
//!
//! 这些结构在多个页面重复出现，尺寸与间距只在这里定义一次。

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;

/// 一个设置分区：上间距 22px、1px 分隔线、标题 13px 600。
pub fn section(
    title: impl Into<SharedString>,
    description: Option<impl Into<SharedString>>,
    rows: Vec<AnyElement>,
) -> Div {
    section_parts(title, description, rows, false)
}

/// 页面里的第一个分区：没有上分隔线，也不额外留上间距。
///
/// 设置页没有页面标题了，第一行就是分区标题；再画一条线只会让它看着
/// 像页面被切掉了一截。
pub fn leading_section(
    title: impl Into<SharedString>,
    description: Option<impl Into<SharedString>>,
    rows: Vec<AnyElement>,
) -> Div {
    section_parts(title, description, rows, true)
}

fn section_parts(
    title: impl Into<SharedString>,
    description: Option<impl Into<SharedString>>,
    rows: Vec<AnyElement>,
    leading: bool,
) -> Div {
    let body = v_flex()
        .gap(px(0.))
        .child(
            div()
                .text_size(theme::TYPE_SECTION)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .when_some(description, |section, description| {
            section.child(
                div()
                    .mt(px(5.))
                    .mb(px(10.))
                    .text_size(theme::TYPE_META)
                    .line_height(px(19.))
                    .text_color(theme::solid(theme::SUBTLE))
                    .child(description.into()),
            )
        })
        .children(rows);
    if leading {
        body
    } else {
        body.mt(theme::SPACE_XL)
            .pt(px(18.))
            .border_t_1()
            .border_color(theme::line())
    }
}

/// 设置行：左侧标题 + 说明，右侧控件；上下 9px。说明为空时只留标题。
pub fn settings_row(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
) -> AnyElement {
    let description = description.into();
    let label = v_flex().flex_1().w_0().min_w_0().gap(px(2.)).child(
        div()
            .text_size(theme::TYPE_META)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::solid(theme::MUTED_STRONG))
            .child(title.into()),
    );
    let label = if description.is_empty() {
        label
    } else {
        label.child(
            div()
                .text_size(theme::TYPE_MICRO)
                .line_height(px(17.))
                .text_color(theme::solid(theme::SUBTLE))
                .child(description),
        )
    };
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap(px(18.))
        .py(px(9.))
        .child(label)
        .child(div().flex_shrink_0().child(control))
        .into_any_element()
}

/// 设置内容列：最大 760px，水平居中。
pub fn content_column(children: Vec<AnyElement>) -> Div {
    h_flex().w_full().justify_center().items_start().child(
        v_flex()
            .w_full()
            .max_w(theme::CONTENT_MAX_WIDTH)
            .children(children),
    )
}

/// 侧栏 / 面板顶部的 40px 标题栏。
pub fn panel_header(title: impl Into<SharedString>, action: Option<AnyElement>) -> Div {
    h_flex()
        // 与标签栏同高：主机侧栏从第二层开始，两条分隔线必须对齐。
        .h(theme::BAR_HEIGHT)
        .w_full()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .pl(px(14.))
        // 与右侧按钮距上/下边缘的 4px 保持一致。
        .pr(px(4.))
        .border_b_1()
        .border_color(theme::line())
        .child(
            div()
                .text_size(theme::TYPE_LABEL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::solid(theme::MUTED_STRONG))
                .child(title.into()),
        )
        .when_some(action, |row, action| row.child(action))
}

/// 空状态：一句话说明，保持密度，不放大面积插画。
pub fn empty_state(message: impl Into<SharedString>) -> Div {
    div()
        .py(px(18.))
        .px(px(10.))
        .text_center()
        .text_size(theme::TYPE_META)
        .text_color(theme::solid(theme::SUBTLE))
        .child(message.into())
}
