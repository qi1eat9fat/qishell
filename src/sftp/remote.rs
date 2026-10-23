//! 远程目录与文件传输：一条独立的 SSH 连接加一条 SFTP 通道。
//!
//! 工作线程有自己的 tokio 运行时；界面只通过命令与事件频道和它说话。
//! 目录读取、上传、下载、断点续传、冲突询问与并发调度都在这条连接上进行。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow, bail};
use async_channel::{Receiver, Sender};
use russh_sftp::{
    client::{RawSftpSession, error::Error},
    protocol::{FileAttributes, OpenFlags, StatusCode},
};
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt, ReadBuf, SeekFrom,
};
use tokio::sync::{mpsc, oneshot, watch};

use crate::connection::{ConnectionPrompt, ConnectionPromptReply};
use crate::host::HostLogin;
use crate::ssh::{SshConnectionConfig, SshConnector, SshPrompts, describe_login_error};

const CHUNK_SIZE: usize = 64 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);
const PART_SUFFIX: &str = ".filepart";
/// 小于这个体积的文件直接写目标，不生成断点文件。
const RESUME_MIN_SIZE: u64 = 100 * 1024;

/// 远程目录的一项。
#[derive(Clone, Debug)]
pub struct RemoteEntry {
    pub name: String,
    /// 目录，或指向目录的符号链接。
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: Option<u64>,
    /// Unix 时间戳（秒）。
    pub modified: Option<u32>,
    pub permissions: String,
    pub owner: String,
    pub target: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RemoteListing {
    pub path: String,
    pub entries: Vec<RemoteEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferPhase {
    Waiting,
    Scanning,
    Running,
    Paused,
    Partial,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferItemPhase {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Clone, Debug)]
pub struct TransferItemSnapshot {
    pub name: String,
    pub size: u64,
    pub transferred: u64,
    pub phase: TransferItemPhase,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TransferSnapshot {
    pub direction: TransferDirection,
    pub phase: TransferPhase,
    pub title: String,
    pub target: String,
    pub transferred: u64,
    pub total: u64,
    pub speed: f64,
    pub eta_secs: Option<u64>,
    pub items: Vec<TransferItemSnapshot>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Skip,
    Overwrite,
    Cancel,
}

/// 同名文件冲突，交给界面询问用户。
#[derive(Clone, Debug)]
pub struct ConflictInfo {
    pub batch_id: u64,
    pub conflict_id: u64,
    pub name: String,
    pub source_size: u64,
    pub target_size: u64,
    pub source_modified: String,
    pub target_modified: String,
}

/// 界面发给工作线程的命令。
pub enum RemoteCommand {
    List {
        request_id: u64,
        path: String,
    },
    Transfer {
        batch_id: u64,
        direction: TransferDirection,
        sources: Vec<String>,
        destination: String,
    },
    Pause {
        batch_id: u64,
    },
    Resume {
        batch_id: u64,
    },
    Cancel {
        batch_id: u64,
    },
    ConflictReply {
        conflict_id: u64,
        choice: ConflictChoice,
        apply_to_batch: bool,
    },
    Mkdir {
        request_id: u64,
        path: String,
    },
    CreateFile {
        request_id: u64,
        path: String,
    },
    Rename {
        request_id: u64,
        from: String,
        to: String,
    },
    Remove {
        request_id: u64,
        path: String,
        is_dir: bool,
    },
    /// 读取一个文本文件交给外部编辑器。
    FetchFile {
        request_id: u64,
        path: String,
    },
    /// 外部编辑器保存后写回远端。
    SaveEditedFile {
        path: String,
        contents: Vec<u8>,
    },
    SetConcurrency(u8),
    PromptReply {
        request_id: u64,
        reply: ConnectionPromptReply,
    },
    Shutdown,
}

/// 工作线程发回界面的事件。
pub enum RemoteEvent {
    Prompt(ConnectionPrompt),
    Connected {
        home: String,
    },
    Listing {
        request_id: u64,
        result: Result<RemoteListing, String>,
    },
    Transfer {
        batch_id: u64,
        snapshot: TransferSnapshot,
    },
    Conflict(ConflictInfo),
    Mutation {
        request_id: u64,
        result: Result<(), String>,
    },
    /// 外部编辑器取回的远端文件内容。
    FetchedFile {
        request_id: u64,
        result: Result<Vec<u8>, String>,
    },
    /// 外部编辑器保存后回传远端的结果。
    EditedFileSaved {
        path: String,
        result: Result<(), String>,
    },
    Failed(String),
    Closed,
}

pub fn spawn(
    connector: SshConnector,
    login: HostLogin,
    max_concurrency: u8,
    commands: Receiver<RemoteCommand>,
    events: Sender<RemoteEvent>,
) {
    let builder = thread::Builder::new().name("qishell-sftp".into());
    let fallback_events = events.clone();
    let spawned = builder.spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = events.send_blocking(RemoteEvent::Failed(format!(
                    "无法启动 SFTP 运行时：{error}"
                )));
                let _ = events.send_blocking(RemoteEvent::Closed);
                return;
            }
        };
        let result = runtime.block_on(run(
            connector,
            login,
            max_concurrency,
            commands,
            events.clone(),
        ));
        if let Err(error) = result {
            let _ = events.send_blocking(RemoteEvent::Failed(readable_error(&error)));
        }
        let _ = events.send_blocking(RemoteEvent::Closed);
    });
    if let Err(error) = spawned {
        let _ = fallback_events
            .send_blocking(RemoteEvent::Failed(format!("无法启动 SFTP 线程：{error}")));
        let _ = fallback_events.send_blocking(RemoteEvent::Closed);
    }
}

#[derive(Default)]
struct TransferControl {
    paused: AtomicBool,
    cancelled: AtomicBool,
}

impl TransferControl {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

struct Reply {
    choice: ConflictChoice,
    apply_to_batch: bool,
}

struct PendingConflict {
    batch_id: u64,
    reply: oneshot::Sender<Reply>,
}

struct Conflicts {
    events: Sender<RemoteEvent>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, PendingConflict>>,
}

impl Conflicts {
    fn new(events: Sender<RemoteEvent>) -> Arc<Self> {
        Arc::new(Self {
            events,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        })
    }

    async fn ask(
        &self,
        batch_id: u64,
        name: String,
        source_size: u64,
        target_size: u64,
        source_modified: String,
        target_modified: String,
    ) -> Reply {
        let conflict_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(
            conflict_id,
            PendingConflict {
                batch_id,
                reply: sender,
            },
        );
        let info = ConflictInfo {
            batch_id,
            conflict_id,
            name,
            source_size,
            target_size,
            source_modified,
            target_modified,
        };
        if self.events.send(RemoteEvent::Conflict(info)).await.is_err() {
            self.lock().remove(&conflict_id);
            return Reply {
                choice: ConflictChoice::Cancel,
                apply_to_batch: false,
            };
        }
        receiver.await.unwrap_or(Reply {
            choice: ConflictChoice::Cancel,
            apply_to_batch: false,
        })
    }

    fn respond(&self, conflict_id: u64, choice: ConflictChoice, apply_to_batch: bool) {
        if let Some(pending) = self.lock().remove(&conflict_id) {
            let _ = pending.reply.send(Reply {
                choice,
                apply_to_batch,
            });
        }
    }

    fn cancel_batch(&self, batch_id: u64) {
        let mut pending = self.lock();
        let ids: Vec<u64> = pending
            .iter()
            .filter(|(_, item)| item.batch_id == batch_id)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(item) = pending.remove(&id) {
                let _ = item.reply.send(Reply {
                    choice: ConflictChoice::Cancel,
                    apply_to_batch: false,
                });
            }
        }
    }

    fn cancel_all(&self) {
        let mut pending = self.lock();
        for (_, item) in pending.drain() {
            let _ = item.reply.send(Reply {
                choice: ConflictChoice::Cancel,
                apply_to_batch: false,
            });
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, PendingConflict>> {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

enum Inner {
    List {
        request_id: u64,
        path: String,
    },
    Transfer(QueuedTransfer),
    Mkdir {
        request_id: u64,
        path: String,
    },
    CreateFile {
        request_id: u64,
        path: String,
    },
    Rename {
        request_id: u64,
        from: String,
        to: String,
    },
    Remove {
        request_id: u64,
        path: String,
        is_dir: bool,
    },
    FetchFile {
        request_id: u64,
        path: String,
    },
    SaveEditedFile {
        path: String,
        contents: Vec<u8>,
    },
    SetConcurrency(u8),
    Shutdown,
}

struct QueuedTransfer {
    batch_id: u64,
    direction: TransferDirection,
    sources: Vec<String>,
    destination: String,
    control: Arc<TransferControl>,
}

async fn run(
    connector: SshConnector,
    login: HostLogin,
    max_concurrency: u8,
    commands: Receiver<RemoteCommand>,
    events: Sender<RemoteEvent>,
) -> Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let prompt_events = events.clone();
    let broker = Arc::new(SshPrompts::new(
        Arc::new(move |prompt| prompt_events.try_send(RemoteEvent::Prompt(prompt)).is_ok()),
        shutdown_rx,
    ));
    let conflicts = Conflicts::new(events.clone());

    let controls: Arc<Mutex<HashMap<u64, Arc<TransferControl>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // 登录期间也要能收到弹窗答案，所以命令由独立任务路由。
    let (inner_tx, mut inner_rx) = mpsc::unbounded_channel();
    let router_broker = broker.clone();
    let router_controls = controls.clone();
    let router_conflicts = conflicts.clone();
    tokio::spawn(async move {
        while let Ok(command) = commands.recv().await {
            match command {
                RemoteCommand::PromptReply { request_id, reply } => {
                    router_broker.respond(request_id, reply);
                }
                RemoteCommand::Shutdown => {
                    let _ = shutdown_tx.send(true);
                    router_broker.cancel_all();
                    for control in lock_controls(&router_controls).values() {
                        control.cancelled.store(true, Ordering::Relaxed);
                    }
                    router_conflicts.cancel_all();
                    let _ = inner_tx.send(Inner::Shutdown);
                    break;
                }
                RemoteCommand::Pause { batch_id } => {
                    if let Some(control) = lock_controls(&router_controls).get(&batch_id) {
                        control.paused.store(true, Ordering::Relaxed);
                    }
                }
                RemoteCommand::Resume { batch_id } => {
                    if let Some(control) = lock_controls(&router_controls).get(&batch_id) {
                        control.paused.store(false, Ordering::Relaxed);
                    }
                }
                RemoteCommand::Cancel { batch_id } => {
                    if let Some(control) = lock_controls(&router_controls).remove(&batch_id) {
                        control.cancelled.store(true, Ordering::Relaxed);
                    }
                    router_conflicts.cancel_batch(batch_id);
                }
                RemoteCommand::ConflictReply {
                    conflict_id,
                    choice,
                    apply_to_batch,
                } => router_conflicts.respond(conflict_id, choice, apply_to_batch),
                RemoteCommand::Mkdir { request_id, path } => {
                    let _ = inner_tx.send(Inner::Mkdir { request_id, path });
                }
                RemoteCommand::CreateFile { request_id, path } => {
                    let _ = inner_tx.send(Inner::CreateFile { request_id, path });
                }
                RemoteCommand::Rename {
                    request_id,
                    from,
                    to,
                } => {
                    let _ = inner_tx.send(Inner::Rename {
                        request_id,
                        from,
                        to,
                    });
                }
                RemoteCommand::Remove {
                    request_id,
                    path,
                    is_dir,
                } => {
                    let _ = inner_tx.send(Inner::Remove {
                        request_id,
                        path,
                        is_dir,
                    });
                }
                RemoteCommand::FetchFile { request_id, path } => {
                    let _ = inner_tx.send(Inner::FetchFile { request_id, path });
                }
                RemoteCommand::SaveEditedFile { path, contents } => {
                    let _ = inner_tx.send(Inner::SaveEditedFile { path, contents });
                }
                RemoteCommand::SetConcurrency(max) => {
                    let _ = inner_tx.send(Inner::SetConcurrency(max));
                }
                RemoteCommand::List { request_id, path } => {
                    let _ = inner_tx.send(Inner::List { request_id, path });
                }
                RemoteCommand::Transfer {
                    batch_id,
                    direction,
                    sources,
                    destination,
                } => {
                    let control = Arc::new(TransferControl::default());
                    lock_controls(&router_controls).insert(batch_id, control.clone());
                    let _ = inner_tx.send(Inner::Transfer(QueuedTransfer {
                        batch_id,
                        direction,
                        sources,
                        destination,
                        control,
                    }));
                }
            }
        }
    });

    let (handle, _fingerprint) = connector
        .connect(&SshConnectionConfig::from(&login), broker.clone())
        .await
        .map_err(|error| anyhow!(describe_login_error(&error)))?;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|_| anyhow!("无法创建 SFTP 会话通道"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|_| anyhow!("服务器拒绝启动 SFTP 子系统"))?;

    let (closed_tx, mut closed) = watch::channel(false);
    let raw = Arc::new(RawSftpSession::new(WatchedStream {
        inner: channel.into_stream(),
        closed: closed_tx,
    }));
    let version = raw.init().await?;
    if version.version != 3 {
        bail!("SFTP 服务器协议版本为 {}，需要 3", version.version);
    }

    let home = raw
        .realpath(".")
        .await
        .ok()
        .and_then(|name| name.files.first().map(|file| file.filename.clone()))
        .filter(|home| !home.is_empty())
        .unwrap_or_else(|| "/".into());
    events
        .send(RemoteEvent::Connected { home })
        .await
        .map_err(|_| anyhow!("SFTP 标签页已关闭"))?;

    let mut queue: VecDeque<QueuedTransfer> = VecDeque::new();
    let mut active = 0usize;
    let mut max = max_concurrency.clamp(1, 10) as usize;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<()>();

    loop {
        tokio::select! {
            command = inner_rx.recv() => match command {
                Some(Inner::List { request_id, path }) => {
                    let result = list_path(&raw, &path)
                        .await
                        .map_err(|error| readable_error(&error));
                    if events
                        .send(RemoteEvent::Listing { request_id, result })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Some(Inner::Transfer(job)) => {
                    emit_waiting(&events, &job);
                    queue.push_back(job);
                    pump_jobs(
                        &mut queue,
                        &mut active,
                        max,
                        &raw,
                        &events,
                        &conflicts,
                        &done_tx,
                    );
                }
                Some(Inner::Mkdir { request_id, path }) => {
                    let result = raw
                        .mkdir(path, FileAttributes::empty())
                        .await
                        .map(|_| ())
                        .map_err(|error| readable_error(&error.into()));
                    let _ = events
                        .send(RemoteEvent::Mutation { request_id, result })
                        .await;
                }
                Some(Inner::CreateFile { request_id, path }) => {
                    let result = async {
                        let handle = raw
                            .open(
                                path,
                                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                                FileAttributes::empty(),
                            )
                            .await?;
                        raw.close(handle.handle.as_str()).await?;
                        Ok::<(), anyhow::Error>(())
                    }
                    .await
                    .map_err(|error| readable_error(&error));
                    let _ = events
                        .send(RemoteEvent::Mutation { request_id, result })
                        .await;
                }
                Some(Inner::Rename {
                    request_id,
                    from,
                    to,
                }) => {
                    let result = raw
                        .rename(from, to)
                        .await
                        .map(|_| ())
                        .map_err(|error| readable_error(&error.into()));
                    let _ = events
                        .send(RemoteEvent::Mutation { request_id, result })
                        .await;
                }
                Some(Inner::Remove {
                    request_id,
                    path,
                    is_dir,
                }) => {
                    let result = remove_remote(&raw, path, is_dir)
                        .await
                        .map_err(|error| readable_error(&error));
                    let _ = events
                        .send(RemoteEvent::Mutation { request_id, result })
                        .await;
                }
                Some(Inner::FetchFile { request_id, path }) => {
                    let result = fetch_remote(&raw, &path)
                        .await
                        .map_err(|error| readable_error(&error));
                    let _ = events
                        .send(RemoteEvent::FetchedFile { request_id, result })
                        .await;
                }
                Some(Inner::SaveEditedFile { path, contents }) => {
                    let result = write_remote(&raw, &path, &contents)
                        .await
                        .map_err(|error| readable_error(&error));
                    let _ = events
                        .send(RemoteEvent::EditedFileSaved { path, result })
                        .await;
                }
                Some(Inner::SetConcurrency(value)) => {
                    max = value.clamp(1, 10) as usize;
                    pump_jobs(
                        &mut queue,
                        &mut active,
                        max,
                        &raw,
                        &events,
                        &conflicts,
                        &done_tx,
                    );
                }
                Some(Inner::Shutdown) | None => break,
            },
            finished = done_rx.recv() => {
                if finished.is_some() {
                    active = active.saturating_sub(1);
                    pump_jobs(
                        &mut queue,
                        &mut active,
                        max,
                        &raw,
                        &events,
                        &conflicts,
                        &done_tx,
                    );
                }
            },
            _ = closed.changed() => {
                for control in lock_controls(&controls).values() {
                    control.cancelled.store(true, Ordering::Relaxed);
                }
                conflicts.cancel_all();
                let _ = events.send(RemoteEvent::Failed("SFTP 连接已断开".into())).await;
                break;
            }
        }
    }
    Ok(())
}

fn lock_controls(
    controls: &Arc<Mutex<HashMap<u64, Arc<TransferControl>>>>,
) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<TransferControl>>> {
    controls.lock().unwrap_or_else(|error| error.into_inner())
}

fn emit_waiting(events: &Sender<RemoteEvent>, job: &QueuedTransfer) {
    emit(
        events,
        job.batch_id,
        &TransferSnapshot {
            direction: job.direction,
            phase: TransferPhase::Waiting,
            title: default_title(&job.sources),
            target: job.destination.clone(),
            transferred: 0,
            total: 0,
            speed: 0.0,
            eta_secs: None,
            items: Vec::new(),
            error: None,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn pump_jobs(
    queue: &mut VecDeque<QueuedTransfer>,
    active: &mut usize,
    max: usize,
    raw: &Arc<RawSftpSession>,
    events: &Sender<RemoteEvent>,
    conflicts: &Arc<Conflicts>,
    done_tx: &mpsc::UnboundedSender<()>,
) {
    while *active < max {
        let Some(job) = queue.pop_front() else {
            break;
        };
        if job.control.cancelled() {
            continue;
        }
        *active += 1;
        let raw = raw.clone();
        let events = events.clone();
        let conflicts = conflicts.clone();
        let control = job.control.clone();
        let done = done_tx.clone();
        tokio::spawn(async move {
            run_transfer(
                raw,
                job.batch_id,
                job.direction,
                job.sources,
                job.destination,
                control,
                conflicts,
                events,
            )
            .await;
            let _ = done.send(());
        });
    }
}

// —— 传输 ——

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConflictPolicy {
    Skip,
    Overwrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConflictDecision {
    Proceed,
    Skip,
    Cancel,
}

struct LocalFile {
    path: PathBuf,
    relative: String,
    size: u64,
    modified: Option<SystemTime>,
}

struct RemoteFile {
    path: String,
    relative: String,
    size: u64,
    modified: Option<u32>,
}

#[allow(clippy::too_many_arguments)]
async fn run_transfer(
    raw: Arc<RawSftpSession>,
    batch_id: u64,
    direction: TransferDirection,
    sources: Vec<String>,
    destination: String,
    control: Arc<TransferControl>,
    conflicts: Arc<Conflicts>,
    events: Sender<RemoteEvent>,
) {
    let snapshot = TransferSnapshot {
        direction,
        phase: TransferPhase::Scanning,
        title: default_title(&sources),
        target: destination.clone(),
        transferred: 0,
        total: 0,
        speed: 0.0,
        eta_secs: None,
        items: Vec::new(),
        error: None,
    };
    emit(&events, batch_id, &snapshot);

    match direction {
        TransferDirection::Upload => {
            run_upload(
                raw,
                batch_id,
                &sources,
                &destination,
                &control,
                &conflicts,
                &events,
                snapshot,
            )
            .await
        }
        TransferDirection::Download => {
            run_download(
                raw,
                batch_id,
                &sources,
                &destination,
                &control,
                &conflicts,
                &events,
                snapshot,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_upload(
    raw: Arc<RawSftpSession>,
    batch_id: u64,
    sources: &[String],
    destination: &str,
    control: &TransferControl,
    conflicts: &Arc<Conflicts>,
    events: &Sender<RemoteEvent>,
    mut snapshot: TransferSnapshot,
) {
    let files = match collect_local_files(sources) {
        Ok(files) => files,
        Err(error) => {
            fail(events, batch_id, &mut snapshot, error);
            return;
        }
    };
    if files.is_empty() {
        finish(events, batch_id, &mut snapshot, false);
        return;
    }
    prepare_snapshot(&mut snapshot, sources);
    snapshot.items = files
        .iter()
        .map(|file| TransferItemSnapshot {
            name: file.relative.clone(),
            size: file.size,
            transferred: 0,
            phase: TransferItemPhase::Pending,
            error: None,
        })
        .collect();
    snapshot.total = files.iter().map(|file| file.size).sum();
    snapshot.phase = TransferPhase::Running;
    emit(events, batch_id, &snapshot);

    let mut progress = Progress::new(events, batch_id);
    let mut created = HashSet::new();
    let mut policy: Option<ConflictPolicy> = None;
    for (index, file) in files.iter().enumerate() {
        if control.cancelled() {
            cancelled(events, batch_id, &mut snapshot);
            return;
        }
        snapshot.items[index].phase = TransferItemPhase::Running;
        emit(events, batch_id, &snapshot);

        let final_path = join_remote(destination, &file.relative);
        let existing = raw.stat(&final_path).await.ok().map(|attrs| attrs.attrs);
        if let Some(attrs) = existing {
            let decision = resolve_conflict(
                conflicts,
                batch_id,
                file.relative.clone(),
                file.size,
                attrs.len(),
                format_system_time(file.modified),
                format_unix_time(attrs.mtime),
                &mut policy,
            )
            .await;
            match decision {
                ConflictDecision::Skip => {
                    snapshot.items[index].phase = TransferItemPhase::Skipped;
                    emit(events, batch_id, &snapshot);
                    continue;
                }
                ConflictDecision::Cancel => {
                    control.cancelled.store(true, Ordering::Relaxed);
                    cancelled(events, batch_id, &mut snapshot);
                    return;
                }
                ConflictDecision::Proceed => {
                    if attrs.is_dir() {
                        let error = anyhow!("目标同名项是文件夹");
                        snapshot.items[index].phase = TransferItemPhase::Failed;
                        snapshot.items[index].error = Some(readable_error(&error));
                        snapshot.error.get_or_insert_with(|| readable_error(&error));
                        emit(events, batch_id, &snapshot);
                        continue;
                    }
                }
            }
        }

        let part_path = if file.size >= RESUME_MIN_SIZE {
            format!("{final_path}{PART_SUFFIX}")
        } else {
            final_path.clone()
        };
        let resume = if part_path != final_path {
            match raw.stat(&part_path).await {
                Ok(attrs) => {
                    let length = attrs.attrs.len();
                    if length <= file.size {
                        length
                    } else {
                        let _ = raw.remove(&part_path).await;
                        0
                    }
                }
                Err(_) => 0,
            }
        } else {
            0
        };
        snapshot.items[index].transferred = resume;
        snapshot.transferred = snapshot
            .items
            .iter()
            .map(|item| item.transferred)
            .sum::<u64>();
        emit(events, batch_id, &snapshot);

        let result = async {
            ensure_remote_parents(&raw, destination, &file.relative, &mut created).await?;
            if resume < file.size || part_path == final_path {
                upload_one(
                    &raw,
                    file,
                    &part_path,
                    resume,
                    index,
                    &mut snapshot,
                    control,
                    &mut progress,
                )
                .await?;
            }
            if part_path != final_path {
                if raw.stat(&final_path).await.is_ok() {
                    let _ = raw.remove(&final_path).await;
                }
                raw.rename(&part_path, &final_path).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                snapshot.items[index].phase = TransferItemPhase::Done;
                snapshot.items[index].transferred = file.size;
            }
            Err(error) => {
                snapshot.items[index].phase = TransferItemPhase::Failed;
                snapshot.items[index].error = Some(readable_error(&error));
                snapshot.error.get_or_insert_with(|| readable_error(&error));
            }
        }
        snapshot.transferred = snapshot.items.iter().map(|item| item.transferred).sum();
        emit(events, batch_id, &snapshot);
    }
    let failed = snapshot
        .items
        .iter()
        .any(|item| item.phase == TransferItemPhase::Failed);
    finish(events, batch_id, &mut snapshot, failed);
}

#[allow(clippy::too_many_arguments)]
async fn run_download(
    raw: Arc<RawSftpSession>,
    batch_id: u64,
    sources: &[String],
    destination: &str,
    control: &TransferControl,
    conflicts: &Arc<Conflicts>,
    events: &Sender<RemoteEvent>,
    mut snapshot: TransferSnapshot,
) {
    let files = match collect_remote_files(&raw, sources).await {
        Ok(files) => files,
        Err(error) => {
            fail(events, batch_id, &mut snapshot, error);
            return;
        }
    };
    if files.is_empty() {
        finish(events, batch_id, &mut snapshot, false);
        return;
    }
    prepare_snapshot(&mut snapshot, sources);
    snapshot.items = files
        .iter()
        .map(|file| TransferItemSnapshot {
            name: file.relative.clone(),
            size: file.size,
            transferred: 0,
            phase: TransferItemPhase::Pending,
            error: None,
        })
        .collect();
    snapshot.total = files.iter().map(|file| file.size).sum();
    snapshot.phase = TransferPhase::Running;
    emit(events, batch_id, &snapshot);

    let mut progress = Progress::new(events, batch_id);
    let mut policy: Option<ConflictPolicy> = None;
    for (index, file) in files.iter().enumerate() {
        if control.cancelled() {
            cancelled(events, batch_id, &mut snapshot);
            return;
        }
        snapshot.items[index].phase = TransferItemPhase::Running;
        emit(events, batch_id, &snapshot);

        let final_path = Path::new(destination).join(&file.relative);
        if let Ok(metadata) = tokio::fs::metadata(&final_path).await {
            let decision = resolve_conflict(
                conflicts,
                batch_id,
                file.relative.clone(),
                file.size,
                metadata.len(),
                format_unix_time(file.modified),
                format_system_time(metadata.modified().ok()),
                &mut policy,
            )
            .await;
            match decision {
                ConflictDecision::Skip => {
                    snapshot.items[index].phase = TransferItemPhase::Skipped;
                    emit(events, batch_id, &snapshot);
                    continue;
                }
                ConflictDecision::Cancel => {
                    control.cancelled.store(true, Ordering::Relaxed);
                    cancelled(events, batch_id, &mut snapshot);
                    return;
                }
                ConflictDecision::Proceed => {
                    if metadata.is_dir() {
                        let error = anyhow!("目标同名项是文件夹");
                        snapshot.items[index].phase = TransferItemPhase::Failed;
                        snapshot.items[index].error = Some(readable_error(&error));
                        snapshot.error.get_or_insert_with(|| readable_error(&error));
                        emit(events, batch_id, &snapshot);
                        continue;
                    }
                }
            }
        }

        let part_path = if file.size >= RESUME_MIN_SIZE {
            local_part_path(&final_path)
        } else {
            final_path.clone()
        };
        let resume = if part_path != final_path {
            match tokio::fs::metadata(&part_path).await {
                Ok(metadata) => {
                    let length = metadata.len();
                    if length <= file.size {
                        length
                    } else {
                        let _ = tokio::fs::remove_file(&part_path).await;
                        0
                    }
                }
                Err(_) => 0,
            }
        } else {
            0
        };
        snapshot.items[index].transferred = resume;
        snapshot.transferred = snapshot
            .items
            .iter()
            .map(|item| item.transferred)
            .sum::<u64>();
        emit(events, batch_id, &snapshot);

        let result = async {
            if let Some(parent) = part_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            if resume < file.size || part_path == final_path {
                download_one(
                    &raw,
                    file,
                    &part_path,
                    resume,
                    index,
                    &mut snapshot,
                    control,
                    &mut progress,
                )
                .await?;
            }
            if part_path != final_path {
                if tokio::fs::metadata(&final_path).await.is_ok() {
                    let _ = tokio::fs::remove_file(&final_path).await;
                }
                tokio::fs::rename(&part_path, &final_path).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                snapshot.items[index].phase = TransferItemPhase::Done;
                snapshot.items[index].transferred = file.size;
            }
            Err(error) => {
                snapshot.items[index].phase = TransferItemPhase::Failed;
                snapshot.items[index].error = Some(readable_error(&error));
                snapshot.error.get_or_insert_with(|| readable_error(&error));
            }
        }
        snapshot.transferred = snapshot.items.iter().map(|item| item.transferred).sum();
        emit(events, batch_id, &snapshot);
    }
    let failed = snapshot
        .items
        .iter()
        .any(|item| item.phase == TransferItemPhase::Failed);
    finish(events, batch_id, &mut snapshot, failed);
}

#[allow(clippy::too_many_arguments)]
async fn upload_one(
    raw: &RawSftpSession,
    file: &LocalFile,
    remote_path: &str,
    resume: u64,
    index: usize,
    snapshot: &mut TransferSnapshot,
    control: &TransferControl,
    progress: &mut Progress<'_>,
) -> Result<()> {
    let mut local = tokio::fs::File::open(&file.path).await?;
    let flags = if resume > 0 {
        OpenFlags::WRITE
    } else {
        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE
    };
    let handle = raw
        .open(remote_path, flags, FileAttributes::empty())
        .await?
        .handle;
    if resume > 0 {
        local.seek(SeekFrom::Start(resume)).await?;
    }
    let mut offset = resume;
    let mut buffer = vec![0u8; CHUNK_SIZE];
    loop {
        progress.gate(control, snapshot).await?;
        let read = local.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        raw.write(handle.as_str(), offset, buffer[..read].to_vec())
            .await?;
        offset += read as u64;
        snapshot.items[index].transferred = offset;
        snapshot.transferred = snapshot
            .items
            .iter()
            .map(|item| item.transferred)
            .sum::<u64>();
        progress.tick(snapshot).await;
    }
    raw.close(handle.as_str()).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_one(
    raw: &RawSftpSession,
    file: &RemoteFile,
    local_path: &Path,
    resume: u64,
    index: usize,
    snapshot: &mut TransferSnapshot,
    control: &TransferControl,
    progress: &mut Progress<'_>,
) -> Result<()> {
    let handle = raw
        .open(file.path.as_str(), OpenFlags::READ, FileAttributes::empty())
        .await?
        .handle;
    let mut local = if resume > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(local_path)
            .await?
    } else {
        tokio::fs::File::create(local_path).await?
    };
    let mut offset = resume;
    loop {
        progress.gate(control, snapshot).await?;
        match raw.read(handle.as_str(), offset, CHUNK_SIZE as u32).await {
            Ok(data) => {
                if data.data.is_empty() {
                    break;
                }
                local.write_all(&data.data).await?;
                offset += data.data.len() as u64;
                snapshot.items[index].transferred = offset;
                snapshot.transferred = snapshot
                    .items
                    .iter()
                    .map(|item| item.transferred)
                    .sum::<u64>();
                progress.tick(snapshot).await;
            }
            Err(Error::Status(status)) if status.status_code == StatusCode::Eof => break,
            Err(error) => {
                let _ = raw.close(handle.as_str()).await;
                return Err(error.into());
            }
        }
    }
    local.flush().await?;
    raw.close(handle.as_str()).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn resolve_conflict(
    conflicts: &Arc<Conflicts>,
    batch_id: u64,
    name: String,
    source_size: u64,
    target_size: u64,
    source_modified: String,
    target_modified: String,
    policy: &mut Option<ConflictPolicy>,
) -> ConflictDecision {
    if let Some(policy) = policy {
        return match policy {
            ConflictPolicy::Skip => ConflictDecision::Skip,
            ConflictPolicy::Overwrite => ConflictDecision::Proceed,
        };
    }
    let reply = conflicts
        .ask(
            batch_id,
            name,
            source_size,
            target_size,
            source_modified,
            target_modified,
        )
        .await;
    match reply.choice {
        ConflictChoice::Skip => {
            if reply.apply_to_batch {
                *policy = Some(ConflictPolicy::Skip);
            }
            ConflictDecision::Skip
        }
        ConflictChoice::Overwrite => {
            if reply.apply_to_batch {
                *policy = Some(ConflictPolicy::Overwrite);
            }
            ConflictDecision::Proceed
        }
        ConflictChoice::Cancel => ConflictDecision::Cancel,
    }
}

struct Progress<'a> {
    events: &'a Sender<RemoteEvent>,
    batch_id: u64,
    last_emit: Instant,
    last_bytes: u64,
}

impl<'a> Progress<'a> {
    fn new(events: &'a Sender<RemoteEvent>, batch_id: u64) -> Self {
        Self {
            events,
            batch_id,
            last_emit: Instant::now(),
            last_bytes: 0,
        }
    }

    /// 暂停期间不发数据，取消时直接结束当前文件。
    async fn gate(
        &mut self,
        control: &TransferControl,
        snapshot: &mut TransferSnapshot,
    ) -> Result<()> {
        if control.cancelled() {
            bail!("已取消");
        }
        if !control.paused() {
            return Ok(());
        }
        snapshot.phase = TransferPhase::Paused;
        snapshot.speed = 0.0;
        snapshot.eta_secs = None;
        emit(self.events, self.batch_id, snapshot);
        while control.paused() {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if control.cancelled() {
                bail!("已取消");
            }
        }
        snapshot.phase = TransferPhase::Running;
        emit(self.events, self.batch_id, snapshot);
        Ok(())
    }

    async fn tick(&mut self, snapshot: &mut TransferSnapshot) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_emit);
        if elapsed < PROGRESS_INTERVAL {
            return;
        }
        let delta = snapshot.transferred.saturating_sub(self.last_bytes);
        snapshot.speed = delta as f64 / elapsed.as_secs_f64();
        let remaining = snapshot.total.saturating_sub(snapshot.transferred);
        snapshot.eta_secs =
            (snapshot.speed > 1024.0).then(|| (remaining as f64 / snapshot.speed).ceil() as u64);
        self.last_emit = now;
        self.last_bytes = snapshot.transferred;
        emit(self.events, self.batch_id, snapshot);
    }
}

fn prepare_snapshot(snapshot: &mut TransferSnapshot, sources: &[String]) {
    snapshot.title = batch_title(sources);
    snapshot.transferred = 0;
    snapshot.total = 0;
    snapshot.speed = 0.0;
    snapshot.eta_secs = None;
    snapshot.error = None;
}

fn finish(
    events: &Sender<RemoteEvent>,
    batch_id: u64,
    snapshot: &mut TransferSnapshot,
    failed: bool,
) {
    snapshot.phase = if failed {
        TransferPhase::Partial
    } else {
        TransferPhase::Completed
    };
    snapshot.speed = 0.0;
    snapshot.eta_secs = None;
    if !failed {
        snapshot.transferred = snapshot.total;
    }
    emit(events, batch_id, snapshot);
}

fn cancelled(events: &Sender<RemoteEvent>, batch_id: u64, snapshot: &mut TransferSnapshot) {
    snapshot.phase = TransferPhase::Failed;
    snapshot.error = Some("已取消".into());
    snapshot.speed = 0.0;
    snapshot.eta_secs = None;
    emit(events, batch_id, snapshot);
}

fn fail(
    events: &Sender<RemoteEvent>,
    batch_id: u64,
    snapshot: &mut TransferSnapshot,
    error: anyhow::Error,
) {
    snapshot.phase = TransferPhase::Failed;
    snapshot.error = Some(readable_error(&error));
    snapshot.speed = 0.0;
    snapshot.eta_secs = None;
    emit(events, batch_id, snapshot);
}

fn emit(events: &Sender<RemoteEvent>, batch_id: u64, snapshot: &TransferSnapshot) {
    let _ = events.try_send(RemoteEvent::Transfer {
        batch_id,
        snapshot: snapshot.clone(),
    });
}

fn default_title(sources: &[String]) -> String {
    sources
        .first()
        .and_then(|source| Path::new(source).file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "传输".into())
}

fn batch_title(sources: &[String]) -> String {
    let first = default_title(sources);
    if sources.len() > 1 {
        format!("{first} 等 {} 项", sources.len())
    } else {
        // 单个来源（多为文件夹）只显示名字，不再附带「（N 项）」。
        first
    }
}

fn collect_local_files(sources: &[String]) -> Result<Vec<LocalFile>> {
    let mut files = Vec::new();
    for source in sources {
        let root = PathBuf::from(source);
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| source.clone());
        let mut stack = vec![(root, name)];
        while let Some((path, relative)) = stack.pop() {
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| anyhow!("无法读取 {}：{error}", path.display()))?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let directory = std::fs::read_dir(&path)
                    .map_err(|error| anyhow!("无法读取 {}：{error}", path.display()))?;
                for entry in directory {
                    let Ok(entry) = entry else { continue };
                    let child_name = entry.file_name().to_string_lossy().to_string();
                    stack.push((entry.path(), format!("{relative}/{child_name}")));
                }
            } else {
                files.push(LocalFile {
                    path,
                    relative,
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                });
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

async fn collect_remote_files(raw: &RawSftpSession, sources: &[String]) -> Result<Vec<RemoteFile>> {
    let mut files = Vec::new();
    for source in sources {
        let name = source
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(source)
            .to_string();
        let mut stack = vec![(source.clone(), name)];
        while let Some((path, relative)) = stack.pop() {
            let attrs = match raw.lstat(&path).await {
                Ok(attrs) => attrs.attrs,
                Err(error) => return Err(anyhow!("无法读取 {path}：{error}")),
            };
            if attrs.is_dir() && !attrs.is_symlink() {
                let handle = raw.opendir(&path).await?.handle;
                loop {
                    match raw.readdir(&handle).await {
                        Ok(page) => {
                            for entry in page.files {
                                if matches!(entry.filename.as_str(), "." | "..") {
                                    continue;
                                }
                                stack.push((
                                    join_remote(&path, &entry.filename),
                                    format!("{relative}/{}", entry.filename),
                                ));
                            }
                        }
                        Err(Error::Status(status)) if status.status_code == StatusCode::Eof => {
                            break;
                        }
                        Err(error) => {
                            let _ = raw.close(&handle).await;
                            return Err(error.into());
                        }
                    }
                }
                let _ = raw.close(&handle).await;
            } else {
                files.push(RemoteFile {
                    path,
                    relative,
                    size: attrs.len(),
                    modified: attrs.mtime,
                });
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

async fn ensure_remote_parents(
    raw: &RawSftpSession,
    destination: &str,
    relative: &str,
    created: &mut HashSet<String>,
) -> Result<()> {
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() <= 1 {
        return Ok(());
    }
    let mut current = destination.trim_end_matches('/').to_string();
    for part in &parts[..parts.len() - 1] {
        current = join_remote(&current, part);
        if !created.insert(current.clone()) {
            continue;
        }
        ensure_remote_dir(raw, &current).await?;
    }
    Ok(())
}

async fn ensure_remote_dir(raw: &RawSftpSession, path: &str) -> Result<()> {
    if raw.mkdir(path, FileAttributes::empty()).await.is_ok() {
        return Ok(());
    }
    if raw
        .stat(path)
        .await
        .map(|attrs| attrs.attrs.is_dir())
        .unwrap_or(false)
    {
        return Ok(());
    }
    bail!("无法创建远程目录 {path}");
}

fn join_remote(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", base.trim_end_matches('/'), name)
    }
}

fn local_part_path(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_os_string();
    part.push(PART_SUFFIX);
    PathBuf::from(part)
}

fn format_system_time(time: Option<SystemTime>) -> String {
    let Some(time) = time else {
        return "—".into();
    };
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

fn format_unix_time(seconds: Option<u32>) -> String {
    let Some(seconds) = seconds else {
        return "—".into();
    };
    chrono::DateTime::from_timestamp(seconds as i64, 0)
        .map(|datetime| {
            datetime
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
}

/// 外部编辑器最多打开 5 MB 的文本文件。
const MAX_EDIT_SIZE: u64 = 5 * 1024 * 1024;

async fn fetch_remote(raw: &RawSftpSession, path: &str) -> Result<Vec<u8>> {
    let attrs = raw.stat(path).await?;
    let length = attrs.attrs.len();
    if length > MAX_EDIT_SIZE {
        bail!("文件超过 5 MB，无法用文本编辑器打开");
    }
    let handle = raw
        .open(path, OpenFlags::READ, FileAttributes::empty())
        .await?
        .handle;
    let mut data = Vec::with_capacity(length as usize);
    let mut offset = 0u64;
    loop {
        match raw.read(handle.as_str(), offset, CHUNK_SIZE as u32).await {
            Ok(chunk) => {
                if chunk.data.is_empty() {
                    break;
                }
                offset += chunk.data.len() as u64;
                if offset > MAX_EDIT_SIZE {
                    let _ = raw.close(handle.as_str()).await;
                    bail!("文件超过 5 MB，无法用文本编辑器打开");
                }
                data.extend_from_slice(&chunk.data);
            }
            Err(Error::Status(status)) if status.status_code == StatusCode::Eof => break,
            Err(error) => {
                let _ = raw.close(handle.as_str()).await;
                return Err(error.into());
            }
        }
    }
    raw.close(handle.as_str()).await?;
    Ok(data)
}

async fn write_remote(raw: &RawSftpSession, path: &str, contents: &[u8]) -> Result<()> {
    let handle = raw
        .open(
            path,
            OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            FileAttributes::empty(),
        )
        .await?
        .handle;
    let mut offset = 0u64;
    for chunk in contents.chunks(CHUNK_SIZE) {
        raw.write(handle.as_str(), offset, chunk.to_vec()).await?;
        offset += chunk.len() as u64;
    }
    raw.close(handle.as_str()).await?;
    Ok(())
}

async fn remove_remote(raw: &RawSftpSession, path: String, is_dir: bool) -> Result<()> {
    let mut stack = vec![(path, is_dir, false)];
    let mut dirs = Vec::new();
    while let Some((path, is_dir, visited)) = stack.pop() {
        if visited {
            dirs.push(path);
            continue;
        }
        if !is_dir {
            raw.remove(path).await?;
            continue;
        }
        let attrs = raw.lstat(&path).await?.attrs;
        if attrs.is_symlink() {
            raw.remove(path).await?;
            continue;
        }
        stack.push((path.clone(), true, true));
        let handle = raw.opendir(&path).await?.handle;
        loop {
            match raw.readdir(&handle).await {
                Ok(page) => {
                    for entry in page.files {
                        if matches!(entry.filename.as_str(), "." | "..") {
                            continue;
                        }
                        let child = join_remote(&path, &entry.filename);
                        let child_is_dir = entry.attrs.is_dir() && !entry.attrs.is_symlink();
                        stack.push((child, child_is_dir, false));
                    }
                }
                Err(Error::Status(status)) if status.status_code == StatusCode::Eof => break,
                Err(error) => {
                    let _ = raw.close(handle.as_str()).await;
                    return Err(error.into());
                }
            }
        }
        let _ = raw.close(handle.as_str()).await;
    }
    for dir in dirs {
        raw.rmdir(dir).await?;
    }
    Ok(())
}

// —— 目录 ——

async fn list_path(raw: &RawSftpSession, path: &str) -> Result<RemoteListing> {
    let canonical = raw
        .realpath(path)
        .await?
        .files
        .first()
        .map(|file| file.filename.clone())
        .ok_or_else(|| anyhow!("路径不存在：{path}"))?;
    let handle = raw.opendir(&canonical).await?.handle;
    let mut entries = Vec::new();

    let result: Result<()> = async {
        loop {
            match raw.readdir(&handle).await {
                Ok(page) => {
                    for entry in page.files {
                        if matches!(entry.filename.as_str(), "." | "..") {
                            continue;
                        }
                        let attrs = entry.attrs;
                        let mut is_dir = attrs.is_dir();
                        let mut target = None;
                        if attrs.is_symlink() {
                            let full = join_remote(&canonical, &entry.filename);
                            target = raw.readlink(&full).await.ok().and_then(|name| {
                                name.files.first().map(|file| file.filename.clone())
                            });
                            if let Ok(stat) = raw.stat(&full).await {
                                is_dir = stat.attrs.is_dir();
                            }
                        }
                        let (owner, _group) =
                            parse_longname(&entry.longname).unwrap_or_else(|| {
                                (
                                    attrs
                                        .uid
                                        .map(|id| id.to_string())
                                        .unwrap_or_else(|| "—".into()),
                                    attrs
                                        .gid
                                        .map(|id| id.to_string())
                                        .unwrap_or_else(|| "—".into()),
                                )
                            });
                        let type_char = if is_dir {
                            "d"
                        } else if attrs.is_symlink() {
                            "l"
                        } else {
                            "-"
                        };
                        entries.push(RemoteEntry {
                            name: entry.filename,
                            is_dir,
                            is_symlink: attrs.is_symlink(),
                            size: (!is_dir).then(|| attrs.len()),
                            modified: attrs.mtime,
                            permissions: format!("{type_char}{}", attrs.permissions()),
                            owner,
                            target,
                        });
                    }
                }
                Err(Error::Status(status)) if status.status_code == StatusCode::Eof => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    .await;
    let _ = raw.close(handle).await;
    result?;
    sort_entries(&mut entries);
    Ok(RemoteListing {
        path: canonical,
        entries,
    })
}

fn sort_entries(entries: &mut [RemoteEntry]) {
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

fn readable_error(error: &anyhow::Error) -> String {
    let mut messages = Vec::new();
    for cause in error.chain() {
        let text = cause.to_string();
        if !text.is_empty() && messages.last() != Some(&text) {
            messages.push(text);
        }
    }
    if messages.is_empty() {
        "未知错误".into()
    } else {
        messages.join("：")
    }
}

/// `drwxr-xr-x    2 root     wheel        4096 Jan  1 12:00 bin`
fn parse_longname(line: &str) -> Option<(String, String)> {
    let mut fields = line.split_whitespace();
    let mode = fields.next()?;
    let mut chars = mode.chars();
    if !chars.next().is_some_and(|c| "-dlcbpsD".contains(c))
        || chars.by_ref().take(9).count() != 9
        || !mode
            .chars()
            .skip(1)
            .take(9)
            .all(|c| "rwxsStTl-".contains(c))
    {
        return None;
    }
    fields.next()?.parse::<u64>().ok()?;
    let owner = fields.next()?;
    let group = fields.next()?;
    fields.next()?.parse::<u64>().ok()?;
    Some((owner.to_string(), group.to_string()))
}

/// 读到流结束或出错时把 `closed` 置真，界面才能及时显示断线。
struct WatchedStream<S> {
    inner: S,
    closed: watch::Sender<bool>,
}

impl<S: AsyncRead + Unpin> AsyncRead for WatchedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let wanted = buf.remaining() > 0;
        let before = buf.filled().len();
        let poll = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(result) = &poll
            && (result.is_err() || (wanted && buf.filled().len() == before))
        {
            this.closed.send_replace(true);
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for WatchedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RemoteEntry, collect_local_files, join_remote, local_part_path, parse_longname,
        sort_entries,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn parses_openssh_longname() {
        assert_eq!(
            parse_longname("drwxr-xr-x    2 root     wheel        4096 Jan  1 12:00 bin"),
            Some(("root".into(), "wheel".into()))
        );
        assert_eq!(parse_longname("not a longname"), None);
    }

    #[test]
    fn joins_remote_paths() {
        assert_eq!(join_remote("/", "home"), "/home");
        assert_eq!(join_remote("/var/www/", "atlas"), "/var/www/atlas");
    }

    #[test]
    fn sorts_directories_first() {
        let entry = |name: &str, is_dir: bool| RemoteEntry {
            name: name.into(),
            is_dir,
            is_symlink: false,
            size: None,
            modified: None,
            permissions: String::new(),
            owner: String::new(),
            target: None,
        };
        let mut entries = vec![
            entry("b.txt", false),
            entry("a", true),
            entry("A.txt", false),
        ];
        sort_entries(&mut entries);
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["a", "A.txt", "b.txt"]);
    }
    #[test]
    fn collects_local_files_recursively() {
        let directory = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(directory.path().join("sub")).expect("sub dir");
        std::fs::write(directory.path().join("a.txt"), b"hello").expect("a");
        std::fs::write(directory.path().join("sub").join("b.bin"), b"1234567").expect("b");

        let root = directory.path().to_string_lossy().to_string();
        let files = collect_local_files(&[root]).expect("files");
        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .any(|file| file.relative.ends_with("a.txt") && file.size == 5)
        );
        assert!(
            files
                .iter()
                .any(|file| file.relative.ends_with("sub/b.bin") && file.size == 7)
        );
    }
    #[test]
    fn names_part_files_beside_the_target() {
        assert_eq!(
            local_part_path(Path::new(r"C:\Users\Qi\big.bin")),
            PathBuf::from(r"C:\Users\Qi\big.bin.filepart")
        );
    }
}
