//! 主机模型：库里的一台主机，以及表单提交上来的草稿。

use gpui_kit::SharedString;
use rand::Rng as _;

pub const SSH_PORT: u16 = 22;
pub const DEFAULT_USER: &str = "root";

/// 主机的内部 id。重启后从库里最大的 id + 1 继续，删掉的最大号会被复用。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostId(pub u64);

impl std::fmt::Display for HostId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// 对外的稳定标识：16 位随机串，改名、改地址都不变，也不复用。
/// 界面暂时不用，但将来给外部工具（CLI）留的。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PublicId(SharedString);

impl PublicId {
    fn alphabet() -> &'static [u8] {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
    }

    /// 一个新的随机 id。
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        let alphabet = Self::alphabet();
        let text: String = (0..16)
            .map(|_| alphabet[rng.random_range(0..alphabet.len())] as char)
            .collect();
        Self(text.into())
    }

    /// 避开 `in_use` 里的那些。
    pub fn generate_unused(in_use: &[PublicId]) -> Self {
        loop {
            let candidate = Self::generate();
            if !in_use.contains(&candidate) {
                return candidate;
            }
        }
    }

    /// 库里存的形状是否对：16 位字母数字。
    pub fn has_shape(text: &str) -> bool {
        text.len() == 16 && text.bytes().all(|byte| Self::alphabet().contains(&byte))
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        Self::has_shape(text).then(|| Self(text.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 登录方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthKind {
    #[default]
    Password,
    Key,
}

impl AuthKind {
    /// 存进库里的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            AuthKind::Password => "password",
            AuthKind::Key => "key",
        }
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        match text {
            "password" => Some(AuthKind::Password),
            "key" => Some(AuthKind::Key),
            _ => None,
        }
    }
}

/// 库里的一台主机。秘密不在这里：密码在加密秘密库，私钥只记路径。
#[derive(Clone, Debug, PartialEq)]
pub struct Host {
    pub id: HostId,
    pub public_id: PublicId,
    pub sort_order: i64,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthKind,
    /// 只在 `auth == Key` 时有。
    pub key_path: Option<String>,
    pub notes: String,
    /// 上次连上的时间（秒），`None` 表示没连过。
    pub last_connected_at: Option<u64>,
}

impl Host {
    /// 由表单草稿建一台主机。
    pub fn new(id: HostId, draft: HostDraft) -> Self {
        Self {
            id,
            public_id: PublicId::generate(),
            sort_order: 0,
            name: draft.name,
            address: draft.address,
            port: draft.port,
            username: draft.username,
            auth: draft.auth,
            key_path: draft.key_path,
            notes: draft.notes,
            last_connected_at: None,
        }
    }

    /// 悬停与状态栏用的 `用户@地址:端口`。
    pub fn endpoint(&self) -> String {
        if self.port == SSH_PORT {
            format!("{}@{}", self.username, self.address)
        } else {
            format!("{}@{}:{}", self.username, self.address, self.port)
        }
    }

    /// 名字首字，给主机图标用。
    pub fn initial(&self) -> String {
        self.name.chars().next().unwrap_or('?').to_string()
    }

    /// 名字留空时用地址。
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.address
        } else {
            &self.name
        }
    }

    pub fn draft(&self) -> HostDraft {
        HostDraft {
            name: self.name.clone(),
            address: self.address.clone(),
            port: self.port,
            username: self.username.clone(),
            auth: self.auth,
            key_path: self.key_path.clone(),
            notes: self.notes.clone(),
        }
    }
}

/// 表单提交上来的值。**不含任何秘密**，所以派生了 `Debug` 也安全。
#[derive(Clone, Debug, PartialEq)]
pub struct HostDraft {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthKind,
    pub key_path: Option<String>,
    pub notes: String,
}

impl HostDraft {
    pub fn new(
        name: impl Into<String>,
        address: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        auth: AuthKind,
    ) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
            port,
            username: username.into(),
            auth,
            key_path: None,
            notes: String::new(),
        }
    }

    /// 去掉首尾空白、补齐默认值，并校验。
    pub fn validated(mut self) -> Result<Self, HostDraftError> {
        self.name = self.name.trim().to_string();
        self.address = self.address.trim().to_string();
        self.username = self.username.trim().to_string();
        self.notes = self.notes.trim().to_string();
        self.key_path = self
            .key_path
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());

        if self.address.is_empty() {
            return Err(HostDraftError::NoAddress);
        }
        // 名字留空就用地址。
        if self.name.is_empty() {
            self.name = self.address.clone();
        }
        if self.username.is_empty() {
            self.username = DEFAULT_USER.to_string();
        }
        if self.port == 0 {
            return Err(HostDraftError::NoPort);
        }
        // 密钥登录必须选一把私钥。
        if self.auth == AuthKind::Key && self.key_path.is_none() {
            return Err(HostDraftError::NoKeyPath);
        }
        // 密码登录不留路径，否则库里那条 CHECK 不成立。
        if self.auth == AuthKind::Password {
            self.key_path = None;
        }
        Ok(self)
    }
}

/// 草稿不合法的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostDraftError {
    NoAddress,
    NoPort,
    NoKeyPath,
}

impl HostDraftError {
    pub fn message(self) -> SharedString {
        match self {
            HostDraftError::NoAddress => "请输入主机地址".into(),
            HostDraftError::NoPort => "端口无效".into(),
            HostDraftError::NoKeyPath => "请选择私钥文件".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HostDraft {
        HostDraft::new("atlas", "10.0.0.1", 22, "ops", AuthKind::Password)
    }

    #[test]
    fn a_password_draft_fills_defaults_and_drops_key_paths() {
        let mut draft = sample();
        draft.name = "  ".into();
        draft.username = " ".into();
        draft.key_path = Some("  ".into());
        let draft = draft.validated().unwrap();
        assert_eq!(
            draft.name, "10.0.0.1",
            "an empty name falls back to the address"
        );
        assert_eq!(draft.username, DEFAULT_USER);
        assert_eq!(draft.key_path, None);
    }

    #[test]
    fn a_key_draft_needs_a_key_file() {
        let mut draft = sample();
        draft.auth = AuthKind::Key;
        assert_eq!(draft.clone().validated(), Err(HostDraftError::NoKeyPath));
        draft.key_path = Some("C:/keys/id_ed25519".into());
        assert_eq!(
            draft.validated().unwrap().key_path.as_deref(),
            Some("C:/keys/id_ed25519")
        );
    }

    #[test]
    fn an_address_and_a_port_are_required() {
        let mut draft = sample();
        draft.address = " ".into();
        assert_eq!(draft.validated(), Err(HostDraftError::NoAddress));
        let mut draft = sample();
        draft.port = 0;
        assert_eq!(draft.validated(), Err(HostDraftError::NoPort));
    }

    #[test]
    fn public_ids_have_the_stored_shape() {
        let id = PublicId::generate();
        assert!(PublicId::has_shape(id.as_str()));
        assert_eq!(PublicId::from_stored(id.as_str()), Some(id));
        assert_eq!(PublicId::from_stored("short"), None);
        assert_eq!(PublicId::from_stored("bad/id/with/slash"), None);
    }
}
