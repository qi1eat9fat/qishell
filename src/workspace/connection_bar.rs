//! 顶栏中间的连接信息。顶栏本身由 `title_bar.rs` 统一绘制。

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::Tab;
use crate::terminal::TerminalLifecycle;
use crate::theme;

use super::Workspace;

impl Workspace {
    /// 当前标签在顶栏中间显示的状态与地址；设置页显示「设置」，无标签返回空。
    pub(super) fn render_connection_info(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.active_tab();
        if tab.is_some_and(Tab::is_settings) {
            return Some(
                div()
                    .text_size(theme::TYPE_SECTION)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("设置")
                    .into_any_element(),
            );
        }

        let is_sftp = tab.is_some_and(Tab::is_sftp);
        let sftp = tab.and_then(|tab| tab.sftp.clone());
        let host = self.active_login()?;
        let sftp_connecting =
            is_sftp && sftp.as_ref().is_some_and(|sftp| sftp.read(cx).connecting());
        let connected = if is_sftp {
            sftp.as_ref().is_some_and(|sftp| sftp.read(cx).connected())
        } else {
            tab.and_then(|tab| tab.terminal.as_ref())
                .is_some_and(|terminal| {
                    matches!(terminal.read(cx).lifecycle(cx), TerminalLifecycle::Running)
                })
        };

        // SFTP 不再把「SFTP」塞进状态词；状态只说连接结果，协议跟在主机名后。
        let (label, dot) = if sftp_connecting {
            ("连接中", theme::GOLD)
        } else if connected {
            ("已连接", theme::FOAM)
        } else {
            ("已断开", theme::GOLD)
        };

        let chip = h_flex()
            .id("connection-state")
            .aria_label(label)
            .gap(px(5.))
            .px(px(8.))
            .py(px(2.))
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(theme::colored(theme::FOAM, 0.22))
            .bg(theme::colored(theme::FOAM, 0.12))
            .text_size(theme::TYPE_MICRO)
            .text_color(theme::solid(theme::FOAM))
            .child(
                div()
                    .size(px(7.))
                    .rounded_full()
                    .bg(theme::solid(dot))
                    .shadow(theme::ring(theme::solid(dot))),
            )
            .child(label);

        Some(
            h_flex()
                .min_w_0()
                .h_full()
                .items_center()
                .gap(px(10.))
                .window_control_area(WindowControlArea::Drag)
                .child(chip.flex_shrink_0())
                .child(
                    h_flex()
                        .min_w_0()
                        .items_center()
                        .gap(px(9.))
                        .child(
                            h_flex()
                                .min_w_0()
                                .max_w(px(260.))
                                .truncate()
                                .gap(px(0.))
                                .text_size(theme::TYPE_SECTION)
                                .font_weight(FontWeight::SEMIBOLD)
                                // 名字里可能有 emoji（国旗），单独用 Twemoji 画；
                                // SFTP 的「· SFTP」跟在名字后面，仍在同一行。
                                .child(crate::components::emoji::with_flags(
                                    host.name.clone(),
                                    theme::TYPE_SECTION,
                                ))
                                .when(is_sftp, |row| row.child(" · SFTP")),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(cx.theme().font_family.clone())
                                .text_size(theme::TYPE_META)
                                .text_color(theme::solid(theme::SUBTLE))
                                .child(host.endpoint()),
                        ),
                )
                .into_any_element(),
        )
    }
}
