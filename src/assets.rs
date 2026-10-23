//! 内嵌资源：界面字体与应用图标。界面里的功能图标使用 gpui-kit 自带的 Lucide 图标集。

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use gpui_kit::*;

/// 标题栏里的应用图标。exe 与任务栏用的是 `assets/qishell.ico`（见 `build.rs`）。
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/qishell-icon.png");

/// 解码好的应用图标。
///
/// 只解一次：`img()` 每次渲染都要一个 `ImageSource`，每帧新建一份 2.8 KB 的
/// 字节副本没必要——图标在进程里不会变。
pub fn app_icon() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();
    ICON.get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Png, APP_ICON_PNG.to_vec())))
        .clone()
}

/// 界面字体族：中文、英文、数字与符号统一使用鸿蒙 Sans。
///
/// 等宽字体只留给终端与终端预览；地址、指纹、数值等数据字段同样走这个
/// 字族，并由 `theme::tabular_numbers()` 保证数字等宽。不要再把 IBM Plex
/// Sans 之类的拉丁字体引回正文，否则中英文会重新出现两套字形。
pub const UI_FONT_FAMILY: &str = "HarmonyOS Sans SC";

/// 注册界面字体。中英文、数字与符号都走 HarmonyOS Sans SC。
///
/// 三份 TTF 是鸿蒙官方发布的**未修改**原文件：许可明确禁止改动字体本身，
/// 所以不能像早期那样按字符集做子集化。代价是 exe 大 24 MB 左右。
pub fn register_fonts(cx: &mut App) {
    let fonts: [(&str, Cow<'static, [u8]>); 3] = [
        (
            "HarmonyOS Sans SC Regular",
            Cow::Borrowed(include_bytes!(
                "../assets/fonts/HarmonyOS_Sans_SC_Regular.ttf"
            )),
        ),
        (
            "HarmonyOS Sans SC Medium",
            Cow::Borrowed(include_bytes!(
                "../assets/fonts/HarmonyOS_Sans_SC_Medium.ttf"
            )),
        ),
        (
            "HarmonyOS Sans SC Bold",
            Cow::Borrowed(include_bytes!("../assets/fonts/HarmonyOS_Sans_SC_Bold.ttf")),
        ),
    ];
    for (name, bytes) in fonts {
        if let Err(error) = cx.text_system().add_fonts(vec![bytes]) {
            eprintln!("qishell: 无法注册字体 {name}: {error}");
        }
    }
}

/// 系统里第一个可用的等宽字体。
///
/// 栈按 DESIGN.md：Cascadia Code 优先（Windows 自带，原型实际渲染的一直是
/// 它），未安装时退回 Consolas。不要再把未打包的 JetBrains Mono 放在首位：
/// 那会让「设置里的字体选项」和真实渲染不一致。
pub fn mono_font_family(cx: &App) -> SharedString {
    let names = cx.text_system().all_font_names();
    for candidate in ["Cascadia Code", "Consolas"] {
        if names.iter().any(|name| name == candidate) {
            return candidate.into();
        }
    }
    "Consolas".into()
}
