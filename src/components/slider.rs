//! 设置页的滑杆：轨道 3px、拇指 10px，比 gpui-component 默认的 6px / 16px
//! 更紧凑。行为仍由 gpui-base 的无样式滑杆负责——拖动、点击轨道、键盘与
//! 读屏都在那边，这里只画外观。

use gpui_kit::base::{Slider as BaseSlider, SliderIndicator, SliderThumb, SliderTrack};
use gpui_kit::component::slider::SliderState;
use gpui_kit::*;

/// 轨道高度。
pub const TRACK_HEIGHT: Pixels = px(3.);
/// 拇指直径。
pub const THUMB_SIZE: Pixels = px(10.);
/// 拇指比轨道高出的部分，上下各一半，让它骑在轨道正中。
const THUMB_OVERHANG: Pixels = px(3.5);
/// 整行高度：给指针留出比轨道大得多的命中区。
const ROW_HEIGHT: Pixels = px(18.);

#[derive(IntoElement)]
pub struct AppSlider {
    state: Entity<SliderState>,
    color: Hsla,
    disabled: bool,
}

impl AppSlider {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            color: Hsla::default(),
            disabled: false,
        }
    }

    /// 已填充的轨道与拇指共用的颜色。
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = color;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for AppSlider {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state;
        let percentage = state.read(cx).percentage();
        let color = self.color;
        let disabled = self.disabled;

        BaseSlider::new(&state)
            .disabled(disabled)
            .flex()
            .flex_1()
            .items_center()
            .h(ROW_HEIGHT)
            .w_full()
            .child(
                SliderTrack::new(&state)
                    .disabled(disabled)
                    .flex()
                    .items_center()
                    .h(ROW_HEIGHT)
                    .w_full()
                    .flex_shrink_0()
                    .child(
                        SliderIndicator::new(&state)
                            .relative()
                            .w_full()
                            .h(TRACK_HEIGHT)
                            .rounded_full()
                            .bg(color.opacity(0.2))
                            .child(
                                div()
                                    .absolute()
                                    .h_full()
                                    .left(relative(percentage.start))
                                    .right(relative(1. - percentage.end))
                                    .rounded_full()
                                    .bg(color),
                            )
                            .child(
                                SliderThumb::new(&state)
                                    .disabled(disabled)
                                    .absolute()
                                    .top(-THUMB_OVERHANG)
                                    .left(relative(percentage.end))
                                    .ml(-THUMB_SIZE / 2.)
                                    .size(THUMB_SIZE)
                                    .rounded_full()
                                    .bg(color),
                            ),
                    ),
            )
    }
}
