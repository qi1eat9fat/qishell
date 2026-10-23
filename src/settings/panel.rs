//! 设置标签的外壳：分类、可拖动分栏与内容滚动。

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{
    Icon, IndexPath, h_flex,
    input::{InputEvent, InputState},
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
    searchable_list::SearchableVec,
    select::{SelectEvent, SelectState},
    slider::{SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::*;

use crate::components::{AppSlider, layout};
use crate::model::SettingsState;
use crate::theme;

use super::SettingsPage;
use super::appearance_tokens::{
    BACKDROP_MAX, BACKDROP_MIN, BLUR_MAX, BLUR_MIN, CLARITY_MAX, CLARITY_MIN, GLASS_MAX, GLASS_MIN,
    MODAL_MAX, MODAL_MIN, OVERLAY_MAX, OVERLAY_MIN,
};
use super::store::SettingsStore;

pub struct SettingsPanel {
    pub(crate) store: Entity<SettingsStore>,
    pub(crate) page: SettingsPage,
    pub(crate) split: Entity<ResizableState>,

    pub(crate) glass: Entity<SliderState>,
    pub(crate) clarity: Entity<SliderState>,
    pub(crate) blur: Entity<SliderState>,
    pub(crate) overlay_alpha: Entity<SliderState>,
    pub(crate) modal_alpha: Entity<SliderState>,
    pub(crate) backdrop_alpha: Entity<SliderState>,

    pub(crate) terminal_font: Entity<SelectState<SearchableVec<SharedString>>>,
    pub(crate) terminal_size: Entity<InputState>,
    pub(crate) terminal_line_height: Entity<InputState>,
    /// 连接页列出的私钥与信任主机；打开页面或增删后重新读一遍。
    pub(crate) keys: Vec<crate::ssh::StoredKey>,
    pub(crate) known_hosts: Vec<crate::ssh::KnownHost>,
    /// 连接页两个分区各自的一行提示（操作成功或失败）。
    pub(crate) key_notice: Option<SharedString>,
    pub(crate) host_notice: Option<SharedString>,

    pub(crate) editor_path: Entity<InputState>,
    pub(crate) sftp_local_root: Entity<InputState>,
    pub(crate) sftp_max: Entity<InputState>,

    pub(crate) _subscriptions: Vec<Subscription>,
}

/// 终端字体选项：系统里所有等宽字体（按字母序），再加上当前设置里的那一款
/// ——它可能没装在本机，但列表里得能显示成选中项。
pub(crate) fn terminal_font_families(current: &str, cx: &App) -> Vec<SharedString> {
    let mut families: Vec<SharedString> =
        crate::terminal::monospace_font_families(cx.text_system()).to_vec();
    if !families.iter().any(|family| family.as_ref() == current) {
        families.insert(0, SharedString::from(current.to_string()));
    }
    families
}

impl SettingsPanel {
    pub fn new(store: Entity<SettingsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = store.read(cx).settings().clone();
        let appearance = &settings.appearance;

        let glass = slider(
            cx,
            appearance.glass as f32,
            GLASS_MIN as f32,
            GLASS_MAX as f32,
        );
        let clarity = slider(
            cx,
            appearance.clarity as f32,
            CLARITY_MIN as f32,
            CLARITY_MAX as f32,
        );
        let blur = slider(cx, appearance.blur as f32, BLUR_MIN as f32, BLUR_MAX as f32);
        let overlay_alpha = slider(
            cx,
            appearance.overlay_alpha as f32,
            OVERLAY_MIN as f32,
            OVERLAY_MAX as f32,
        );
        let modal_alpha = slider(
            cx,
            appearance.modal_alpha as f32,
            MODAL_MIN as f32,
            MODAL_MAX as f32,
        );
        let backdrop_alpha = slider(
            cx,
            appearance.backdrop_alpha as f32,
            BACKDROP_MIN as f32,
            BACKDROP_MAX as f32,
        );

        let families = terminal_font_families(&settings.terminal.font, cx);
        let font_ix = families
            .iter()
            .position(|family| family.as_ref() == settings.terminal.font)
            .map(|ix| IndexPath::default().row(ix));
        let fonts = SearchableVec::new(families);
        let terminal_font = cx.new(|cx| SelectState::new(fonts, font_ix, window, cx));

        let terminal_size = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.terminal.size.to_string())
        });
        let terminal_line_height = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(format!("{:.1}", settings.terminal.line_height))
        });
        let editor_path =
            cx.new(|cx| InputState::new(window, cx).default_value(settings.editor_path.clone()));
        let sftp_local_root = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.sftp.local_root.clone())
                .placeholder(home_dir_hint())
        });
        let sftp_max = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.sftp.max_transfers.to_string())
        });

        let split = cx.new(|_| ResizableState::default());

        let mut this = Self {
            store: store.clone(),
            page: SettingsPage::Appearance,
            split,
            glass,
            clarity,
            blur,
            overlay_alpha,
            modal_alpha,
            backdrop_alpha,
            terminal_font,
            terminal_size,
            terminal_line_height,
            keys: Vec::new(),
            known_hosts: Vec::new(),
            key_notice: None,
            host_notice: None,
            editor_path,
            sftp_local_root,
            sftp_max,
            _subscriptions: Vec::new(),
        };

        this._subscriptions.push(cx.observe(&store, |_this, _, cx| {
            cx.notify();
        }));

        this.bind_sliders(window, cx);
        this.bind_terminal(window, cx);
        this
    }

    pub(crate) fn update_settings(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut SettingsState),
    ) {
        self.store.update(cx, |store, cx| {
            store.update(cx, change);
        });
        cx.notify();
    }

    fn bind_sliders(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let subscriptions = [
            bind_slider(cx, &self.glass, |appearance, value| {
                appearance.glass = value as u8
            }),
            bind_slider(cx, &self.clarity, |appearance, value| {
                appearance.clarity = value as u8
            }),
            bind_slider(cx, &self.blur, |appearance, value| {
                appearance.blur = value as u8
            }),
            bind_slider(cx, &self.overlay_alpha, |appearance, value| {
                appearance.overlay_alpha = value as u8
            }),
            bind_slider(cx, &self.modal_alpha, |appearance, value| {
                appearance.modal_alpha = value as u8
            }),
            bind_slider(cx, &self.backdrop_alpha, |appearance, value| {
                appearance.backdrop_alpha = value as u8
            }),
        ];
        self._subscriptions.extend(subscriptions);
    }

    fn bind_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.push(cx.subscribe_in(
            &self.terminal_font,
            window,
            |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                if let SelectEvent::Confirm(Some(font)) = event {
                    let font = font.to_string();
                    this.update_settings(cx, |settings| settings.terminal.font = font);
                }
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.terminal_size,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let value = this.terminal_size.read(cx).value().to_string();
                    if let Ok(size) = value.parse::<u8>()
                        && (10..=28).contains(&size)
                    {
                        this.update_settings(cx, |settings| settings.terminal.size = size);
                    }
                }
                InputEvent::Blur => {
                    let value = this.store.read(cx).settings().terminal.size.to_string();
                    this.terminal_size
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                }
                _ => {}
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.terminal_line_height,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let value = this.terminal_line_height.read(cx).value().to_string();
                    if let Ok(line_height) = value.parse::<f32>()
                        && (1.0..=2.2).contains(&line_height)
                    {
                        this.update_settings(cx, |settings| {
                            settings.terminal.line_height = line_height
                        });
                    }
                }
                InputEvent::Blur => {
                    let value =
                        format!("{:.1}", this.store.read(cx).settings().terminal.line_height);
                    this.terminal_line_height
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                }
                _ => {}
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.editor_path,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = this.editor_path.read(cx).value().to_string();
                    this.update_settings(cx, |settings| settings.editor_path = value);
                }
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.sftp_local_root,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let value = this.sftp_local_root.read(cx).value().trim().to_string();
                    this.update_settings(cx, |settings| settings.sftp.local_root = value);
                }
                // 失焦时清掉首尾空白，让输入框和真正存下的值一致。
                InputEvent::Blur => {
                    let value = this.store.read(cx).settings().sftp.local_root.clone();
                    this.sftp_local_root
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                }
                _ => {}
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.sftp_max,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let value = this.sftp_max.read(cx).value().to_string();
                    if let Ok(value) = value.parse::<u8>()
                        && (1..=10).contains(&value)
                    {
                        this.update_settings(cx, |settings| settings.sftp.max_transfers = value);
                    }
                }
                InputEvent::Blur => {
                    let value = this
                        .store
                        .read(cx)
                        .settings()
                        .sftp
                        .max_transfers
                        .to_string();
                    this.sftp_max
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                }
                _ => {}
            },
        ));
    }
}

/// 「本机默认打开路径」留空时的落点：用户主目录。拿不到就只给一句说明。
pub(crate) fn home_dir_hint() -> SharedString {
    match dirs::home_dir() {
        Some(dir) => SharedString::from(dir.display().to_string()),
        None => SharedString::from("留空使用用户主目录"),
    }
}

fn slider(cx: &mut App, value: f32, min: f32, max: f32) -> Entity<SliderState> {
    cx.new(|_| {
        SliderState::new()
            .min(min)
            .max(max)
            .step(1.0)
            .default_value(value)
    })
}

fn bind_slider(
    cx: &mut Context<SettingsPanel>,
    state: &Entity<SliderState>,
    set: fn(&mut crate::model::Appearance, f32),
) -> Subscription {
    cx.subscribe(state, move |this, _, event: &SliderEvent, cx| {
        if let SliderEvent::Change(value) = event {
            let value = value.end();
            this.update_settings(cx, |settings| set(&mut settings.appearance, value));
        }
    })
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.store.read(cx).settings().clone();
        let content = self.render_content(&settings, window, cx);
        let nav = self.render_nav(cx);

        div().size_full().child(
            h_resizable("settings-split")
                .with_state(&self.split)
                .child(
                    resizable_panel()
                        .size(theme::SETTINGS_NAV_WIDTH)
                        .size_range(theme::SETTINGS_NAV_MIN_WIDTH..theme::SETTINGS_NAV_MAX_WIDTH)
                        .child(nav),
                )
                .child(resizable_panel().child(content)),
        )
    }
}

impl SettingsPanel {
    fn render_nav(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let items: Vec<AnyElement> =
            SettingsPage::ALL
                .iter()
                .copied()
                .map(|page| {
                    let active = self.page == page;
                    div()
                        .id(ElementId::Name(
                            format!("settings-page-{}", page as usize).into(),
                        ))
                        .test_support()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(9.))
                        .py(px(8.))
                        .rounded(theme::RADIUS_MD)
                        .border_1()
                        .border_color(if active {
                            theme::colored(theme::FOAM, 0.20)
                        } else {
                            Hsla::default()
                        })
                        .bg(if active {
                            theme::colored(theme::FOAM, 0.08)
                        } else {
                            Hsla::default()
                        })
                        .text_size(theme::TYPE_META)
                        .text_color(theme::solid(if active {
                            theme::TEXT
                        } else {
                            theme::SUBTLE
                        }))
                        .cursor_pointer()
                        .focus_visible(|style| {
                            style
                                .border_color(theme::colored(theme::FOAM, 0.55))
                                .text_color(theme::solid(theme::TEXT))
                        })
                        .hover(|style| {
                            style
                                .text_color(theme::solid(theme::MUTED_STRONG))
                                .bg(theme::hover_subtle())
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_page(page, cx);
                        }))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space" | " ") {
                                cx.stop_propagation();
                                this.open_page(page, cx);
                            }
                        }))
                        .child(Icon::new(page.icon()).size(theme::ICON_SM).text_color(
                            theme::solid(if active { theme::FOAM } else { theme::SUBTLE }),
                        ))
                        .child(page.label())
                        .into_any_element()
                })
                .collect();

        v_flex()
            .h_full()
            .w_full()
            .min_h_0()
            .px(px(10.))
            .py(px(16.))
            .border_r_1()
            .border_color(theme::line())
            .bg(theme::colored(theme::BASE_RGB, theme::PANEL_TINT))
            .child(
                div()
                    .mx(px(8.))
                    .mt(px(2.))
                    .mb(px(16.))
                    .text_size(theme::TYPE_META)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::solid(theme::MUTED_STRONG))
                    .child("设置"),
            )
            .child(
                v_flex()
                    .id("settings-nav-list")
                    .gap(px(2.))
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .children(items),
            )
            .into_any_element()
    }

    fn render_content(
        &mut self,
        settings: &SettingsState,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = match self.page {
            SettingsPage::Appearance => self.render_appearance(settings, window, cx),
            SettingsPage::Terminal => self.render_terminal(settings, window, cx),
            SettingsPage::Connection => self.render_connection(settings, window, cx),
            SettingsPage::Sftp => self.render_sftp(settings, window, cx),
        };

        div()
            .id("settings-content")
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .overflow_y_scrollbar()
            .px(theme::SETTINGS_CONTENT_PX)
            .pt(theme::SETTINGS_CONTENT_PT)
            .pb(theme::SETTINGS_CONTENT_PB)
            .child(layout::content_column(body))
            .into_any_element()
    }

    /// 切到 `page`。连接页每次进入都重新读一遍磁盘上的私钥与信任记录，
    /// 别的地方删了文件也能看到最新的。
    fn open_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        // 换页时清掉连接页的旧提示：它说的是上一次操作，留着会误导。
        self.key_notice = None;
        self.host_notice = None;
        self.page = page;
        if page == SettingsPage::Connection {
            self.reload_connection_files();
        }
        cx.notify();
    }

    /// 清掉连接页的两条提示。设置标签关掉、或重新打开时调用：那两条说的是
    /// 上一次操作，留着会让人以为刚刚又发生了一遍。
    pub(crate) fn clear_notices(&mut self, cx: &mut Context<Self>) {
        if self.key_notice.is_none() && self.host_notice.is_none() {
            return;
        }
        self.key_notice = None;
        self.host_notice = None;
        cx.notify();
    }

    /// 连接页私钥区的提示：操作成功或失败时显示在私钥列表下方。
    pub(crate) fn set_key_notice(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.key_notice = Some(message.into());
        cx.notify();
    }

    /// 连接页信任主机区的提示：显示在主机列表下方。
    pub(crate) fn set_host_notice(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.host_notice = Some(message.into());
        cx.notify();
    }

    pub(crate) fn section(
        &self,
        title: &'static str,
        description: Option<&'static str>,
        rows: Vec<AnyElement>,
    ) -> Div {
        layout::section(title, description, rows)
    }

    /// 页面第一个分区：不画上分隔线。设置页从分区标题直接开始。
    pub(crate) fn leading_section(
        &self,
        title: &'static str,
        description: Option<&'static str>,
        rows: Vec<AnyElement>,
    ) -> Div {
        layout::leading_section(title, description, rows)
    }

    pub(crate) fn row(
        &self,
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
    ) -> AnyElement {
        layout::settings_row(title, description, control)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slider_row(
        &self,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        state: &Entity<SliderState>,
        value: u8,
        unit: &'static str,
        cx: &App,
    ) -> AnyElement {
        self.slider_row_enabled(id, title, description, state, value, unit, true, cx)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slider_row_enabled(
        &self,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        state: &Entity<SliderState>,
        value: u8,
        unit: &'static str,
        enabled: bool,
        cx: &App,
    ) -> AnyElement {
        // 滑杆在窄内容列里无法与说明并排：标签与说明放上方，滑杆占满整行。
        // 这样 960px 最小窗口下也不会把中文说明挤成竖排。说明为空时只留标签。
        let label = v_flex().w_full().min_w_0().gap(px(2.)).child(
            div()
                .text_size(theme::TYPE_META)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::solid(theme::MUTED_STRONG))
                .child(title),
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
        v_flex()
            .w_full()
            .gap(px(7.))
            .py(px(9.))
            .child(label)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        // 给滑杆一个稳定的 id：UI 测试按它定位并点击轨道。
                        // 整行滑杆区域都能点：按点击位置直接换算成值。
                        div()
                            .id(ElementId::Name(format!("slider-{id}").into()))
                            .on_mouse_down(MouseButton::Left, {
                                let state = state.clone();
                                move |event: &MouseDownEvent, window, cx| {
                                    state.update(cx, |state, cx| {
                                        state.update_value_by_position(
                                            gpui_kit::Axis::Horizontal,
                                            event.position,
                                            false,
                                            window,
                                            cx,
                                        );
                                    });
                                }
                            })
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .child(
                                AppSlider::new(state)
                                    .color(theme::solid(theme::FOAM))
                                    .disabled(!enabled),
                            ),
                    )
                    .child(
                        div()
                            .w(px(44.))
                            .flex_shrink_0()
                            .text_right()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_META)
                            .text_color(theme::solid(if enabled {
                                theme::FOAM
                            } else {
                                theme::SUBTLE
                            }))
                            .child(format!("{value}{unit}")),
                    ),
            )
            .into_any_element()
    }
}
