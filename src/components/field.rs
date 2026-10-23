//! 统一输入控件。
//!
//! 设计系统里的输入框是「34px 外壳 + 13px 文字 + 1px line + 聚焦 2px foam」。
//! gpui-kit 的 `Input` / `Select` 把高度和边框写在自己的尺寸档里，聚焦时还会
//! 额外画一圈外环，所以这里统一用一层外壳画边框，控件本身只负责文字与光标。

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;

/// 去掉 gpui-kit 自带外观的输入框，交给 [`input_shell`] 画边框。
pub fn bare_input(input: Input) -> Input {
    input
        .appearance(false)
        .text_size(theme::TYPE_BODY)
        .px(px(10.))
}

/// 控件外壳：常态 1px `line()`；聚焦时换成 2px `FOAM`。
///
/// 外壳不参与焦点，聚焦状态由调用方按控件的 Focus 状态给出，因此不改变
/// Tab 顺序，也能稳定画出 2px 边框。输入框、下拉框、色板旁的文本框都走这一层，
/// 全应用只有这一种聚焦外观。
fn control_shell(
    id: impl Into<ElementId>,
    focused: bool,
    height: Pixels,
    radius: Pixels,
    control: impl IntoElement,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .w_full()
        .h(height)
        .rounded(radius)
        .border_1()
        .border_color(if focused {
            theme::solid(theme::FOAM)
        } else {
            theme::line()
        })
        .when(focused, |shell| shell.border_2())
        .bg(theme::colored(theme::BASE_RGB, theme::FIELD_TINT))
        .overflow_hidden()
        .child(control)
}

/// 标准输入框外壳（34px、`RADIUS_MD`）。
pub fn input_shell(
    id: impl Into<ElementId>,
    focused: bool,
    input: impl IntoElement,
) -> Stateful<Div> {
    control_shell(id, focused, theme::INPUT_HEIGHT, theme::RADIUS_MD, input)
}

/// 标准单行输入框，允许在 `build` 里追加前缀、占位符等配置。
pub fn text_field_with(
    id: impl Into<ElementId>,
    state: &Entity<InputState>,
    window: &Window,
    cx: &App,
    build: impl FnOnce(Input) -> Input,
) -> Stateful<Div> {
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    let state = state.clone();
    input_shell(
        id,
        focused,
        build(bare_input(Input::new(&state))).into_any_element(),
    )
    .cursor_text()
    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
        state.update(cx, |state, cx| state.focus(window, cx));
    })
}

/// 标准下拉框：与输入框共用同一层外壳，聚焦时同样是单层 2px `FOAM`。
///
/// `Select` 开着原生外观时，聚焦会把它自己的 1px 边框换成 `ring` 色，再在
/// 外面叠一圈 3px 半透明外环——看上去就是双边框。关掉原生外观
/// （`appearance(false)`），只保留文字与箭头，边框交给外壳。
///
/// `width` 是触发器宽度：外壳是 `w_full`，宽度必须落在外层包一层，否则边框
/// 会跟着内容列拉满。
pub fn select_field<D>(
    id: impl Into<ElementId>,
    state: &Entity<SelectState<D>>,
    window: &Window,
    cx: &App,
    width: Pixels,
    build: impl FnOnce(Select<D>) -> Select<D>,
) -> Div
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    let focused = state.focus_handle(cx).is_focused(window);
    let select = build(
        Select::new(state)
            .appearance(false)
            .text_size(theme::TYPE_BODY),
    )
    .w_full();
    div()
        .w(width)
        .flex_shrink_0()
        .child(input_shell(id, focused, select))
}

/// 标准多行输入框。与单行输入共用同一层外壳，避免 Textarea 原生外观在
/// 全局焦点环下叠出「外环 + 内环」的双边框。
pub fn text_area(
    id: impl Into<ElementId>,
    state: &Entity<TextareaState>,
    window: &Window,
    cx: &App,
    height: Pixels,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    let state = state.clone();
    input_shell(
        id,
        focused,
        Textarea::new(&state)
            .appearance(false)
            .aria_label(label)
            .h(height)
            .text_size(theme::TYPE_BODY)
            .w_full(),
    )
    .h(height)
    .items_stretch()
    .cursor_text()
    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
        state.update(cx, |state, cx| state.focus(window, cx));
    })
}
