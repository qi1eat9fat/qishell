//! 秘密：主机密码与私钥口令存在应用目录的加密文件里，**数据库里永远不出现**。
//!
//! 接口全是阻塞的，只在后台执行器或工作线程上调用，别在界面线程上用。
//!
//! 选文件而不是系统钥匙串，是为了「拷贝整个应用文件夹就能迁移」；代价是
//! 拿到文件夹的人也能拿到解密密钥。详见 [`file`]。

mod file;
mod memory;

use std::path::Path;
use std::sync::Arc;

use zeroize::Zeroizing;

pub use file::{FILE_NAME, FileSecretStore, KEY_FILE_NAME};
pub use memory::{InMemorySecretStore, NoSecretStore};

/// 一条秘密在秘密库里的归属。按「哪个用户连到哪个端点」记，所以改名、改
/// 备注、移动主机都不会丢密码。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SecretRef {
    /// `用户@地址:端口` 的密码。
    Password {
        username: String,
        address: String,
        port: u16,
    },
    /// 某个私钥文件的口令，按路径归属：同一把钥匙只问一次。
    Passphrase { key_path: String },
}

impl SecretRef {
    pub fn password(username: &str, address: &str, port: u16) -> Self {
        SecretRef::Password {
            username: username.to_string(),
            address: address.to_string(),
            port,
        }
    }

    pub fn passphrase(key_path: impl AsRef<std::path::Path>) -> Self {
        SecretRef::Passphrase {
            key_path: key_path.as_ref().display().to_string(),
        }
    }

    /// 秘密库里的账号名。
    pub fn account(&self) -> String {
        match self {
            SecretRef::Password {
                username,
                address,
                port,
            } => format!("password:{username}@{address}:{port}"),
            SecretRef::Passphrase { key_path } => format!("passphrase:{key_path}"),
        }
    }
}

/// 秘密库的读写。实现都是阻塞的。
pub trait SecretStore: Send + Sync + 'static {
    fn get(&self, reference: &SecretRef) -> anyhow::Result<Option<Zeroizing<String>>>;
    fn set(&self, reference: &SecretRef, secret: &str) -> anyhow::Result<()>;
    fn delete(&self, reference: &SecretRef) -> anyhow::Result<()>;
    /// 秘密库现在能不能用。
    fn is_available(&self) -> bool;
}

pub type SharedSecretStore = Arc<dyn SecretStore>;

/// 应用目录里的秘密库。打不开时退回「什么都不存」，界面仍能用，只是密码
/// 每次都要重新输入。
pub fn local_secret_store(data_dir: &Path) -> SharedSecretStore {
    match FileSecretStore::open(data_dir) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            eprintln!("qishell: 无法打开秘密库: {error}");
            Arc::new(NoSecretStore)
        }
    }
}
