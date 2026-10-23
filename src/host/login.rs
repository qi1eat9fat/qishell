//! 登录快照：打开连接时向 store 取一次，之后每次重连都用它。
//!
//! 工作线程读不到 store，拿到的是这里的东西：端点和登录方式都已经定好。

use std::path::PathBuf;

use crate::secrets::SecretRef;

use super::{AuthKind, Host, HostId, SSH_PORT};

/// 用哪种方式登录。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginMethod {
    /// 用秘密库里的密码；没有就问用户。
    Password,
    /// 用这个私钥文件登录，口令同样在秘密库里。
    Key,
}

/// 一台主机的登录配置。标签打开时取一次，名字之外的部分一旦定了就不再变
/// （改主机不影响已经打开的标签）。
#[derive(Clone, Debug, PartialEq)]
pub struct HostLogin {
    pub host: HostId,
    /// 打开时主机叫什么，只用于提示文案。
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub method: LoginMethod,
    /// 只有 `Key` 有。已经按当前数据目录解析过，可以直接打开。
    pub key_path: Option<PathBuf>,
    /// 私钥在库里的那个路径（数据目录内的是相对路径）。口令按它归属，
    /// 这样整个文件夹换个位置打开，口令仍然找得到。
    pub key_id: Option<String>,
}

impl HostLogin {
    /// 由一台主机和它的登录方式建快照。
    pub fn of(host: &Host) -> Self {
        let method = match host.auth {
            AuthKind::Password => LoginMethod::Password,
            AuthKind::Key => LoginMethod::Key,
        };
        Self {
            host: host.id,
            name: host.name.clone(),
            address: host.address.clone(),
            port: host.port,
            username: host.username.clone(),
            method,
            key_path: host
                .key_path
                .as_ref()
                .map(|path| crate::app::paths::resolve(std::path::Path::new(path))),
            key_id: host.key_path.clone(),
        }
    }

    /// 密码在秘密库里的归属：按 `用户@地址:端口`，所以改名或移动主机不丢。
    pub fn password_secret(&self) -> SecretRef {
        SecretRef::password(&self.username, &self.address, self.port)
    }

    /// 私钥口令在秘密库里的归属。用库里的路径而不是解析后的绝对路径：
    /// 拷走文件夹后绝对路径变了，按绝对路径记的口令就找不回来了。
    pub fn passphrase_secret(&self) -> Option<SecretRef> {
        self.key_id.as_ref().map(SecretRef::passphrase)
    }

    pub fn endpoint(&self) -> String {
        if self.port == SSH_PORT {
            format!("{}@{}", self.username, self.address)
        } else {
            format!("{}@{}:{}", self.username, self.address, self.port)
        }
    }
}

/// 标签打开时给终端（以及将来的 SFTP）的连接。
#[derive(Clone, Debug, PartialEq)]
pub enum HostConnection {
    Ssh(HostLogin),
}

impl HostConnection {
    pub fn login(&self) -> &HostLogin {
        match self {
            HostConnection::Ssh(login) => login,
        }
    }

    pub fn host(&self) -> HostId {
        self.login().host
    }
}

impl From<HostLogin> for HostConnection {
    fn from(login: HostLogin) -> Self {
        HostConnection::Ssh(login)
    }
}
