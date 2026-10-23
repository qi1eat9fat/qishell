//! 设置 › 终端：字体、配色与预览。

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::h_flex;
use gpui_kit::*;

use crate::components::field;
use crate::model::{SettingsState, TerminalSettings};
use crate::terminal::{TerminalColors, TerminalTheme};
use crate::theme;

use super::SettingsPanel;

impl SettingsPanel {
    pub(super) fn render_terminal(
        &mut self,
        settings: &SettingsState,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut body = Vec::new();

        body.push(
            self.leading_section(
                "字体",
                None,
                vec![
                    self.row(
                        "字体",
                        "等宽字体",
                        field::select_field(
                            "terminal-font",
                            &self.terminal_font,
                            window,
                            cx,
                            px(210.),
                            |select| {
                                select
                                    .accessibility_label("终端字体")
                                    .placeholder("选择字体")
                            },
                        ),
                    ),
                    self.row(
                        "字号",
                        "10–28 px",
                        h_flex()
                            .gap(px(7.))
                            .child(field::text_field_with(
                                "terminal-size",
                                &self.terminal_size,
                                window,
                                cx,
                                // 纯数字字段居中，与 SFTP 页的步进器一致。
                                |input| {
                                    input
                                        .aria_label("终端字号")
                                        .w(px(62.))
                                        .text_align(TextAlign::Center)
                                },
                            ))
                            .child(
                                div()
                                    .font_family(cx.theme().font_family.clone())
                                    .text_size(theme::TYPE_META)
                                    .text_color(theme::solid(theme::SUBTLE))
                                    .child("px"),
                            ),
                    ),
                    self.row(
                        "行高",
                        "1.0–2.2",
                        field::text_field_with(
                            "terminal-line-height",
                            &self.terminal_line_height,
                            window,
                            cx,
                            |input| {
                                input
                                    .aria_label("终端行高")
                                    .w(px(62.))
                                    .text_align(TextAlign::Center)
                            },
                        ),
                    ),
                ],
            )
            .into_any_element(),
        );

        body.push(self.render_terminal_colors(settings, cx).into_any_element());
        body.push(
            self.section(
                "预览",
                None,
                vec![self.render_preview(&settings.terminal, cx)],
            )
            .into_any_element(),
        );

        body
    }

    /// 终端配色：主题卡片网格。卡片只画名字与 16 色，字与配色一起看的地方
    /// 是页面末尾的「预览」。
    fn render_terminal_colors(&self, settings: &SettingsState, cx: &mut Context<Self>) -> Div {
        // 已下线的旧 key（老设置里可能还留着 solarized-dark 之类）与空 key
        // 一样按「默认」处理：投影到默认那张卡上，否则一张都不高亮。
        let selected = TerminalTheme::find(&settings.terminal.theme).map_or("", TerminalTheme::key);
        let mut cards = Vec::with_capacity(1 + TerminalTheme::all().len());
        cards.push(self.terminal_theme_card("", "默认（跟随应用）", selected, cx));
        for theme in TerminalTheme::all() {
            cards.push(self.terminal_theme_card(theme.key(), theme.name(), selected, cx));
        }
        self.section(
            "配色",
            None,
            vec![
                div()
                    .grid()
                    .grid_cols(3)
                    .gap(px(8.))
                    .children(cards)
                    .into_any_element(),
            ],
        )
    }

    fn terminal_theme_card(
        &self,
        key: &'static str,
        name: &'static str,
        selected: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let terminal = TerminalTheme::selected(key);
        let is_selected = selected == key;
        let id = if key.is_empty() { "default" } else { key };
        let bars = (0..8)
            .map(|index| {
                div()
                    .flex_1()
                    .h(px(5.))
                    .rounded(px(2.5))
                    .bg(terminal.indexed(index))
            })
            .collect::<Vec<_>>();

        div()
            .id(ElementId::Name(format!("terminal-theme-{id}").into()))
            .test_support()
            .tab_index(0)
            .p(px(10.))
            .rounded(theme::RADIUS_LG)
            .border_1()
            .border_color(if is_selected {
                theme::solid(theme::FOAM)
            } else {
                theme::line()
            })
            .bg(terminal.background())
            .cursor_pointer()
            .focus_visible(|style| style.border_color(theme::solid(theme::FOAM)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.update_settings(cx, |settings| settings.terminal.theme = key.to_string());
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space" | " ") {
                    cx.stop_propagation();
                    this.update_settings(cx, |settings| {
                        settings.terminal.theme = key.to_string();
                    });
                }
            }))
            .child(
                div()
                    .text_size(theme::TYPE_META)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(terminal.foreground())
                    .child(name),
            )
            .child(h_flex().gap(px(3.)).mt(px(9.)).children(bars))
            .into_any_element()
    }

    /// 预览：当前字体与当前配色画在一起的几行输出。
    ///
    /// 字体与配色分开调，效果却要一起看：字号、行高与字族的松紧只有落在
    /// 真实文字上才看得出来，配色也一样。所以放在两者之后、页面最后。
    fn render_preview(&self, terminal: &TerminalSettings, cx: &App) -> AnyElement {
        let terminal_theme = TerminalColors::current(cx);
        let line = |user: &'static str,
                    path: &'static str,
                    symbol: &'static str,
                    command: &'static str| {
            h_flex()
                .gap_0()
                .child(div().text_color(terminal_theme.indexed(2)).child(user))
                .child(div().text_color(terminal_theme.foreground()).child(path))
                .child(div().text_color(terminal_theme.indexed(3)).child(symbol))
                .child(div().text_color(terminal_theme.foreground()).child(command))
        };

        div()
            .p(px(14.))
            .rounded(theme::RADIUS_LG)
            .border_1()
            .border_color(theme::line())
            .bg(terminal_theme.background())
            .text_color(terminal_theme.foreground())
            .font_family(terminal.font.clone())
            .text_size(px(terminal.size as f32))
            .line_height(px(terminal.size as f32 * terminal.line_height))
            .child(line("ops", "@atlas:~ ", "%", " git status --short"))
            .child(
                div()
                    .text_color(terminal_theme.indexed(8))
                    .child(" M services/edge-router"),
            )
            .child(
                div()
                    .text_color(terminal_theme.indexed(8))
                    .child("?? snapshots/2024-10-17/"),
            )
            .into_any_element()
    }
}
