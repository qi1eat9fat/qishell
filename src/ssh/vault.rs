//! 连接要用的两样文件：私钥与主机信任列表。
//!
//! 都放在应用目录里（`keys/` 与 `known_hosts`），拷贝整个文件夹就能迁移。
//! 这里只做文件层面的读写，界面在 `settings::connection`。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey, parse_public_key_base64};

/// 应用目录里放私钥的子目录。
pub const KEYS_DIR: &str = "keys";

/// 一把存在应用目录里的私钥。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredKey {
    /// 文件名，也是界面上显示的名字。
    pub name: String,
    /// 私钥文件路径。
    pub path: PathBuf,
    /// `ssh-ed25519` 这样的算法名。
    pub algorithm: String,
    /// `SHA256:...` 指纹。
    pub fingerprint: String,
}

/// `dir` 里的私钥，按文件名排序。读不出私钥的文件跳过。
pub fn list_keys(dir: &Path) -> Vec<StoredKey> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut keys: Vec<StoredKey> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter(|entry| !entry.file_name().to_string_lossy().ends_with(".pub"))
        .filter_map(|entry| {
            let path = entry.path();
            let key = russh::keys::load_secret_key(&path, None).ok()?;
            Some(StoredKey {
                name: entry.file_name().to_string_lossy().to_string(),
                algorithm: key.algorithm().to_string(),
                fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
                path,
            })
        })
        .collect();
    keys.sort_by(|left, right| left.name.cmp(&right.name));
    keys
}

/// 在 `dir` 里生成一把新的 Ed25519 私钥，同时写出 `.pub`。
///
/// `name` 不能为空，也不能和已有的文件重名。返回新密钥。
pub fn generate_key(dir: &Path, name: &str) -> Result<StoredKey> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("密钥名称不能为空"));
    }
    if name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(anyhow!("密钥名称不能包含路径分隔符"));
    }
    std::fs::create_dir_all(dir).with_context(|| format!("无法创建密钥目录：{}", dir.display()))?;
    let path = dir.join(name);
    if path.exists() {
        return Err(anyhow!("{name} 已存在"));
    }
    // 系统随机源：项目里的 rand 0.9 与 ssh-key 的 rand_core 不是同一套，
    // 所以直接用 ssh-key 自己带的那一个。
    let mut rng = ssh_key::rand_core::UnwrapErr(ssh_key::getrandom::SysRng);
    let key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(|error| anyhow!("无法生成密钥：{error}"))?;
    key.write_openssh_file(&path, Default::default())
        .with_context(|| format!("无法写入私钥：{}", path.display()))?;
    let public = key.public_key().to_openssh().unwrap_or_default();
    std::fs::write(path.with_extension("pub"), format!("{public}\n"))
        .with_context(|| format!("无法写入公钥：{}", path.display()))?;
    Ok(StoredKey {
        name: name.to_string(),
        algorithm: key.algorithm().to_string(),
        fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
        path,
    })
}

/// 删掉一把私钥，连同旁边的 `.pub`。
pub fn delete_key(key: &StoredKey) -> Result<()> {
    std::fs::remove_file(&key.path)
        .with_context(|| format!("无法删除私钥：{}", key.path.display()))?;
    let _ = std::fs::remove_file(key.path.with_extension("pub"));
    Ok(())
}

/// 把主机表单里选的私钥收进数据目录，返回放好之后的路径。
///
/// 「拷贝整个文件夹就迁移」要求私钥也在这个文件夹里：用户可能在 `~/.ssh`
/// 里选了一把钥匙，换台机器就找不到了。已经在 `dir` 里的原样返回；在外面
/// 就复制一份进来（原件不动，用户自己的钥匙照样留在原处）。同名但内容不同
/// 时加序号，不覆盖已有的密钥。
pub fn adopt_key(dir: &Path, source: &Path) -> Option<PathBuf> {
    if !source.is_file() {
        return None;
    }
    if source.starts_with(dir) {
        return Some(source.to_path_buf());
    }
    let bytes = std::fs::read(source).ok()?;
    std::fs::create_dir_all(dir).ok()?;
    let name = source.file_name()?.to_string_lossy().into_owned();
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.clone());
    let extension = source.extension().map(|ext| {
        let mut ext = ext.to_string_lossy().into_owned();
        ext.insert(0, '.');
        ext
    });
    // 只给同名冲突加序号，第一把仍是原名；同一把钥匙已经复制过就直接用，
    // 别攒副本。
    let mut target = None;
    for counter in 1..=1000 {
        let candidate = match counter {
            1 => dir.join(&name),
            _ => dir.join(format!(
                "{stem}-{counter}{}",
                extension.as_deref().unwrap_or("")
            )),
        };
        if !candidate.exists() {
            target = Some(candidate);
            break;
        }
        if std::fs::read(&candidate).ok().as_deref() == Some(bytes.as_slice()) {
            return Some(candidate);
        }
    }
    let target = target?;
    std::fs::write(&target, &bytes).ok()?;
    // 公钥一起带上：设置里的密钥列表靠它显示指纹，能少算一次。
    let public_source = source.with_extension("pub");
    if public_source.is_file() {
        let _ = std::fs::copy(&public_source, target.with_extension("pub"));
    }
    Some(target)
}

/// 把一把外部的私钥复制进 `dir`，连同 `.pub`（原文件没有就现生成）。
pub fn import_key(dir: &Path, source: &Path) -> Result<StoredKey> {
    let name = source
        .file_name()
        .ok_or_else(|| anyhow!("选中的文件没有文件名"))?
        .to_string_lossy()
        .to_string();
    // 先确认它真是一把能读的私钥，免得把乱七八糟的文件搬进数据目录。
    russh::keys::load_secret_key(source, None)
        .map_err(|_| anyhow!("{name} 不是可读的私钥（加密的私钥需要口令，暂不支持导入）"))?;
    std::fs::create_dir_all(dir).with_context(|| format!("无法创建密钥目录：{}", dir.display()))?;
    let path = dir.join(&name);
    std::fs::copy(source, &path).with_context(|| format!("无法复制私钥：{}", path.display()))?;
    let public_source = source.with_extension("pub");
    let public_target = path.with_extension("pub");
    if public_source.exists() {
        let _ = std::fs::copy(&public_source, &public_target);
    } else if let Ok(key) = russh::keys::load_secret_key(&path, None) {
        let public = key.public_key().to_openssh().unwrap_or_default();
        let _ = std::fs::write(&public_target, format!("{public}\n"));
    }
    list_keys(dir)
        .into_iter()
        .find(|key| key.name == name)
        .ok_or_else(|| anyhow!("导入后读不出这把密钥"))
}

/// `known_hosts` 里信任的一台主机。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownHost {
    /// 文件里的那一行（`[地址]:端口` 或 `地址`）。
    pub entry: String,
    /// `ssh-ed25519` 这样的算法名。
    pub algorithm: String,
    /// `SHA256:...` 指纹。
    pub fingerprint: String,
}

/// `path` 里信任的全部主机，按出现顺序。坏行跳过。
pub fn list_known_hosts(path: &Path) -> Vec<KnownHost> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut fields = line.split_whitespace();
            let entry = fields.next()?.to_string();
            let algorithm = fields.next()?.to_string();
            let key = parse_public_key_base64(fields.next()?).ok()?;
            Some(KnownHost {
                entry,
                algorithm,
                fingerprint: fingerprint(&key),
            })
        })
        .collect()
}

/// 从 `path` 里去掉 `entry` 那一行的信任记录。其它主机原样保留。
pub fn forget_known_host(path: &Path, entry: &str) -> Result<()> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取主机信任文件：{}", path.display()))?;
    let kept: String = contents
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('#')
                || !trimmed
                    .split_whitespace()
                    .next()
                    .is_some_and(|hosts| hosts.split(',').any(|known| known == entry))
        })
        .map(|line| format!("{line}\n"))
        .collect();
    // 先写临时文件再改名：半路失败也不会留下半份记录。
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, kept)
        .with_context(|| format!("无法写入主机信任文件：{}", path.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("无法写入主机信任文件：{}", path.display()))?;
    Ok(())
}

/// 公钥的 `SHA256:` 指纹。
fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_key_lands_in_the_directory_with_its_public_half() {
        let directory = tempfile::tempdir().unwrap();
        let key = generate_key(directory.path(), "work").unwrap();
        assert_eq!(key.name, "work");
        assert_eq!(key.algorithm, "ssh-ed25519");
        assert!(key.path.exists());
        assert!(key.path.with_extension("pub").exists());

        let listed = list_keys(directory.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].fingerprint, key.fingerprint);
    }

    #[test]
    fn a_name_that_already_exists_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        generate_key(directory.path(), "work").unwrap();
        assert!(generate_key(directory.path(), "work").is_err());
    }

    #[test]
    fn a_key_from_outside_is_copied_into_the_data_dir() {
        let outside = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let source = outside.path().join("id_ed25519");
        std::fs::write(&source, b"private").unwrap();

        let adopted = vault_adopt(directory.path(), &source).unwrap();
        assert_eq!(adopted, directory.path().join("id_ed25519"));
        assert_eq!(std::fs::read(&adopted).unwrap(), b"private");
        assert!(source.is_file(), "原文件不动");

        // 再选一次同一把：用已有的副本，不攒第二个。
        assert_eq!(vault_adopt(directory.path(), &source).unwrap(), adopted);
    }

    #[test]
    fn a_key_already_inside_is_left_where_it_is() {
        let directory = tempfile::tempdir().unwrap();
        let key = generate_key(directory.path(), "work").unwrap();
        assert_eq!(vault_adopt(directory.path(), &key.path).unwrap(), key.path);
    }

    #[test]
    fn a_second_key_of_the_same_name_gets_a_suffix() {
        let outside = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        generate_key(directory.path(), "id_ed25519").unwrap();
        let source = outside.path().join("id_ed25519");
        std::fs::write(&source, b"another").unwrap();

        let adopted = vault_adopt(directory.path(), &source).unwrap();
        assert_eq!(adopted.file_name().unwrap(), "id_ed25519-2");
        assert_eq!(std::fs::read(&adopted).unwrap(), b"another");
    }

    fn vault_adopt(dir: &Path, source: &Path) -> Option<PathBuf> {
        super::adopt_key(dir, source)
    }

    #[test]
    fn deleting_a_key_takes_its_public_half_too() {
        let directory = tempfile::tempdir().unwrap();
        let key = generate_key(directory.path(), "work").unwrap();
        delete_key(&key).unwrap();
        assert!(list_keys(directory.path()).is_empty());
        assert!(!key.path.with_extension("pub").exists());
    }

    #[test]
    fn known_hosts_lines_become_entries_and_forget_removes_one() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        let first = generate_key(directory.path(), "a").unwrap();
        let second = generate_key(directory.path(), "b").unwrap();
        let key_a = russh::keys::load_secret_key(&first.path, None).unwrap();
        let key_b = russh::keys::load_secret_key(&second.path, None).unwrap();
        std::fs::write(
            &path,
            format!(
                "host-a {}\n[host-b]:2222 {}\n",
                key_a.public_key().to_openssh().unwrap(),
                key_b.public_key().to_openssh().unwrap()
            ),
        )
        .unwrap();

        let hosts = list_known_hosts(&path);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].entry, "host-a");
        assert_eq!(hosts[1].entry, "[host-b]:2222");
        assert!(hosts[0].fingerprint.starts_with("SHA256:"));

        forget_known_host(&path, "host-a").unwrap();
        let hosts = list_known_hosts(&path);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].entry, "[host-b]:2222");
    }
}
