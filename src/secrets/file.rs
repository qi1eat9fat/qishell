//! 应用目录里的秘密库：一个加密文件，跟着整个文件夹一起走。
//!
//! 文件格式：`secrets.dat` = 版本 1 字节 + 12 字节随机 nonce + AES-256-GCM
//! 密文；明文是 `账号 -> 秘密` 的 JSON。密钥是同目录的 `secret.key`。
//!
//! **安全边界**：密钥和密文在同一个文件夹里，所以这能防随手打开文件看到
//! 密码，不能防拿到整个文件夹的人。选择这条路是为了让拷贝文件夹就能迁移。

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aes_gcm::aead::{Aead as _, KeyInit as _};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context as _, Result, anyhow};
use rand::Rng as _;
use zeroize::Zeroizing;

use super::{SecretRef, SecretStore};

/// 秘密库文件。
pub const FILE_NAME: &str = "secrets.dat";
/// 加密密钥文件。
pub const KEY_FILE_NAME: &str = "secret.key";
/// 格式版本。
const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// `u12`：AES-GCM 的标准 nonce 长度。
type Nonce12 = Nonce<aes_gcm::aes::cipher::consts::U12>;

pub struct FileSecretStore {
    key: Zeroizing<[u8; KEY_LEN]>,
    data_path: PathBuf,
    secrets: Mutex<BTreeMap<String, Zeroizing<String>>>,
}

impl FileSecretStore {
    /// 打开（必要时新建）`dir` 里的秘密库。
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("无法创建数据目录：{}", dir.display()))?;
        let key_path = dir.join(KEY_FILE_NAME);
        let data_path = dir.join(FILE_NAME);
        let key = load_or_create_key(&key_path)?;
        let store = Self {
            key,
            data_path,
            secrets: Mutex::new(BTreeMap::new()),
        };
        let loaded = store.read_file()?;
        if let Ok(mut secrets) = store.secrets.lock() {
            *secrets = loaded;
        }
        store.adopt_key_paths(dir);
        Ok(store)
    }

    /// 把按旧机器绝对路径记的口令挪到相对路径名下。
    ///
    /// 口令的归属是私钥路径：整个文件夹拷过来后，主机表里的路径当天就会
    /// 改成相对路径，秘密库里这条还停在 `passphrase:D:\旧机器\...\keys\id`，
    /// 不跟着改就对不上、口令白存。
    fn adopt_key_paths(&self, dir: &Path) {
        let mut secrets = self.lock();
        let stale: Vec<(String, String)> = secrets
            .keys()
            .filter_map(|account| {
                let path = account.strip_prefix("passphrase:")?;
                let adopted = crate::app::paths::adopt(dir, path)?;
                Some((account.clone(), format!("passphrase:{adopted}")))
            })
            .collect();
        if stale.is_empty() {
            return;
        }
        for (from, to) in stale {
            if let Some(value) = secrets.remove(&from) {
                secrets.insert(to, value);
            }
        }
        if let Err(error) = self.persist(&secrets) {
            eprintln!("qishell: 无法改写秘密库里的私钥路径: {error}");
        }
    }

    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    fn cipher(&self) -> Result<Aes256Gcm> {
        Aes256Gcm::new_from_slice(&self.key[..]).map_err(|_| anyhow!("秘密库密钥长度不对"))
    }

    fn read_file(&self) -> Result<BTreeMap<String, Zeroizing<String>>> {
        let bytes = match std::fs::read(&self.data_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法读取秘密库：{}", self.data_path.display()));
            }
        };
        if bytes.len() < 1 + NONCE_LEN || bytes[0] != VERSION {
            return Err(anyhow!("秘密库格式不认识：{}", self.data_path.display()));
        }
        let nonce = Nonce12::try_from(&bytes[1..1 + NONCE_LEN])
            .map_err(|_| anyhow!("秘密库 nonce 长度不对"))?;
        let plaintext = self
            .cipher()?
            .decrypt(&nonce, &bytes[1 + NONCE_LEN..])
            .map_err(|_| anyhow!("秘密库无法解密（密钥文件被换过？）"))?;
        let plaintext = Zeroizing::new(
            String::from_utf8(plaintext).map_err(|_| anyhow!("秘密库内容不是 UTF-8"))?,
        );
        let stored: BTreeMap<String, String> = serde_json::from_str(&plaintext)
            .with_context(|| format!("秘密库内容损坏：{}", self.data_path.display()))?;
        Ok(stored
            .into_iter()
            .map(|(key, value)| (key, Zeroizing::new(value)))
            .collect())
    }

    /// 整份重写：先写临时文件再改名，写坏也不会留下半份。
    fn persist(&self, secrets: &BTreeMap<String, Zeroizing<String>>) -> Result<()> {
        let stored: BTreeMap<&str, &str> = secrets
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let plaintext = Zeroizing::new(serde_json::to_vec(&stored)?);

        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill(&mut nonce_bytes);
        let nonce = Nonce12::from(nonce_bytes);
        let ciphertext = self
            .cipher()?
            .encrypt(&nonce, plaintext.as_slice())
            .map_err(|_| anyhow!("无法加密秘密库"))?;

        let mut bytes = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        bytes.push(VERSION);
        bytes.extend_from_slice(&nonce_bytes);
        bytes.extend_from_slice(&ciphertext);

        let temporary = self.data_path.with_extension("dat.tmp");
        {
            let mut file = std::fs::File::create(&temporary)
                .with_context(|| format!("无法写入秘密库：{}", temporary.display()))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&temporary, &self.data_path)
            .with_context(|| format!("无法替换秘密库：{}", self.data_path.display()))?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Zeroizing<String>>> {
        self.secrets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, reference: &SecretRef) -> Result<Option<Zeroizing<String>>> {
        Ok(self.lock().get(&reference.account()).cloned())
    }

    fn set(&self, reference: &SecretRef, secret: &str) -> Result<()> {
        let mut secrets = self.lock();
        secrets.insert(reference.account(), Zeroizing::new(secret.to_string()));
        self.persist(&secrets)
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        let mut secrets = self.lock();
        if secrets.remove(&reference.account()).is_none() {
            return Ok(());
        }
        self.persist(&secrets)
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// 读已有的密钥，没有就生成一个。密钥文件必须是 32 字节。
fn load_or_create_key(path: &Path) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let key: [u8; KEY_LEN] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("密钥文件长度不对：{}", path.display()))?;
            Ok(Zeroizing::new(key))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut key = Zeroizing::new([0u8; KEY_LEN]);
            rand::rng().fill(&mut key[..]);
            let temporary = path.with_extension("key.tmp");
            {
                let mut file = std::fs::File::create(&temporary)
                    .with_context(|| format!("无法写入密钥文件：{}", temporary.display()))?;
                restrict_permissions(&file)?;
                file.write_all(&key[..])?;
                file.sync_all()?;
            }
            std::fs::rename(&temporary, path)
                .with_context(|| format!("无法替换密钥文件：{}", path.display()))?;
            Ok(key)
        }
        Err(error) => Err(error).with_context(|| format!("无法读取密钥文件：{}", path.display())),
    }
}

#[cfg(unix)]
fn restrict_permissions(file: &std::fs::File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_: &std::fs::File) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_survive_reopening_the_folder() {
        let directory = tempfile::tempdir().unwrap();
        let reference = SecretRef::password("root", "10.0.0.1", 22);
        {
            let store = FileSecretStore::open(directory.path()).unwrap();
            store.set(&reference, "hunter2").unwrap();
        }
        let reopened = FileSecretStore::open(directory.path()).unwrap();
        assert_eq!(
            reopened
                .get(&reference)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("hunter2")
        );
        assert!(directory.path().join(FILE_NAME).is_file());
        assert!(directory.path().join(KEY_FILE_NAME).is_file());
    }

    #[test]
    fn deleting_the_last_secret_leaves_an_empty_vault() {
        let directory = tempfile::tempdir().unwrap();
        let reference = SecretRef::password("root", "10.0.0.1", 22);
        let store = FileSecretStore::open(directory.path()).unwrap();
        store.set(&reference, "hunter2").unwrap();
        store.delete(&reference).unwrap();
        assert!(store.get(&reference).unwrap().is_none());
        let reopened = FileSecretStore::open(directory.path()).unwrap();
        assert!(reopened.get(&reference).unwrap().is_none());
    }

    #[test]
    fn a_passphrase_moves_with_the_folder() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("keys")).unwrap();
        std::fs::write(directory.path().join("keys").join("id"), b"pk").unwrap();
        // 旧机器留下的绝对路径：拷贝过来后文件在，前缀不对。
        let stale = format!("Z:\\gone\\data\\keys{}id", std::path::MAIN_SEPARATOR);
        let relative = SecretRef::passphrase("keys/id");

        {
            let store = FileSecretStore::open(directory.path()).unwrap();
            store
                .set(&SecretRef::passphrase(&stale), "hunter2")
                .unwrap();
        }
        let reopened = FileSecretStore::open(directory.path()).unwrap();
        assert_eq!(
            reopened
                .get(&relative)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("hunter2"),
            "口令跟着私钥路径改到相对路径名下"
        );
    }

    #[test]
    fn a_changed_key_cannot_read_the_old_vault() {
        let directory = tempfile::tempdir().unwrap();
        let reference = SecretRef::password("root", "10.0.0.1", 22);
        {
            let store = FileSecretStore::open(directory.path()).unwrap();
            store.set(&reference, "hunter2").unwrap();
        }
        std::fs::write(directory.path().join(KEY_FILE_NAME), [7u8; KEY_LEN]).unwrap();
        assert!(FileSecretStore::open(directory.path()).is_err());
    }
}
