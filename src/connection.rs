//! 连接问答与测试连接：SSH 登录过程中要问用户的事，都在这里定形。
//!
//! 登录在工作线程上进行，问不出口，所以它把问题发回界面线程，界面弹出
//! 对话框，再把答案经同一通道送回去。

use std::path::PathBuf;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::host::{HostId, LoginMethod};

/// 一次登录要问的问题。
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionPrompt {
    pub request_id: u64,
    pub kind: ConnectionPromptKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionPromptKind {
    /// 这台主机的密钥没见过。
    UnknownHost(UnknownHostPrompt),
    /// 见过的密钥和这次的不一样。
    HostKeyChanged(HostKeyChangedPrompt),
    /// 服务器要密码（或验证码）。
    Authentication(AuthenticationPrompt),
}

impl ConnectionPromptKind {
    pub fn unknown_host(
        host: impl Into<String>,
        port: u16,
        algorithm: impl Into<String>,
        fingerprint: impl Into<String>,
    ) -> Self {
        ConnectionPromptKind::UnknownHost(UnknownHostPrompt {
            host: host.into(),
            port,
            algorithm: algorithm.into(),
            fingerprint: fingerprint.into(),
        })
    }

    pub fn host_key_changed(
        host: impl Into<String>,
        port: u16,
        algorithm: impl Into<String>,
        old_fingerprints: Vec<String>,
        fingerprint: impl Into<String>,
    ) -> Self {
        ConnectionPromptKind::HostKeyChanged(HostKeyChangedPrompt {
            host: host.into(),
            port,
            algorithm: algorithm.into(),
            old_fingerprints,
            fingerprint: fingerprint.into(),
        })
    }

    pub fn authentication(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        title: impl Into<String>,
        instructions: impl Into<String>,
        prompts: Vec<AuthenticationQuestion>,
    ) -> Self {
        ConnectionPromptKind::Authentication(AuthenticationPrompt {
            host: host.into(),
            port,
            username: username.into(),
            title: title.into(),
            instructions: instructions.into(),
            prompts,
        })
    }
}

impl ConnectionPrompt {
    pub fn new(request_id: u64, kind: ConnectionPromptKind) -> Self {
        Self { request_id, kind }
    }

    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn kind(&self) -> &ConnectionPromptKind {
        &self.kind
    }

    pub fn unknown_host(host: String, port: u16, algorithm: String, fingerprint: String) -> Self {
        Self {
            request_id: 0,
            kind: ConnectionPromptKind::UnknownHost(UnknownHostPrompt {
                host,
                port,
                algorithm,
                fingerprint,
            }),
        }
    }

    pub fn host_key_changed(
        host: String,
        port: u16,
        algorithm: String,
        old_fingerprints: Vec<String>,
        fingerprint: String,
    ) -> Self {
        Self {
            request_id: 0,
            kind: ConnectionPromptKind::HostKeyChanged(HostKeyChangedPrompt {
                host,
                port,
                algorithm,
                old_fingerprints,
                fingerprint,
            }),
        }
    }

    pub fn authentication(
        host: String,
        port: u16,
        username: String,
        title: String,
        instructions: String,
        prompts: Vec<AuthenticationQuestion>,
    ) -> Self {
        Self {
            request_id: 0,
            kind: ConnectionPromptKind::Authentication(AuthenticationPrompt {
                host,
                port,
                username,
                title,
                instructions,
                prompts,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnknownHostPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

/// 把 `SHA256:` 指纹按 4 个字符一组分开，方便人眼逐段核对。
///
/// 只改显示：复制时仍用原始值。其他格式（例如带冒号的 MD5）原样返回。
pub fn grouped_fingerprint(fingerprint: &str) -> String {
    let trimmed = fingerprint.trim();
    let Some(payload) = trimmed.strip_prefix("SHA256:") else {
        return trimmed.to_string();
    };
    if payload.is_empty() || payload.contains(':') {
        return trimmed.to_string();
    }
    let mut grouped = String::with_capacity(payload.len() + payload.len() / 4);
    for (index, character) in payload.chars().enumerate() {
        if index > 0 && index % 4 == 0 {
            grouped.push(' ');
        }
        grouped.push(character);
    }
    format!("SHA256:{grouped}")
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostKeyChangedPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub old_fingerprints: Vec<String>,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthenticationPrompt {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// 对话框标题，例如「输入密码」。
    pub title: String,
    /// 标题下的一段说明。
    pub instructions: String,
    pub prompts: Vec<AuthenticationQuestion>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthenticationQuestion {
    pub text: String,
    /// 输入要不要遮起来（密码是，验证码不是）。
    pub echo: bool,
}

/// 用户输入的密码/回答。`Debug` 不露明文。
#[derive(Clone)]
pub struct ConnectionSecret(Zeroizing<String>);

impl ConnectionSecret {
    pub fn new(secret: String) -> Self {
        Self(Zeroizing::new(secret))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ConnectionSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<已隐藏>")
    }
}

/// 用户对一次提问的回答。
pub enum ConnectionPromptReply {
    /// 信任并记下这把密钥。
    TrustAndSave,
    /// 认证问题的答案，顺序与提问一致。
    Answers(Vec<ConnectionSecret>),
    /// 用户取消了。
    Cancel,
}

impl std::fmt::Debug for ConnectionPromptReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionPromptReply::TrustAndSave => formatter.write_str("TrustAndSave"),
            ConnectionPromptReply::Answers(answers) => formatter
                .debug_tuple("Answers")
                .field(&format_args!("{} 条（已隐藏）", answers.len()))
                .finish(),
            ConnectionPromptReply::Cancel => formatter.write_str("Cancel"),
        }
    }
}

/// 问「这台主机的密钥能不能信」的回调：返回 `true` 就记下来。
/// 没人能答的场合（测试连接、重连）返回 `false`，当场拒绝。
pub type TrustCallback = Box<dyn Fn(UnknownHostPrompt) -> bool + Send + Sync>;

/// 测试连接要试的登录。
#[derive(Clone, Debug)]
pub struct LoginTest {
    pub host: HostId,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub method: LoginMethod,
    pub key_path: Option<PathBuf>,
    /// 表单里当场输入的密码（可能为空，那就按「没有密码」试）。
    pub password: Option<String>,
    /// 表单里当场输入的口令。
    pub passphrase: Option<String>,
}

impl LoginTest {
    pub fn endpoint(&self) -> String {
        format!("{}@{}:{}", self.username, self.address, self.port)
    }

    /// 测试用的登录快照。
    pub fn login(&self) -> crate::host::HostLogin {
        // 表单给的是完整路径，来自主机的可能是库里的相对路径，统一按当前
        // 数据目录解析一次；`resolve` 对绝对路径原样返回，转两次不会走样。
        crate::host::HostLogin {
            host: self.host,
            name: self.name.clone(),
            address: self.address.clone(),
            port: self.port,
            username: self.username.clone(),
            method: self.method,
            key_path: self.key_path.as_deref().map(crate::app::paths::resolve),
            key_id: self.key_path.as_deref().map(crate::app::paths::stored),
        }
    }

    /// 表单里的密码留空（按「没有密码」试一次）。
    pub fn password_left_empty(&self) -> bool {
        self.password.as_deref().is_none_or(str::is_empty)
    }
}

/// 试一次登录。**阻塞**：调用方自己开线程。
pub trait ConnectionTester: Send + Sync + 'static {
    fn test(&self, test: LoginTest, trust: TrustCallback) -> Result<(), String>;
}

pub type SharedConnectionTester = std::sync::Arc<dyn ConnectionTester>;

/// 一条活动连接的一次往返耗时。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Latency {
    Measured(Duration),
    TimedOut,
}

#[cfg(test)]
mod tests {
    use super::grouped_fingerprint;

    #[test]
    fn a_sha256_fingerprint_is_grouped_in_fours() {
        assert_eq!(
            grouped_fingerprint("SHA256:cxN98YJ1hoWsjPSRTq3UamJUHSiMDoVxydPEFaZBDS"),
            "SHA256:cxN9 8YJ1 hoWs jPSR Tq3U amJU HSiM DoVx ydPE FaZB DS"
        );
    }

    #[test]
    fn other_formats_are_left_alone() {
        assert_eq!(grouped_fingerprint("MD5:aa:bb:cc:dd"), "MD5:aa:bb:cc:dd");
        assert_eq!(
            grouped_fingerprint("not-a-fingerprint"),
            "not-a-fingerprint"
        );
        assert_eq!(grouped_fingerprint(""), "");
    }
}
