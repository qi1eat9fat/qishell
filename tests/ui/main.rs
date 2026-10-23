//! 原型 UI 的集成测试：用真实窗口驱动点击、输入与动作。

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, ScrollDelta, TestAppContext, WindowHandle, point, px, size,
};
use qishell::connection::{
    AuthenticationQuestion, ConnectionPrompt, ConnectionPromptKind, ConnectionPromptReply,
    SharedConnectionTester,
};
use qishell::host::{AuthKind, HostId};
use qishell::secrets::{InMemorySecretStore, NoSecretStore, SharedSecretStore};
use qishell::settings::SettingsStore;
use qishell::ssh::{SshConnectionTester, SshConnector};
use qishell::terminal::{
    FixedRemoteTerminalTransportProvider, SharedRemoteTerminalTransportProvider, TerminalTransport,
    TerminalTransportCommand, TerminalTransportEvent, TerminalTransportFactory,
};
use qishell::workspace::Workspace;

/// 假终端：连上就打印一行，然后把输入原样回显。
struct EchoTransport;

impl TerminalTransport for EchoTransport {
    fn run(
        self: Box<Self>,
        _initial_size: qishell::terminal::TerminalSize,
        commands: std::sync::mpsc::Receiver<TerminalTransportCommand>,
        events: async_channel::Sender<TerminalTransportEvent>,
    ) -> anyhow::Result<()> {
        let _ = events.send_blocking(TerminalTransportEvent::Started);
        let _ = events.send_blocking(TerminalTransportEvent::Output(
            b"Connected to test host\r\n".to_vec(),
        ));
        while let Ok(command) = commands.recv() {
            match command {
                TerminalTransportCommand::Write(bytes) => {
                    let _ = events.send_blocking(TerminalTransportEvent::Output(bytes.clone()));
                    if bytes.contains(&13) {
                        let mut marker = vec![13u8, 10u8];
                        marker.extend_from_slice(b"ENTER_SEEN");
                        marker.extend_from_slice(&[13, 10]);
                        let _ = events.send_blocking(TerminalTransportEvent::Output(marker));
                    }
                }
                TerminalTransportCommand::Shutdown => break,
                _ => {}
            }
        }
        Ok(())
    }
}

struct EchoFactory;

impl TerminalTransportFactory for EchoFactory {
    fn create(&self) -> Box<dyn TerminalTransport> {
        Box::new(EchoTransport)
    }
}

/// 先问主机密钥，再问密码；密码正确才打印欢迎语。
struct PromptingTransport;

impl TerminalTransport for PromptingTransport {
    fn run(
        self: Box<Self>,
        _initial_size: qishell::terminal::TerminalSize,
        commands: std::sync::mpsc::Receiver<TerminalTransportCommand>,
        events: async_channel::Sender<TerminalTransportEvent>,
    ) -> anyhow::Result<()> {
        let _ = events.send_blocking(TerminalTransportEvent::Started);
        let _ = events.send_blocking(TerminalTransportEvent::Prompt(ConnectionPrompt::new(
            1,
            ConnectionPromptKind::unknown_host(
                "10.0.0.1",
                22,
                "ED25519",
                "SHA256:cxN98YJ1hoWsjPSRTq3UamJUHSiMDoVxydPEFaZBDS",
            ),
        )));
        while let Ok(command) = commands.recv() {
            match command {
                TerminalTransportCommand::PromptReply { reply, .. } => match reply {
                    ConnectionPromptReply::TrustAndSave => {
                        let _ = events.send_blocking(TerminalTransportEvent::Prompt(
                            ConnectionPrompt::new(
                                2,
                                ConnectionPromptKind::authentication(
                                    "10.0.0.1",
                                    22,
                                    "ops",
                                    "输入密码",
                                    "请输入登录密码。",
                                    vec![AuthenticationQuestion {
                                        text: "密码".into(),
                                        echo: false,
                                    }],
                                ),
                            ),
                        ));
                    }
                    ConnectionPromptReply::Answers(answers) => {
                        let correct = answers
                            .first()
                            .is_some_and(|answer| answer.expose() == "secret");
                        let _ = events.send_blocking(TerminalTransportEvent::Output(if correct {
                            b"welcome\r\n".to_vec()
                        } else {
                            b"denied\r\n".to_vec()
                        }));
                    }
                    ConnectionPromptReply::Cancel => {
                        let _ = events
                            .send_blocking(TerminalTransportEvent::Failed("认证已取消".into()));
                    }
                },
                TerminalTransportCommand::Shutdown => break,
                _ => {}
            }
        }
        Ok(())
    }
}

struct PromptingFactory;

impl TerminalTransportFactory for PromptingFactory {
    fn create(&self) -> Box<dyn TerminalTransport> {
        Box::new(PromptingTransport)
    }
}

fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, WindowHandle<Root>) {
    open_workspace_with(cx, Arc::new(EchoFactory))
}

fn open_workspace_with(
    cx: &mut TestAppContext,
    factory: Arc<dyn TerminalTransportFactory>,
) -> (Entity<Workspace>, WindowHandle<Root>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let mut workspace = None;
    let window = cx.open_window(size(px(1280.), px(800.)), |window, cx| {
        let settings = cx.new(|_| SettingsStore::in_memory());
        let hosts = cx.new(|_| qishell::host::seed());
        let connector = Arc::new(SshConnector::new(
            std::env::temp_dir().join("qishell-ui-test-known_hosts"),
            Arc::new(NoSecretStore),
        ));
        let tester: SharedConnectionTester =
            Arc::new(SshConnectionTester::new(connector.as_ref().clone()));
        let remote: SharedRemoteTerminalTransportProvider =
            Arc::new(FixedRemoteTerminalTransportProvider::new(factory));
        let view =
            cx.new(|cx| Workspace::with_services(settings, hosts, tester, remote, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    (
        workspace.expect("workspace is built in the window closure"),
        window,
    )
}

fn open_workspace_with_secrets(
    cx: &mut TestAppContext,
) -> (
    Entity<Workspace>,
    WindowHandle<Root>,
    Arc<InMemorySecretStore>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let mut workspace = None;
    let secrets = Arc::new(InMemorySecretStore::new());
    let shared: SharedSecretStore = secrets.clone();
    let window = cx.open_window(size(px(1280.), px(800.)), |window, cx| {
        let settings = cx.new(|_| SettingsStore::in_memory());
        let hosts = cx.new(|_| qishell::host::seed().with_secrets(shared.clone()));
        let connector = Arc::new(SshConnector::new(
            std::env::temp_dir().join("qishell-ui-test-known_hosts"),
            Arc::new(NoSecretStore),
        ));
        let tester: SharedConnectionTester =
            Arc::new(SshConnectionTester::new(connector.as_ref().clone()));
        let remote: SharedRemoteTerminalTransportProvider = Arc::new(
            FixedRemoteTerminalTransportProvider::new(Arc::new(EchoFactory)),
        );
        let view =
            cx.new(|cx| Workspace::with_services(settings, hosts, tester, remote, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    (
        workspace.expect("workspace is built in the window closure"),
        window,
        secrets,
    )
}

/// 推进终端引擎的 16ms 轮询，让假终端的事件画到屏幕上。
fn settle(cx: &mut TestAppContext) {
    for _ in 0..20 {
        // 传输在自己的线程上跑，先让它有机会把事件放进通道。
        std::thread::sleep(Duration::from_millis(1));
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn starts_with_the_host_list_and_no_tab(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = workspace.read(cx).test_state(cx);
        assert_eq!(state.hosts, 3);
        assert_eq!(state.tabs, 0);
        assert!(window.try_find("start-hint").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_host_panel_collapses_to_a_rail_and_restores(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(!workspace.read(cx).test_state(cx).host_panel_collapsed);
        assert!(window.try_find("host-1").is_some());

        window.click("host-panel-toggle", cx);
        window.render_frame(cx);
        let state = workspace.read(cx).test_state(cx);
        assert!(state.host_panel_collapsed);
        assert!(
            window.try_find("host-1").is_none(),
            "收起的窄栏不再渲染主机列表"
        );

        window.click("host-panel-toggle", cx);
        window.render_frame(cx);
        assert!(!workspace.read(cx).test_state(cx).host_panel_collapsed);
        assert!(window.try_find("host-1").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn double_clicking_a_host_opens_its_tab(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);

        let state = workspace.read(cx).test_state(cx);
        assert_eq!(state.tabs, 1);
        assert_eq!(state.active_host.as_deref(), Some("Atlas production"));
        assert!(
            workspace
                .read(cx)
                .test_terminal_text(cx)
                .contains("Connected to test host"),
            "the terminal shows the transport's output"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_settings_button_opens_one_settings_tab(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);

        let state = workspace.read(cx).test_state(cx);
        assert_eq!(state.settings_tabs, 1);
        assert_eq!(state.tabs, 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_the_last_tab_returns_to_the_start_page(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);
        window.click("tab-close-1", cx);
        window.render_frame(cx);

        assert_eq!(workspace.read(cx).test_state(cx).tabs, 0);
        assert!(window.try_find("start-hint").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn saves_a_host_through_the_dialog(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        window.click("host-name", cx);
        window.input("lab box", cx);
        window.click("host-address", cx);
        window.input("10.9.9.9", cx);
        assert_eq!(window.find("host-name").value(), Some("lab box"));
        assert_eq!(window.find("host-address").value(), Some("10.9.9.9"));
        window.click("save-host", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("save-host").is_none(),
            "the dialog closes after saving"
        );

        let workspace = workspace.read(cx);
        assert_eq!(workspace.test_state(cx).hosts, 4);
        assert_eq!(
            workspace.test_host(cx, "lab box"),
            Some(("10.9.9.9".into(), AuthKind::Password, None))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_flag_typed_into_the_name_field_is_shown_as_a_picture(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // `subscribe_in` 的订阅是惰性的：`new_subscription` 用 `defer` 激活它，
    // 而 defer 要等这一轮 update 结束才跑。对话框和输入必须分在两轮里，
    // 否则订阅还没生效就开始打字。
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("host-name", cx);
        // 输入框画不了国旗（区域指示符在 Windows 上没有彩色轮廓），所以敲
        // 进去的旗要立刻从文字里摘出来，另画成图片；输入框只留名字本身。
        window.input("\u{1F1ED}\u{1F1F0}lab box", cx);
    })
    .unwrap();
    // 摘旗在订阅回调里做，回调由 `flush_effects` 派发，而它只在这一轮
    // update 结束时运行——所以要在下一轮里看结果。
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("host-name").value(),
            Some("lab box"),
            "the flag must not stay in the input text"
        );
        window.click("host-address", cx);
        window.input("10.9.9.12", cx);
        window.click("save-host", cx);
        window.render_frame(cx);

        // 保存时旗要接回名字前面，用户看到的和存下来的一致。
        let workspace = workspace.read(cx);
        assert!(
            workspace
                .test_host(cx, "\u{1F1ED}\u{1F1F0}lab box")
                .is_some(),
            "the saved name keeps its leading flag"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_host_opens_a_tab_after_saving(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        window.click("host-name", cx);
        window.input("secure box", cx);
        window.click("host-address", cx);
        window.input("10.9.9.10", cx);
        window.click("save-host", cx);
        window.render_frame(cx);

        window.double_click("host-4", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);

        let state = workspace.read(cx).test_state(cx);
        assert_eq!(state.active_host.as_deref(), Some("secure box"));
        assert_eq!(state.tabs, 1);
        assert!(
            workspace
                .read(cx)
                .test_terminal_text(cx)
                .contains("Connected to test host"),
            "a new host opens a working terminal"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn cancels_the_host_dialog(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(window.try_find("cancel-host").is_some());
        window.click("cancel-host", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("cancel-host").is_none(),
            "cancel closes the dialog"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn switches_the_authentication_method(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(window.try_find("host-password").is_some());
        window.click("auth-method-密钥", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("host-key-path").is_some(),
            "the key field appears"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_key_field_lists_imported_keys_beside_browsing_for_a_file(cx: &mut TestAppContext) {
    // 私钥不能只是一个文件输入框：已经导入到 `keys/` 的密钥要能直接选。
    // 这里只验证下拉里同时有「已导入的密钥」和「浏览文件…」两类选项，
    // 文件对话框本身由系统弹出，测试里不点它。
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        window.click("auth-method-密钥", cx);
        window.render_frame(cx);

        // 下拉触发器在，且显示的是占位文字（还没选私钥）。
        let trigger = window.find("host-key-path");
        assert_eq!(trigger.label(), Some("私钥文件"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_key_already_in_the_key_dir_is_used_as_is(_cx: &mut TestAppContext) {
    // 从下拉里选已导入的私钥时，`adopt_key` 不该再复制一份——它已经在
    // `keys/` 里了，原样返回即可。
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    let key = keys.join("test_key");
    std::fs::write(&key, b"placeholder").unwrap();

    assert_eq!(qishell::ssh::vault::adopt_key(&keys, &key), Some(key));
}

#[gpui_kit::test]
fn the_host_dialog_focuses_the_name_and_tab_advances(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        assert_eq!(window.find("host-name").focused(), Some(true));

        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("host-address").focused(), Some(true));

        window.press("shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("host-name").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn deleting_a_host_keeps_its_open_tab(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.test_mark_removed(HostId(1), cx)
        });
        window.render_frame(cx);

        let state = workspace.read(cx).test_state(cx);
        assert_eq!(state.hosts, 2);
        assert_eq!(state.active_tab_label.as_deref(), Some("Atlas production"));
        assert_eq!(state.active_host.as_deref(), Some("Atlas production"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_focused_host_row_connects_with_enter(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("host-2", cx);
        window.render_frame(cx);
        window.press("enter", cx);
        window.render_frame(cx);

        assert_eq!(
            workspace.read(cx).test_state(cx).active_host.as_deref(),
            Some("Aurora staging")
        );
    })
    .unwrap();
}

/// 空的开始页用 Enter 打开选中主机；终端里的 Enter 属于 shell。
#[gpui_kit::test]
fn enter_reaches_the_terminal_instead_of_opening_a_host(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let workspace = workspace.read(cx);
        assert!(
            workspace.test_terminal_text(cx).contains("ENTER_SEEN"),
            "回车应作为 \\r 发给 shell"
        );
        assert_eq!(
            workspace.test_state(cx).tabs,
            1,
            "回车不该再打开一个主机标签"
        );
    })
    .unwrap();
}

/// 设置页在回车键上没有「打开主机」这一层，Enter 只属于页面自己的控件。
#[gpui_kit::test]
fn enter_on_the_settings_page_opens_no_host(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        window.press("enter", cx);
        window.render_frame(cx);

        let state = workspace.read(cx).test_state(cx);
        assert_eq!(state.tabs, 1);
        assert_eq!(state.settings_tabs, 1);
        assert_eq!(state.active_host, None, "设置页的回车不该连主机");
    })
    .unwrap();
}

/// 右键不用先去设置里挑行为：选着文字时复制并清掉选区，没选中时把剪贴板
/// 贴到输入行。
#[gpui_kit::test]
fn right_click_copies_a_selection_and_pastes_without_one(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("PASTED0".into())));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.right_click(("terminal", 1u64), cx);
        window.render_frame(cx);
        assert!(
            workspace
                .read(cx)
                .test_terminal_text(cx)
                .contains("PASTED0"),
            "没有选中时右键应把剪贴板贴给 shell"
        );
        // 用真实拖拽建选区：键盘全选在终端里被 shell 收走了。
        let bounds = window.find(("terminal", 1u64)).bounds();
        window.drag(
            point(bounds.left() + px(4.), bounds.top() + px(4.)),
            point(bounds.left() + px(300.), bounds.top() + px(40.)),
            cx,
        );
        window.render_frame(cx);
        window.right_click(("terminal", 1u64), cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);

    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert!(
        copied.unwrap_or_default().contains("to test host"),
        "有选中时右键应把它复制到剪贴板"
    );

    cx.update(|cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("PASTED".into()));
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.right_click(("terminal", 1u64), cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            workspace.read(cx).test_terminal_text(cx).contains("PASTED"),
            "复制后选区已清掉，再右键应粘贴"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_tab_closes_from_its_close_button(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);
        window.click("tab-close-1", cx);
        window.render_frame(cx);

        assert_eq!(workspace.read(cx).test_state(cx).tabs, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_and_window_controls_use_the_standard_hit_targets(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);

        // 连接操作已从顶栏移除；设置入口移到右侧栏，窗口按钮仍在顶栏。
        assert!(window.try_find("copy-address").is_none());
        assert!(window.try_find("split-terminal").is_none());
        assert!(window.try_find("toggle-connection").is_none());

        // 紧凑图标按钮统一使用 28×28 命中区。
        for id in [
            "open-settings",
            "minimize-window",
            "maximize-window",
            "close-window",
        ] {
            assert_eq!(
                window.find(id).bounds().size,
                size(px(28.), px(28.)),
                "{id}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_connection_asks_for_the_host_key_and_password(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace_with(cx, Arc::new(PromptingFactory));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.double_click("host-1", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("trust").is_some(),
            "the host key dialog opens"
        );
        window.click("trust", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("connect").is_some(),
            "the password dialog opens"
        );
        window.input("secret", cx);
        window.click("connect", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            workspace
                .read(cx)
                .test_terminal_text(cx)
                .contains("welcome"),
            "the password answer reaches the transport"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_password_saved_in_the_host_dialog_goes_to_the_secret_store(cx: &mut TestAppContext) {
    let (_workspace, window, secrets) = open_workspace_with_secrets(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        window.click("host-name", cx);
        window.input("pw box", cx);
        window.click("host-address", cx);
        window.input("10.9.9.11", cx);
        window.click("host-password", cx);
        window.input("hunter2", cx);
        window.click("save-host", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        secrets.account_names(),
        vec!["password:root@10.9.9.11:22".to_string()],
        "用户名留空按默认 root 保存，密码写进秘密库"
    );
}

#[gpui_kit::test]
fn the_right_rail_opens_one_sftp_tab(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);

        let state = workspace.read(cx).test_state(cx);
        assert_eq!(state.tabs, 1);
        assert_eq!(
            state.active_tab_label.as_deref(),
            Some("Atlas production · SFTP")
        );

        // 同一主机只保留一个 SFTP 标签。
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).test_state(cx).tabs, 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn sftp_header_and_size_column_share_the_same_right_edge(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
        let header = window
            .try_find("sftp-local-header-Size")
            .expect("size header")
            .bounds();
        let cell = window
            .try_find("sftp-local-size-0")
            .expect("size cell")
            .bounds();
        assert_eq!(
            header.right(),
            cell.right(),
            "header {header:?} and cell {cell:?} must align"
        );

        let header = window
            .try_find("sftp-local-header-Modified")
            .expect("modified header")
            .bounds();
        let cell = window
            .try_find("sftp-local-modified-0")
            .expect("modified cell")
            .bounds();
        assert_eq!(
            header.left(),
            cell.left(),
            "modified header {header:?} and cell {cell:?} must align"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sftp_header_click_cycles_sort_direction(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);

        let size_header = |window: &gpui_kit::Window| {
            window
                .find("sftp-local-header-Size")
                .label()
                .map(str::to_string)
        };
        assert_eq!(size_header(window).as_deref(), Some("按大小排序"));

        window.click("sftp-local-header-Size", cx);
        window.render_frame(cx);
        assert_eq!(size_header(window).as_deref(), Some("大小，降序"));

        window.click("sftp-local-header-Size", cx);
        window.render_frame(cx);
        assert_eq!(size_header(window).as_deref(), Some("大小，升序"));

        window.click("sftp-local-header-Size", cx);
        window.render_frame(cx);
        assert_eq!(size_header(window).as_deref(), Some("大小，降序"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_a_local_file_to_the_remote_pane_enqueues_a_batch(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);

        let from = window
            .try_find("sftp-local-draggable-0")
            .expect("a local row to drag")
            .bounds()
            .center();
        let to = window
            .try_find("sftp-remote-pane")
            .expect("remote pane")
            .bounds()
            .center();
        window.drag(from, to, cx);
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).test_sftp_batch_count(cx),
            1,
            "dropping on the remote pane creates a transfer batch"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sftp_conflict_dialog_offers_skip_and_overwrite(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
    })
    .unwrap();

    cx.update(|cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.test_emit_sftp_conflict(cx);
        });
    });
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("sftp-conflict-skip").is_some(),
            "skip button is shown"
        );
        assert!(
            window.try_find("sftp-conflict-overwrite").is_some(),
            "overwrite button is shown"
        );
        assert!(
            window.try_find("sftp-conflict-apply-all").is_some(),
            "apply-to-all toggle is shown"
        );

        window.click("sftp-conflict-skip", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("sftp-conflict-skip").is_none(),
            "choosing skip closes the dialog"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sftp_context_menu_deletes_and_creates_local_entries(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("old.txt"), b"hello").expect("old file");
    let (workspace, window) = open_workspace(cx);

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.test_set_sftp_local_path(directory.path().to_path_buf(), cx);
        });
        window.render_frame(cx);

        // 父级「..」单独渲染，old.txt 是索引 0。
        window.right_click("sftp-local-row-0", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("sftp-menu-delete").is_some(),
            "context menu is shown"
        );
        window.click("sftp-menu-delete", cx);
        window.render_frame(cx);
        window.click("sftp-delete-confirm", cx);
        window.render_frame(cx);
        assert!(
            !directory.path().join("old.txt").exists(),
            "delete removes the file"
        );

        // 右键「..」等同于空白处 → 新建文件夹。
        window.right_click("sftp-local-parent", cx);
        window.render_frame(cx);
        window.click("sftp-menu-new-folder", cx);
        window.render_frame(cx);
        window.input("new-folder", cx);
        window.render_frame(cx);
        window.click("sftp-name-confirm", cx);
        window.render_frame(cx);
        assert!(
            directory.path().join("new-folder").is_dir(),
            "new folder is created"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_host_dialog_keeps_keyboard_focus_inside(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);

        // 对话框打开后，连续 Tab 不应把焦点送回背后的标题栏或主机列表。
        for _ in 0..30 {
            window.press("tab", cx);
            window.render_frame(cx);
        }
        assert_eq!(
            window.find("open-settings").focused(),
            Some(false),
            "焦点逃出了对话框"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_leaves_the_notes_textarea(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-host", cx);
        window.render_frame(cx);
        window.click("host-notes-field", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("host-name").focused(),
            Some(false),
            "点击备注后焦点应离开名称"
        );
        // 备注 → 认证方式「密码」→「密钥」→ 密码输入框。
        for _ in 0..3 {
            window.press("tab", cx);
            window.render_frame(cx);
        }

        assert_eq!(
            window.find("host-password").focused(),
            Some(true),
            "Tab 应离开备注框，继续沿表单焦点顺序前进"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn host_menu_supports_keyboard(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.right_click("host-1", cx);
        window.render_frame(cx);
        assert!(
            workspace.read(cx).test_state(cx).host_menu_open,
            "右键后菜单状态应存在"
        );

        window.press("escape", cx);
        window.render_frame(cx);
        assert!(
            !workspace.read(cx).test_state(cx).host_menu_open,
            "Esc 应关闭菜单"
        );

        // 方向键选中第一项「编辑主机」，Enter 打开编辑对话框。
        window.right_click("host-1", cx);
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("host-notes-field").is_some(),
            "Enter 应执行第一项「编辑主机」"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sftp_menu_supports_keyboard(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("old.txt"), b"hello").expect("old file");
    let (workspace, window) = open_workspace(cx);

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.test_set_sftp_local_path(directory.path().to_path_buf(), cx);
        });
        window.render_frame(cx);

        window.right_click("sftp-local-row-0", cx);
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("sftp-name-confirm").is_some(),
            "Down + Enter 应执行第一项「新建文件夹」"
        );

        window.press("escape", cx);
        window.render_frame(cx);
        window.right_click("sftp-local-row-0", cx);
        window.render_frame(cx);
        window.press("end", cx);
        window.render_frame(cx);
        window.press("up", cx);
        window.render_frame(cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("sftp-delete-confirm").is_some(),
            "End + Up + Enter 应执行「删除」"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn debug_alignment_bounds(cx: &mut TestAppContext) {
    let (_workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
        let from = window
            .try_find("sftp-local-draggable-0")
            .expect("row")
            .bounds()
            .center();
        let to = window
            .try_find("sftp-remote-pane")
            .expect("pane")
            .bounds()
            .center();
        window.drag(from, to, cx);
        window.render_frame(cx);
        for id in [
            "sftp-copy-path-remote",
            "sftp-refresh-remote",
            "sftp-remove-1",
            "sftp-pause-1",
            "sftp-clear-completed",
            "sftp-toggle-queue",
        ] {
            match window.try_find(id) {
                Some(el) => {
                    let b = el.bounds();
                    println!(
                        "{id}: left={:?} right={:?} cx={:?}",
                        b.left(),
                        b.right(),
                        b.center().x
                    );
                }
                None => println!("{id}: missing"),
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn appearance_sliders_and_reset_reach_the_store(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        window.click("settings-page-0", cx);
        window.render_frame(cx);

        // 滑杆写的是 store，而不只是拇指位置：点轨道 60% 处后值必须变。
        let bounds = window.find("slider-glass").bounds();
        window.click_at(
            "slider-glass",
            point(
                px(bounds.size.width.as_f32() * 0.6),
                px(bounds.size.height.as_f32() * 0.5),
            ),
            cx,
        );
    })
    .unwrap();
    // 订阅回调由 gpui 在事件循环末尾派发。
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let glass = workspace.read(cx).test_state(cx).glass;
        assert!(glass > 5, "玻璃滑杆应写回设置，实际 {glass}");

        // 设置内容本身的滚动容器改写了元素 id，不可直接观察；以内容里的
        // 滑杆为落点滚轮，滚到页面末尾的恢复默认。
        window.scroll(
            "slider-glass",
            ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            cx,
        );
        window.render_frame(cx);

        window.click("reset-appearance", cx);
        window.render_frame(cx);
        let state = workspace.read(cx).test_state(cx);
        // 恢复默认走的是常量，产品调参时这里不用跟着改。
        assert_eq!(
            state.glass,
            qishell::settings::appearance_tokens::GLASS_DEFAULT
        );
        assert_eq!(
            state.overlay_alpha,
            qishell::settings::appearance_tokens::OVERLAY_DEFAULT
        );
        // 恢复外观默认也把配色重置回「默认」。
        assert_eq!(state.terminal_theme, "");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_sftp_local_pane_starts_at_the_configured_folder(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("temp dir");
    let configured = directory.path().to_string_lossy().to_string();
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        // SFTP 是第四页。
        window.click("settings-page-3", cx);
        window.render_frame(cx);
        window.click("sftp-local-root", cx);
        window.input(&configured, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("sftp-local-root").value(),
            Some(configured.as_str())
        );
    })
    .unwrap();
    // 输入框的 Change 事件由订阅在事件循环末尾写进 store，之后新标签才读得到。
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // 设置只决定打开时的落点：新开的 SFTP 标签本地栏就停在这里。
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).test_sftp_local_path(cx),
            Some(directory.path().to_path_buf())
        );
    })
    .unwrap();
}

/// SFTP 标签的开关提示只报 IP，不报用户自己取的主机名。
#[gpui_kit::test]
fn the_sftp_tab_toasts_report_the_address_only(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("right-rail-sftp", cx);
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).test_state(cx).toast.as_deref(),
            Some("已打开 10.24.0.18 的 SFTP 连接")
        );

        window.click("tab-close-1", cx);
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).test_state(cx).toast.as_deref(),
            Some("已关闭 10.24.0.18 的 SFTP 连接")
        );
    })
    .unwrap();
}

/// 片段面板在任何标签都能打开；只有真要上屏时才提示需要终端。
#[gpui_kit::test]
fn the_snippets_panel_opens_anywhere_and_only_warns_when_inserting(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);

        // 设置标签里也能打开面板：看片段、增删改都不需要终端。
        window.click("right-rail-snippets", cx);
        window.render_frame(cx);
        assert!(workspace.read(cx).test_state(cx).snippets_open);
        assert!(window.try_find("snippets-panel").is_some());
        assert_eq!(workspace.read(cx).test_state(cx).toast, None);

        // 点片段要上屏时才提示。
        window.click("snippet-row-1", cx);
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).test_state(cx).toast.as_deref(),
            Some("命令片段只在终端标签中可用")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn terminal_page_theme_cards_reach_the_store_and_the_terminal(cx: &mut TestAppContext) {
    let (workspace, window) = open_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("open-settings", cx);
        window.render_frame(cx);
        // 终端页是第二页；配色卡片在字体分区之后。
        window.click("settings-page-1", cx);
        window.render_frame(cx);

        // 主题卡片只有三张：默认（Rosé Pine Moon）、Tokyo Night、Catppuccin Mocha。
        window.click("terminal-theme-tokyo-night", cx);
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).test_state(cx).terminal_theme,
            "tokyo-night"
        );
    })
    .unwrap();
    // 设置写进 store 后，apply_settings 由订阅在事件末尾把主题装进 gpui 全局量。
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // 只看 store 无法说明终端真的换了色：终端取色走 gpui 全局量，
        // 这里确认它已经指向同一个主题，且主题实底与透明默认背景的约定没变。
        let theme = qishell::terminal::TerminalColors::current(cx);
        assert_eq!(theme.key(), "tokyo-night");
        let chrome = qishell::terminal::TerminalChrome::current(cx);
        assert_eq!(chrome.opaque_background, theme.background());
        assert_eq!(chrome.default_background.a, 0.);

        // 点回「默认」那张卡：空 key，终端回到应用自己的 Rosé Pine Moon。
        window.click("terminal-theme-default", cx);
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).test_state(cx).terminal_theme, "");
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            qishell::terminal::TerminalColors::current(cx).key(),
            "rose-pine-moon"
        );
    })
    .unwrap();
}
