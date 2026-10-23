//! UI 规范守卫：防止视图层重新出现裸色值与裸字体族。
//!
//! 设计令牌集中在 `src/theme.rs`；终端 ANSI 调色板是唯一例外。

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("读取源码目录") {
        let entry = entry.expect("读取目录项");
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn allowed(path: &Path) -> bool {
    let path = path.to_string_lossy().replace('\\', "/");
    path.ends_with("src/theme.rs") || path.ends_with("src/terminal/theme.rs")
}

fn view_sources() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files
        .into_iter()
        .filter(|path| !allowed(path))
        .map(|path| {
            let source = fs::read_to_string(&path).expect("读取源码");
            (path, source)
        })
        .collect()
}

#[test]
fn views_do_not_hardcode_hex_colors() {
    let mut violations = Vec::new();
    for (path, source) in view_sources() {
        for (index, line) in source.lines().enumerate() {
            let bytes = line.as_bytes();
            let mut cursor = 0;
            while cursor + 2 <= bytes.len() {
                if &bytes[cursor..cursor + 2] == b"0x" {
                    let rest = &bytes[cursor + 2..];
                    let digits = rest
                        .iter()
                        .take_while(|byte| byte.is_ascii_hexdigit())
                        .count();
                    if digits == 6 || digits == 8 {
                        violations.push(format!(
                            "{}:{}: {}",
                            path.display(),
                            index + 1,
                            line.trim()
                        ));
                        break;
                    }
                }
                cursor += 1;
            }
        }
    }
    assert!(
        violations.is_empty(),
        "视图层出现了裸色值，请改用 src/theme.rs 的令牌：\n{}",
        violations.join("\n")
    );
}

#[test]
fn views_use_semantic_line_tokens() {
    let mut violations = Vec::new();
    for (path, source) in view_sources() {
        for (index, line) in source.lines().enumerate() {
            if line.contains("theme::HIGHLIGHT_HIGH") {
                violations.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "视图层直接引用了调色板常量，请改用 theme::line()/track() 等语义令牌：\n{}",
        violations.join("\n")
    );
}

/// 外观只有一个事实来源：`settings::appearance_tokens`。视图层不许再用
/// 写死的透明度调用玻璃画笔，否则滑杆会「假」掉。
#[test]
fn views_do_not_hardcode_glass_alpha() {
    let mut violations = Vec::new();
    for (path, source) in view_sources() {
        for (index, line) in source.lines().enumerate() {
            let hardcoded = [
                "theme::glass(",
                "theme::workspace_glass(",
                "theme::modal_surface(",
            ]
            .iter()
            .any(|call| {
                line.match_indices(call).any(|(start, _)| {
                    line[start + call.len()..]
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_digit())
                })
            });
            if hardcoded {
                violations.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "视图层写死了玻璃透明度，请改读 AppearanceTokens：\n{}",
        violations.join("\n")
    );
}

/// 输入框、下拉框、文本域都必须走 `components::field`：原生外观在聚焦时会画
/// 1px 边框 + 3px 外环，叠在外壳上就是双边框。全应用只允许一种聚焦外观。
#[test]
fn views_do_not_build_native_inputs_or_selects() {
    let mut violations = Vec::new();
    for (path, source) in view_sources() {
        let path_text = path.to_string_lossy().replace('\\', "/");
        if path_text.ends_with("src/components/field.rs") {
            continue;
        }
        for (index, line) in source.lines().enumerate() {
            let native = ["Textarea::new(", "Input::new(", "Select::new("]
                .iter()
                .find(|call| line.contains(**call));
            if let Some(call) = native {
                violations.push(format!(
                    "{}:{}: {}（{}）",
                    path.display(),
                    index + 1,
                    line.trim(),
                    call
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "输入框 / 下拉框 / 文本域必须走 components::field 的统一外壳，避免原生焦点环与外壳叠成双边框：\n{}",
        violations.join("\n")
    );
}

#[test]
fn views_do_not_hardcode_font_families() {
    let mut violations = Vec::new();
    for (path, source) in view_sources() {
        for (index, line) in source.lines().enumerate() {
            if line.contains(".font_family(\"") {
                violations.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "视图层写死了字体族，请改用 cx.theme().mono_font_family 或 assets：\n{}",
        violations.join("\n")
    );
}
