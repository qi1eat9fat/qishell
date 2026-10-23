//! 把应用图标编进 exe。
//!
//! Windows 上的窗口与任务栏图标不由 `WindowOptions::icon` 决定（那个字段只在
//! X11 生效）：gpui 的 Windows 后端用 `LoadImageW(模块句柄, ID 1, IMAGE_ICON)`
//! 直接从 exe 自己的资源里取图标。所以图标必须作为资源 ID 1 链进二进制，
//! 光把 .ico 放在仓库里看不到任何效果。
//!
//! 资源文件在 `assets/qishell.rc`，图标是 `assets/qishell.ico`（16–256 共 7 档）。

fn main() {
    // 换图标或改资源脚本时重新编译。
    println!("cargo:rerun-if-changed=assets/qishell.rc");
    println!("cargo:rerun-if-changed=assets/qishell.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // 缺资源编译器时只丢图标，不该让整个构建失败。
    if let Err(result) =
        embed_resource::compile("assets/qishell.rc", embed_resource::NONE).manifest_optional()
    {
        println!("cargo:warning=应用图标未嵌入：{result}");
    }
}
