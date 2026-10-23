//! SFTP 标签页：本地/远程双栏文件列表与传输队列。
//!
//! 本地与远程目录都是真实读取：本地走标准库，远程走独立 SSH 连接上的
//! SFTP 子系统。文件列表支持上传下载、断点续传、冲突询问、右键新建/重命名/删除。

#[cfg(test)]
mod local_tests;
mod remote;

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, Icon, WindowExt as _, h_flex, input::InputState,
    scroll::ScrollableElement as _, tooltip::Tooltip, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use gpui_kit::assets::IconName;
use gpui_kit::base::ElementExt as _;

use crate::components::{button, dialog, field};
use crate::connection::{ConnectionPrompt, ConnectionPromptReply};
use crate::host::HostLogin;
use crate::settings::appearance_tokens::{AppearanceTokens, UiTokens};
use crate::ssh::SshConnector;
use crate::theme;

pub use remote::{ConflictChoice, ConflictInfo};
use remote::{RemoteCommand, RemoteEntry, RemoteEvent};

// —— 连接事件 ——

/// 连接状态变化或连接过程中的提问，交给工作区处理。
#[derive(Clone, Debug)]
pub enum SftpEvent {
    ConnectionChanged(bool),
    Prompt(ConnectionPrompt),
    Conflict(remote::ConflictInfo),
    Toast(SharedString),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Local,
    Remote,
}

#[derive(Clone)]
enum CrumbTarget {
    Local(PathBuf),
    Remote(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileKind {
    Folder,
    Symlink,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortKey {
    Name,
    Size,
    Modified,
    Permissions,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug)]
struct SortState {
    key: SortKey,
    dir: SortDir,
    /// 用户是否手动点过表头；未点过时不显示排序箭头。
    touched: bool,
}

impl Default for SortState {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            dir: SortDir::Asc,
            touched: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Size,
    Modified,
    Permissions,
    Owner,
}

impl Column {
    fn sort_key(self) -> SortKey {
        match self {
            Column::Size => SortKey::Size,
            Column::Modified => SortKey::Modified,
            Column::Permissions => SortKey::Permissions,
            Column::Owner => SortKey::Owner,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ColumnWidths {
    size: f32,
    modified: f32,
    permissions: f32,
    owner: f32,
}

impl ColumnWidths {
    fn local() -> Self {
        Self {
            size: 78.0,
            modified: 126.0,
            permissions: 0.0,
            owner: 0.0,
        }
    }

    fn remote() -> Self {
        Self {
            size: 68.0,
            modified: 108.0,
            permissions: 76.0,
            owner: 64.0,
        }
    }

    fn get(self, column: Column) -> f32 {
        match column {
            Column::Size => self.size,
            Column::Modified => self.modified,
            Column::Permissions => self.permissions,
            Column::Owner => self.owner,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct FileSelection {
    indices: BTreeSet<usize>,
    /// Shift 连选的起点。
    anchor: Option<usize>,
}

impl FileSelection {
    fn clear(&mut self) {
        self.indices.clear();
        self.anchor = None;
    }

    fn set_single(&mut self, index: usize) {
        self.indices.clear();
        self.indices.insert(index);
        self.anchor = Some(index);
    }

    fn toggle(&mut self, index: usize) {
        if !self.indices.remove(&index) {
            self.indices.insert(index);
        }
        self.anchor = Some(index);
    }

    fn select_range(&mut self, index: usize) {
        let Some(anchor) = self.anchor else {
            self.set_single(index);
            return;
        };
        let (start, end) = if anchor <= index {
            (anchor, index)
        } else {
            (index, anchor)
        };
        self.indices = (start..=end).collect();
    }

    fn contains(&self, index: usize) -> bool {
        self.indices.contains(&index)
    }

    #[cfg(test)]
    fn count(&self) -> usize {
        self.indices.len()
    }

    fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.indices.iter().copied()
    }
}

#[derive(Clone, Debug)]
struct FileEntry {
    name: String,
    kind: FileKind,
    /// 目录，或指向目录的符号链接。
    is_dir: bool,
    /// 以 KB 计的整数；目录为 `None`。
    size_kb: Option<u64>,
    modified: String,
    permissions: String,
    owner: String,
    /// 符号链接的目标。
    target: Option<String>,
}

/// 以 KB 计的整数按 KB → MB → GB 逐级显示，GB 为上限。
fn format_size_kb(kb: u64) -> String {
    const MB: u64 = 1024;
    const GB: u64 = 1024 * 1024;
    if kb >= GB {
        format!("{:.1} GB", kb as f64 / GB as f64)
    } else if kb >= MB {
        format!("{:.1} MB", kb as f64 / MB as f64)
    } else {
        format!("{kb} KB")
    }
}

impl FileEntry {
    fn size_label(&self) -> String {
        self.size_kb
            .map(format_size_kb)
            .unwrap_or_else(|| "-".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferStatus {
    Running,
    Scanning,
    Waiting,
    Reconnecting,
    Stopped,
    Partial,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Clone, Debug)]
struct TransferItem {
    name: String,
    status: ItemStatus,
    size: u64,
    transferred: u64,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Upload,
    Download,
}

#[derive(Clone, Debug)]
struct TransferBatch {
    id: u64,
    direction: Direction,
    status: TransferStatus,
    title: String,
    target: String,
    transferred: u64,
    total: u64,
    speed: f64,
    eta_secs: Option<u64>,
    expanded: bool,
    items: Vec<TransferItem>,
    sources: Vec<String>,
    destination: String,
    error: Option<String>,
}

impl TransferBatch {
    fn can_expand(&self) -> bool {
        self.items.len() > 1
    }

    fn progress(&self) -> f32 {
        if self.total > 0 {
            (self.transferred as f64 / self.total as f64 * 100.0) as f32
        } else if self.status == TransferStatus::Completed {
            100.0
        } else {
            0.0
        }
    }
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn format_speed(bytes_per_second: f64) -> String {
    format!("{}/s", format_bytes(bytes_per_second.max(0.0) as u64))
}

fn format_eta(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return "—".into();
    };
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes >= 60 {
        format!("{:02}:{:02}:{:02}", minutes / 60, minutes % 60, seconds)
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn map_phase(phase: remote::TransferPhase) -> TransferStatus {
    match phase {
        remote::TransferPhase::Waiting => TransferStatus::Waiting,
        remote::TransferPhase::Scanning => TransferStatus::Scanning,
        remote::TransferPhase::Running => TransferStatus::Running,
        remote::TransferPhase::Paused => TransferStatus::Stopped,
        remote::TransferPhase::Partial => TransferStatus::Partial,
        remote::TransferPhase::Completed => TransferStatus::Completed,
        remote::TransferPhase::Failed => TransferStatus::Failed,
    }
}

fn map_item_phase(phase: remote::TransferItemPhase) -> ItemStatus {
    match phase {
        remote::TransferItemPhase::Pending => ItemStatus::Pending,
        remote::TransferItemPhase::Running => ItemStatus::Running,
        remote::TransferItemPhase::Done => ItemStatus::Done,
        remote::TransferItemPhase::Failed => ItemStatus::Failed,
        remote::TransferItemPhase::Skipped => ItemStatus::Skipped,
    }
}

fn map_direction(direction: remote::TransferDirection) -> Direction {
    match direction {
        remote::TransferDirection::Upload => Direction::Upload,
        remote::TransferDirection::Download => Direction::Download,
    }
}

// —— 本地目录 ——

fn read_local_dir(path: &Path) -> std::result::Result<Vec<FileEntry>, String> {
    let directory =
        std::fs::read_dir(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let mut entries = Vec::new();
    for item in directory {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().to_string();
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        let Ok(metadata) = item
            .metadata()
            .or_else(|_| std::fs::symlink_metadata(item.path()))
        else {
            continue;
        };
        let is_symlink = file_type.is_symlink();
        let is_dir = metadata.is_dir();
        let target = is_symlink
            .then(|| std::fs::read_link(item.path()).ok())
            .flatten()
            .map(|target| target.display().to_string());
        entries.push(FileEntry {
            name: name.clone(),
            kind: if is_symlink {
                FileKind::Symlink
            } else if is_dir {
                FileKind::Folder
            } else {
                FileKind::File
            },
            is_dir,
            size_kb: (!is_dir).then(|| metadata.len().div_ceil(1024)),
            modified: metadata
                .modified()
                .ok()
                .map(format_system_time)
                .unwrap_or_else(|| "-".into()),
            permissions: String::new(),
            owner: String::new(),
            target,
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn format_system_time(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

fn format_unix_time(seconds: u32) -> String {
    chrono::DateTime::from_timestamp(seconds as i64, 0)
        .map(|datetime| {
            datetime
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "-".into())
}

fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

fn sort_file_entries(entries: &mut [FileEntry], sort: SortState) {
    entries.sort_by(|left, right| {
        let group = right.is_dir.cmp(&left.is_dir);
        if group != std::cmp::Ordering::Equal {
            return group;
        }
        let primary = match sort.key {
            SortKey::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            SortKey::Size => left.size_kb.unwrap_or(0).cmp(&right.size_kb.unwrap_or(0)),
            SortKey::Modified => left.modified.cmp(&right.modified),
            SortKey::Permissions => left
                .permissions
                .to_lowercase()
                .cmp(&right.permissions.to_lowercase()),
            SortKey::Owner => left.owner.to_lowercase().cmp(&right.owner.to_lowercase()),
        };
        let primary = if sort.dir == SortDir::Desc {
            primary.reverse()
        } else {
            primary
        };
        // 同值时名称始终升序，方向只影响主键。
        primary.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

fn remote_file_entry(entry: RemoteEntry) -> FileEntry {
    FileEntry {
        name: entry.name,
        kind: if entry.is_symlink {
            FileKind::Symlink
        } else if entry.is_dir {
            FileKind::Folder
        } else {
            FileKind::File
        },
        is_dir: entry.is_dir,
        size_kb: entry.size.map(|size| size.div_ceil(1024)),
        modified: entry
            .modified
            .map(format_unix_time)
            .unwrap_or_else(|| "-".into()),
        permissions: entry.permissions,
        owner: entry.owner,
        target: entry.target,
    }
}

fn local_parent(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// 本地栏打开时的位置：设置里的「本机默认打开路径」，留空或不可读就用
/// 主目录。设置只影响打开时的落点，之后在栏里到处走不会改它。
fn starting_local_path(configured: &str) -> PathBuf {
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.is_dir() {
            return path;
        }
    }
    dirs::home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 外部编辑器使用的临时副本路径。
fn edit_temp_path(name: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let directory = std::env::temp_dir().join("qishell-edit");
    let _ = std::fs::create_dir_all(&directory);
    let safe: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    directory.join(format!("{}-{safe}", SEQ.fetch_add(1, Ordering::Relaxed)))
}

/// 盯着临时副本：编辑器每次保存后把内容写回远端，直到标签关闭。
fn watch_temp_file(
    temp: PathBuf,
    remote: String,
    commands: async_channel::Sender<RemoteCommand>,
    alive: Arc<AtomicBool>,
) {
    let _ = std::thread::Builder::new()
        .name("qishell-editor-sync".into())
        .spawn(move || {
            let mut last = std::fs::read(&temp).unwrap_or_default();
            let mut last_modified = std::fs::metadata(&temp)
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            while alive.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
                if !alive.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(metadata) = std::fs::metadata(&temp) else {
                    break;
                };
                let modified = metadata.modified().ok();
                if modified == last_modified {
                    continue;
                }
                // 编辑器可能分多次写入，稍等再读完整内容。
                std::thread::sleep(Duration::from_millis(250));
                let Ok(bytes) = std::fs::read(&temp) else {
                    continue;
                };
                last_modified = modified;
                if bytes == last {
                    continue;
                }
                last = bytes.clone();
                if commands
                    .try_send(RemoteCommand::SaveEditedFile {
                        path: remote.clone(),
                        contents: bytes,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
}

/// 本地「此电脑」视图：探测 A:–Z: 中实际存在的盘符。
fn local_drives() -> Vec<FileEntry> {
    let mut drives = Vec::new();
    for letter in 'A'..='Z' {
        let root = format!("{letter}:{}", std::path::MAIN_SEPARATOR);
        if Path::new(&root).is_dir() {
            drives.push(FileEntry {
                name: format!("{letter}:"),
                kind: FileKind::Folder,
                is_dir: true,
                size_kb: None,
                modified: "-".into(),
                permissions: "-".into(),
                owner: "-".into(),
                target: None,
            });
        }
    }
    drives
}

fn remote_parent(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => Some("/".into()),
        Some(index) => Some(trimmed[..index].to_string()),
    }
}

fn join_remote_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", base.trim_end_matches('/'), name)
    }
}

// —— 视图 ——

/// 正在拖动的文件或文件夹；拖动已选中的行时携带整组选择。
#[derive(Clone, Debug)]
pub(crate) struct FileDrag {
    side: Side,
    names: Vec<String>,
}

struct FileDragPreview {
    name: SharedString,
}

impl Render for FileDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = UiTokens::current(cx).overlay;
        h_flex()
            .gap(theme::ICON_TEXT_GAP)
            .px(px(10.))
            .py(px(6.))
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(theme::colored(theme::FOAM, 0.32))
            .bg(theme::overlay(overlay))
            .shadow_lg()
            .text_size(theme::TYPE_META)
            .text_color(theme::solid(theme::TEXT))
            .child(
                Icon::new(IconName::File)
                    .size(theme::ICON_SM)
                    .text_color(theme::solid(theme::FOAM)),
            )
            .child(self.name.clone())
    }
}

#[derive(Clone, Debug)]
struct FileMenu {
    side: Side,
    x: Pixels,
    y: Pixels,
    target: Option<usize>,
    active: Option<usize>,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameMode {
    NewFolder,
    NewFile,
    Rename(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    NewFolder,
    NewFile,
    Rename(usize),
    Edit(usize),
    Delete(usize),
    Refresh,
}

/// 远程连接的阶段。列表读取失败不改变连接本身。
#[derive(Clone, Debug)]
enum RemotePhase {
    /// 测试或预览环境没有注入连接器。
    Disabled,
    Connecting,
    Connected,
    Failed(String),
}

impl RemotePhase {
    fn connected(&self) -> bool {
        matches!(self, RemotePhase::Connected)
    }
}

pub struct SftpView {
    login: HostLogin,
    connector: Option<SshConnector>,
    max_transfers: u8,
    /// 外观五类的最终 alpha：菜单用浮层、弹窗用模态。
    appearance: AppearanceTokens,
    local_path: PathBuf,
    /// 本地侧正在显示「此电脑」磁盘列表，而不是某个具体目录。
    local_roots: bool,
    local_entries: Vec<FileEntry>,
    local_error: Option<String>,
    remote_path: String,
    remote_entries: Vec<FileEntry>,
    remote_phase: RemotePhase,
    remote_loading: bool,
    remote_list_error: Option<String>,
    remote_request: u64,
    remote_generation: u64,
    commands: Option<async_channel::Sender<RemoteCommand>>,
    local_sort: SortState,
    remote_sort: SortState,
    local_columns: ColumnWidths,
    remote_columns: ColumnWidths,
    local_selection: FileSelection,
    remote_selection: FileSelection,
    active_side: Side,
    queue_collapsed: bool,
    selected_row: Option<String>,
    batches: Vec<TransferBatch>,
    next_batch_id: u64,
    file_menu: Option<FileMenu>,
    view_bounds: Option<Bounds<Pixels>>,
    next_request_id: u64,
    pending_mutations: HashMap<u64, String>,
    /// 外部编辑器的可执行文件路径（设置 › SFTP）。
    editor_path: String,
    /// 已发出取文件请求、等待远端内容的编辑任务。
    pending_edits: HashMap<u64, PendingEdit>,
    /// 标签关闭后让外部编辑器的同步线程退出。
    alive: Arc<AtomicBool>,
}

struct PendingEdit {
    temp: PathBuf,
    remote: String,
    name: String,
}

impl EventEmitter<SftpEvent> for SftpView {}

impl Drop for SftpView {
    fn drop(&mut self) {
        // 标签关闭后不再把临时文件的改动回传远端。
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl SftpView {
    pub fn new(
        login: &HostLogin,
        connector: Option<SshConnector>,
        max_transfers: u8,
        appearance: AppearanceTokens,
        editor_path: String,
        local_root: String,
    ) -> Self {
        let local_path = starting_local_path(&local_root);
        let (mut local_entries, local_error) = match read_local_dir(&local_path) {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        sort_file_entries(&mut local_entries, SortState::default());
        let remote_phase = if connector.is_some() {
            RemotePhase::Connecting
        } else {
            RemotePhase::Disabled
        };
        Self {
            login: login.clone(),
            connector,
            max_transfers: max_transfers.clamp(1, 10),
            appearance,
            local_path,
            local_roots: false,
            local_entries,
            local_error,
            remote_path: "/".into(),
            remote_entries: Vec::new(),
            remote_phase,
            remote_loading: false,
            remote_list_error: None,
            remote_request: 0,
            remote_generation: 0,
            commands: None,
            local_sort: SortState::default(),
            remote_sort: SortState::default(),
            local_columns: ColumnWidths::local(),
            remote_columns: ColumnWidths::remote(),
            local_selection: FileSelection::default(),
            remote_selection: FileSelection::default(),
            active_side: Side::Remote,
            queue_collapsed: false,
            selected_row: None,
            batches: Vec::new(),
            next_batch_id: 1,
            file_menu: None,
            view_bounds: None,
            next_request_id: 1,
            pending_mutations: HashMap::new(),
            editor_path,
            pending_edits: HashMap::new(),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }

    /// 设置 › SFTP 里的外部编辑器路径变化时同步。
    pub fn set_editor_path(&mut self, path: String, cx: &mut Context<Self>) {
        if self.editor_path != path {
            self.editor_path = path;
            cx.notify();
        }
    }

    /// 跟随外观设置更新菜单与弹窗用的令牌。
    pub fn set_appearance(&mut self, tokens: AppearanceTokens, cx: &mut Context<Self>) {
        if self.appearance != tokens {
            self.appearance = tokens;
            cx.notify();
        }
    }

    /// 建立远程连接并开始接收事件。工作区订阅完事件后调用。
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.connector.is_some() {
            self.start_remote(cx);
        }
    }

    pub fn connected(&self) -> bool {
        self.remote_phase.connected()
    }

    #[doc(hidden)]
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// 只给 `tests/ui` 用：本地栏现在停在哪。
    #[doc(hidden)]
    pub fn test_local_path(&self) -> PathBuf {
        self.local_path.clone()
    }

    /// 只给 `tests/ui` 用：把本地侧指向临时目录。
    #[doc(hidden)]
    pub fn test_set_local_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.local_path = path;
        self.local_roots = false;
        self.reload_local(cx);
    }

    /// 只给 `tests/ui` 用：模拟工作线程发来一个同名冲突。
    #[doc(hidden)]
    pub fn test_emit_conflict(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpEvent::Conflict(ConflictInfo {
            batch_id: 1,
            conflict_id: 1,
            name: "demo.txt".into(),
            source_size: 1024,
            target_size: 2048,
            source_modified: "2026-10-20 10:00".into(),
            target_modified: "2026-10-19 10:00".into(),
        }));
    }

    pub fn connecting(&self) -> bool {
        matches!(self.remote_phase, RemotePhase::Connecting)
    }

    /// 连接栏与远程面板的「断开 / 重连」都走这里。
    pub fn toggle_connection(&mut self, cx: &mut Context<Self>) {
        if self.connected() || self.connecting() {
            self.disconnect(cx);
        } else {
            self.start_remote(cx);
        }
    }

    pub fn reply_to_prompt(
        &mut self,
        request_id: u64,
        reply: ConnectionPromptReply,
        cx: &mut Context<Self>,
    ) {
        if let Some(commands) = &self.commands {
            let _ = commands.try_send(RemoteCommand::PromptReply { request_id, reply });
        }
        cx.notify();
    }

    pub fn reply_conflict(
        &mut self,
        conflict_id: u64,
        choice: remote::ConflictChoice,
        apply_to_batch: bool,
        cx: &mut Context<Self>,
    ) {
        self.command(RemoteCommand::ConflictReply {
            conflict_id,
            choice,
            apply_to_batch,
        });
        cx.notify();
    }

    /// 设置 › SFTP 里的最大同时传输数量变化时同步给工作线程。
    pub fn set_max_transfers(&mut self, max: u8, cx: &mut Context<Self>) {
        let max = max.clamp(1, 10);
        if self.max_transfers == max {
            return;
        }
        self.max_transfers = max;
        self.command(RemoteCommand::SetConcurrency(max));
        cx.notify();
    }

    fn start_remote(&mut self, cx: &mut Context<Self>) {
        let Some(connector) = self.connector.clone() else {
            self.remote_phase = RemotePhase::Disabled;
            cx.notify();
            return;
        };
        // 旧工作线程随命令通道被丢弃而退出。
        self.commands = None;
        self.remote_generation += 1;
        let generation = self.remote_generation;
        self.remote_phase = RemotePhase::Connecting;
        self.remote_loading = false;
        self.remote_list_error = None;
        self.remote_entries.clear();
        self.remote_selection.clear();
        self.mark_queue_reconnecting();

        let (command_tx, command_rx) = async_channel::unbounded();
        let (event_tx, event_rx) = async_channel::unbounded();
        self.commands = Some(command_tx);
        remote::spawn(
            connector,
            self.login.clone(),
            self.max_transfers,
            command_rx,
            event_tx,
        );
        cx.spawn(async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                let alive = this.update(cx, |this, cx| {
                    if this.remote_generation == generation {
                        this.on_remote_event(event, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        if let Some(commands) = self.commands.take() {
            let _ = commands.try_send(RemoteCommand::Shutdown);
        }
        self.remote_phase = RemotePhase::Failed("已断开".into());
        self.remote_loading = false;
        self.mark_queue_reconnecting();
        cx.emit(SftpEvent::ConnectionChanged(false));
        cx.notify();
    }

    fn on_remote_event(&mut self, event: RemoteEvent, cx: &mut Context<Self>) {
        match event {
            RemoteEvent::Prompt(prompt) => cx.emit(SftpEvent::Prompt(prompt)),
            RemoteEvent::Conflict(info) => cx.emit(SftpEvent::Conflict(info)),
            RemoteEvent::Mutation { request_id, result } => {
                if let Some(message) = self.pending_mutations.remove(&request_id) {
                    match result {
                        Ok(()) => {
                            self.request_remote_list(cx);
                            cx.emit(SftpEvent::Toast(message.into()));
                        }
                        Err(error) => cx.emit(SftpEvent::Toast(error.into())),
                    }
                }
            }
            RemoteEvent::FetchedFile { request_id, result } => {
                let Some(edit) = self.pending_edits.remove(&request_id) else {
                    return;
                };
                match result {
                    Ok(bytes) => self.open_remote_editor(edit, bytes, cx),
                    Err(error) => cx.emit(SftpEvent::Toast(
                        format!("无法读取 {}：{error}", edit.name).into(),
                    )),
                }
            }
            RemoteEvent::EditedFileSaved { path, result } => {
                if let Err(error) = result {
                    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                    cx.emit(SftpEvent::Toast(
                        format!("保存 {name} 失败：{error}").into(),
                    ));
                }
            }
            RemoteEvent::Connected { home } => {
                self.remote_phase = RemotePhase::Connected;
                self.remote_path = home;
                self.remote_list_error = None;
                self.fail_interrupted_batches();
                cx.emit(SftpEvent::ConnectionChanged(true));
                self.request_remote_list(cx);
            }
            RemoteEvent::Listing { request_id, result } => {
                if request_id != self.remote_request {
                    return;
                }
                self.remote_loading = false;
                match result {
                    Ok(listing) => {
                        self.remote_path = listing.path;
                        let mut entries: Vec<FileEntry> =
                            listing.entries.into_iter().map(remote_file_entry).collect();
                        sort_file_entries(&mut entries, self.remote_sort);
                        self.remote_entries = entries;
                        self.remote_list_error = None;
                        self.remote_selection.clear();
                    }
                    Err(message) => self.remote_list_error = Some(message),
                }
            }
            RemoteEvent::Transfer { batch_id, snapshot } => {
                let previous = self
                    .batches
                    .iter()
                    .find(|batch| batch.id == batch_id)
                    .map(|batch| batch.status);
                let direction = map_direction(snapshot.direction);
                let finished = matches!(
                    snapshot.phase,
                    remote::TransferPhase::Completed | remote::TransferPhase::Partial
                );
                if let Some(batch) = self.batches.iter_mut().find(|batch| batch.id == batch_id) {
                    batch.direction = direction;
                    batch.status = map_phase(snapshot.phase);
                    batch.title = snapshot.title;
                    batch.target = snapshot.target;
                    batch.transferred = snapshot.transferred;
                    batch.total = snapshot.total;
                    batch.speed = snapshot.speed;
                    batch.eta_secs = snapshot.eta_secs;
                    batch.error = snapshot.error;
                    batch.items = snapshot
                        .items
                        .into_iter()
                        .map(|item| TransferItem {
                            name: item.name,
                            status: map_item_phase(item.phase),
                            size: item.size,
                            transferred: item.transferred,
                            error: item.error,
                        })
                        .collect();
                }
                let was_finished = matches!(
                    previous,
                    Some(TransferStatus::Completed | TransferStatus::Partial)
                );
                if finished && !was_finished {
                    match direction {
                        Direction::Upload => self.request_remote_list(cx),
                        Direction::Download => self.reload_local(cx),
                    }
                }
            }
            RemoteEvent::Failed(message) => {
                self.remote_loading = false;
                self.remote_phase = RemotePhase::Failed(message);
                self.commands = None;
                self.mark_queue_reconnecting();
                cx.emit(SftpEvent::ConnectionChanged(false));
            }
            RemoteEvent::Closed => {
                if matches!(
                    self.remote_phase,
                    RemotePhase::Connected | RemotePhase::Connecting
                ) {
                    self.remote_phase = RemotePhase::Failed("SFTP 连接已断开".into());
                    self.commands = None;
                    self.mark_queue_reconnecting();
                    cx.emit(SftpEvent::ConnectionChanged(false));
                }
            }
        }
        cx.notify();
    }

    fn mark_queue_reconnecting(&mut self) {
        for batch in &mut self.batches {
            if matches!(
                batch.status,
                TransferStatus::Running | TransferStatus::Scanning
            ) {
                batch.status = TransferStatus::Reconnecting;
            }
        }
    }

    /// 断线后的传输不会自动续传：重连后把它们标为失败，交给用户重试。
    fn fail_interrupted_batches(&mut self) {
        for batch in &mut self.batches {
            if batch.status == TransferStatus::Reconnecting {
                batch.status = TransferStatus::Failed;
                batch.speed = 0.0;
                batch.eta_secs = None;
                batch.error = Some("连接中断，请重试".into());
            }
        }
    }

    fn request_remote_list(&mut self, cx: &mut Context<Self>) {
        if !self.remote_phase.connected() {
            return;
        }
        self.remote_request += 1;
        self.remote_loading = true;
        self.remote_list_error = None;
        if let Some(commands) = &self.commands {
            let _ = commands.try_send(RemoteCommand::List {
                request_id: self.remote_request,
                path: self.remote_path.clone(),
            });
        }
        cx.notify();
    }

    fn refresh(&mut self, side: Side, cx: &mut Context<Self>) {
        match side {
            Side::Local => self.reload_local(cx),
            Side::Remote => self.request_remote_list(cx),
        }
    }

    fn reload_local(&mut self, cx: &mut Context<Self>) {
        if self.local_roots {
            self.local_entries = local_drives();
            self.local_error = None;
            self.local_selection.clear();
            cx.notify();
            return;
        }
        match read_local_dir(&self.local_path) {
            Ok(mut entries) => {
                sort_file_entries(&mut entries, self.local_sort);
                self.local_entries = entries;
                self.local_error = None;
                self.local_selection.clear();
            }
            Err(error) => self.local_error = Some(error),
        }
        cx.notify();
    }

    fn navigate_local(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.local_path = path;
        self.local_roots = false;
        self.local_selection.clear();
        self.active_side = Side::Local;
        match read_local_dir(&self.local_path) {
            Ok(mut entries) => {
                sort_file_entries(&mut entries, self.local_sort);
                self.local_entries = entries;
                self.local_error = None;
            }
            Err(error) => {
                self.local_entries.clear();
                self.local_error = Some(error);
            }
        }
        cx.notify();
    }

    /// 本地侧切换到「此电脑」，列出所有磁盘。
    fn show_local_roots(&mut self, cx: &mut Context<Self>) {
        self.local_roots = true;
        self.local_selection.clear();
        self.active_side = Side::Local;
        self.local_entries = local_drives();
        self.local_error = None;
        cx.notify();
    }

    fn navigate_remote(&mut self, path: String, cx: &mut Context<Self>) {
        self.remote_path = path;
        self.remote_selection.clear();
        self.active_side = Side::Remote;
        self.request_remote_list(cx);
    }

    fn open_entry(&mut self, side: Side, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries(side).get(index).cloned() else {
            return;
        };
        if !entry.is_dir {
            return;
        }
        match side {
            Side::Local => {
                // 「此电脑」里的条目是盘符，需要拼成根路径而不是当前目录的子项。
                let path = if self.local_roots {
                    PathBuf::from(format!("{}{}", entry.name, std::path::MAIN_SEPARATOR))
                } else {
                    self.local_path.join(&entry.name)
                };
                if path.is_dir() {
                    self.navigate_local(path, cx);
                }
            }
            Side::Remote => {
                let path = join_remote_path(&self.remote_path, &entry.name);
                self.navigate_remote(path, cx);
            }
        }
    }

    fn open_parent(&mut self, side: Side, cx: &mut Context<Self>) {
        match side {
            Side::Local => {
                // 磁盘根目录再往上就是「此电脑」，让用户选择其他磁盘。
                match local_parent(&self.local_path) {
                    Some(parent) => self.navigate_local(parent, cx),
                    None => self.show_local_roots(cx),
                }
            }
            Side::Remote => {
                if let Some(parent) = remote_parent(&self.remote_path) {
                    self.navigate_remote(parent, cx);
                }
            }
        }
    }

    fn entries(&self, side: Side) -> &[FileEntry] {
        match side {
            Side::Local => &self.local_entries,
            Side::Remote => &self.remote_entries,
        }
    }

    fn path(&self, side: Side) -> String {
        match side {
            Side::Local if self.local_roots => "此电脑".into(),
            Side::Local => self.local_path.display().to_string(),
            Side::Remote => self.remote_path.clone(),
        }
    }

    fn has_parent(&self, side: Side) -> bool {
        match side {
            // 任何真实目录都能向上：普通目录到父级，磁盘根目录到「此电脑」。
            Side::Local => !self.local_roots,
            Side::Remote => {
                self.remote_phase.connected() && remote_parent(&self.remote_path).is_some()
            }
        }
    }

    fn sort(&self, side: Side) -> SortState {
        match side {
            Side::Local => self.local_sort,
            Side::Remote => self.remote_sort,
        }
    }

    fn columns(&self, side: Side) -> ColumnWidths {
        match side {
            Side::Local => self.local_columns,
            Side::Remote => self.remote_columns,
        }
    }

    fn column_width(&self, side: Side, column: Column) -> f32 {
        self.columns(side).get(column)
    }

    /// 点击表头：同一列在升序与降序之间切换；切到其他列时先降序。
    fn toggle_sort(&mut self, side: Side, key: SortKey, cx: &mut Context<Self>) {
        let current = self.sort(side);
        let next = if current.key == key {
            SortState {
                key,
                dir: match current.dir {
                    SortDir::Asc => SortDir::Desc,
                    SortDir::Desc => SortDir::Asc,
                },
                touched: true,
            }
        } else {
            SortState {
                key,
                dir: SortDir::Desc,
                touched: true,
            }
        };
        match side {
            Side::Local => {
                self.local_sort = next;
                self.local_selection.clear();
                sort_file_entries(&mut self.local_entries, next);
            }
            Side::Remote => {
                self.remote_sort = next;
                self.remote_selection.clear();
                sort_file_entries(&mut self.remote_entries, next);
            }
        }
        cx.notify();
    }

    fn selection(&self, side: Side) -> &FileSelection {
        match side {
            Side::Local => &self.local_selection,
            Side::Remote => &self.remote_selection,
        }
    }

    fn selection_mut(&mut self, side: Side) -> &mut FileSelection {
        match side {
            Side::Local => &mut self.local_selection,
            Side::Remote => &mut self.remote_selection,
        }
    }

    fn select_file(&mut self, side: Side, index: usize, cx: &mut Context<Self>) {
        self.active_side = side;
        self.selection_mut(side).set_single(index);
        cx.notify();
    }

    fn select_file_with_modifiers(
        &mut self,
        side: Side,
        index: usize,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.active_side = side;
        let selection = self.selection_mut(side);
        if modifiers.shift {
            selection.select_range(index);
        } else if modifiers.secondary() {
            selection.toggle(index);
        } else {
            selection.set_single(index);
        }
        cx.notify();
    }

    fn select_parent(&mut self, side: Side, cx: &mut Context<Self>) {
        self.active_side = side;
        self.selection_mut(side).clear();
        cx.notify();
    }

    // —— 文件列表右键菜单 ——

    fn open_file_menu(
        &mut self,
        side: Side,
        target: Option<usize>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 「此电脑」里的盘符没有新建 / 重命名 / 删除等文件操作。
        if side == Side::Local && self.local_roots {
            return;
        }
        match target {
            Some(index) => {
                if index >= self.entries(side).len() {
                    return;
                }
                // 右键已选中的行时保留多选；右键其他行才改为单选。
                if !self.selection(side).contains(index) {
                    self.select_file(side, index, cx);
                }
            }
            None => self.select_parent(side, cx),
        }
        let focus = cx.focus_handle();
        let return_focus = window.focused(cx);
        window.focus(&focus, cx);
        self.file_menu = Some(FileMenu {
            side,
            x: position.x,
            y: position.y,
            target,
            active: None,
            focus,
            return_focus,
        });
        cx.notify();
    }

    /// 当前菜单里可用的动作，顺序即方向键顺序；禁用项不参与。
    fn file_menu_actions(&self) -> Vec<MenuAction> {
        let Some(menu) = self.file_menu.as_ref() else {
            return Vec::new();
        };
        let side = menu.side;
        let connected = side == Side::Local || self.remote_phase.connected();
        let selected = menu.target.and_then(|index| self.entries(side).get(index));
        let target = menu.target.unwrap_or(0);
        let mut actions = Vec::new();
        if connected {
            actions.push(MenuAction::NewFolder);
            actions.push(MenuAction::NewFile);
        }
        if connected && selected.is_some() {
            actions.push(MenuAction::Rename(target));
            if selected.is_some_and(is_text_file) && !self.editor_path.trim().is_empty() {
                actions.push(MenuAction::Edit(target));
            }
            actions.push(MenuAction::Delete(target));
        }
        if connected {
            actions.push(MenuAction::Refresh);
        }
        actions
    }

    fn close_file_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.file_menu.take() {
            if let Some(handle) = menu.return_focus {
                window.focus(&handle, cx);
            }
            cx.notify();
        }
    }

    fn activate_file_menu(
        &mut self,
        action: MenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.file_menu.take() else {
            return;
        };
        if let Some(handle) = menu.return_focus {
            window.focus(&handle, cx);
        }
        self.run_menu_action(menu.side, action, window, cx);
        cx.notify();
    }

    fn file_menu_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let actions = self.file_menu_actions();
        if actions.is_empty() {
            if event.keystroke.key.as_str() == "escape" {
                cx.stop_propagation();
                self.close_file_menu(window, cx);
            }
            return;
        }
        let Some(menu) = self.file_menu.as_ref() else {
            return;
        };
        let active = menu.active;
        let count = actions.len();
        match event.keystroke.key.as_str() {
            "down" | "tab" => {
                if let Some(menu) = self.file_menu.as_mut() {
                    menu.active = Some(match active {
                        Some(index) => (index + 1) % count,
                        None => 0,
                    });
                }
                cx.stop_propagation();
                cx.notify();
            }
            "up" | "shift-tab" => {
                if let Some(menu) = self.file_menu.as_mut() {
                    menu.active = Some(match active {
                        Some(index) => (index + count - 1) % count,
                        None => count - 1,
                    });
                }
                cx.stop_propagation();
                cx.notify();
            }
            "home" => {
                if let Some(menu) = self.file_menu.as_mut() {
                    menu.active = Some(0);
                }
                cx.stop_propagation();
                cx.notify();
            }
            "end" => {
                if let Some(menu) = self.file_menu.as_mut() {
                    menu.active = Some(count - 1);
                }
                cx.stop_propagation();
                cx.notify();
            }
            "enter" | "space" | " " => {
                if let Some(index) = active {
                    cx.stop_propagation();
                    self.activate_file_menu(actions[index], window, cx);
                }
            }
            "escape" => {
                cx.stop_propagation();
                self.close_file_menu(window, cx);
            }
            _ => {}
        }
    }

    #[allow(clippy::vec_init_then_push)]
    fn render_file_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.file_menu.clone()?;
        let side = menu.side;
        let entries = self.entries(side);
        let selected = menu.target.and_then(|index| entries.get(index));
        let connected = side == Side::Local || self.remote_phase.connected();
        let can_rename = connected && selected.is_some();
        // 文本文件交给设置里的外部编辑器；未配置或不可用时不显示可用态。
        let can_edit =
            connected && selected.is_some_and(is_text_file) && !self.editor_path.trim().is_empty();
        let can_delete = can_rename;
        let actions = self.file_menu_actions();
        let is_active = |action: MenuAction| {
            menu.active
                .is_some_and(|index| actions.get(index) == Some(&action))
        };

        let mut items: Vec<AnyElement> = Vec::new();
        items.push(self.file_menu_item(
            "sftp-menu-new-folder",
            "新建文件夹…",
            false,
            connected,
            is_active(MenuAction::NewFolder),
            MenuAction::NewFolder,
            cx,
        ));
        items.push(self.file_menu_item(
            "sftp-menu-new-file",
            "新建文件…",
            false,
            connected,
            is_active(MenuAction::NewFile),
            MenuAction::NewFile,
            cx,
        ));
        items.push(menu_divider());
        items.push(self.file_menu_item(
            "sftp-menu-rename",
            "重命名…",
            false,
            can_rename,
            is_active(MenuAction::Rename(menu.target.unwrap_or(0))),
            MenuAction::Rename(menu.target.unwrap_or(0)),
            cx,
        ));
        items.push(self.file_menu_item(
            "sftp-menu-edit",
            "用外部编辑器打开",
            false,
            can_edit,
            is_active(MenuAction::Edit(menu.target.unwrap_or(0))),
            MenuAction::Edit(menu.target.unwrap_or(0)),
            cx,
        ));
        items.push(menu_divider());
        items.push(self.file_menu_item(
            "sftp-menu-delete",
            "删除",
            true,
            can_delete,
            is_active(MenuAction::Delete(menu.target.unwrap_or(0))),
            MenuAction::Delete(menu.target.unwrap_or(0)),
            cx,
        ));
        items.push(menu_divider());
        items.push(self.file_menu_item(
            "sftp-menu-refresh",
            "刷新",
            false,
            connected,
            is_active(MenuAction::Refresh),
            MenuAction::Refresh,
            cx,
        ));

        let origin = self
            .view_bounds
            .map(|bounds| bounds.origin)
            .unwrap_or_else(|| point(px(0.), px(0.)));
        let view_size = self
            .view_bounds
            .map(|bounds| bounds.size)
            .unwrap_or_else(|| gpui_kit::size(px(480.), px(480.)));
        let width = px(176.);
        let height = px(230.);
        let x = (menu.x - origin.x).clamp(px(8.), (view_size.width - width - px(8.)).max(px(8.)));
        let y = (menu.y - origin.y).clamp(px(8.), (view_size.height - height - px(8.)).max(px(8.)));

        Some(
            deferred(
                div()
                    .id("sftp-file-menu")
                    .absolute()
                    .left(x)
                    .top(y)
                    .w(width)
                    .track_focus(&menu.focus)
                    .tab_index(0)
                    .on_key_down(cx.listener(Self::file_menu_key))
                    .child(
                        crate::components::overlay::menu_surface(self.appearance.overlay)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .children(items),
                    )
                    .test_support(),
            )
            .into_any_element(),
        )
    }

    /// 窄窗口下隐藏远程侧的「权限 / 所有者」列，保证名称与大小可读。
    /// 阈值取整个 SFTP 视图的宽度，960px 最小窗口会进入紧凑模式。
    fn compact_columns(&self) -> bool {
        self.view_bounds
            .is_some_and(|bounds| bounds.size.width < px(900.))
    }

    #[allow(clippy::too_many_arguments)]
    fn file_menu_item(
        &self,
        id: &'static str,
        label: &'static str,
        danger: bool,
        enabled: bool,
        active: bool,
        action: MenuAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::components::overlay::menu_item(id, label, enabled, danger)
            .test_support()
            .when(active && enabled, |item| {
                item.bg(theme::hover_tint_strong())
            })
            .when(enabled, |item| {
                item.on_click(cx.listener(move |this, _, window, cx| {
                    this.activate_file_menu(action, window, cx);
                }))
            })
            .into_any_element()
    }

    fn run_menu_action(
        &mut self,
        side: Side,
        action: MenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            MenuAction::NewFolder => {
                self.open_name_dialog(side, NameMode::NewFolder, String::new(), window, cx);
            }
            MenuAction::NewFile => {
                self.open_name_dialog(side, NameMode::NewFile, String::new(), window, cx);
            }
            MenuAction::Rename(index) => {
                let Some(name) = self
                    .entries(side)
                    .get(index)
                    .map(|entry| entry.name.clone())
                else {
                    return;
                };
                self.open_name_dialog(side, NameMode::Rename(index), name, window, cx);
            }
            MenuAction::Edit(index) => self.edit_entry(side, index, cx),
            MenuAction::Delete(index) => {
                self.open_delete_dialog(side, index, window, cx);
            }
            MenuAction::Refresh => self.refresh(side, cx),
        }
    }

    fn validate_name(
        &self,
        side: Side,
        mode: NameMode,
        name: &str,
    ) -> std::result::Result<(), &'static str> {
        if name.is_empty() {
            return Err("请输入名称");
        }
        if name == "." || name == ".." {
            return Err("名称不能是 . 或 ..");
        }
        if name.contains('/') || name.contains('\\') {
            return Err("名称不能包含路径分隔符");
        }
        let entries = self.entries(side);
        let rename_index = match mode {
            NameMode::Rename(index) => Some(index),
            _ => None,
        };
        if rename_index
            .is_some_and(|index| entries.get(index).is_some_and(|entry| entry.name == name))
        {
            return Err("名称没有变化");
        }
        let duplicate = entries.iter().enumerate().any(|(index, entry)| {
            Some(index) != rename_index
                && if side == Side::Local {
                    entry.name.eq_ignore_ascii_case(name)
                } else {
                    entry.name == name
                }
        });
        if duplicate {
            return Err("同名文件已存在");
        }
        Ok(())
    }

    fn open_name_dialog(
        &mut self,
        side: Side,
        mode: NameMode,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modal_alpha = self.appearance.modal;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("名称"));
        input.update(cx, |input, cx| {
            input.set_value(initial, window, cx);
            input.focus(window, cx);
        });
        let form = cx.new(|_| NameForm {
            input: input.clone(),
            error: None,
        });
        let dialog_form = form.clone();
        let view = cx.entity();
        let title = match mode {
            NameMode::NewFolder => "新建文件夹",
            NameMode::NewFile => "新建文件",
            NameMode::Rename(_) => "重命名",
        };
        let confirm_label = if matches!(mode, NameMode::Rename(_)) {
            "保存"
        } else {
            "创建"
        };
        window.open_dialog(cx, move |dialog, window, _| {
            let cancel_form = dialog_form.clone();
            let confirm_form = dialog_form.clone();
            let ok_form = dialog_form.clone();
            let confirm_view = view.clone();
            let ok_view = view.clone();
            dialog::surface(dialog, theme::DIALOG_WIDTH, modal_alpha)
                .title(dialog::title(title))
                .margin_top(window.viewport_size().height * 0.32)
                .overlay_closable(true)
                .child(dialog_form.clone())
                .footer(
                    dialog::footer()
                        .child(button::form_secondary("sftp-name-cancel", "取消").on_click(
                            move |_, window, cx| {
                                let _ = &cancel_form;
                                window.close_dialog(cx);
                            },
                        ))
                        .child(
                            button::form_primary("sftp-name-confirm", confirm_label).on_click(
                                move |_, window, cx| {
                                    confirm_name(
                                        &confirm_view,
                                        &confirm_form,
                                        side,
                                        mode,
                                        window,
                                        cx,
                                    );
                                },
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    confirm_name(&ok_view, &ok_form, side, mode, window, cx)
                })
        });
        let focus_input = form.read(cx).input.clone();
        focus_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
    }

    fn apply_name(&mut self, side: Side, mode: NameMode, name: String, cx: &mut Context<Self>) {
        match side {
            Side::Local => {
                let target = self.local_path.join(&name);
                let result: std::result::Result<(), String> = match mode {
                    NameMode::NewFolder => {
                        std::fs::create_dir(&target).map_err(|error| error.to_string())
                    }
                    NameMode::NewFile => std::fs::File::create(&target)
                        .map(|_| ())
                        .map_err(|error| error.to_string()),
                    NameMode::Rename(index) => {
                        let Some(from_name) = self
                            .entries(side)
                            .get(index)
                            .map(|entry| entry.name.clone())
                        else {
                            return;
                        };
                        if from_name == name {
                            return;
                        }
                        std::fs::rename(self.local_path.join(&from_name), target)
                            .map_err(|error| error.to_string())
                    }
                };
                match result {
                    Ok(()) => {
                        self.reload_local(cx);
                        cx.emit(SftpEvent::Toast(
                            match mode {
                                NameMode::NewFolder => "已创建文件夹",
                                NameMode::NewFile => "已创建文件",
                                NameMode::Rename(_) => "已重命名",
                            }
                            .into(),
                        ));
                    }
                    Err(error) => cx.emit(SftpEvent::Toast(error.into())),
                }
            }
            Side::Remote => {
                if !self.remote_phase.connected() {
                    cx.emit(SftpEvent::Toast("SFTP 未连接".into()));
                    return;
                }
                let request_id = self.next_request_id;
                self.next_request_id += 1;
                let (command, message) = match mode {
                    NameMode::NewFolder => (
                        RemoteCommand::Mkdir {
                            request_id,
                            path: join_remote_path(&self.remote_path, &name),
                        },
                        "已创建文件夹",
                    ),
                    NameMode::NewFile => (
                        RemoteCommand::CreateFile {
                            request_id,
                            path: join_remote_path(&self.remote_path, &name),
                        },
                        "已创建文件",
                    ),
                    NameMode::Rename(index) => {
                        let Some(from_name) = self
                            .entries(side)
                            .get(index)
                            .map(|entry| entry.name.clone())
                        else {
                            return;
                        };
                        if from_name == name {
                            return;
                        }
                        (
                            RemoteCommand::Rename {
                                request_id,
                                from: join_remote_path(&self.remote_path, &from_name),
                                to: join_remote_path(&self.remote_path, &name),
                            },
                            "已重命名",
                        )
                    }
                };
                self.pending_mutations.insert(request_id, message.into());
                self.command(command);
            }
        }
        cx.notify();
    }

    fn open_delete_dialog(
        &mut self,
        side: Side,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entries(side).get(index).cloned() else {
            return;
        };
        let modal_alpha = self.appearance.modal;
        let name = entry.name.clone();
        let is_dir = entry.is_dir;
        let title = if is_dir {
            "删除文件夹"
        } else {
            "删除文件"
        };
        let note = match side {
            Side::Local => "本地文件将移到回收站；文件夹会连同其中的内容一起移入。",
            Side::Remote => "远程文件将被永久删除；文件夹会连同其中的内容一起删除。",
        };
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view_ok = view.clone();
            let view_button = view.clone();
            let name_ok = name.clone();
            let name_button = name.clone();
            dialog::surface(dialog, theme::DIALOG_WIDTH, modal_alpha)
                .title(dialog::title(title))
                .margin_top(window.viewport_size().height * 0.36)
                .overlay_closable(true)
                .child(
                    v_flex()
                        .gap(px(6.))
                        .child(format!("确定删除 “{name}” 吗？"))
                        .child(note),
                )
                .footer(
                    dialog::footer()
                        .child(
                            button::form_secondary("sftp-delete-cancel", "取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(button::form_danger("sftp-delete-confirm", "删除").on_click(
                            move |_, window, cx| {
                                let name = name_button.clone();
                                view_button.update(cx, |view, cx| {
                                    view.delete_entry(side, name, is_dir, cx);
                                });
                                window.close_dialog(cx);
                            },
                        )),
                )
                .on_ok(move |_, window, cx| {
                    let name = name_ok.clone();
                    view_ok.update(cx, |view, cx| {
                        view.delete_entry(side, name, is_dir, cx);
                    });
                    window.close_dialog(cx);
                    true
                })
        });
    }

    fn delete_entry(&mut self, side: Side, name: String, is_dir: bool, cx: &mut Context<Self>) {
        match side {
            Side::Local => {
                let path = self.local_path.join(&name);
                match trash::delete(&path) {
                    Ok(()) => {
                        self.reload_local(cx);
                        cx.emit(SftpEvent::Toast("已移到回收站".into()));
                    }
                    Err(error) => cx.emit(SftpEvent::Toast(error.to_string().into())),
                }
            }
            Side::Remote => {
                if !self.remote_phase.connected() {
                    cx.emit(SftpEvent::Toast("SFTP 未连接".into()));
                    return;
                }
                let request_id = self.next_request_id;
                self.next_request_id += 1;
                self.pending_mutations.insert(request_id, "已删除".into());
                self.command(RemoteCommand::Remove {
                    request_id,
                    path: join_remote_path(&self.remote_path, &name),
                    is_dir,
                });
            }
        }
        cx.notify();
    }

    /// 编辑：本地文件直接交给外部编辑器；远端文件先取回临时副本，
    /// 编辑器保存后由后台线程自动写回远端。
    fn edit_entry(&mut self, side: Side, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries(side).get(index).cloned() else {
            return;
        };
        if !is_text_file(&entry) {
            cx.emit(SftpEvent::Toast("该文件不是可编辑的文本文件".into()));
            return;
        }
        let editor = self.editor_path.trim().to_string();
        if editor.is_empty() {
            cx.emit(SftpEvent::Toast(
                "请先在设置 › SFTP 里配置外部编辑器".into(),
            ));
            return;
        }
        if !Path::new(&editor).is_file() {
            cx.emit(SftpEvent::Toast(format!("找不到编辑器：{editor}").into()));
            return;
        }
        match side {
            Side::Local => {
                let path = self.local_path.join(&entry.name);
                if let Err(error) = std::process::Command::new(&editor).arg(&path).spawn() {
                    cx.emit(SftpEvent::Toast(format!("无法打开编辑器：{error}").into()));
                }
            }
            Side::Remote => {
                if !self.remote_phase.connected() {
                    cx.emit(SftpEvent::Toast("SFTP 未连接".into()));
                    return;
                }
                let request_id = self.next_request_id;
                self.next_request_id += 1;
                let remote = join_remote_path(&self.remote_path, &entry.name);
                let temp = edit_temp_path(&entry.name);
                self.pending_edits.insert(
                    request_id,
                    PendingEdit {
                        temp,
                        remote: remote.clone(),
                        name: entry.name,
                    },
                );
                self.command(RemoteCommand::FetchFile {
                    request_id,
                    path: remote,
                });
            }
        }
    }

    fn open_remote_editor(&mut self, edit: PendingEdit, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.contains(&0) {
            cx.emit(SftpEvent::Toast(
                format!("{} 看起来是二进制文件，无法编辑", edit.name).into(),
            ));
            return;
        }
        if let Err(error) = std::fs::write(&edit.temp, bytes) {
            cx.emit(SftpEvent::Toast(
                format!("无法创建临时文件：{error}").into(),
            ));
            return;
        }
        let editor = self.editor_path.trim().to_string();
        if let Err(error) = std::process::Command::new(&editor).arg(&edit.temp).spawn() {
            cx.emit(SftpEvent::Toast(format!("无法打开编辑器：{error}").into()));
            return;
        }
        let Some(commands) = self.commands.clone() else {
            return;
        };
        watch_temp_file(edit.temp, edit.remote, commands, self.alive.clone());
    }

    fn has_completed(&self) -> bool {
        self.batches
            .iter()
            .any(|batch| batch.status == TransferStatus::Completed)
    }

    // —— 面板 ——

    /// 路径面包屑：每一段可点击跳到对应祖先目录，当前段只显示。
    fn render_breadcrumbs(
        &self,
        side: Side,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let is_remote = side == Side::Remote;
        let mut segments: Vec<(String, CrumbTarget)> = Vec::new();

        if is_remote {
            segments.push(("/".into(), CrumbTarget::Remote("/".into())));
            let mut acc = String::new();
            for part in self.remote_path.split('/').filter(|part| !part.is_empty()) {
                acc.push('/');
                acc.push_str(part);
                segments.push((part.to_string(), CrumbTarget::Remote(acc.clone())));
            }
        } else if self.local_roots {
            segments.push(("此电脑".into(), CrumbTarget::Local(self.local_path.clone())));
        } else {
            let mut acc = PathBuf::new();
            for component in self.local_path.components() {
                match component {
                    Component::Prefix(prefix) => {
                        acc.push(prefix.as_os_str());
                        segments.push((
                            prefix.as_os_str().to_string_lossy().into_owned(),
                            CrumbTarget::Local(acc.clone()),
                        ));
                    }
                    Component::RootDir => {
                        acc.push(std::path::MAIN_SEPARATOR.to_string());
                        if let Some(last) = segments.last_mut() {
                            if last.0.ends_with(':') {
                                last.1 = CrumbTarget::Local(acc.clone());
                            } else {
                                segments.push((
                                    std::path::MAIN_SEPARATOR.to_string(),
                                    CrumbTarget::Local(acc.clone()),
                                ));
                            }
                        } else {
                            segments.push((
                                std::path::MAIN_SEPARATOR.to_string(),
                                CrumbTarget::Local(acc.clone()),
                            ));
                        }
                    }
                    Component::Normal(name) => {
                        acc.push(name);
                        segments.push((
                            name.to_string_lossy().into_owned(),
                            CrumbTarget::Local(acc.clone()),
                        ));
                    }
                    Component::ParentDir => {
                        acc.push("..");
                        segments.push(("..".into(), CrumbTarget::Local(acc.clone())));
                    }
                    Component::CurDir => {}
                }
            }
            if segments.is_empty() {
                segments.push((
                    self.local_path.display().to_string(),
                    CrumbTarget::Local(self.local_path.clone()),
                ));
            }
        }

        let count = segments.len();
        let mut crumbs = Vec::with_capacity(count * 2);
        for (index, (label, target)) in segments.into_iter().enumerate() {
            if index > 0 {
                crumbs.push(
                    Icon::new(IconName::ChevronRight)
                        .size(px(10.))
                        .text_color(theme::solid(theme::SUBTLE))
                        .into_any_element(),
                );
            }
            let current = index + 1 == count;
            let mut crumb = div()
                .id(ElementId::Name(
                    format!(
                        "sftp-{}-crumb-{index}",
                        if is_remote { "remote" } else { "local" }
                    )
                    .into(),
                ))
                .px(px(4.))
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(theme::RADIUS_XS)
                .text_size(theme::TYPE_META)
                .text_color(theme::solid(if current {
                    theme::TEXT
                } else {
                    theme::SUBTLE
                }))
                .child(label);
            if connected && !current {
                let target = target.clone();
                crumb = crumb
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(theme::hover_subtle())
                            .text_color(theme::solid(theme::TEXT))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| match &target {
                        CrumbTarget::Local(path) => this.navigate_local(path.clone(), cx),
                        CrumbTarget::Remote(path) => this.navigate_remote(path.clone(), cx),
                    }));
            }
            crumbs.push(crumb.into_any_element());
        }
        crumbs
    }

    fn render_pane(&self, side: Side, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let is_remote = side == Side::Remote;
        let entries = self.entries(side);
        let connected = !is_remote || self.remote_phase.connected();
        let selection = self.selection(side);

        // 先决定这一侧是列表还是内联状态。
        let mut state: Option<AnyElement> = None;
        let mut rows: Vec<AnyElement> = Vec::new();
        if is_remote {
            match &self.remote_phase {
                RemotePhase::Disabled => {
                    state = Some(
                        pane_state(
                            IconName::CloudOff,
                            theme::solid(theme::SUBTLE),
                            "SFTP 连接不可用",
                            "当前环境没有启用远程连接。".into(),
                        )
                        .into_any_element(),
                    );
                }
                RemotePhase::Connecting => {
                    state = Some(
                        pane_state(
                            IconName::RefreshCw,
                            theme::solid(theme::FOAM),
                            "正在连接 SFTP…",
                            "正在建立 SSH 连接并打开 SFTP 子系统。".into(),
                        )
                        .into_any_element(),
                    );
                }
                RemotePhase::Failed(reason) => {
                    let reason = if reason == "已断开" {
                        "传输队列已暂停。重新连接后会从当前进度继续。".to_string()
                    } else {
                        reason.clone()
                    };
                    state = Some(
                        pane_state(
                            IconName::CloudOff,
                            theme::solid(theme::GOLD),
                            "SFTP 连接已断开",
                            reason,
                        )
                        .child(pane_action(
                            "sftp-reconnect",
                            "重新连接",
                            cx.listener(|this, _, _, cx| this.toggle_connection(cx)),
                        ))
                        .into_any_element(),
                    );
                }
                RemotePhase::Connected => {
                    if let Some(error) = &self.remote_list_error {
                        state = Some(
                            pane_state(
                                IconName::TriangleAlert,
                                theme::solid(theme::GOLD),
                                "无法读取远程目录",
                                error.clone(),
                            )
                            .child(pane_action(
                                "sftp-remote-retry",
                                "重试",
                                cx.listener(|this, _, _, cx| this.refresh(Side::Remote, cx)),
                            ))
                            .into_any_element(),
                        );
                    } else if self.remote_loading {
                        state = Some(
                            pane_state(
                                IconName::RefreshCw,
                                theme::solid(theme::FOAM),
                                "正在读取目录…",
                                self.remote_path.clone(),
                            )
                            .into_any_element(),
                        );
                    } else if self.has_parent(side) {
                        rows.push(self.render_parent_row(side, cx).into_any_element());
                    }
                }
            }
        } else if let Some(error) = &self.local_error {
            state = Some(
                pane_state(
                    IconName::TriangleAlert,
                    theme::solid(theme::GOLD),
                    "无法读取本地目录",
                    error.clone(),
                )
                .child(pane_action(
                    "sftp-local-retry",
                    "重试",
                    cx.listener(|this, _, _, cx| this.refresh(Side::Local, cx)),
                ))
                .into_any_element(),
            );
        } else if self.has_parent(side) {
            rows.push(self.render_parent_row(side, cx).into_any_element());
        }

        if state.is_none() {
            for (index, entry) in entries.iter().enumerate() {
                rows.push(
                    self.render_file_row(side, index, entry, selection.contains(index), cx)
                        .into_any_element(),
                );
            }
        }

        let list: AnyElement = if let Some(state) = state {
            state
        } else if rows.is_empty() {
            pane_state(
                IconName::FolderOpen,
                theme::solid(theme::SUBTLE),
                "空目录",
                "此目录中没有文件。".into(),
            )
            .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .id(if is_remote {
                    "sftp-remote-list"
                } else {
                    "sftp-local-list"
                })
                .test_support()
                .overflow_y_scrollbar()
                .py(px(4.))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_file_menu(side, None, event.position, window, cx);
                    }),
                )
                .children(rows)
                .into_any_element()
        };

        let header_cells = self.render_header_cells(side, cx);

        v_flex()
            .id(if is_remote {
                "sftp-remote-pane"
            } else {
                "sftp-local-pane"
            })
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(theme::colored(theme::SURFACE_RGB, theme::PANEL_TINT))
            .on_drop(cx.listener(move |this, drag: &FileDrag, _, cx| {
                this.handle_drop(side, drag, cx);
            }))
            .child(
                h_flex()
                    .h(theme::BAR_HEIGHT)
                    .flex_shrink_0()
                    .items_center()
                    .gap(theme::ICON_TEXT_GAP)
                    .pl(px(10.))
                    // 与刷新按钮距上/下边缘的 4px 保持一致。
                    .pr(px(4.))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(
                        Icon::new(if is_remote {
                            IconName::Server
                        } else {
                            IconName::Monitor
                        })
                        .size(theme::ICON_SM)
                        .text_color(theme::solid(theme::MUTED_STRONG)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(theme::TYPE_LABEL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::solid(theme::MUTED_STRONG))
                            .child(if is_remote { "远程" } else { "本地" }),
                    )
                    .child({
                        let path_text = self.path(side);
                        crate::components::icons::with_tooltip(
                            if is_remote {
                                "sftp-path-remote-tip"
                            } else {
                                "sftp-path-local-tip"
                            },
                            path_text,
                            div()
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .min_w_0()
                                .overflow_hidden()
                                .children(self.render_breadcrumbs(side, connected, cx)),
                        )
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                    })
                    .child({
                        let path_text = self.path(side);
                        crate::components::icons::with_tooltip(
                            if is_remote {
                                "sftp-copy-path-remote-tip"
                            } else {
                                "sftp-copy-path-local-tip"
                            },
                            "复制路径",
                            button::icon(if is_remote {
                                "sftp-copy-path-remote"
                            } else {
                                "sftp-copy-path-local"
                            })
                            .icon(IconName::Copy)
                            .accessibility_label("复制路径")
                            .on_click(cx.listener(
                                move |_this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        path_text.clone(),
                                    ));
                                    cx.emit(SftpEvent::Toast("已复制路径".into()));
                                },
                            )),
                        )
                    })
                    .child(
                        // 抵掉标题栏的 8px 内容间距，让复制与刷新按钮贴合。
                        div().ml(px(-8.)).child(
                            button::icon(if is_remote {
                                "sftp-refresh-remote"
                            } else {
                                "sftp-refresh-local"
                            })
                            .icon(IconName::RefreshCw)
                            .accessibility_label(if is_remote {
                                "刷新远程目录"
                            } else {
                                "刷新本地目录"
                            })
                            // 顶栏背景较密，悬停底比常规图标略强一档。
                            .hover_color(theme::hover_tint_strong())
                            .disabled(!connected)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if connected {
                                        this.refresh(side, cx);
                                    }
                                },
                            )),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .h(px(30.))
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .border_b_1()
                    .border_color(theme::line())
                    .text_size(theme::TYPE_MICRO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::solid(theme::SUBTLE))
                    .children(header_cells),
            )
            .child(list)
            .child(self.render_pane_status(side, cx))
    }

    /// 文件列表底部状态：总项数，以及已选项数与合计大小。
    fn render_pane_status(&self, side: Side, cx: &App) -> impl IntoElement + use<> {
        let entries = self.entries(side);
        let selected: Vec<&FileEntry> = self
            .selection(side)
            .indices()
            .filter_map(|index| entries.get(index))
            .collect();
        let label = if selected.is_empty() {
            "未选择".to_string()
        } else {
            format!("已选 {} 项", selected.len())
        };

        h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .px(px(10.))
            .border_t_1()
            .border_color(theme::line_subtle())
            .font_family(cx.theme().font_family.clone())
            .text_size(theme::TYPE_MICRO)
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme::solid(theme::SUBTLE))
                    .child(format!("{} 项", entries.len())),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::solid(theme::SUBTLE))
                    .child(label),
            )
    }

    fn render_header_cells(&self, side: Side, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let is_remote = side == Side::Remote;
        let sort = self.sort(side);
        let caret = |key: SortKey| {
            (sort.touched && sort.key == key).then(|| {
                if sort.dir == SortDir::Asc {
                    "▲"
                } else {
                    "▼"
                }
            })
        };

        let mut cells: Vec<AnyElement> = Vec::new();
        cells.push(
            div()
                .id(ElementId::Name(
                    format!(
                        "sftp-{}-header-name",
                        if is_remote { "remote" } else { "local" }
                    )
                    .into(),
                ))
                .aria_label(if sort.key == SortKey::Name {
                    if sort.dir == SortDir::Asc {
                        "名称，升序"
                    } else {
                        "名称，降序"
                    }
                } else {
                    "按名称排序"
                })
                .test_support()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .hover(|style| style.text_color(theme::solid(theme::TEXT)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_sort(side, SortKey::Name, cx);
                }))
                .child(div().min_w_0().truncate().child("名称"))
                .when_some(caret(SortKey::Name), |cell, caret| {
                    cell.child(
                        div()
                            .flex_shrink_0()
                            .text_size(theme::TYPE_MICRO)
                            .child(caret),
                    )
                })
                .into_any_element(),
        );
        cells.push(self.render_header_cell(side, Column::Size, "大小", cx));
        if is_remote {
            cells.push(self.render_header_cell(side, Column::Modified, "修改时间", cx));
            if !self.compact_columns() {
                cells.push(self.render_header_cell(side, Column::Permissions, "权限", cx));
                cells.push(self.render_header_cell(side, Column::Owner, "所有者", cx));
            }
        } else {
            cells.push(self.render_header_cell(side, Column::Modified, "修改时间", cx));
        }
        cells
    }

    fn render_header_cell(
        &self,
        side: Side,
        column: Column,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = column.sort_key();
        let sort = self.sort(side);
        let caret = (sort.touched && sort.key == key).then(|| {
            if sort.dir == SortDir::Asc {
                "▲"
            } else {
                "▼"
            }
        });
        let cell = div()
            .id(ElementId::Name(
                format!(
                    "sftp-{}-header-{column:?}",
                    if side == Side::Remote {
                        "remote"
                    } else {
                        "local"
                    }
                )
                .into(),
            ))
            .aria_label(if sort.key == key {
                if sort.dir == SortDir::Asc {
                    format!("{label}，升序")
                } else {
                    format!("{label}，降序")
                }
            } else {
                format!("按{label}排序")
            })
            .test_support()
            .relative()
            .w(px(self.column_width(side, column)))
            .h_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(8.))
            .cursor_pointer()
            .hover(|style| style.text_color(theme::solid(theme::TEXT)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_sort(side, key, cx);
            }))
            .child(div().truncate().child(label))
            .when_some(caret, |cell, caret| {
                cell.child(
                    div()
                        .flex_shrink_0()
                        .text_size(theme::TYPE_MICRO)
                        .child(caret),
                )
            })
            .child(
                div()
                    .absolute()
                    .left(px(-4.))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(1.))
                    .bg(theme::line()),
            );
        cell.into_any_element()
    }

    fn render_file_row(
        &self,
        side: Side,
        index: usize,
        file: &FileEntry,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let is_remote = side == Side::Remote;
        let icon = match file.kind {
            FileKind::Folder => IconName::Folder,
            FileKind::Symlink => IconName::FolderSymlink,
            FileKind::File => IconName::File,
        };
        let icon_color = if selected {
            theme::solid(theme::FOAM)
        } else if matches!(file.kind, FileKind::Folder | FileKind::Symlink) {
            theme::solid(theme::MUTED_STRONG)
        } else {
            theme::solid(theme::SUBTLE)
        };

        div()
            .id(ElementId::Name(
                format!(
                    "sftp-{}-row-{index}",
                    if is_remote { "remote" } else { "local" }
                )
                .into(),
            ))
            .test_support()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .mx(px(4.))
            .px(px(6.))
            .rounded(theme::RADIUS_SM)
            .tab_index(0)
            .aria_label(format!("{}，{}", file.name, file.size_label()))
            .focus_visible(|style| style.bg(theme::colored(theme::FOAM, 0.06)))
            .cursor_pointer()
            .when(selected, |row| {
                row.bg(theme::colored(theme::FOAM, 0.08)).child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(4.))
                        .bottom(px(4.))
                        .w(px(2.))
                        .rounded(px(2.))
                        .bg(theme::solid(theme::FOAM)),
                )
            })
            .hover(|style| style.bg(theme::hover_subtle()))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.select_file_with_modifiers(side, index, event.modifiers(), cx);
                if event.click_count() == 2 {
                    this.open_entry(side, index, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_file_menu(side, Some(index), event.position, window, cx);
                }),
            )
            .child(
                h_flex()
                    .id(ElementId::Name(
                        format!(
                            "sftp-{}-draggable-{index}",
                            if is_remote { "remote" } else { "local" }
                        )
                        .into(),
                    ))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(6.))
                    .cursor_move()
                    .text_size(theme::TYPE_META)
                    .text_color(theme::solid(theme::TEXT))
                    // 「此电脑」里的盘符不能作为传输来源。
                    .when(!(side == Side::Local && self.local_roots), |name| {
                        // 拖已选中的行时带上整组选择；否则只拖当前行。
                        let names: Vec<String> = if self.selection(side).contains(index) {
                            self.selection(side)
                                .indices()
                                .filter_map(|selected| self.entries(side).get(selected))
                                .map(|entry| entry.name.clone())
                                .collect()
                        } else {
                            vec![file.name.clone()]
                        };
                        name.on_drag(FileDrag { side, names }, |drag: &FileDrag, _, _, cx| {
                            let label = match drag.names.as_slice() {
                                [first] => first.clone(),
                                [first, ..] => {
                                    format!("{first} 等 {} 项", drag.names.len())
                                }
                                [] => String::new(),
                            };
                            cx.new(|_| FileDragPreview { name: label.into() })
                        })
                    })
                    .child(Icon::new(icon).size(theme::ICON_SM).text_color(icon_color))
                    .child(div().min_w_0().truncate().child(file.name.clone()))
                    .when_some(file.target.clone(), |name, target| {
                        name.child(
                            div()
                                .max_w(px(56.))
                                .flex_shrink_0()
                                .truncate()
                                .font_family(cx.theme().font_family.clone())
                                .text_size(theme::TYPE_MICRO)
                                .text_color(theme::solid(theme::SUBTLE))
                                .child(format!("→ {target}")),
                        )
                    }),
            )
            .child(
                div()
                    .id(ElementId::Name(
                        format!(
                            "sftp-{}-size-{index}",
                            if is_remote { "remote" } else { "local" }
                        )
                        .into(),
                    ))
                    .test_support()
                    .w(px(self.column_width(side, Column::Size)))
                    .flex_shrink_0()
                    .text_right()
                    .truncate()
                    .font_family(cx.theme().font_family.clone())
                    .text_size(theme::TYPE_MICRO)
                    .text_color(theme::solid(if selected {
                        theme::MUTED_STRONG
                    } else {
                        theme::SUBTLE
                    }))
                    .child(file.size_label()),
            )
            .when(is_remote, |row| {
                let row = row.child(
                    div()
                        .id(ElementId::Name(
                            format!(
                                "sftp-{}-modified-{index}",
                                if is_remote { "remote" } else { "local" }
                            )
                            .into(),
                        ))
                        .test_support()
                        .w(px(self.column_width(side, Column::Modified)))
                        .flex_shrink_0()
                        .truncate()
                        .font_family(cx.theme().font_family.clone())
                        .text_size(theme::TYPE_MICRO)
                        .text_color(theme::solid(theme::SUBTLE))
                        .child(file.modified.clone()),
                );
                if self.compact_columns() {
                    row
                } else {
                    row.child(
                        div()
                            .w(px(self.column_width(side, Column::Permissions)))
                            .flex_shrink_0()
                            .truncate()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::MUTED_STRONG))
                            .child(file.permissions.clone()),
                    )
                    .child(
                        div()
                            .w(px(self.column_width(side, Column::Owner)))
                            .flex_shrink_0()
                            .truncate()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child(file.owner.clone()),
                    )
                }
            })
            .when(!is_remote, |row| {
                row.child(
                    div()
                        .id(ElementId::Name(
                            format!(
                                "sftp-{}-modified-{index}",
                                if is_remote { "remote" } else { "local" }
                            )
                            .into(),
                        ))
                        .test_support()
                        .w(px(self.column_width(side, Column::Modified)))
                        .flex_shrink_0()
                        .truncate()
                        .font_family(cx.theme().font_family.clone())
                        .text_size(theme::TYPE_MICRO)
                        .text_color(theme::solid(theme::SUBTLE))
                        .child(file.modified.clone()),
                )
            })
    }

    fn render_parent_row(&self, side: Side, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let is_remote = side == Side::Remote;
        div()
            .id(if is_remote {
                "sftp-remote-parent"
            } else {
                "sftp-local-parent"
            })
            .test_support()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .mx(px(4.))
            .px(px(6.))
            .rounded(theme::RADIUS_SM)
            .tab_index(0)
            .aria_label("返回上一级")
            .focus_visible(|style| style.bg(theme::colored(theme::FOAM, 0.06)))
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover_subtle()))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.select_parent(side, cx);
                if event.click_count() == 2 {
                    this.open_parent(side, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_file_menu(side, None, event.position, window, cx);
                }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(6.))
                    .text_size(theme::TYPE_META)
                    .text_color(theme::solid(theme::MUTED_STRONG))
                    .child(
                        Icon::new(IconName::ArrowUp)
                            .size(theme::ICON_SM)
                            .text_color(theme::solid(theme::MUTED_STRONG)),
                    )
                    .child(div().min_w_0().truncate().child("..")),
            )
            .child(
                div()
                    .w(px(self.column_width(side, Column::Size)))
                    .flex_shrink_0()
                    .text_right()
                    .font_family(cx.theme().font_family.clone())
                    .text_size(theme::TYPE_MICRO)
                    .text_color(theme::solid(theme::SUBTLE))
                    .child("-"),
            )
            .when(is_remote, |row| {
                let row = row.child(
                    div()
                        .w(px(self.column_width(side, Column::Modified)))
                        .flex_shrink_0()
                        .text_size(theme::TYPE_MICRO)
                        .text_color(theme::solid(theme::SUBTLE))
                        .child("-"),
                );
                if self.compact_columns() {
                    row
                } else {
                    row.child(
                        div()
                            .w(px(self.column_width(side, Column::Permissions)))
                            .flex_shrink_0()
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child("-"),
                    )
                    .child(
                        div()
                            .w(px(self.column_width(side, Column::Owner)))
                            .flex_shrink_0()
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child("-"),
                    )
                }
            })
            .when(!is_remote, |row| {
                row.child(
                    div()
                        .w(px(self.column_width(side, Column::Modified)))
                        .flex_shrink_0()
                        .text_size(theme::TYPE_MICRO)
                        .text_color(theme::solid(theme::SUBTLE))
                        .child("-"),
                )
            })
    }

    // —— 传输队列 ——

    /// 队列右侧内缩：与文件面板标题栏的刷新按钮一致，距右边框 4px。
    /// 标题栏与所有行共用，保证行内的操作图标与标题栏图标对齐。
    const QUEUE_RIGHT_INSET: Pixels = px(4.);

    fn render_queue(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // 空队列与折叠态只保留标题行，不占一整块空白区域。
        let empty = self.batches.is_empty();
        let height = if self.queue_collapsed || empty {
            theme::BAR_HEIGHT
        } else {
            px(238.)
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        for batch in &self.batches {
            rows.push(self.render_batch(batch, cx).into_any_element());
        }
        let body: AnyElement = if self.queue_collapsed || empty {
            div().into_any_element()
        } else {
            div()
                .id("sftp-queue-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .children(rows)
                .into_any_element()
        };

        v_flex()
            .h(height)
            .flex_shrink_0()
            .overflow_hidden()
            .border_t_1()
            .border_color(theme::line())
            .bg(theme::colored(theme::BASE_DEEP_RGB, theme::QUEUE_TINT))
            .child(
                h_flex()
                    .h(theme::BAR_HEIGHT)
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    // 与 SFTP 面板标题栏相同的 10px 左缩进；右侧避开滚动条。
                    .pl(px(10.))
                    .pr(Self::QUEUE_RIGHT_INSET)
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_center()
                            .gap(theme::ICON_TEXT_GAP)
                            .child(
                                Icon::new(IconName::List)
                                    .size(theme::ICON_SM)
                                    .text_color(theme::solid(theme::MUTED_STRONG)),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(theme::TYPE_LABEL)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::solid(theme::MUTED_STRONG))
                                    .child("传输队列"),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(2.))
                            .child(crate::components::icons::with_tooltip(
                                "sftp-clear-completed-tip",
                                "清除已完成",
                                button::icon("sftp-clear-completed")
                                    .icon(IconName::Broom)
                                    .accessibility_label("清除已完成")
                                    .disabled(!self.has_completed())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clear_completed(cx);
                                    })),
                            ))
                            .child(crate::components::icons::with_tooltip(
                                "sftp-toggle-queue-tip",
                                if self.queue_collapsed {
                                    "展开传输队列"
                                } else {
                                    "收起传输队列"
                                },
                                button::icon("sftp-toggle-queue")
                                    .icon(if self.queue_collapsed {
                                        IconName::ChevronUp
                                    } else {
                                        IconName::ChevronDown
                                    })
                                    .accessibility_label(if self.queue_collapsed {
                                        "展开传输队列"
                                    } else {
                                        "收起传输队列"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.queue_collapsed = !this.queue_collapsed;
                                        cx.notify();
                                    })),
                            )),
                    ),
            )
            .child(body)
    }

    fn render_batch(
        &self,
        batch: &TransferBatch,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let batch_row = format!("batch-{}", batch.id);
        let selected = self
            .selected_row
            .as_deref()
            .is_some_and(|row| row == batch_row);

        let mut children: Vec<AnyElement> = Vec::new();
        if batch.expanded && batch.can_expand() {
            let count = batch.items.len();
            for (index, item) in batch.items.iter().enumerate() {
                children.push(
                    self.render_child(batch, index, item, index + 1 == count, cx)
                        .into_any_element(),
                );
            }
        }

        v_flex()
            .child(self.render_batch_row(batch, selected, cx))
            .children(children)
    }

    fn render_batch_row(
        &self,
        batch: &TransferBatch,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = batch.id;
        let status = batch.status;
        let direction_icon = match batch.direction {
            Direction::Upload => IconName::ArrowUp,
            Direction::Download => IconName::ArrowDown,
        };
        // 上传与下载用两种颜色区分，仍不加背景。
        let direction_color = match batch.direction {
            Direction::Upload => theme::solid(theme::GOLD),
            Direction::Download => theme::solid(theme::FOAM),
        };
        // 状态列是整行唯一的着色处；其余信息一律用中性色。
        let status_color = match status {
            TransferStatus::Running | TransferStatus::Scanning => theme::solid(theme::FOAM),
            TransferStatus::Reconnecting | TransferStatus::Stopped => theme::solid(theme::GOLD),
            TransferStatus::Partial | TransferStatus::Failed => theme::solid(theme::LOVE),
            TransferStatus::Waiting | TransferStatus::Completed => theme::solid(theme::SUBTLE),
        };
        let progress_color = theme::solid(theme::MUTED_STRONG);
        // 百分比列只放数字；扫描中没有确定进度，留空。
        let percent_label = if status == TransferStatus::Scanning {
            String::new()
        } else {
            format!("{:.0}%", batch.progress())
        };
        let status_text = match status {
            TransferStatus::Running => "进行中",
            TransferStatus::Scanning => "扫描中",
            TransferStatus::Waiting => "等待",
            TransferStatus::Reconnecting => "重连中",
            TransferStatus::Stopped => "已停止",
            TransferStatus::Partial => "部分失败",
            TransferStatus::Completed => "已完成",
            TransferStatus::Failed => "失败",
        };
        let metrics = self.render_metrics(batch, status, cx);
        let actions = self.render_batch_actions(batch, cx);

        div()
            .id(ElementId::Name(format!("sftp-batch-{}", batch.id).into()))
            .relative()
            .h(px(36.))
            .flex()
            .items_center()
            // 与标题栏同宽的内缩，图标列上下对齐且不被滚动条遮住。
            .pr(Self::QUEUE_RIGHT_INSET)
            .border_b_1()
            .border_color(theme::line_subtle())
            .tab_index(0)
            .aria_label(match &batch.error {
                Some(error) => format!("{}，{}，{}", batch.title, status_text, error),
                None => format!("{}，{}", batch.title, status_text),
            })
            .focus_visible(|style| style.bg(theme::colored(theme::FOAM, 0.06)))
            .cursor_pointer()
            .when(selected, |row| {
                row.bg(theme::colored(theme::FOAM, 0.06)).child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(6.))
                        .bottom(px(6.))
                        .w(px(2.))
                        .rounded(px(2.))
                        .bg(theme::solid(theme::FOAM)),
                )
            })
            .hover(|style| style.bg(theme::hover_subtle()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_row = Some(format!("batch-{id}"));
                cx.notify();
            }))
            .child(
                h_flex()
                    // 展开列固定 28px：可展开与不可展开的行共用同一列宽，
                    // 方向图标与标题始终对齐；图标与文件名之间留 8px。
                    .w(px(54.))
                    .h_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(4.))
                    .pr(px(8.))
                    .child(if batch.can_expand() {
                        div()
                            .id(ElementId::Name(format!("sftp-expand-{id}").into()))
                            .aria_label(if batch.expanded {
                                "收起明细"
                            } else {
                                "展开明细"
                            })
                            .size(theme::ICON_BUTTON_SIZE)
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(theme::RADIUS_XS)
                            .cursor_pointer()
                            .text_color(theme::solid(theme::SUBTLE))
                            .hover(|style| {
                                style
                                    .bg(theme::hover_tint())
                                    .text_color(theme::solid(theme::TEXT))
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_expanded(id, cx);
                            }))
                            .child(
                                Icon::new(if batch.expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(12.)),
                            )
                            .into_any_element()
                    } else {
                        // 没有明细的行占住同一列，保持方向图标对齐。
                        div().w(theme::ICON_BUTTON_SIZE).into_any_element()
                    })
                    .child(
                        // 方向箭头只用图标本身占位，不再保留 22px 的背景盒。
                        div()
                            .size(px(14.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(direction_icon)
                                    .size(theme::ICON_SM)
                                    .text_color(direction_color),
                            ),
                    ),
            )
            .child(
                // 标题与路径都不再吃掉剩余空间：信息区紧跟在路径后面，
                // 富余宽度留给行尾的空白，操作按钮仍在最右侧。
                h_flex()
                    .min_w_0()
                    .flex_shrink_0()
                    .h_full()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        // 标题列固定宽度，后面的箭头与路径从同一位置开始。
                        div()
                            .w(px(210.))
                            .flex_shrink_0()
                            .truncate()
                            .text_size(theme::TYPE_META)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::solid(theme::TEXT))
                            .child(batch.title.clone()),
                    )
                    .child({
                        // 路径列固定上限：缩短它与右侧信息区的距离，超长截断。
                        // 240px 与右侧信息区、子行合起来正好放进常见窗口宽度。
                        let target = format!("→ {}", batch.target);
                        let tooltip_text = target.clone();
                        div()
                            .id(ElementId::Name(format!("sftp-target-{id}").into()))
                            .w(px(232.))
                            .min_w_0()
                            .truncate()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .tooltip(move |window, cx| {
                                Tooltip::new(tooltip_text.clone())
                                    .text_size(theme::TYPE_META)
                                    .build(window, cx)
                            })
                            .child(target)
                    }),
            )
            .child(
                // 信息区：总大小 | 进度条 | 百分比 | 状态 | 速度，
                // 列间距统一 10px（以百分比与速度之间为准）。
                h_flex()
                    .h_full()
                    .flex_shrink_0()
                    .items_center()
                    // 路径之后、以及各信息组之间统一 10px。
                    .gap(px(10.))
                    .pl(px(10.))
                    .child(
                        div()
                            .w(px(80.))
                            .flex_shrink_0()
                            .truncate()
                            .text_right()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child(if batch.total > 0 {
                                format_bytes(batch.total)
                            } else {
                                "—".to_string()
                            }),
                    )
                    .child(
                        // 进度条与百分比作为一个整体，内部间距比列间距更紧。
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(4.))
                            .child(progress_bar(
                                batch.progress(),
                                status == TransferStatus::Scanning,
                                progress_color,
                            ))
                            .child(
                                div()
                                    .w(px(40.))
                                    .flex_shrink_0()
                                    .text_right()
                                    .truncate()
                                    .font_family(cx.theme().font_family.clone())
                                    .text_size(theme::TYPE_MICRO)
                                    .text_color(theme::solid(theme::SUBTLE))
                                    .child(percent_label),
                            ),
                    )
                    .child(metrics)
                    .child(
                        // 状态列居中，并且是整行唯一保留状态色的位置。
                        div()
                            .w(px(56.))
                            .flex_shrink_0()
                            .truncate()
                            .text_center()
                            .text_size(theme::TYPE_MICRO)
                            .text_color(status_color)
                            .child(status_text),
                    ),
            )
            // 余量放在状态与按钮之间，操作按钮贴右，与标题栏图标对齐。
            .child(div().flex_1())
            .child(actions)
    }

    fn render_metrics(
        &self,
        batch: &TransferBatch,
        status: TransferStatus,
        cx: &App,
    ) -> AnyElement {
        // 只显示速度与剩余时间；两者任一不可用时就留空。
        let label = if status == TransferStatus::Running && batch.speed > 0.0 {
            batch.eta_secs.map(|eta| {
                format!(
                    "{} · 剩余 {}",
                    format_speed(batch.speed),
                    format_eta(Some(eta))
                )
            })
        } else {
            None
        };

        h_flex()
            // 留足宽度，速度到 GB/s、剩余时间到小时级也不会被截断。
            .w(px(200.))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .overflow_hidden()
            .font_family(cx.theme().font_family.clone())
            .text_size(theme::TYPE_MICRO)
            .when_some(label, |row, label| {
                row.child(
                    div()
                        .truncate()
                        .text_color(theme::solid(theme::SUBTLE))
                        .child(label),
                )
            })
            .into_any_element()
    }

    fn render_batch_actions(
        &self,
        batch: &TransferBatch,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = batch.id;
        let mut buttons: Vec<AnyElement> = Vec::new();
        match batch.status {
            TransferStatus::Running | TransferStatus::Scanning | TransferStatus::Reconnecting => {
                buttons.push(
                    self.queue_button(
                        format!("sftp-pause-{id}"),
                        IconName::Pause,
                        "暂停",
                        QueueAction::Pause,
                        id,
                        None,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            TransferStatus::Stopped => {
                buttons.push(
                    self.queue_button(
                        format!("sftp-resume-{id}"),
                        IconName::Play,
                        "继续",
                        QueueAction::Resume,
                        id,
                        None,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            TransferStatus::Partial | TransferStatus::Failed => {
                buttons.push(
                    self.queue_button(
                        format!("sftp-retry-{id}"),
                        IconName::RotateCw,
                        "重试",
                        QueueAction::Retry,
                        id,
                        None,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            TransferStatus::Waiting | TransferStatus::Completed => {}
        }
        // 只有已完成的任务显示扫把「清理」，其余状态显示 X「移出队列」。
        let (remove_icon, remove_tip) = if batch.status == TransferStatus::Completed {
            (IconName::Broom, "清理")
        } else {
            (IconName::X, "移出队列")
        };
        buttons.push(
            self.queue_button(
                format!("sftp-remove-{id}"),
                remove_icon,
                remove_tip,
                QueueAction::Remove,
                id,
                None,
                cx,
            )
            .into_any_element(),
        );

        h_flex()
            // 两个 28px 图标按钮 + 2px 间隙，固定宽度让各行的按钮列对齐；
            // 与状态列至少保持 10px。
            .w(px(58.))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .justify_end()
            .gap(px(2.))
            .ml(px(10.))
            .children(buttons)
    }

    fn render_child(
        &self,
        batch: &TransferBatch,
        index: usize,
        item: &TransferItem,
        last: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let batch_id = batch.id;
        let selected = self
            .selected_row
            .as_deref()
            .is_some_and(|row| row == format!("{batch_id}:{index}"));
        let scanning =
            batch.status == TransferStatus::Scanning && item.status == ItemStatus::Pending;
        let status_color = match item.status {
            ItemStatus::Done | ItemStatus::Skipped | ItemStatus::Pending => {
                theme::solid(theme::SUBTLE)
            }
            ItemStatus::Failed => theme::solid(theme::LOVE),
            ItemStatus::Running => theme::solid(theme::FOAM),
        };
        let progress = if item.size > 0 {
            (item.transferred as f64 / item.size as f64 * 100.0) as f32
        } else if item.status == ItemStatus::Done {
            100.0
        } else {
            0.0
        };
        let status_label = match item.status {
            ItemStatus::Done => "完成",
            ItemStatus::Failed => "失败",
            ItemStatus::Pending => "等待",
            ItemStatus::Skipped => "跳过",
            ItemStatus::Running => "进行中",
        };
        let percent_label = if scanning {
            String::new()
        } else {
            format!("{progress:.0}%")
        };

        // 子行不画文件图标：树线从展开箭头下方引到文件名前。
        let tree = div()
            .relative()
            .w(px(62.))
            .h_full()
            .flex_shrink_0()
            .child(
                div()
                    .absolute()
                    .left(px(18.))
                    .top(px(0.))
                    .w(px(1.))
                    .h(px(if last { 18. } else { 36. }))
                    .bg(theme::track()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(18.))
                    .top(px(18.))
                    .w(px(38.))
                    .h(px(1.))
                    .bg(theme::track()),
            );

        div()
            .id(ElementId::Name(
                format!("sftp-child-{batch_id}-{index}").into(),
            ))
            .relative()
            .h(px(36.))
            .flex()
            .items_center()
            .pr(Self::QUEUE_RIGHT_INSET)
            .border_b_1()
            .border_color(theme::line_subtle())
            .tab_index(0)
            .aria_label(match &item.error {
                Some(error) => format!("{}，{status_label}，{error}", item.name),
                None => format!("{}，{status_label}", item.name),
            })
            .when_some(item.error.clone(), |row, error| {
                row.tooltip(move |window, cx| {
                    Tooltip::new(error.clone())
                        .text_size(theme::TYPE_META)
                        .build(window, cx)
                })
            })
            .focus_visible(|style| style.bg(theme::colored(theme::FOAM, 0.06)))
            .cursor_pointer()
            .when(selected, |row| {
                row.bg(theme::colored(theme::FOAM, 0.06)).child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(6.))
                        .bottom(px(6.))
                        .w(px(2.))
                        .rounded(px(2.))
                        .bg(theme::solid(theme::FOAM)),
                )
            })
            .hover(|style| style.bg(theme::hover_subtle()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_row = Some(format!("{batch_id}:{index}"));
                cx.notify();
            }))
            .child(tree)
            .child(
                // 名称比父行标题缩进一级；名称列与父行标题 + 路径两列等宽，
                // 之后的字段与父行使用完全相同的列宽与间距。
                h_flex()
                    .w(px(440.))
                    .flex_shrink_0()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::TYPE_META)
                            .text_color(theme::solid(theme::MUTED_STRONG))
                            .child(item.name.clone()),
                    ),
            )
            .child(
                h_flex()
                    .h_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(10.))
                    .child(
                        div()
                            .w(px(80.))
                            .flex_shrink_0()
                            .truncate()
                            .text_right()
                            .font_family(cx.theme().font_family.clone())
                            .text_size(theme::TYPE_MICRO)
                            .text_color(theme::solid(theme::SUBTLE))
                            .child(format_bytes(item.size)),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(4.))
                            .child(progress_bar(
                                progress,
                                scanning,
                                theme::solid(theme::MUTED_STRONG),
                            ))
                            .child(
                                div()
                                    .w(px(40.))
                                    .flex_shrink_0()
                                    .text_right()
                                    .truncate()
                                    .font_family(cx.theme().font_family.clone())
                                    .text_size(theme::TYPE_MICRO)
                                    .text_color(theme::solid(theme::SUBTLE))
                                    .child(percent_label),
                            ),
                    )
                    // 子行没有速度/剩余时间，保留同宽空列以对齐父行。
                    .child(div().w(px(200.)).flex_shrink_0())
                    .child(
                        div()
                            .w(px(56.))
                            .flex_shrink_0()
                            .truncate()
                            .text_center()
                            .text_size(theme::TYPE_MICRO)
                            .text_color(status_color)
                            .child(status_label),
                    ),
            )
            // 余量放在状态与按钮之间，操作按钮与父行、标题栏对齐。
            .child(div().flex_1())
            .child({
                let mut buttons: Vec<AnyElement> = Vec::new();
                if item.status == ItemStatus::Failed {
                    buttons.push(
                        self.queue_button(
                            format!("sftp-child-retry-{batch_id}-{index}"),
                            IconName::RotateCw,
                            "重试",
                            QueueAction::Retry,
                            batch_id,
                            Some(index),
                            cx,
                        )
                        .into_any_element(),
                    );
                }
                // 已完成的行显示扫把「清理」，其余显示 X。
                let (remove_icon, remove_tip) = if item.status == ItemStatus::Done {
                    (IconName::Broom, "清理")
                } else {
                    (IconName::X, "移出队列")
                };
                buttons.push(
                    self.queue_button(
                        format!("sftp-child-remove-{batch_id}-{index}"),
                        remove_icon,
                        remove_tip,
                        QueueAction::Remove,
                        batch_id,
                        Some(index),
                        cx,
                    )
                    .into_any_element(),
                );
                h_flex()
                    .w(px(58.))
                    .h_full()
                    .flex_shrink_0()
                    .items_center()
                    .justify_end()
                    .gap(px(2.))
                    .ml(px(10.))
                    .children(buttons)
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_button(
        &self,
        id: String,
        icon: IconName,
        tip: &'static str,
        action: QueueAction,
        batch_id: u64,
        item_index: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // 与标题栏一致的图标按钮：28×28 命中区、14px 图标、12px 提示。
        button::icon(ElementId::Name(id.into()))
            .icon(icon)
            .accessibility_label(tip)
            .tooltip(tip)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.apply_queue_action(batch_id, item_index, action, cx);
            }))
    }

    // —— 队列操作（本地模拟） ——

    fn toggle_expanded(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(batch) = self.batches.iter_mut().find(|batch| batch.id == id) {
            batch.expanded = !batch.expanded;
        }
        cx.notify();
    }

    fn clear_completed(&mut self, cx: &mut Context<Self>) {
        self.batches
            .retain(|batch| batch.status != TransferStatus::Completed);
        cx.notify();
    }

    fn apply_queue_action(
        &mut self,
        batch_id: u64,
        _item_index: Option<usize>,
        action: QueueAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            QueueAction::Pause => self.pause_batch(batch_id, cx),
            QueueAction::Resume => self.resume_batch(batch_id, cx),
            QueueAction::Retry => self.retry_batch(batch_id, cx),
            QueueAction::Remove => self.cancel_batch(batch_id, cx),
        }
    }

    fn command(&self, command: RemoteCommand) {
        if let Some(commands) = &self.commands {
            let _ = commands.try_send(command);
        }
    }

    fn pause_batch(&mut self, batch_id: u64, cx: &mut Context<Self>) {
        self.command(RemoteCommand::Pause { batch_id });
        if let Some(batch) = self.batches.iter_mut().find(|batch| batch.id == batch_id)
            && matches!(
                batch.status,
                TransferStatus::Running | TransferStatus::Scanning | TransferStatus::Reconnecting
            )
        {
            batch.status = TransferStatus::Stopped;
            batch.speed = 0.0;
            batch.eta_secs = None;
        }
        cx.notify();
    }

    fn resume_batch(&mut self, batch_id: u64, cx: &mut Context<Self>) {
        self.command(RemoteCommand::Resume { batch_id });
        if let Some(batch) = self.batches.iter_mut().find(|batch| batch.id == batch_id)
            && batch.status == TransferStatus::Stopped
        {
            batch.status = TransferStatus::Running;
        }
        cx.notify();
    }

    fn cancel_batch(&mut self, batch_id: u64, cx: &mut Context<Self>) {
        self.command(RemoteCommand::Cancel { batch_id });
        self.batches.retain(|batch| batch.id != batch_id);
        if self
            .selected_row
            .as_deref()
            .is_some_and(|row| row == format!("batch-{batch_id}"))
        {
            self.selected_row = None;
        }
        cx.notify();
    }

    fn retry_batch(&mut self, batch_id: u64, cx: &mut Context<Self>) {
        let Some(batch) = self
            .batches
            .iter()
            .find(|batch| batch.id == batch_id)
            .cloned()
        else {
            return;
        };
        self.batches.retain(|batch| batch.id != batch_id);
        self.enqueue_sources(
            batch.direction,
            batch.sources,
            batch.destination,
            batch.title,
            batch.target,
            cx,
        );
    }

    /// 拖动松手：从源侧文件或文件夹发起一次真实传输。
    fn handle_drop(&mut self, target: Side, drag: &FileDrag, cx: &mut Context<Self>) {
        if drag.side == target || drag.names.is_empty() {
            return;
        }
        let local_display = self.local_path.display().to_string();
        let (direction, sources, destination, target_display) = match drag.side {
            Side::Local => (
                Direction::Upload,
                drag.names
                    .iter()
                    .map(|name| self.local_path.join(name).display().to_string())
                    .collect::<Vec<_>>(),
                self.remote_path.clone(),
                self.remote_path.clone(),
            ),
            Side::Remote => (
                Direction::Download,
                drag.names
                    .iter()
                    .map(|name| join_remote_path(&self.remote_path, name))
                    .collect::<Vec<_>>(),
                local_display.clone(),
                local_display,
            ),
        };
        let title = match drag.names.as_slice() {
            [first] => first.clone(),
            [first, ..] => format!("{first} 等 {} 项", drag.names.len()),
            [] => "传输".into(),
        };
        self.enqueue_sources(direction, sources, destination, title, target_display, cx);
    }

    fn enqueue_sources(
        &mut self,
        direction: Direction,
        sources: Vec<String>,
        destination: String,
        title: String,
        target: String,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_batch_id;
        self.next_batch_id += 1;
        let connected = self.commands.is_some();
        self.batches.insert(
            0,
            TransferBatch {
                id,
                direction,
                status: if connected {
                    TransferStatus::Scanning
                } else {
                    TransferStatus::Failed
                },
                title,
                target,
                transferred: 0,
                total: 0,
                speed: 0.0,
                eta_secs: None,
                expanded: false,
                items: Vec::new(),
                sources: sources.clone(),
                destination: destination.clone(),
                error: (!connected).then(|| "SFTP 未连接".to_string()),
            },
        );
        self.selected_row = Some(format!("batch-{id}"));
        if connected {
            let direction = match direction {
                Direction::Upload => remote::TransferDirection::Upload,
                Direction::Download => remote::TransferDirection::Download,
            };
            self.command(RemoteCommand::Transfer {
                batch_id: id,
                direction,
                sources,
                destination,
            });
        }
        cx.notify();
    }
}

impl Render for SftpView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 拖动期间盖一层透明层：鼠标移出表头后仍能收到移动与松开。
        let bounds_entity = cx.entity();
        let divider = div().w(px(1.)).h_full().flex_shrink_0().bg(theme::line());

        v_flex()
            .size_full()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .bg(theme::colored(theme::BASE_DEEP_RGB, theme::ROOT_TINT))
            .on_prepaint(move |bounds, _, cx| {
                bounds_entity.update(cx, |this, cx| {
                    let was_compact = this.compact_columns();
                    this.view_bounds = Some(bounds);
                    if was_compact != this.compact_columns() {
                        cx.notify();
                    }
                });
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.file_menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_pane(Side::Local, cx))
                    .child(divider)
                    .child(self.render_pane(Side::Remote, cx)),
            )
            .child(self.render_queue(cx))
            .children(self.render_file_menu(cx))
    }
}

struct NameForm {
    input: Entity<InputState>,
    error: Option<SharedString>,
}

impl Render for NameForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(px(6.))
            .child(
                field::text_field_with(
                    ElementId::Name("sftp-name-input-shell".into()),
                    &self.input,
                    window,
                    cx,
                    |input| input.aria_label("名称").w_full(),
                )
                .into_any_element(),
            )
            .when_some(self.error.clone(), |form, error| {
                form.child(
                    div()
                        .text_size(theme::TYPE_MICRO)
                        .text_color(theme::solid(theme::LOVE))
                        .child(error),
                )
            })
    }
}

fn confirm_name(
    view: &Entity<SftpView>,
    form: &Entity<NameForm>,
    side: Side,
    mode: NameMode,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let name = form.read(cx).input.read(cx).value().trim().to_string();
    match view.read(cx).validate_name(side, mode, &name) {
        Ok(()) => {
            view.update(cx, |view, cx| view.apply_name(side, mode, name, cx));
            window.close_dialog(cx);
            true
        }
        Err(message) => {
            form.update(cx, |form, cx| {
                form.error = Some(message.into());
                cx.notify();
            });
            false
        }
    }
}

fn menu_divider() -> AnyElement {
    crate::components::overlay::menu_divider().into_any_element()
}

fn is_text_file(entry: &FileEntry) -> bool {
    if entry.is_dir || entry.kind == FileKind::Symlink {
        return false;
    }
    let lower = entry.name.to_lowercase();
    // 没有扩展名的文件（dotfile、Makefile、known_hosts 等）按文本处理。
    let dotfile = lower.starts_with('.') && !lower[1..].contains('.');
    if !lower.contains('.') || dotfile {
        return true;
    }
    let Some(extension) = lower.rsplit('.').next() else {
        return false;
    };
    matches!(
        extension,
        "txt"
            | "md"
            | "markdown"
            | "conf"
            | "cfg"
            | "ini"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "log"
            | "sh"
            | "ps1"
            | "bat"
            | "rs"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "html"
            | "css"
            | "xml"
            | "csv"
            | "sql"
            | "py"
            | "go"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "java"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueueAction {
    Pause,
    Resume,
    Retry,
    Remove,
}

fn progress_bar(progress: f32, indeterminate: bool, color: Hsla) -> impl IntoElement {
    let clamped = progress.clamp(0.0, 100.0);
    let fill = if indeterminate {
        div()
            .absolute()
            .left(px(0.))
            .top(px(0.))
            .bottom(px(0.))
            .w(px(48.))
            .rounded_full()
            .bg(linear_gradient(
                90.,
                linear_color_stop(color.alpha(0.10), 0.0),
                linear_color_stop(color.alpha(0.85), 1.0),
            ))
            .into_any_element()
    } else {
        div()
            .absolute()
            .left(px(0.))
            .top(px(0.))
            .bottom(px(0.))
            .w(px(108.0 * clamped / 100.0))
            .rounded_full()
            .bg(color)
            .into_any_element()
    };

    div()
        .relative()
        .w(px(108.))
        .h(px(4.))
        .flex_shrink_0()
        .overflow_hidden()
        .rounded_full()
        .bg(theme::hover_tint_strong())
        .child(fill)
}

fn pane_state(icon: IconName, icon_color: Hsla, title: &'static str, description: String) -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .p(px(24.))
        .child(Icon::new(icon).size(theme::ICON_LG).text_color(icon_color))
        .child(
            div()
                .text_size(theme::TYPE_SECTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::solid(theme::TEXT))
                .child(title),
        )
        .child(
            div()
                .max_w(px(260.))
                .text_center()
                .text_size(theme::TYPE_META)
                .line_height(px(19.))
                .text_color(theme::solid(theme::SUBTLE))
                .child(description),
        )
}

fn pane_action(
    id: &'static str,
    label: &'static str,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    button::secondary(id, label)
        .mt(px(6.))
        .px(px(12.))
        .border_color(theme::colored(theme::FOAM, 0.28))
        .bg(theme::colored(theme::FOAM, 0.10))
        .text_color(theme::solid(theme::FOAM))
        .on_click(handler)
}
