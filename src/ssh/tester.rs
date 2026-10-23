//! 「测试连接」：用主机表单里的值真登录一次，失败时给出能看懂的原因。

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::watch;
use zeroize::Zeroizing;

use super::connection::{
    MissingCredential, PasswordWanted, SshConnectionConfig, SshConnector, SshPrompts, lock,
    timeout_excluding_prompts,
};
use crate::connection::{
    ConnectionPrompt, ConnectionPromptKind, ConnectionPromptReply, ConnectionTester, LoginTest,
    TrustCallback,
};
use crate::secrets::{SecretRef, SecretStore, SharedSecretStore};

/// 一次测试最多多久，用户决定要不要信任主机密钥的时间不算。
const TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// 和终端、SFTP 用同一个连接器，主机信任也走同一个文件和锁。
pub struct SshConnectionTester {
    connector: SshConnector,
}

impl SshConnectionTester {
    pub fn new(connector: SshConnector) -> Self {
        Self { connector }
    }
}

impl ConnectionTester for SshConnectionTester {
    fn test(&self, request: LoginTest, trust: TrustCallback) -> Result<(), String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("无法启动测试连接：{error}"))?;
        runtime.block_on(self.run(request, trust))
    }
}

/// 一路上主机密钥检查的结论，连接错误本身说不出来。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HostTrust {
    declined: bool,
    key_changed: bool,
}

impl SshConnectionTester {
    async fn run(&self, request: LoginTest, trust: TrustCallback) -> Result<(), String> {
        let login = request.login();
        if tokio::net::lookup_host((login.address.as_str(), login.port))
            .await
            .is_err()
        {
            return Err(format!("无法解析主机名：{}", login.address));
        }
        let config = SshConnectionConfig::from(&login);
        let secrets: SharedSecretStore =
            Arc::new(FormSecrets::new(&request, self.connector.secrets().clone()));
        let (broker, host_trust, _shutdown) = test_broker(trust);

        let connected = timeout_excluding_prompts(
            self.connector
                .connect_with(&config, broker.clone(), &secrets),
            broker.prompt_activity_receiver(),
            TEST_TIMEOUT,
        )
        .await
        .and_then(|result| result);
        match connected {
            Ok((handle, _)) => {
                let _ = handle
                    .disconnect(russh::Disconnect::ByApplication, "connection test", "zh-CN")
                    .await;
                Ok(())
            }
            // 表单里留空的密码是「没有密码」，不是做错了什么。
            Err(error)
                if request.password_left_empty()
                    && error.chain().any(|cause| cause.is::<PasswordWanted>()) =>
            {
                Err(MissingCredential::Password { rejected: false }.to_string())
            }
            Err(error) => Err(describe_test_failure(&error, *lock(&host_trust))),
        }
    }
}

/// 测试用的问答 broker：不问密码；第一次见到的主机密钥交给 `trust` 决定；
/// 密钥变了直接拒绝（换密钥留给真正的连接）。
fn test_broker(
    trust: TrustCallback,
) -> (Arc<SshPrompts>, Arc<Mutex<HostTrust>>, watch::Sender<bool>) {
    let host_trust = Arc::new(Mutex::new(HostTrust::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker_slot: Arc<OnceLock<Weak<SshPrompts>>> = Arc::new(OnceLock::new());
    let events = {
        let trust = Arc::new(trust);
        let host_trust = host_trust.clone();
        let broker_slot = broker_slot.clone();
        Arc::new(move |prompt: ConnectionPrompt| match prompt.kind() {
            ConnectionPromptKind::UnknownHost(question) => {
                let (trust, host_trust, broker_slot) =
                    (trust.clone(), host_trust.clone(), broker_slot.clone());
                let (request_id, question) = (prompt.request_id(), question.clone());
                // 用户要想一会儿，别占着连接的超时。
                tokio::task::spawn_blocking(move || {
                    let trusted = trust(question);
                    if !trusted {
                        lock(&host_trust).declined = true;
                    }
                    if let Some(broker) = broker_slot.get().and_then(Weak::upgrade) {
                        broker.respond(
                            request_id,
                            if trusted {
                                ConnectionPromptReply::TrustAndSave
                            } else {
                                ConnectionPromptReply::Cancel
                            },
                        );
                    }
                });
                true
            }
            ConnectionPromptKind::HostKeyChanged(_) => {
                lock(&host_trust).key_changed = true;
                false
            }
            // 凭据问题到不了这里：broker 是非交互的，直接以缺凭据失败。
            ConnectionPromptKind::Authentication(_) => false,
        })
    };
    let broker = Arc::new(SshPrompts::new(events, shutdown_rx).non_interactive());
    let _ = broker_slot.set(Arc::downgrade(&broker));
    (broker, host_trust, shutdown_tx)
}

fn describe_test_failure(error: &anyhow::Error, host_trust: HostTrust) -> String {
    if host_trust.key_changed {
        return "主机密钥与已保存的不一致".into();
    }
    if host_trust.declined {
        return "未信任该主机的密钥".into();
    }
    describe_login_error(error)
}

/// 登录失败的原因：网络自己的回答、缺少凭据、算法谈不拢，或者错误链最里
/// 面的那句话——只说「无法建立 SSH 连接」没法让人知道改什么。
pub fn describe_login_error(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        if let Some(need) = cause.downcast_ref::<MissingCredential>() {
            return need.to_string();
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return describe_connect_error(io);
        }
        match cause.downcast_ref::<russh::Error>() {
            Some(russh::Error::IO(io)) => return describe_connect_error(io),
            Some(russh::Error::ConnectionTimeout) => return "连接超时".into(),
            Some(russh::Error::NoCommonAlgo { kind, theirs, .. }) => {
                return no_common_algorithm(kind, &theirs.join("、"));
            }
            _ => {}
        }
    }
    format!("{error:#}")
}

fn no_common_algorithm(kind: &russh::AlgorithmKind, theirs: &str) -> String {
    let what = match kind {
        russh::AlgorithmKind::Kex => "密钥交换",
        russh::AlgorithmKind::Key => "主机密钥",
        russh::AlgorithmKind::Cipher => "加密",
        russh::AlgorithmKind::Mac => "消息校验（MAC）",
        russh::AlgorithmKind::Compression => "压缩",
    };
    format!("和服务器没有共同的{what}算法，服务器支持的是 {theirs}")
}

fn describe_connect_error(error: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::ConnectionRefused => "连接被拒绝，该端口上没有服务在监听".into(),
        ErrorKind::TimedOut => "连接超时".into(),
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => "主机不可达".into(),
        _ => format!("连接失败：{error}"),
    }
}

/// 测试登录用的秘密：表单里填的密码/口令优先于秘密库里的，其余从秘密库
/// 读。从不写入。
struct FormSecrets {
    typed: Vec<(SecretRef, Option<Zeroizing<String>>)>,
    fallback: SharedSecretStore,
}

impl FormSecrets {
    fn new(request: &LoginTest, fallback: SharedSecretStore) -> Self {
        let owned = |value: Option<&str>| value.map(|value| Zeroizing::new(value.to_string()));
        let login = request.login();
        let mut typed = Vec::new();
        typed.push((login.password_secret(), owned(request.password.as_deref())));
        if let Some(secret) = login.passphrase_secret() {
            typed.push((secret, owned(request.passphrase.as_deref())));
        }
        Self { typed, fallback }
    }
}

impl SecretStore for FormSecrets {
    fn get(&self, secret: &SecretRef) -> anyhow::Result<Option<Zeroizing<String>>> {
        match self.typed.iter().find(|(key, _)| key == secret) {
            // 表单里留空就是没有，不回落到秘密库。
            Some((_, value)) => Ok(value.clone()),
            None => self.fallback.get(secret),
        }
    }

    fn set(&self, _: &SecretRef, _: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn delete(&self, _: &SecretRef) -> anyhow::Result<()> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        false
    }
}
