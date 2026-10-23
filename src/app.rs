//! 应用启动：字体、主题、窗口与全局快捷键。

pub mod paths;

use gpui_kit::component::{Root, TitleBar};
use gpui_kit::*;

use crate::actions::*;
use crate::assets;
use crate::settings::SettingsStore;
use crate::theme;
use crate::workspace::Workspace;

/// 每次启动的初始窗口大小（1440×900，最小窗口是 960×600）。
const STARTUP_SIZE: Size<Pixels> = size(px(1440.), px(900.));

pub fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            assets::register_fonts(cx);
            theme::init(cx);
            bind_keys(cx);
            cx.activate(true);

            let settings_path = paths::settings_path().ok();
            let (settings, settings_problem) = match &settings_path {
                Some(path) => SettingsStore::load(path.clone()),
                None => (SettingsStore::in_memory(), None),
            };
            let options = window_options(cx);

            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    window.activate_window();
                    window.set_window_title(crate::APP_NAME);
                    // Input 的 keymap 在第一次创建 InputState 时注册，晚于
                    // app::bind_keys；先建一个临时状态触发注册，再覆盖 Tab。
                    let _input_probe =
                        cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx));
                    cx.bind_keys([
                        KeyBinding::new("tab", FocusNext, Some("Input")),
                        KeyBinding::new("shift-tab", FocusPrev, Some("Input")),
                    ]);
                    let settings = cx.new(|_| settings);
                    let workspace = cx.new(|cx| {
                        let mut workspace = Workspace::new(settings, window, cx);
                        if let Some(problem) = settings_problem {
                            workspace.show_toast(problem, cx);
                        }
                        workspace
                    });
                    cx.new(|cx| Root::new(workspace, window, cx))
                })
                .expect("无法打开 Qi Shell 窗口");
            })
            .detach();
        });
}

/// 每次启动都按初始大小、居中在主屏打开：位置与大小不跨重启记忆。
fn window_options(cx: &mut App) -> WindowOptions {
    let window_bounds = centered_on_primary(STARTUP_SIZE, cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(window_bounds)),
        display_id: None,
        window_min_size: Some(size(
            px(theme::MIN_WINDOW_WIDTH),
            px(theme::MIN_WINDOW_HEIGHT),
        )),
        ..TitleBar::window_options()
    }
}

/// 窗口取「初始大小」与当前屏幕可用区域中较小的一个，居中放在主屏
/// （取不到主屏时退回 `cx.displays()` 的第一块）。显示器比初始尺寸还小时
/// 窗口不会越出屏幕；换到小屏也不会留下抓不住的部分。
fn centered_on_primary(startup: Size<Pixels>, cx: &App) -> Bounds<Pixels> {
    let Some(screen) = cx
        .primary_display()
        .or_else(|| cx.displays().first().cloned())
    else {
        return Bounds::new(point(px(0.), px(0.)), startup);
    };
    let screen = screen.visible_bounds();
    let fitted = size(
        startup.width.min(screen.size.width),
        startup.height.min(screen.size.height),
    );
    Bounds::centered_at(screen.center(), fitted)
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-n", NewHost, Some("Workspace")),
        KeyBinding::new("ctrl-t", NewTab, Some("Workspace")),
        KeyBinding::new("ctrl-,", OpenSettings, Some("Workspace")),
        KeyBinding::new("ctrl-w", CloseActiveTab, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Workspace")),
        // 只在空工作区（「双击或按 Enter 打开左侧主机」）里绑定：它有标签页时
        // 会先吃掉终端的 Enter，让 shell 收不到回车，也会在设置页里误开主机。
        KeyBinding::new("enter", ConnectSelected, Some("StartHint")),
        KeyBinding::new("escape", DismissOverlay, Some("Workspace")),
        KeyBinding::new("tab", FocusNext, Some("Workspace")),
        KeyBinding::new("shift-tab", FocusPrev, Some("Workspace")),
        // gpui-kit 在 Input 上下文里把 Tab 绑定为缩进（多行输入会吃掉 Tab）。
        // 本应用没有代码编辑器；表单输入里 Tab 应始终移动焦点。这里的绑定
        // 在 gpui_kit::init 之后加载，覆盖同深度的 IndentInline。
        KeyBinding::new("tab", FocusNext, Some("Input")),
        KeyBinding::new("shift-tab", FocusPrev, Some("Input")),
    ]);
    cx.bind_keys(crate::terminal::terminal_key_bindings());
}
