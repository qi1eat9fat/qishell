//! 工作区：持有全部界面状态，绘制窗口并处理用户命令。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{Root, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::*;
use crate::connection::SharedConnectionTester;
use crate::host::{Host, HostDatabase, HostId, HostLogin, HostStore, HostStoreEvent};
use crate::model::*;
use crate::secrets::SharedSecretStore;
use crate::settings::appearance_tokens::AppearanceTokens;
use crate::settings::{SettingsPanel, SettingsStore};
use crate::sftp::{SftpEvent, SftpView};
use crate::snippets::SnippetStore;
use crate::ssh::{SshConnectionTester, SshConnector, SshTerminalTransportProvider};
use crate::terminal::{
    SharedRemoteTerminalTransportProvider, TerminalEvent, TerminalLifecycle, TerminalView,
};
use crate::theme;

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub message: SharedString,
}

#[derive(Clone, Debug)]
pub(crate) struct HostMenu {
    pub host_id: HostId,
    pub x: Pixels,
    pub y: Pixels,
    pub active: Option<usize>,
    pub focus: FocusHandle,
    pub return_focus: Option<FocusHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostMenuAction {
    Edit,
    CopyIp,
    Delete,
}

/// 主机菜单的实际动作；删除项前插入分隔线。
const HOST_MENU_ITEMS: [(HostMenuAction, &str, bool); 3] = [
    (HostMenuAction::Edit, "编辑主机", false),
    (HostMenuAction::CopyIp, "复制 IP", false),
    (HostMenuAction::Delete, "删除主机", true),
];

pub struct Workspace {
    pub(crate) focus_handle: FocusHandle,
    /// 主机列表：内存是唯一事实来源，改动写穿到 SQLite。
    pub(crate) hosts: Entity<HostStore>,
    pub(crate) selected_host: Option<HostId>,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active_tab: Option<TabId>,
    next_tab_id: TabId,
    pub(crate) store: Entity<SettingsStore>,
    pub(crate) settings_panel: Entity<SettingsPanel>,
    /// 「测试连接」用的登录器（测试里可注入假实现）。
    pub(crate) tester: SharedConnectionTester,
    /// 终端字节流的来源（测试里注入假终端）。
    pub(crate) remote: SharedRemoteTerminalTransportProvider,
    /// SFTP 标签自己建立连接用；测试注入的构造没有。
    pub(crate) sftp_connector: Option<SshConnector>,
    pub(crate) toast: Option<Toast>,
    toast_seq: u64,
    pub(crate) host_menu: Option<HostMenu>,
    /// 主机列表是否收起为左侧窄栏；只影响界面，不写设置。
    pub(crate) host_panel_collapsed: bool,
    /// 命令片段数据（生产环境落盘，测试与预览用内存副本）。
    pub(crate) snippets: SnippetStore,
    /// 右侧命令片段面板是否展开。
    pub(crate) snippets_open: bool,
    pub(crate) wallpaper_path: Option<PathBuf>,
    pub(crate) wallpaper_image: Option<Arc<RenderImage>>,
    /// 已加载（或已失败）的壁纸键；失败也保留，避免每次重画都重试。
    wallpaper_key: Option<super::wallpaper::WallpaperKey>,
    /// 从文件解码并缩放好的底图。只取决于图片本身，不随模糊滑杆重做；
    /// 拖杆时只对这份像素重新做模糊。
    wallpaper_source: Option<Arc<image::RgbaImage>>,
    /// 防抖中的解码/模糊任务；下一次设置变化会替换它。
    wallpaper_task: Option<Task<()>>,
    /// 上一次准备壁纸失败了。只有这种情况才回退到直接画原图：
    /// 等待期间画原图会先清晰再变糊，比空着背景更刺眼。
    wallpaper_failed: bool,
    /// 代次：后台结果回来时过期就丢弃。
    wallpaper_generation: u64,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// 从磁盘读取主机与设置，打开工作区。
    pub fn new(
        settings: Entity<SettingsStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let data_dir = crate::app::paths::data_dir();
        let secrets = crate::secrets::local_secret_store(&data_dir);
        let hosts = cx.new(|_| load_host_store(secrets.clone()));
        let connector = Arc::new(SshConnector::new(
            crate::app::paths::known_hosts_path(),
            secrets,
        ));
        let tester: SharedConnectionTester =
            Arc::new(SshConnectionTester::new(connector.as_ref().clone()));
        let remote: SharedRemoteTerminalTransportProvider = Arc::new(
            SshTerminalTransportProvider::with_connector(connector.as_ref().clone()),
        );
        let sftp_connector = connector.as_ref().clone();
        let mut workspace = Self::with_services_and_sftp(
            settings,
            hosts,
            tester,
            remote,
            Some(sftp_connector),
            window,
            cx,
        );
        // 命令片段只在生产路径落盘；测试与预览用内存副本。
        let (snippets, problem) = SnippetStore::load(data_dir.join("snippets.json"));
        workspace.snippets = snippets;
        if let Some(problem) = problem {
            workspace.show_toast(problem, cx);
        }
        workspace
    }

    /// 注入全部外部服务；测试里换成假实现，也不建立 SFTP 连接。
    #[doc(hidden)]
    pub fn with_services(
        settings: Entity<SettingsStore>,
        hosts: Entity<HostStore>,
        tester: SharedConnectionTester,
        remote: SharedRemoteTerminalTransportProvider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_services_and_sftp(settings, hosts, tester, remote, None, window, cx)
    }

    fn with_services_and_sftp(
        settings: Entity<SettingsStore>,
        hosts: Entity<HostStore>,
        tester: SharedConnectionTester,
        remote: SharedRemoteTerminalTransportProvider,
        sftp_connector: Option<SshConnector>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let settings_panel = cx.new(|cx| SettingsPanel::new(settings.clone(), window, cx));
        let selected_host = hosts.read(cx).hosts().next().map(|host| host.id);

        let mut this = Self {
            focus_handle,
            hosts: hosts.clone(),
            selected_host,
            tabs: Vec::new(),
            active_tab: None,
            next_tab_id: 1,
            store: settings,
            settings_panel,
            tester,
            remote,
            sftp_connector,
            toast: None,
            toast_seq: 0,
            host_menu: None,
            host_panel_collapsed: false,
            snippets: SnippetStore::in_memory(),
            snippets_open: false,
            wallpaper_path: None,
            wallpaper_image: None,
            wallpaper_key: None,
            wallpaper_source: None,
            wallpaper_task: None,
            wallpaper_failed: false,
            wallpaper_generation: 0,
            _subscriptions: Vec::new(),
        };

        // 观察设置变化：外观参数与壁纸由工作区落到窗口上。
        let subscription = cx.observe(&this.store.clone(), |this, _, cx| {
            this.apply_settings(cx);
            cx.notify();
        });
        this._subscriptions.push(subscription);

        // 主机存储变化：重绘，并保证选中项仍然存在。
        let observed = hosts.clone();
        this._subscriptions
            .push(cx.observe(&observed, |this, _, cx| {
                if this
                    .selected_host
                    .is_some_and(|id| this.hosts.read(cx).host(id).is_none())
                {
                    this.selected_host = this.hosts.read(cx).hosts().next().map(|host| host.id);
                }
                cx.notify();
            }));

        // 写库失败：内存里的改动还在，界面要说一声。
        this._subscriptions
            .push(
                cx.subscribe(&hosts, |this, _, event: &HostStoreEvent, cx| match event {
                    HostStoreEvent::PersistFailed(message) => {
                        this.show_toast(message.clone(), cx);
                    }
                }),
            );

        this.apply_settings(cx);
        window.focus(&this.focus_handle, cx);
        this
    }

    // —— 查询 ——

    pub(crate) fn active_tab(&self) -> Option<&Tab> {
        let id = self.active_tab?;
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub(crate) fn host<'a>(&'a self, cx: &'a App, id: HostId) -> Option<&'a Host> {
        self.hosts.read(cx).host(id)
    }

    /// 当前标签打开时的登录快照。主机被删掉后仍然可用。
    pub(crate) fn active_login(&self) -> Option<&HostLogin> {
        self.active_tab()?.login.as_ref()
    }

    pub(crate) fn current_host_id(&self) -> Option<HostId> {
        self.active_tab().and_then(Tab::host_id)
    }

    /// 这台主机现在有没有打开着的终端连接。SFTP 是独立连接，不计入。
    pub(crate) fn host_connected(&self, id: HostId) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.kind == TabKind::Host(id) && tab.connected)
    }

    pub(crate) fn tab_label(&self, tab: &Tab) -> String {
        match tab.kind {
            TabKind::Settings => "设置".into(),
            TabKind::Sftp(_) => {
                let Some(login) = tab.login.as_ref() else {
                    return "已删除的主机 · SFTP".into();
                };
                format!("{} · SFTP", login.name)
            }
            TabKind::Host(id) => {
                let Some(login) = tab.login.as_ref() else {
                    return "已删除的主机".into();
                };
                let same: Vec<&Tab> = self
                    .tabs
                    .iter()
                    .filter(|other| other.kind == TabKind::Host(id))
                    .collect();
                if same.len() > 1 {
                    let ix = same
                        .iter()
                        .position(|other| other.id == tab.id)
                        .unwrap_or(0)
                        + 1;
                    format!("{} · {}", login.name, ix)
                } else {
                    login.name.clone()
                }
            }
        }
    }

    pub(crate) fn visible_hosts<'a>(&'a self, cx: &'a App) -> impl Iterator<Item = &'a Host> + 'a {
        self.hosts.read(cx).hosts()
    }

    // —— 标签与连接 ——

    pub(crate) fn open_host(
        &mut self,
        host_id: HostId,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|tab| tab.kind == TabKind::Host(host_id))
        {
            if activate {
                self.active_tab = Some(existing.id);
                if let Some(terminal) = existing.terminal.clone() {
                    terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
                }
                cx.notify();
            }
            return;
        }
        let Some(login) = self.hosts.read(cx).login(host_id) else {
            return;
        };
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let (terminal, subscriptions) = self.create_terminal(id, &login, window, cx);
        self.tabs.push(Tab {
            id,
            kind: TabKind::Host(host_id),
            login: Some(login),
            connected: false,
            terminal: Some(terminal.clone()),
            sftp: None,
            subscriptions,
        });
        self.selected_host = Some(host_id);
        if activate {
            self.active_tab = Some(id);
            terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
        }
        cx.notify();
    }

    pub(crate) fn new_tab_for(
        &mut self,
        host_id: HostId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(login) = self.hosts.read(cx).login(host_id) else {
            return;
        };
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let (terminal, subscriptions) = self.create_terminal(id, &login, window, cx);
        self.tabs.push(Tab {
            id,
            kind: TabKind::Host(host_id),
            login: Some(login),
            connected: false,
            terminal: Some(terminal.clone()),
            sftp: None,
            subscriptions,
        });
        self.active_tab = Some(id);
        terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
        cx.notify();
    }

    /// 右侧栏图标：为当前主机打开 SFTP 标签。
    pub(crate) fn open_sftp_for_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let host_id = self
            .current_host_id()
            .or(self.selected_host)
            .or_else(|| self.hosts.read(cx).hosts().next().map(|host| host.id));
        match host_id {
            Some(host_id) => self.open_sftp(host_id, true, window, cx),
            None => self.show_toast("请先新建或选择一台主机", cx),
        }
    }

    /// 一台主机只保留一个 SFTP 标签。
    pub(crate) fn open_sftp(
        &mut self,
        host_id: HostId,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|tab| tab.kind == TabKind::Sftp(host_id))
        {
            if activate {
                self.active_tab = Some(existing.id);
                window.focus(&self.focus_handle, cx);
                cx.notify();
            }
            return;
        }
        let Some(login) = self.hosts.read(cx).login(host_id) else {
            return;
        };
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let max_transfers = self.store.read(cx).settings().sftp.max_transfers;
        let appearance = AppearanceTokens::new(&self.store.read(cx).settings().appearance);
        let editor_path = self.store.read(cx).settings().editor_path.clone();
        let local_root = self.store.read(cx).settings().sftp.local_root.clone();
        let sftp = cx.new(|_| {
            SftpView::new(
                &login,
                self.sftp_connector.clone(),
                max_transfers,
                appearance,
                editor_path,
                local_root,
            )
        });
        let dialog_handle = window.window_handle();
        let subscription =
            cx.subscribe(
                &sftp,
                move |this, sftp_view, event: &SftpEvent, cx| match event {
                    SftpEvent::ConnectionChanged(connected) => {
                        if let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id == id) {
                            tab.connected = *connected;
                            cx.notify();
                        }
                    }
                    SftpEvent::Toast(message) => {
                        this.show_toast(message.clone(), cx);
                    }
                    SftpEvent::Conflict(info) => {
                        let view = sftp_view.clone();
                        let info = info.clone();
                        let modal_alpha =
                            AppearanceTokens::new(&this.store.read(cx).settings().appearance).modal;
                        let _ = dialog_handle.update(cx, |_, window, cx| {
                            crate::dialogs::sftp_conflict::open(
                                info,
                                view,
                                modal_alpha,
                                window,
                                cx,
                            );
                        });
                    }
                    SftpEvent::Prompt(prompt) => {
                        let view = sftp_view.clone();
                        let responder: crate::dialogs::prompt::PromptResponder =
                            Arc::new(move |request_id, reply, cx: &mut App| {
                                view.update(cx, |view, cx| {
                                    view.reply_to_prompt(request_id, reply, cx);
                                });
                            });
                        let workspace = cx.entity();
                        let prompt = prompt.clone();
                        let modal_alpha =
                            AppearanceTokens::new(&this.store.read(cx).settings().appearance).modal;
                        let _ = dialog_handle.update(cx, |_, window, cx| {
                            crate::dialogs::prompt::open(
                                prompt,
                                responder,
                                workspace,
                                modal_alpha,
                                window,
                                cx,
                            );
                        });
                    }
                },
            );
        sftp.update(cx, |view, cx| view.start(cx));
        // 提示里只报地址：主机名是用户自己取的，地址才好在多台机器之间对上号。
        let address = login.address.clone();
        self.tabs.push(Tab {
            id,
            kind: TabKind::Sftp(host_id),
            login: Some(login),
            connected: true,
            terminal: None,
            sftp: Some(sftp),
            subscriptions: vec![subscription],
        });
        self.selected_host = Some(host_id);
        if activate {
            self.active_tab = Some(id);
            window.focus(&self.focus_handle, cx);
        }
        self.show_toast(format!("已打开 {address} 的 SFTP 连接"), cx);
        cx.notify();
    }

    pub(crate) fn active_sftp(&self) -> Option<Entity<SftpView>> {
        self.active_tab().and_then(|tab| tab.sftp.clone())
    }

    /// 建一个真终端，并订阅它的事件（连接状态、问答、错误）。
    fn create_terminal(
        &self,
        tab_id: TabId,
        login: &HostLogin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<TerminalView>, Vec<Subscription>) {
        let factory = self.remote.factory_for(login);
        let label = login.name.clone();
        let terminal = cx.new(|cx| {
            TerminalView::new(
                ("terminal", tab_id),
                move |_| label.clone().into(),
                factory,
                window,
                cx,
            )
        });
        terminal.update(cx, |terminal, cx| terminal.set_reconnect_on_enter(cx));
        let handle = window.window_handle();
        let subscription = cx.subscribe(&terminal, move |this, _, event: &TerminalEvent, cx| {
            this.on_terminal_event(tab_id, event.clone(), handle, cx);
        });
        // 连接状态（断开、失败、重连成功）由生命周期反映，跟着它更新。
        let observed = terminal.clone();
        let observed_subscription = cx.observe(&observed, move |this, terminal, cx| {
            let connected = matches!(terminal.read(cx).lifecycle(cx), TerminalLifecycle::Running);
            if let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id == tab_id)
                && tab.connected != connected
            {
                tab.connected = connected;
                cx.notify();
            }
        });
        (terminal, vec![subscription, observed_subscription])
    }

    fn on_terminal_event(
        &mut self,
        tab_id: TabId,
        event: TerminalEvent,
        handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalEvent::Started => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                    tab.connected = true;
                }
                if let Some(host_id) = self
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(Tab::host_id)
                {
                    self.hosts.update(cx, |store, cx| {
                        store.record_connection(host_id, cx);
                    });
                }
            }
            TerminalEvent::PromptRequested(prompt) => {
                let Some(terminal) = self
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| tab.terminal.clone())
                else {
                    return;
                };
                let modal_alpha =
                    AppearanceTokens::new(&self.store.read(cx).settings().appearance).modal;
                let workspace = cx.entity();
                let responder: crate::dialogs::prompt::PromptResponder =
                    Arc::new(move |request_id, reply, cx: &mut App| {
                        terminal.read(cx).reply_to_prompt(request_id, reply, cx);
                    });
                let _ = handle.update(cx, |_, window, cx| {
                    crate::dialogs::prompt::open(
                        prompt,
                        responder,
                        workspace,
                        modal_alpha,
                        window,
                        cx,
                    );
                });
                return;
            }
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn activate_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) {
            self.active_tab = Some(id);
            self.host_menu = None;
            if let Some(terminal) = tab.terminal.clone() {
                terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
            } else {
                window.focus(&self.focus_handle, cx);
            }
            cx.notify();
        }
    }

    pub(crate) fn close_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let message = self.close_toast(&self.tabs[index]);
        let mut tab = self.tabs.remove(index);
        if let Some(terminal) = tab.terminal.take() {
            terminal.update(cx, |terminal, cx| terminal.shutdown(cx));
        }
        tab.subscriptions.clear();
        if tab.is_settings() {
            self.settings_panel
                .update(cx, |panel, cx| panel.clear_notices(cx));
        }
        if self.active_tab == Some(id) {
            self.active_tab = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.get(index.saturating_sub(1)))
                .map(|tab| tab.id);
        }
        self.show_toast(message, cx);
        cx.notify();
    }

    /// 关标签时的提示。连接类标签只报地址——主机名是用户自己取的，地址才好在
    /// 多台机器之间对上号，跟打开时的提示一致；设置标签没有地址，仍报名字。
    fn close_toast(&self, tab: &Tab) -> String {
        let Some(login) = tab.login.as_ref() else {
            return format!("已关闭 {}", self.tab_label(tab));
        };
        if tab.is_sftp() {
            format!("已关闭 {} 的 SFTP 连接", login.address)
        } else {
            format!("已关闭 {} 的 SSH 连接", login.address)
        }
    }

    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        // 每次打开设置都从干净的页面开始：连接页上次留下的提示不再显示。
        self.settings_panel
            .update(cx, |panel, cx| panel.clear_notices(cx));
        if let Some(existing) = self.tabs.iter().find(|tab| tab.is_settings()) {
            self.active_tab = Some(existing.id);
            cx.notify();
            return;
        }
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab {
            id,
            kind: TabKind::Settings,
            login: None,
            connected: true,
            terminal: None,
            sftp: None,
            subscriptions: Vec::new(),
        });
        self.active_tab = Some(id);
        cx.notify();
    }

    pub(crate) fn connect_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(host_id) = self.selected_host {
            self.connect_host(host_id, window, cx);
        }
    }

    /// 打开连接。真实 SSH 登录在 S3 接入，这里先落标签。
    pub(crate) fn connect_host(
        &mut self,
        host_id: HostId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_host(host_id, true, window, cx);
    }

    // —— Toast 与菜单 ——

    pub(crate) fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        self.toast = Some(Toast {
            id,
            message: message.into(),
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(3200))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn copy_host_ip(&mut self, host_id: HostId, cx: &mut Context<Self>) {
        let Some(address) = self.host(cx, host_id).map(|host| host.address.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(address));
        self.show_toast("已复制 IP", cx);
    }

    pub(crate) fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let settings = self.store.read(cx).settings().clone();
        let max_transfers = settings.sftp.max_transfers;
        let appearance = AppearanceTokens::new(&settings.appearance);
        let editor_path = settings.editor_path.clone();
        for tab in &self.tabs {
            if let Some(sftp) = &tab.sftp {
                sftp.update(cx, |sftp, cx| {
                    sftp.set_max_transfers(max_transfers, cx);
                    sftp.set_appearance(appearance, cx);
                    sftp.set_editor_path(editor_path.clone(), cx);
                });
            }
        }
        // 终端字体与颜色都是全局量，改动立即作用到已开的标签。
        let family = settings.terminal.font.clone();
        cx.set_global(crate::terminal::TerminalFont {
            family: crate::terminal::is_font_installed(&family, cx).then(|| family.into()),
            size: px(settings.terminal.size as f32),
            line_height: settings.terminal.line_height,
        });
        let terminal_theme = crate::terminal::TerminalTheme::selected(&settings.terminal.theme);
        cx.set_global(crate::terminal::TerminalColors::new(terminal_theme));
        cx.set_global(crate::terminal::TerminalChrome::new(
            terminal_theme,
            appearance.overlay,
        ));
        let appearance = settings.appearance;
        // 外观五类的最终 alpha 只算一次：视图层读全局令牌，gpui-kit 的
        // 菜单/下拉/Tooltip token 也在同一处更新。
        let tokens = crate::settings::appearance_tokens::AppearanceTokens::new(&appearance);
        cx.set_global(crate::settings::appearance_tokens::UiTokens(tokens));
        // 便携壁纸存的是相对路径，按当前数据目录解析；旧设置里的绝对路径
        // 原样使用。
        let wallpaper = appearance
            .wallpaper
            .path
            .as_ref()
            .map(|path| crate::app::paths::resolve(path));
        if wallpaper != self.wallpaper_path {
            self.wallpaper_path = wallpaper;
        }
        self.refresh_wallpaper(&appearance, cx);
        // 遮罩、菜单/下拉与 Tooltip 由 gpui-kit 的主题 token 绘制。改完
        // `colors` 必须重建 `tokens`，否则 Tooltip 停在旧值。
        {
            let theme = gpui_kit::component::Theme::global_mut(cx);
            crate::settings::appearance_tokens::apply_to_theme(theme, tokens);
        }
        gpui_kit::component::Theme::sync_base(cx);
    }

    /// 壁纸路径或模糊半径变化后，在后台重做预模糊图。
    ///
    /// 三级缓存：内存里的底图（`wallpaper_source`）供拖杆时只重做模糊；
    /// 磁盘上的底图与成品（见 `wallpaper::cache_paths`）让第二次启动跳过
    /// 解码。全都没命中才回退到「解码 + 缩放 + 模糊」。首次加载等待 120ms
    /// 去抖（避免启动时连开两次），拖杆只等 60ms：停手就出图。
    fn refresh_wallpaper(&mut self, appearance: &crate::model::Appearance, cx: &mut Context<Self>) {
        let Some(path) = self.wallpaper_path.clone() else {
            self.wallpaper_generation += 1;
            self.wallpaper_task = None;
            self.wallpaper_key = None;
            self.wallpaper_source = None;
            if let Some(old) = self.wallpaper_image.take() {
                cx.drop_image(old, None);
            }
            return;
        };
        let key = super::wallpaper::WallpaperKey::for_path(&path, appearance.blur);
        if self.wallpaper_key.as_ref() == Some(&key) {
            return;
        }
        // 同一张图：底图还能复用，只重做模糊。
        let reuse = self
            .wallpaper_source
            .as_ref()
            .filter(|_| {
                self.wallpaper_key
                    .as_ref()
                    .is_some_and(|old| old.same_file(&key))
            })
            .cloned();
        // 只改模糊半径时几乎不需要等待：停手 60ms 就开始出图。
        let settle = if reuse.is_some() {
            Duration::from_millis(60)
        } else {
            Duration::from_millis(120)
        };
        self.wallpaper_key = Some(key.clone());
        self.wallpaper_generation += 1;
        let generation = self.wallpaper_generation;
        self.wallpaper_failed = false;
        // 启动时还什么都没有：磁盘上的成品只有几 MB，同步读进来能让第一帧
        // 就是模糊后的样子，省掉「先清晰、后变糊」的跳变。拖杆时走后台，
        // 不阻塞输入。
        if self.wallpaper_image.is_none()
            && let Some((source, rgba)) = super::wallpaper::read_ready_now(&key)
        {
            self.wallpaper_source = source.map(Arc::new);
            self.wallpaper_image = Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])));
            return;
        }
        self.wallpaper_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(settle).await;
            let loaded = cx
                .background_spawn(async move {
                    let directory = crate::app::paths::data_dir().join(super::wallpaper::CACHE_DIR);
                    let cache = super::wallpaper::cache_paths(&directory, &key);
                    // 成品命中：直接上屏，底图也一并读回来，下次拖杆不用再解码。
                    if let Some(blurred) = super::wallpaper::read_cache(&cache.blurred) {
                        let source = super::wallpaper::read_cache(&cache.source).map(Arc::new);
                        return Ok((source, super::wallpaper::into_bgra(blurred)));
                    }
                    let source = match reuse {
                        Some(source) => source,
                        None => match super::wallpaper::read_cache(&cache.source) {
                            Some(source) => Arc::new(source),
                            None => match super::wallpaper::load_source_rgba(&key) {
                                Ok(source) => {
                                    let source = Arc::new(source);
                                    // 缓存写失败只影响下次启动的速度，不报错。
                                    let _ = super::wallpaper::write_cache(&cache.source, &source);
                                    source
                                }
                                Err(error) => return Err(error),
                            },
                        },
                    };
                    let blurred = super::wallpaper::blur_source(&source, key.blur);
                    let _ = super::wallpaper::write_cache(&cache.blurred, &blurred);
                    super::wallpaper::prune_cache(&directory, &cache);
                    Ok((Some(source), super::wallpaper::into_bgra(blurred)))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.wallpaper_generation != generation {
                    return;
                }
                match loaded {
                    Ok((source, rgba)) => {
                        // 命中成品时就没再留底图，保留内存里已有的那份。
                        if let Some(source) = source {
                            this.wallpaper_source = Some(source);
                        }
                        let image = Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]));
                        if let Some(old) = this.wallpaper_image.replace(image) {
                            cx.drop_image(old, None);
                        }
                        cx.notify();
                    }
                    Err(error) => {
                        // 保留 key：坏图不要每次重画都重试；渲染回退到原图路径。
                        this.wallpaper_failed = true;
                        eprintln!("qishell: 壁纸模糊失败: {error}");
                    }
                }
            });
        }));
    }

    // —— 背景、工作区、Toast 与菜单 ——

    pub(crate) fn render_background(
        &self,
        settings: &SettingsState,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let appearance = &settings.appearance;
        let image = self.wallpaper_image.clone();
        // 准备好之前只铺纯色底：先画清晰的再变糊，观感是「闪一下」。
        let fallback = self
            .wallpaper_path
            .clone()
            .filter(|_| self.wallpaper_failed);
        div()
            .absolute()
            .inset_0()
            .bg(theme::solid(theme::BASE_RGB))
            .when_some(image, |background, image| {
                // 预模糊版本：解码失败时退回下面的原图路径。
                background.child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Cover),
                )
            })
            .when_some(fallback, |background, path| {
                background.child(
                    img(path)
                        .absolute()
                        .inset_0()
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(theme::veil(appearance.clarity)),
            )
    }

    pub(crate) fn render_workspace_area(
        &self,
        has_tab: bool,
        is_settings: bool,
        is_sftp: bool,
        tokens: &AppearanceTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let content: AnyElement = if !has_tab {
            div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .text_size(theme::TYPE_BODY)
                .line_height(px(21.))
                .text_color(theme::solid(theme::SUBTLE))
                .id("start-hint")
                .test_support()
                .child("双击或按 Enter 打开左侧主机")
                .into_any_element()
        } else if is_settings {
            self.settings_panel.clone().into_any_element()
        } else if is_sftp {
            match self.active_sftp() {
                Some(sftp) => sftp.into_any_element(),
                None => div().flex_1().into_any_element(),
            }
        } else if let Some(terminal) = self.active_tab().and_then(|tab| tab.terminal.clone()) {
            terminal.into_any_element()
        } else {
            div().flex_1().into_any_element()
        };

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(theme::workspace_glass(tokens.glass))
            .when(has_tab, |workspace| {
                workspace.child(self.render_tab_bar(tokens, cx))
            })
            .child(content)
    }

    /// 右侧栏：设置与 SFTP 两个入口，图标不随选中态变色。
    pub(crate) fn render_right_rail(
        &self,
        tokens: &AppearanceTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let target = self.focus_handle.clone();

        v_flex()
            .w(theme::RAIL_WIDTH)
            .h_full()
            .flex_shrink_0()
            .items_center()
            // 36px 标题行里的 28px 图标按钮上下各留 4px，右侧栏首图标与
            // 左侧主机列表的收起按钮共享同一条垂直中线；图标之间留两倍
            // 距离（8px）。
            .pt(px(4.))
            .gap(px(8.))
            .border_l_1()
            .border_color(theme::line())
            .bg(theme::glass(tokens.glass))
            .child(crate::components::icons::with_tooltip(
                "open-settings-tip",
                "设置",
                crate::components::button::icon("open-settings")
                    .icon(gpui_kit::assets::IconName::Cog)
                    .accessibility_label("设置")
                    .on_click(move |_, window, cx| {
                        target.dispatch_action(&OpenSettings, window, cx);
                    }),
            ))
            .child(crate::components::icons::with_tooltip(
                "right-rail-sftp-tip",
                "SFTP 文件",
                div()
                    .id("right-rail-sftp")
                    .test_support()
                    .aria_label("SFTP 文件")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(theme::ICON_BUTTON_SIZE)
                    .rounded(theme::RADIUS_MD)
                    .tab_index(0)
                    .cursor_pointer()
                    .focus_visible(|style| style.shadow(theme::focus_ring()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_sftp_for_current(window, cx);
                    }))
                    .child(
                        // 与顶栏图标一致：32×32 命中区，28×28 可见悬停底。
                        div()
                            .size(theme::ICON_BUTTON_SIZE)
                            .rounded(theme::RADIUS_MD)
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|style| style.bg(theme::hover_tint()))
                            .child(
                                gpui_kit::component::Icon::new(
                                    gpui_kit::assets::IconName::FolderOpen,
                                )
                                .size(theme::ICON_SM)
                                .text_color(theme::solid(theme::MUTED_STRONG)),
                            ),
                    ),
            ))
            .child(crate::components::icons::with_tooltip(
                "right-rail-snippets-tip",
                "命令片段",
                crate::components::button::icon("right-rail-snippets")
                    .icon(gpui_kit::assets::IconName::ListChecks)
                    .accessibility_label("命令片段")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_snippets(cx);
                    })),
            ))
    }

    pub(crate) fn render_toast(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let alpha = AppearanceTokens::new(&self.store.read(cx).settings().appearance).overlay;
        div()
            .absolute()
            // 顶栏 + 标签栏之下水平居中，靠近内容区顶部。
            .top(theme::TITLE_BAR_HEIGHT + theme::BAR_HEIGHT + px(12.))
            .left(px(0.))
            .right(px(0.))
            .flex()
            .justify_center()
            .px(px(20.))
            .when_some(self.toast.as_ref(), |layer, toast| {
                layer.child(
                    crate::components::overlay::toast(toast.message.clone(), alpha).max_w(px(320.)),
                )
            })
    }

    pub(crate) fn open_host_menu(
        &mut self,
        host_id: HostId,
        x: Pixels,
        y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        let return_focus = window.focused(cx);
        window.focus(&focus, cx);
        self.selected_host = Some(host_id);
        self.host_menu = Some(HostMenu {
            host_id,
            x,
            y,
            active: None,
            focus,
            return_focus,
        });
        cx.notify();
    }

    fn close_host_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.host_menu.take() {
            if let Some(handle) = menu.return_focus {
                window.focus(&handle, cx);
            }
            cx.notify();
        }
    }

    fn activate_host_menu(
        &mut self,
        action: HostMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.host_menu.take() else {
            return;
        };
        let host_id = menu.host_id;
        // 先把焦点还给触发菜单的主机行；随后打开的弹窗会把这一行记作
        // 自己的「返回焦点」，关闭时不会落到已经消失的菜单上。
        if let Some(handle) = menu.return_focus {
            window.focus(&handle, cx);
        }
        match action {
            HostMenuAction::Edit => crate::dialogs::host::open(self, Some(host_id), window, cx),
            HostMenuAction::CopyIp => self.copy_host_ip(host_id, cx),
            HostMenuAction::Delete => self.open_delete_host(host_id, window, cx),
        }
        cx.notify();
    }

    fn host_menu_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.host_menu.as_ref() else {
            return;
        };
        let active = menu.active;
        let count = HOST_MENU_ITEMS.len();
        let key = event.keystroke.key.as_str();
        match key {
            "down" | "tab" => {
                if let Some(menu) = self.host_menu.as_mut() {
                    menu.active = Some(match active {
                        Some(index) => (index + 1) % count,
                        None => 0,
                    });
                }
                cx.stop_propagation();
                cx.notify();
            }
            "up" | "shift-tab" => {
                if let Some(menu) = self.host_menu.as_mut() {
                    menu.active = Some(match active {
                        Some(index) => (index + count - 1) % count,
                        None => count - 1,
                    });
                }
                cx.stop_propagation();
                cx.notify();
            }
            "home" => {
                if let Some(menu) = self.host_menu.as_mut() {
                    menu.active = Some(0);
                }
                cx.stop_propagation();
                cx.notify();
            }
            "end" => {
                if let Some(menu) = self.host_menu.as_mut() {
                    menu.active = Some(count - 1);
                }
                cx.stop_propagation();
                cx.notify();
            }
            "enter" | "space" | " " => {
                if let Some(index) = active {
                    let action = HOST_MENU_ITEMS[index].0;
                    cx.stop_propagation();
                    self.activate_host_menu(action, window, cx);
                }
            }
            "escape" => {
                cx.stop_propagation();
                self.close_host_menu(window, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_host_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.host_menu.as_ref()?;
        let host_id = menu.host_id;
        let active = menu.active;
        let focus = menu.focus.clone();
        self.host(cx, host_id)?;

        let mut entries: Vec<AnyElement> = Vec::new();
        for (index, (action, label, danger)) in HOST_MENU_ITEMS.iter().enumerate() {
            if *action == HostMenuAction::Delete {
                entries.push(crate::components::overlay::menu_divider().into_any_element());
            }
            entries.push(
                menu_item(format!("host-menu-{index}"), *label, true, *danger)
                    .when(active == Some(index), |row| {
                        row.bg(theme::hover_tint_strong())
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_host_menu(*action, window, cx);
                    }))
                    .into_any_element(),
            );
        }

        // 右键菜单属于轻量浮层，不再借用主玻璃值。
        let alpha = AppearanceTokens::new(&self.store.read(cx).settings().appearance).overlay;
        Some(
            deferred(
                div()
                    .id("host-menu")
                    .absolute()
                    .left(
                        menu.x
                            .max(px(8.))
                            .min((window.viewport_size().width - px(184.)).max(px(8.))),
                    )
                    .top(
                        menu.y
                            .max(px(8.))
                            .min((window.viewport_size().height - px(140.)).max(px(8.))),
                    )
                    .w(px(176.))
                    .track_focus(&focus)
                    .tab_index(0)
                    .on_key_down(cx.listener(Self::host_menu_key))
                    .child(
                        crate::components::overlay::menu_surface(alpha)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .children(entries),
                    )
                    .test_support(),
            )
            .into_any_element(),
        )
    }

    pub(crate) fn open_delete_host(
        &mut self,
        host_id: HostId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.host(cx, host_id).cloned() else {
            return;
        };
        let this = cx.weak_entity();
        let modal_alpha = AppearanceTokens::new(&self.store.read(cx).settings().appearance).modal;
        window.open_dialog(cx, move |dialog, window, _| {
            let this = this.clone();
            let name = host.name.clone();
            crate::components::dialog::surface(dialog, theme::DIALOG_WIDTH, modal_alpha)
                .title(crate::components::dialog::title("删除主机"))
                .margin_top(window.viewport_size().height * 0.40)
                .overlay_closable(true)
                .child(
                    v_flex()
                        .gap(px(6.))
                        .child(format!("确定删除 {name} 吗？"))
                        .child("此操作无法撤销。已打开的相关标签不会被关闭。"),
                )
                .footer(
                    crate::components::dialog::footer()
                        .child(
                            crate::components::button::form_secondary("cancel-delete", "取消")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child({
                            let this = this.clone();
                            crate::components::button::form_danger("confirm-delete", "删除")
                                .on_click(move |_, window, cx| {
                                    let _ = this.update(cx, |this, cx| {
                                        this.hosts.update(cx, |store, cx| {
                                            store.remove(host_id, cx);
                                        });
                                        this.show_toast("已删除主机", cx);
                                        cx.notify();
                                    });
                                    window.close_dialog(cx);
                                })
                        }),
                )
                // 记录要求「删除确认的初始焦点在删除按钮」；GPUI 的按钮没有
                // 可编程焦点，改为让回车直接确认删除，效果一致。
                .on_ok({
                    let this = this.clone();
                    move |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.hosts.update(cx, |store, cx| {
                                store.remove(host_id, cx);
                            });
                            this.show_toast("已删除主机", cx);
                            cx.notify();
                        });
                        window.close_dialog(cx);
                        true
                    }
                })
        });
    }
    // —— 动作 ——

    fn on_new_host(&mut self, _: &NewHost, window: &mut Window, cx: &mut Context<Self>) {
        crate::dialogs::host::open(self, None, window, cx);
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let host_id = self
            .current_host_id()
            .or(self.selected_host)
            .or_else(|| self.hosts.read(cx).hosts().next().map(|host| host.id));
        if let Some(id) = host_id {
            if self.current_host_id().is_some() {
                self.new_tab_for(id, window, cx);
            } else {
                self.open_host(id, true, window, cx);
            }
        }
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(cx);
    }

    fn on_close_active_tab(&mut self, _: &CloseActiveTab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab {
            self.close_tab(id, cx);
        }
    }

    fn on_connect_selected(
        &mut self,
        _: &ConnectSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connect_selected(window, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(1, cx);
    }

    fn on_previous_tab(&mut self, _: &PreviousTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(-1, cx);
    }

    fn step_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let current = self
            .active_tab
            .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
            .unwrap_or(0) as isize;
        let count = self.tabs.len() as isize;
        let next = (current + step).rem_euclid(count) as usize;
        self.active_tab = Some(self.tabs[next].id);
        cx.notify();
    }

    fn on_focus_next(&mut self, _: &FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    fn on_focus_prev(&mut self, _: &FocusPrev, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_prev(cx);
    }

    fn on_dismiss_overlay(&mut self, _: &DismissOverlay, _: &mut Window, cx: &mut Context<Self>) {
        if self.host_menu.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.store.read(cx).settings().clone();
        let tokens = AppearanceTokens::new(&settings.appearance);
        let has_tab = self.active_tab.is_some();
        let is_settings = self.active_tab().is_some_and(Tab::is_settings);
        let is_sftp = self.active_tab().is_some_and(Tab::is_sftp);
        // 非终端界面统一用鸿蒙 Sans：英文、数字与符号也走同一套字形。
        // 全工作区开启 tnum，数字在任何字段里都保持等宽；回退只处理字库
        // 子集之外的生僻字。
        let font = Font {
            family: crate::assets::UI_FONT_FAMILY.into(),
            features: theme::tabular_numbers(),
            fallbacks: Some(FontFallbacks::from_fonts(vec!["Microsoft YaHei UI".into()])),
            ..Default::default()
        };

        div()
            .id("workspace-root")
            .size_full()
            .relative()
            .overflow_hidden()
            // 空工作区才有「按 Enter 打开主机」的快捷键：有标签页时 Enter
            // 属于终端和设置页里的输入框。
            .key_context(if has_tab {
                "Workspace"
            } else {
                "Workspace StartHint"
            })
            .track_focus(&self.focus_handle)
            .font(font)
            .text_color(theme::solid(theme::TEXT))
            .bg(theme::solid(theme::BASE_RGB))
            .on_action(cx.listener(Self::on_new_host))
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_close_active_tab))
            .on_action(cx.listener(Self::on_connect_selected))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_previous_tab))
            .on_action(cx.listener(Self::on_dismiss_overlay))
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_focus_prev))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.host_menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(self.render_background(&settings, cx))
            .child(
                v_flex()
                    .size_full()
                    // 顶栏通栏；主机列表放回第二层，与标签栏共用一行。
                    .child(self.render_top_bar(&tokens, window, cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_stretch()
                            .child(self.render_host_panel(&tokens, cx))
                            .child(self.render_workspace_area(
                                has_tab,
                                is_settings,
                                is_sftp,
                                &tokens,
                                cx,
                            ))
                            .child(self.render_snippets_panel(&tokens, cx))
                            .child(self.render_right_rail(&tokens, cx)),
                    ),
            )
            .child(self.render_toast(cx))
            .children(self.render_host_menu(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

use crate::components::overlay::menu_item;

/// 从磁盘读主机。库打不开时退回空列表，界面仍能用（只是不落盘）。
fn load_host_store(secrets: SharedSecretStore) -> HostStore {
    let database = match crate::app::paths::database_path() {
        Ok(path) => match HostDatabase::open(&path) {
            Ok(database) => Some(database),
            Err(error) => {
                eprintln!("qishell: 无法打开主机数据库: {error}");
                None
            }
        },
        Err(error) => {
            eprintln!("qishell: 无法定位数据目录: {error}");
            None
        }
    };
    let store = match database {
        Some(database) => HostStore::load(database).unwrap_or_else(|error| {
            eprintln!("qishell: 无法读取主机: {error}");
            HostStore::empty()
        }),
        None => HostStore::empty(),
    };
    store.with_secrets(secrets)
}

/// 集成测试读取的只读快照。
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct WorkspaceTestState {
    pub hosts: usize,
    pub tabs: usize,
    pub settings_tabs: usize,
    pub active_host: Option<String>,
    pub active_tab_label: Option<String>,
    pub host_menu_open: bool,
    pub host_panel_collapsed: bool,
    /// 外观 › 玻璃滑杆。
    pub glass: u8,
    /// 外观 › 轻量浮层滑杆。
    pub overlay_alpha: u8,
    /// 终端配色主题 key（空 = 默认）。
    pub terminal_theme: String,
    /// 当前显示的 Toast 文字；没有就是 `None`。
    pub toast: Option<String>,
    /// 右侧命令片段面板是否展开。
    pub snippets_open: bool,
}

impl Workspace {
    /// 只给 `tests/ui` 用：不暴露内部所有权，只读取测试关心的结果。
    #[doc(hidden)]
    pub fn test_state(&self, cx: &App) -> WorkspaceTestState {
        WorkspaceTestState {
            hosts: self.hosts.read(cx).hosts().count(),
            tabs: self.tabs.len(),
            settings_tabs: self.tabs.iter().filter(|tab| tab.is_settings()).count(),
            active_host: self.active_login().map(|login| login.name.clone()),
            active_tab_label: self.active_tab().map(|tab| self.tab_label(tab)),
            host_menu_open: self.host_menu.is_some(),
            host_panel_collapsed: self.host_panel_collapsed,
            glass: self.store.read(cx).settings().appearance.glass,
            overlay_alpha: self.store.read(cx).settings().appearance.overlay_alpha,
            terminal_theme: self.store.read(cx).settings().terminal.theme.clone(),
            toast: self.toast.as_ref().map(|toast| toast.message.to_string()),
            snippets_open: self.snippets_open,
        }
    }

    /// 只给 `tests/ui` 用：当前 SFTP 标签的本地栏停在哪。
    #[doc(hidden)]
    pub fn test_sftp_local_path(&self, cx: &App) -> Option<std::path::PathBuf> {
        self.active_sftp()
            .map(|sftp| sftp.read(cx).test_local_path())
    }

    /// 只给 `tests/ui` 用：把 SFTP 本地侧指向临时目录。
    #[doc(hidden)]
    pub fn test_set_sftp_local_path(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if let Some(sftp) = self.active_sftp() {
            sftp.update(cx, |view, cx| view.test_set_local_path(path, cx));
        }
    }

    /// 只给 `tests/ui` 用：模拟工作线程发来一个同名冲突。
    #[doc(hidden)]
    pub fn test_emit_sftp_conflict(&mut self, cx: &mut Context<Self>) {
        if let Some(sftp) = self.active_sftp() {
            sftp.update(cx, |view, cx| view.test_emit_conflict(cx));
        }
    }

    /// 只给 `tests/ui` 用：SFTP 队列里有多少个批次。
    #[doc(hidden)]
    pub fn test_sftp_batch_count(&self, cx: &App) -> usize {
        self.active_sftp()
            .map(|sftp| sftp.read(cx).batch_count())
            .unwrap_or(0)
    }

    /// 只给 `tests/ui` 用：当前终端屏幕上的文字。
    #[doc(hidden)]
    pub fn test_terminal_text(&self, cx: &App) -> String {
        self.active_tab()
            .and_then(|tab| tab.terminal.as_ref())
            .map(|terminal| terminal.read(cx).screen_text(cx))
            .unwrap_or_default()
    }

    /// 只给 `tests/ui` 用：删除主机，验证打开中的标签保留内容。
    #[doc(hidden)]
    pub fn test_mark_removed(&mut self, id: HostId, cx: &mut Context<Self>) {
        self.hosts.update(cx, |store, cx| {
            store.remove(id, cx);
        });
        cx.notify();
    }

    /// 只给 `tests/ui` 用：名称、地址、认证方式、私钥路径。
    #[doc(hidden)]
    pub fn test_host(
        &self,
        cx: &App,
        name: &str,
    ) -> Option<(String, crate::host::AuthKind, Option<String>)> {
        self.hosts
            .read(cx)
            .hosts()
            .find(|host| host.name == name)
            .map(|host| (host.address.clone(), host.auth, host.key_path.clone()))
    }
}
