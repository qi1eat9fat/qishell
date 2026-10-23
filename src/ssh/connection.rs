//! SSH 连接与认证：终端、测试连接共用这里。
//!
//! 主机密钥记在应用自己的 `known_hosts` 里，不碰用户的 `~/.ssh`。第一次见
//! 到的密钥、以及和记录不符的密钥，都把问题交给界面上的 [`SshPrompts`]。

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use russh::keys::{
    HashAlg, PrivateKeyWithHashAlg, PublicKey,
    known_hosts::{known_host_keys_path, learn_known_hosts_path},
    load_secret_key, parse_public_key_base64,
};
use russh::{
    MethodKind, MethodSet,
    client::{self, AuthResult, KeyboardInteractiveAuthResponse},
};
use tokio::net::TcpStream;
use tokio::sync::{oneshot, watch};
use zeroize::Zeroizing;

use crate::connection::{
    ConnectionPrompt, ConnectionPromptKind, ConnectionPromptReply, ConnectionSecret,
};
use crate::host::{HostLogin, LoginMethod};
use crate::secrets::{SecretRef, SharedSecretStore};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const AUTH_RETRIES: usize = 3;
static NEXT_PROMPT_ID: AtomicU64 = AtomicU64::new(1);

/// 一次登录要用的主机快照。
#[derive(Clone)]
pub struct SshConnectionConfig {
    login: HostLogin,
}

impl From<&HostLogin> for SshConnectionConfig {
    fn from(login: &HostLogin) -> Self {
        Self {
            login: login.clone(),
        }
    }
}

impl SshConnectionConfig {
    pub fn endpoint(&self) -> String {
        self.login.endpoint()
    }
}

/// 可克隆的连接器。所有克隆共用一把 known_hosts 写锁。
#[derive(Clone)]
pub struct SshConnector {
    known_hosts_path: PathBuf,
    known_hosts_lock: Arc<Mutex<()>>,
    secrets: SharedSecretStore,
}

impl SshConnector {
    pub fn new(path: impl Into<PathBuf>, secrets: SharedSecretStore) -> Self {
        Self {
            known_hosts_path: path.into(),
            known_hosts_lock: Arc::new(Mutex::new(())),
            secrets,
        }
    }

    pub(crate) fn secrets(&self) -> &SharedSecretStore {
        &self.secrets
    }

    /// 登录并返回连接句柄和服务器指纹。**阻塞**：只在工作线程上调用。
    pub async fn connect(
        &self,
        config: &SshConnectionConfig,
        broker: Arc<SshPrompts>,
    ) -> Result<(SshHandle, String)> {
        self.connect_with(config, broker, &self.secrets).await
    }

    /// 与 [`Self::connect`] 相同，但登录时的秘密从 `secrets` 读（测试连接用
    /// 表单里的值覆盖已保存的值）。
    pub(crate) async fn connect_with(
        &self,
        config: &SshConnectionConfig,
        broker: Arc<SshPrompts>,
        secrets: &SharedSecretStore,
    ) -> Result<(SshHandle, String)> {
        let login = &config.login;
        let tunnel = step(&broker, tcp(&login.address, login.port)).await?;
        self.log_in(tunnel, login, &broker, secrets).await
    }

    async fn log_in(
        &self,
        tunnel: TcpStream,
        login: &HostLogin,
        broker: &Arc<SshPrompts>,
        secrets: &SharedSecretStore,
    ) -> Result<(SshHandle, String)> {
        let fingerprint = Arc::new(Mutex::new(String::new()));
        let handler = SshClientHandler {
            host: login.address.clone(),
            port: login.port,
            known_hosts_path: self.known_hosts_path.clone(),
            known_hosts_lock: self.known_hosts_lock.clone(),
            broker: broker.clone(),
            fingerprint: fingerprint.clone(),
        };
        let connect = async {
            client::connect_stream(ssh_config(), tunnel, handler)
                .await
                .map_err(|error| error.context("无法建立 SSH 连接"))
        };
        let mut handle = step(broker, connect).await?;
        let mut shutdown = broker.shutdown_receiver();
        tokio::select! {
            result = authenticate(&mut handle, login, secrets, broker) => result?,
            _ = shutdown.changed() => bail!("连接已取消"),
        }
        let fingerprint = lock(&fingerprint).clone();
        Ok((handle, fingerprint))
    }
}

/// 客户端提供的密钥交换算法，按 OpenSSH 的顺序：russh 自己的列表里没有
/// NIST 曲线，而老 Apache SSHD 堡垒机只会这些。
const KEX_ORDER: &[russh::kex::Name] = &[
    russh::kex::MLKEM768X25519_SHA256,
    russh::kex::CURVE25519,
    russh::kex::CURVE25519_PRE_RFC_8731,
    russh::kex::ECDH_SHA2_NISTP256,
    russh::kex::ECDH_SHA2_NISTP384,
    russh::kex::ECDH_SHA2_NISTP521,
    russh::kex::DH_GEX_SHA256,
    russh::kex::DH_G16_SHA512,
    russh::kex::DH_G18_SHA512,
    russh::kex::DH_G17_SHA512,
    russh::kex::DH_G15_SHA512,
    russh::kex::DH_G14_SHA256,
    russh::kex::EXTENSION_SUPPORT_AS_CLIENT,
    russh::kex::EXTENSION_SUPPORT_AS_SERVER,
    russh::kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
    russh::kex::EXTENSION_OPENSSH_STRICT_KEX_AS_SERVER,
];

/// 组交换接受的最小素数位数：russh 默认要 3072 位，老 Java 的 Apache SSHD
/// 最大只有 2048 位。
const MIN_DH_GROUP_BITS: usize = 2048;

fn ssh_config() -> Arc<client::Config> {
    let defaults = client::GexParams::default();
    Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: 3,
        nodelay: true,
        preferred: russh::Preferred {
            kex: KEX_ORDER.into(),
            ..russh::Preferred::default()
        },
        gex: client::GexParams::new(
            MIN_DH_GROUP_BITS,
            defaults.preferred_group_size(),
            defaults.max_group_size(),
        )
        .expect("2048 位是 russh 允许的最小值"),
        ..Default::default()
    })
}

/// 一步连接动作：15 秒内没结果就算超时，但等用户回答的时间不算。
async fn step<T>(broker: &SshPrompts, future: impl Future<Output = Result<T>>) -> Result<T> {
    let mut shutdown = broker.shutdown_receiver();
    tokio::select! {
        result = timeout_excluding_prompts(
            future,
            broker.prompt_activity_receiver(),
            CONNECT_TIMEOUT,
        ) => result?,
        _ = shutdown.changed() => bail!("连接已取消"),
    }
}

async fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    let socket = TcpStream::connect((host, port))
        .await
        .map_err(russh::Error::IO)
        .with_context(|| format!("无法连接到 {host}:{port}"))?;
    let _ = socket.set_nodelay(true);
    Ok(socket)
}

pub struct SshPrompts {
    pending: Mutex<HashMap<u64, oneshot::Sender<ConnectionPromptReply>>>,
    events: Arc<dyn Fn(ConnectionPrompt) -> bool + Send + Sync>,
    shutdown: watch::Receiver<bool>,
    prompt_activity: watch::Sender<bool>,
    /// 是否允许向用户提问。测试连接等没人能答的场合为 `false`，缺凭据时
    /// 直接以 [`MissingCredential`] 失败。
    interactive: bool,
}

/// 认证需要凭据却没人能回答（测试连接、重连）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingCredential {
    Password { rejected: bool },
    Passphrase { rejected: bool },
    KeyboardInteractive,
}

impl std::fmt::Display for MissingCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            MissingCredential::Password { rejected: true } => "用户名或密码错误",
            MissingCredential::Password { rejected: false } => "未填写密码",
            MissingCredential::Passphrase { rejected: true } => "私钥口令错误",
            MissingCredential::Passphrase { rejected: false } => "私钥已加密，未填写口令",
            MissingCredential::KeyboardInteractive => "服务器要求键盘交互式认证",
        })
    }
}

impl std::error::Error for MissingCredential {}

/// 选了「密码」却把密码留空的登录，服务器偏偏要密码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasswordWanted;

impl std::fmt::Display for PasswordWanted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("服务器要求输入密码")
    }
}

impl std::error::Error for PasswordWanted {}

impl SshPrompts {
    pub fn new(
        events: Arc<dyn Fn(ConnectionPrompt) -> bool + Send + Sync>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        let (prompt_activity, _) = watch::channel(false);
        Self {
            pending: Mutex::new(HashMap::new()),
            events,
            shutdown,
            prompt_activity,
            interactive: true,
        }
    }

    /// 没有人能回答时使用：需要凭据就直接失败。
    pub fn non_interactive(mut self) -> Self {
        self.interactive = false;
        self
    }

    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.shutdown.clone()
    }

    pub(crate) fn prompt_activity_receiver(&self) -> watch::Receiver<bool> {
        self.prompt_activity.subscribe()
    }

    async fn ask(&self, kind: ConnectionPromptKind) -> Result<ConnectionPromptReply> {
        let request_id = NEXT_PROMPT_ID.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        lock(&self.pending).insert(request_id, sender);
        let _ = self.prompt_activity.send(true);
        let result = async {
            if !(self.events)(ConnectionPrompt::new(request_id, kind)) {
                bail!("连接窗口已关闭");
            }
            let mut shutdown = self.shutdown.clone();
            tokio::select! {
                reply = receiver => reply.map_err(|_| anyhow!("认证请求已取消")),
                _ = shutdown.changed() => bail!("连接已取消"),
            }
        }
        .await;
        lock(&self.pending).remove(&request_id);
        let _ = self.prompt_activity.send(false);
        result
    }

    async fn ask_credential(
        &self,
        need: MissingCredential,
        kind: ConnectionPromptKind,
    ) -> Result<ConnectionPromptReply> {
        if !self.interactive {
            return Err(need.into());
        }
        self.ask(kind).await
    }

    pub fn respond(&self, request_id: u64, reply: ConnectionPromptReply) {
        if let Some(sender) = lock(&self.pending).remove(&request_id) {
            let _ = sender.send(reply);
        }
    }

    pub fn cancel_all(&self) {
        lock(&self.pending).clear();
    }
}

/// 网络握手超时，但用户正在回答的问题不算进超时。
pub(crate) async fn timeout_excluding_prompts<F>(
    future: F,
    mut prompt_activity: watch::Receiver<bool>,
    timeout: Duration,
) -> Result<F::Output>
where
    F: Future,
{
    tokio::pin!(future);
    let mut remaining = timeout;
    loop {
        if *prompt_activity.borrow() {
            tokio::select! {
                result = &mut future => return Ok(result),
                changed = prompt_activity.changed() => {
                    if changed.is_err() {
                        bail!("连接已取消");
                    }
                }
            }
            continue;
        }

        let started = tokio::time::Instant::now();
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep(remaining) => return Err(russh::Error::ConnectionTimeout.into()),
            changed = prompt_activity.changed() => {
                if changed.is_err() {
                    bail!("连接已取消");
                }
                remaining = remaining.saturating_sub(started.elapsed());
                if remaining.is_zero() && !*prompt_activity.borrow() {
                    return Err(russh::Error::ConnectionTimeout.into());
                }
            }
        }
    }
}

pub type SshHandle = client::Handle<SshClientHandler>;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

pub struct SshClientHandler {
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    known_hosts_lock: Arc<Mutex<()>>,
    broker: Arc<SshPrompts>,
    fingerprint: Arc<Mutex<String>>,
}

impl client::Handler for SshClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_public_key.public_key();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        *lock(&self.fingerprint) = fingerprint.clone();
        let algorithm = key.algorithm().to_string();
        let known = {
            let _guard = lock(&self.known_hosts_lock);
            read_known_keys(&self.host, self.port, &self.known_hosts_path)?
        };
        if known.iter().any(|(_, saved)| saved == &key) {
            return Ok(true);
        }
        // 没见过的密钥，或和记下的不一样：换过系统的服务器，或者有人在
        // 冒充它。让用户决定；说「信任」就把这台主机记下的旧密钥换掉。
        let question = if known.is_empty() {
            ConnectionPromptKind::unknown_host(&self.host, self.port, algorithm, fingerprint)
        } else {
            let old = known
                .iter()
                .map(|(_, saved)| saved.fingerprint(HashAlg::Sha256).to_string())
                .collect();
            ConnectionPromptKind::host_key_changed(
                &self.host,
                self.port,
                algorithm,
                old,
                fingerprint,
            )
        };
        let reply = self.broker.ask(question).await?;
        if !matches!(reply, ConnectionPromptReply::TrustAndSave) {
            return Ok(false);
        }
        let _guard = lock(&self.known_hosts_lock);
        let now = read_known_keys(&self.host, self.port, &self.known_hosts_path)?;
        if now.iter().any(|(_, saved)| saved == &key) {
            return Ok(true);
        }
        // 用户的回答基于当时记下的密钥；这期间可能有别的连接写进去了。
        if !now
            .iter()
            .map(|(_, saved)| saved)
            .eq(known.iter().map(|(_, saved)| saved))
        {
            bail!("保存主机密钥时，记录已被其他连接修改");
        }
        let write_failed = || anyhow!("无法写入主机信任文件：{}", self.known_hosts_path.display());
        if !known.is_empty() {
            forget_known_host(&self.host, self.port, &self.known_hosts_path)
                .map_err(|_| write_failed())?;
        }
        learn_known_hosts_path(&self.host, self.port, &key, &self.known_hosts_path)
            .map_err(|_| write_failed())?;
        Ok(true)
    }
}

/// 网络或连接本身失败，而不是服务器拒绝登录、用户拒绝主机密钥。只有这类
/// 错误值得不打扰用户地自动重试。
pub fn is_network_error(error: &anyhow::Error) -> bool {
    use std::io::ErrorKind;
    error.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                ErrorKind::NotConnected
                    | ErrorKind::ConnectionRefused
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::TimedOut
                    | ErrorKind::UnexpectedEof
                    | ErrorKind::BrokenPipe
                    | ErrorKind::HostUnreachable
                    | ErrorKind::NetworkUnreachable
                    | ErrorKind::NetworkDown
            )
        }) || matches!(
            cause.downcast_ref::<russh::Error>(),
            Some(
                russh::Error::IO(_)
                    | russh::Error::Disconnect
                    | russh::Error::HUP
                    | russh::Error::ConnectionTimeout
                    | russh::Error::KeepaliveTimeout
                    | russh::Error::InactivityTimeout
                    | russh::Error::SendError
            )
        )
    })
}

/// 删掉这台主机在这个端口上记下的全部密钥，同 `ssh-keygen -R`。
fn forget_known_host(host: &str, port: u16, path: &Path) -> std::io::Result<()> {
    let name = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    let contents = std::fs::read_to_string(path)?;
    let kept: String = contents
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with('#')
                || !line
                    .split_whitespace()
                    .next()
                    .is_some_and(|hosts| hosts.split(',').any(|entry| entry == name))
        })
        .map(|line| format!("{line}\n"))
        .collect();
    // 一步替换，半路崩溃也不会丢掉别的主机的密钥。
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, kept)?;
    std::fs::rename(&temporary, path)
}

fn read_known_keys(host: &str, port: u16, path: &Path) -> Result<Vec<(usize, PublicKey)>> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("无法读取主机信任文件：{}", path.display()))?;
        for line in contents.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let valid = fields.next().is_some()
                && fields.next().is_some()
                && fields
                    .next()
                    .is_some_and(|encoded| parse_public_key_base64(encoded).is_ok());
            if !valid {
                bail!("主机信任文件已损坏：{}", path.display());
            }
        }
    }
    known_host_keys_path(host, port, path)
        .map_err(|_| anyhow!("无法读取主机信任文件：{}", path.display()))
}

async fn authenticate(
    handle: &mut SshHandle,
    login: &HostLogin,
    secrets: &SharedSecretStore,
    broker: &Arc<SshPrompts>,
) -> Result<()> {
    let user = login.username.as_str();
    let method = login.method;
    let first = handle
        .authenticate_none(user)
        .await
        .map_err(|_| anyhow!("无法识别服务器支持的认证方式"))?;
    if first.success() {
        return Ok(());
    }
    let mut methods = remaining_methods(first);
    let mut partial = false;

    if method == LoginMethod::Key && methods.contains(&MethodKind::PublicKey) {
        let path = login
            .key_path
            .clone()
            .ok_or_else(|| anyhow!("未选择私钥文件"))?;
        let Some(key) = load_private_key(&path, login, secrets, broker).await? else {
            bail!("私钥文件不存在：{}", path.display());
        };
        let hash = handle
            .best_supported_rsa_hash()
            .await
            .map_err(|_| anyhow!("无法确定服务器支持的 RSA 签名算法"))?
            .flatten();
        let result = handle
            .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(|_| anyhow!("私钥认证失败"))?;
        if result.success() {
            return Ok(());
        }
        partial = is_partial(&result);
        methods = remaining_methods(result);
        if !partial {
            bail!("服务器未接受指定的私钥");
        }
    }

    if method == LoginMethod::Password || partial {
        // 已保存的密码只发一次，哪种方式先问就用它。
        let mut saved_offered = false;
        let mut saved_rejected = false;
        if methods.contains(&MethodKind::Password) {
            if let Some(saved) = saved_secret(secrets, &login.password_secret()) {
                saved_offered = true;
                let result = handle
                    .authenticate_password(user, saved.to_string())
                    .await
                    .map_err(|_| anyhow!("密码认证失败"))?;
                if result.success() {
                    return Ok(());
                }
                // 条目不删：用户亲手存的，悄悄删掉会让人困惑。这次改成
                // 询问，并说明。
                saved_rejected = true;
                partial = is_partial(&result);
                methods = remaining_methods(result);
            }
            if !partial && methods.contains(&MethodKind::Password) {
                let instructions = if saved_rejected {
                    "已保存的密码被服务器拒绝，请重新输入。"
                } else {
                    "请输入登录密码。"
                };
                for _ in 0..AUTH_RETRIES {
                    let answer = ask_one_secret(
                        broker,
                        MissingCredential::Password {
                            rejected: saved_rejected,
                        },
                        login,
                        "输入密码",
                        instructions,
                        "密码",
                    )
                    .await?;
                    let result = handle
                        .authenticate_password(user, answer.expose().to_string())
                        .await
                        .map_err(|_| anyhow!("密码认证失败"))?;
                    if result.success() {
                        return Ok(());
                    }
                    partial = is_partial(&result);
                    methods = remaining_methods(result);
                    if partial || !methods.contains(&MethodKind::Password) {
                        break;
                    }
                }
            }
        }
        if methods.contains(&MethodKind::KeyboardInteractive) {
            let mut password =
                (method == LoginMethod::Password && !partial).then(|| KeyboardPassword {
                    saved: if saved_offered {
                        None
                    } else {
                        saved_secret(secrets, &login.password_secret())
                    },
                    rejected: saved_rejected,
                });
            for _ in 0..AUTH_RETRIES {
                if keyboard_interactive(handle, login, broker, password.as_mut()).await? {
                    return Ok(());
                }
            }
        }
    }
    bail!("认证失败：服务器未接受任何认证方式")
}

struct KeyboardPassword {
    saved: Option<Zeroizing<String>>,
    rejected: bool,
}

async fn load_private_key(
    path: &Path,
    login: &HostLogin,
    secrets: &SharedSecretStore,
    broker: &Arc<SshPrompts>,
) -> Result<Option<russh::keys::PrivateKey>> {
    if !path.exists() {
        return Ok(None);
    }
    match load_secret_key(path, None) {
        Ok(key) => return Ok(Some(key)),
        Err(russh::keys::Error::KeyIsEncrypted) => {}
        Err(_) => bail!("无法读取私钥文件：{}", path.display()),
    }
    // 口令按私钥文件记，同一把钥匙对所有用它的主机只问一次。归属用库里的
    // 路径（数据目录内是相对路径），拷走文件夹后仍然对得上。
    let mut saved_rejected = false;
    let saved_passphrase = login
        .passphrase_secret()
        .and_then(|secret| saved_secret(secrets, &secret));
    if let Some(saved) = saved_passphrase {
        if let Ok(key) = load_secret_key(path, Some(saved.as_str())) {
            return Ok(Some(key));
        }
        saved_rejected = true;
    }
    let instructions = if saved_rejected {
        format!("{} 已保存的口令被拒绝，请重新输入。", path.display())
    } else {
        format!("{} 已加密，请输入口令。", path.display())
    };
    for _ in 0..AUTH_RETRIES {
        let answer = ask_one_secret(
            broker,
            MissingCredential::Passphrase {
                rejected: saved_rejected,
            },
            login,
            "输入私钥口令",
            &instructions,
            "私钥口令",
        )
        .await?;
        if let Ok(key) = load_secret_key(path, Some(answer.expose())) {
            return Ok(Some(key));
        }
    }
    bail!("私钥口令错误次数过多")
}

/// 读一条已保存的秘密。秘密库出错、锁住或存的是空值都当作没存：接着问
/// 用户总比拒绝连接好。**阻塞**，只在 SSH 工作线程上调用。
fn saved_secret(secrets: &SharedSecretStore, secret: &SecretRef) -> Option<Zeroizing<String>> {
    secrets
        .get(secret)
        .ok()
        .flatten()
        .filter(|value| !value.is_empty())
}

/// 向用户要一条凭据。`login` 只用来在对话框里写明是谁在连。
async fn ask_one_secret(
    broker: &Arc<SshPrompts>,
    need: MissingCredential,
    login: &HostLogin,
    title: &str,
    instructions: &str,
    label: &str,
) -> Result<ConnectionSecret> {
    let reply = broker
        .ask_credential(
            need,
            ConnectionPromptKind::authentication(
                login.address.clone(),
                login.port,
                login.username.clone(),
                title.to_string(),
                instructions.to_string(),
                vec![crate::connection::AuthenticationQuestion {
                    text: label.to_string(),
                    echo: false,
                }],
            ),
        )
        .await?;
    match reply {
        ConnectionPromptReply::Answers(mut answers) if answers.len() == 1 => Ok(answers.remove(0)),
        ConnectionPromptReply::Cancel => bail!("认证已取消"),
        _ => bail!("认证回答无效"),
    }
}

/// 一次键盘交互认证；服务器拒绝时返回 `false`。
async fn keyboard_interactive(
    handle: &mut SshHandle,
    login: &HostLogin,
    broker: &Arc<SshPrompts>,
    mut password: Option<&mut KeyboardPassword>,
) -> Result<bool> {
    let user = login.username.as_str();
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await
        .map_err(|_| anyhow!("无法开始键盘交互认证"))?;
    let mut password_asked = false;
    let mut saved_sent = false;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                if saved_sent && let Some(password) = password.as_deref_mut() {
                    password.rejected = true;
                }
                return Ok(false);
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let asks_password = password.is_some()
                    && !password_asked
                    && matches!(prompts.as_slice(), [prompt] if !prompt.echo);
                password_asked |= asks_password;
                let saved = password
                    .as_deref_mut()
                    .filter(|_| asks_password)
                    .and_then(|password| password.saved.take());
                let answers = if prompts.is_empty() {
                    // 只有一段说明（PAM 经 OpenSSH 传过来），服务器在等空回答。
                    Vec::new()
                } else if let Some(saved) = saved {
                    saved_sent = true;
                    vec![saved.to_string()]
                } else {
                    let rejected = password
                        .as_deref()
                        .is_some_and(|password| password.rejected);
                    let need = if asks_password {
                        MissingCredential::Password { rejected }
                    } else {
                        MissingCredential::KeyboardInteractive
                    };
                    let instructions = if !(asks_password && rejected) {
                        instructions
                    } else if instructions.trim().is_empty() {
                        "已保存的密码被服务器拒绝，请重新输入。".to_string()
                    } else {
                        format!("已保存的密码被服务器拒绝，请重新输入。\n{instructions}")
                    };
                    let fields = prompts
                        .into_iter()
                        .map(|prompt| crate::connection::AuthenticationQuestion {
                            text: prompt.prompt,
                            echo: prompt.echo,
                        })
                        .collect();
                    let reply = broker
                        .ask_credential(
                            need,
                            ConnectionPromptKind::authentication(
                                login.address.clone(),
                                login.port,
                                login.username.clone(),
                                name,
                                instructions,
                                fields,
                            ),
                        )
                        .await?;
                    let ConnectionPromptReply::Answers(answers) = reply else {
                        bail!("键盘交互认证已取消")
                    };
                    answers
                        .into_iter()
                        .map(|answer| answer.expose().to_string())
                        .collect()
                };
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(|_| anyhow!("键盘交互认证失败"))?;
            }
        }
    }
}

fn remaining_methods(result: AuthResult) -> MethodSet {
    match result {
        AuthResult::Success => MethodSet::empty(),
        AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods,
    }
}

fn is_partial(result: &AuthResult) -> bool {
    matches!(
        result,
        AuthResult::Failure {
            partial_success: true,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::read_known_keys;

    #[test]
    fn malformed_known_hosts_is_blocked() {
        let path = std::env::temp_dir().join("qishell-known-hosts-test");
        std::fs::write(&path, "this is not a key\n").unwrap();
        let error = read_known_keys("example.test", 22, &path).unwrap_err();
        assert!(error.to_string().contains("已损坏"));
        let _ = std::fs::remove_file(path);
    }
}
