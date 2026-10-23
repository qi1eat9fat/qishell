//! 终端字节流的传输契约：引擎发命令，传输在工作线程上跑、把事件发回来。

use std::sync::{Arc, mpsc};

use anyhow::Result;
use async_channel::Sender;

use super::TerminalSize;
use crate::connection::Latency;
use crate::connection::{ConnectionPrompt, ConnectionPromptReply};
use crate::host::HostLogin;

/// 每种终端传输都接受的命令。
pub enum TerminalTransportCommand {
    Write(Vec<u8>),
    Resize(TerminalSize),
    PromptReply {
        request_id: u64,
        reply: ConnectionPromptReply,
    },
    Shutdown,
}

impl std::fmt::Debug for TerminalTransportCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Write(bytes) => formatter.debug_tuple("Write").field(&bytes.len()).finish(),
            Self::Resize(size) => formatter.debug_tuple("Resize").field(size).finish(),
            Self::PromptReply { request_id, reply } => formatter
                .debug_struct("PromptReply")
                .field("request_id", request_id)
                .field("reply", &reply)
                .finish(),
            Self::Shutdown => formatter.write_str("Shutdown"),
        }
    }
}

/// 传输发出的事件。
#[derive(Clone, Debug, PartialEq)]
pub enum TerminalTransportEvent {
    Started,
    Output(Vec<u8>),
    Prompt(ConnectionPrompt),
    /// 活动连接的一次往返耗时，只有网络传输会发。
    Latency(Latency),
    Exited {
        code: u32,
        signal: Option<String>,
    },
    Failed(String),
}

/// 一个运行中的传输。实现可以阻塞：引擎总在专用工作线程上调用它。
pub trait TerminalTransport: Send + 'static {
    fn run(
        self: Box<Self>,
        initial_size: TerminalSize,
        commands: mpsc::Receiver<TerminalTransportCommand>,
        events: Sender<TerminalTransportEvent>,
    ) -> Result<()>;
}

/// 每次启动与重连都要一个新的、一次性的传输。
pub trait TerminalTransportFactory: Send + Sync + 'static {
    fn create(&self) -> Box<dyn TerminalTransport>;
}

/// 按主机当前保存的登录快照创建远程传输工厂。
pub trait RemoteTerminalTransportProvider: Send + Sync + 'static {
    fn factory_for(&self, login: &HostLogin) -> SharedTerminalTransportFactory;
}

pub type SharedRemoteTerminalTransportProvider = Arc<dyn RemoteTerminalTransportProvider>;

/// 每台主机都用同一个工厂，给测试与嵌入用。
pub struct FixedRemoteTerminalTransportProvider {
    factory: SharedTerminalTransportFactory,
}

impl FixedRemoteTerminalTransportProvider {
    pub fn new(factory: SharedTerminalTransportFactory) -> Self {
        Self { factory }
    }
}

impl RemoteTerminalTransportProvider for FixedRemoteTerminalTransportProvider {
    fn factory_for(&self, _: &HostLogin) -> SharedTerminalTransportFactory {
        self.factory.clone()
    }
}

pub type SharedTerminalTransportFactory = Arc<dyn TerminalTransportFactory>;
