//! Qi Shell 在磁盘上的数据位置。
//!
//! 默认放在程序自己旁边：exe 同级的 `data\`。整个文件夹拷走就能把主机、
//! 设置、known_hosts 与密码一起带走。
//!
//! 顺序：`QISHELL_DATA_DIR` → exe 旁的 `data` → 系统的应用数据目录。
//! 安装到 Program Files 这类不可写的目录时自动退到 `%APPDATA%\Qi Shell`。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 覆盖数据目录。测试与开发时用。
const DATA_DIR_ENV: &str = "QISHELL_DATA_DIR";

/// 主机数据库文件名。
const DATABASE_FILE: &str = "qishell.db";

/// 系统的应用数据目录下属于本应用的文件夹。
const SYSTEM_APP_DIR: &str = "Qi Shell";

/// 应用目录里的数据文件夹名。
pub const PORTABLE_DATA: &str = "data";

/// 安装版在程序旁留下的卸载程序。它的卸载会删掉整个文件夹，所以那里的
/// `data` 文件夹不能存东西。
pub const SETUP_UNINSTALLER: &str = "unins000.exe";

/// Qi Shell 的数据目录。只算一次，进程里始终是同一个。
pub fn data_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(compute_data_dir).clone()
}

fn compute_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV) {
        return PathBuf::from(dir);
    }
    exe_data_dir().unwrap_or_else(system_data_dir)
}

/// 程序旁边的 `data`。不可写（例如装在 Program Files）时返回 `None`。
fn exe_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // 用系统拼写的路径：8.3 短名或另一种大小写会让路径对不上。
    let exe = dunce::canonicalize(&exe).unwrap_or(exe);
    app_data_dir_of(exe.parent()?)
}

/// `folder` 里的 `data` 是否可用作数据目录：能建出来、能写进去。
pub(crate) fn app_data_dir_of(folder: &Path) -> Option<PathBuf> {
    // 安装目录里的 `data` 会被卸载程序删掉，不算。
    if folder.join(SETUP_UNINSTALLER).exists() {
        return None;
    }
    let data = folder.join(PORTABLE_DATA);
    std::fs::create_dir_all(&data).ok()?;
    let probe = data.join(".write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(probe);
            Some(data)
        }
        Err(_) => None,
    }
}

/// exe 目录不可写时的退路：系统的应用数据目录（Windows 是 `%APPDATA%\Qi Shell`）。
fn system_data_dir() -> PathBuf {
    dirs::data_dir()
        .map(|dir| dir.join(SYSTEM_APP_DIR))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 存主机的 SQLite 文件。父目录不存在时创建。
pub fn database_path() -> std::io::Result<PathBuf> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(DATABASE_FILE))
}

/// 应用目录里放私钥的子目录（设置 › 连接里新建或导入的密钥）。
pub fn keys_dir() -> PathBuf {
    data_dir().join("keys")
}

/// 应用自己的主机信任文件。**不读写用户的 `~/.ssh/known_hosts`。**
pub fn known_hosts_path() -> PathBuf {
    data_dir().join("known_hosts")
}

/// 界面设置，数据库旁边的一个小 JSON 文件。
pub fn settings_path() -> std::io::Result<PathBuf> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("settings.json"))
}

/// 设置里存的文件路径：相对路径按数据目录解析，绝对路径原样。
pub fn resolve(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_dir().join(path)
    }
}

/// 写进设置或数据库的文件路径：数据目录里的文件存相对路径，外面的一律原样。
///
/// 存相对路径是「拷贝整个文件夹就迁移」的前提：私钥放进了 `keys/`，换个
/// 位置打开时旧机器的 `C:\...\keys\id_ed25519` 已经不存在了。分隔符统一
/// 写成 `/`，Windows 上存下的设置拿到别的平台上也能解析。
pub fn stored(path: &Path) -> String {
    stored_in(&data_dir(), path)
}

/// 同 [`stored`]，数据目录由调用方给：单测不必去碰进程级的那一个。
fn stored_in(dir: &Path, path: &Path) -> String {
    match path.strip_prefix(dir) {
        Ok(relative) => relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.to_string_lossy().replace('\\', "/"),
    }
}

/// 老旧记录里写死的绝对路径，换成能在当前数据目录下找到的相对路径。
///
/// 从别处拷来的库里记的是旧机器、旧盘符的绝对路径。文件本身跟着目录一起
/// 搬过来了，只是前缀变了：从路径尾部往里试，哪一段尾巴在数据目录里真有
/// 这个文件，就用那一段。找不到（比如私钥放在 `~/.ssh` 里，那是用户自己
/// 的文件，不该乱动）返回 `None`，原样保留。
pub fn adopt(dir: &Path, path: &str) -> Option<String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return None;
    }
    if let Ok(relative) = path.strip_prefix(dir) {
        return Some(stored_in(dir, relative));
    }
    // 原文件就在那儿（例如用户指定了 `~/.ssh` 里的密钥），别乱猜。
    if path.exists() {
        return None;
    }
    let tail: Vec<_> = path
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    // 后缀越长越具体，从长往短试，取第一个真能落地的。
    let longest = tail.len().min(4);
    for take in (1..=longest).rev() {
        let candidate = tail[tail.len() - take..].join("/");
        if dir.join(&candidate).exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writable_folder_gets_a_data_dir() {
        let temporary = tempfile::tempdir().unwrap();
        let data = app_data_dir_of(temporary.path()).unwrap();
        assert_eq!(data, temporary.path().join(PORTABLE_DATA));
        assert!(data.is_dir());
    }

    #[test]
    fn an_installed_folder_is_not_portable() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::write(temporary.path().join(SETUP_UNINSTALLER), b"").unwrap();
        assert_eq!(app_data_dir_of(temporary.path()), None);
    }

    #[test]
    fn a_path_inside_the_data_dir_is_stored_relative() {
        let dir = std::path::Path::new("/data");
        assert_eq!(
            super::stored_in(dir, &dir.join("keys").join("id_ed25519")),
            "keys/id_ed25519"
        );
        // 分隔符统一成 `/`：Windows 上存的设置拿到别的平台还能解析。
        assert_eq!(
            super::stored_in(dir, &dir.join("wallpaper-1.jpg")),
            "wallpaper-1.jpg"
        );
    }

    #[test]
    fn an_old_absolute_path_is_adopted_by_its_tail() {
        let target = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(target.path().join("keys")).unwrap();
        std::fs::write(target.path().join("keys").join("id_ed25519"), b"pk").unwrap();
        // 旧机器上那条绝对路径如今当然不存在，尾巴还能对上。
        let stale = format!(
            "Z:\\gone\\data\\keys{}id_ed25519",
            std::path::MAIN_SEPARATOR
        );

        assert_eq!(
            super::adopt(target.path(), &stale).as_deref(),
            Some("keys/id_ed25519")
        );
    }

    #[test]
    fn a_path_that_still_exists_outside_is_left_alone() {
        let target = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let key = home.path().join("id_ed25519");
        std::fs::write(&key, b"pk").unwrap();
        // 用户真的在用 `~/.ssh` 里的这把钥匙：文件还在，别搬到数据目录名下。
        assert_eq!(super::adopt(target.path(), &key.to_string_lossy()), None);
    }

    #[test]
    fn a_path_nothing_matches_is_left_alone() {
        let target = tempfile::tempdir().unwrap();
        assert_eq!(super::adopt(target.path(), "D:\\gone\\keys\\missing"), None);
        // 已经是相对路径的不用管。
        assert_eq!(super::adopt(target.path(), "keys/id_ed25519"), None);
    }

    #[test]
    fn a_path_outside_the_data_dir_is_stored_as_is() {
        let dir = std::path::Path::new("/data");
        assert_eq!(
            super::stored_in(dir, std::path::Path::new("/home/me/.ssh/id_ed25519")),
            "/home/me/.ssh/id_ed25519"
        );
    }
}
