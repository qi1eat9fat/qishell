//! 把应用图标与版本信息编进 exe。
//!
//! Windows 上的窗口与任务栏图标不由 `WindowOptions::icon` 决定（那个字段只在
//! X11 生效）：gpui 的 Windows 后端用 `LoadImageW(模块句柄, ID 1, IMAGE_ICON)`
//! 直接从 exe 自己的资源里取图标。所以图标必须作为资源 ID 1 链进二进制，
//! 光把 .ico 放在仓库里看不到任何效果。
//!
//! 版本信息也在同一份资源里生成：「属性」页里看到的 FileVersion / ProductName
//! 都从这里来，版本号取 `CARGO_PKG_VERSION`，只在 `Cargo.toml` 改一处。
//!
//! 资源脚本本身也由这里生成（写到 `OUT_DIR/qishell.rc`），图标路径写成绝对
//! 路径，所以仓库里不需要再放一份 .rc。

fn main() {
    // 换图标或改这个脚本（含版本块）时重新编译。
    println!("cargo:rerun-if-changed=assets/qishell.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let resource = match resource_script() {
        Ok(path) => path,
        Err(error) => {
            println!("cargo:warning=应用图标与版本信息未嵌入：{error}");
            return;
        }
    };

    // 缺资源编译器时只丢图标与版本信息，不该让整个构建失败。
    if let Err(result) =
        embed_resource::compile(&resource, embed_resource::NONE).manifest_optional()
    {
        println!("cargo:warning=应用图标与版本信息未嵌入：{result}");
    }
}

/// 生成资源脚本：图标行 + 按 Cargo.toml 现拼的版本块。
///
/// 生成的脚本放在 `OUT_DIR`，所以图标路径要写成绝对路径，否则 rc 会去 OUT_DIR
/// 下面找。
fn resource_script() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let icon = cwd.join("assets/qishell.ico");
    let icon = icon
        .to_str()
        .ok_or("图标路径含非 UTF-8 字符")?
        // rc 字符串里的反斜杠要转义。
        .replace('\\', "\\\\");

    let version = std::env::var("CARGO_PKG_VERSION")?;
    let mut parts = version.split('.').map(|part| {
        part.parse::<u32>()
            .map_err(|_| format!("版本号 {version} 不是 a.b.c 形式"))
    });
    let (major, minor, patch) = (
        parts.next().transpose()?.unwrap_or(0),
        parts.next().transpose()?.unwrap_or(0),
        parts.next().transpose()?.unwrap_or(0),
    );
    let numeric = format!("{major},{minor},{patch},0");

    let script = format!(
        r#"// 由 build.rs 生成，不要直接改。
// rc.exe 默认按系统 ANSI 代码页读文件，字符串里有中文就必须声明 UTF-8。
#pragma code_page(65001)

1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "080404b0"
    BEGIN
      VALUE "FileDescription", "Qi Shell: SSH 工作区"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "qishell"
      VALUE "LegalCopyright", "GPL-3.0-only"
      VALUE "OriginalFilename", "qishell.exe"
      VALUE "ProductName", "Qi Shell"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x804, 1200
  END
END
"#
    );

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?).join("qishell.rc");
    std::fs::write(&out, script)?;
    Ok(out)
}
