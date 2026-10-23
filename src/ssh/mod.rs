//! SSH 连接服务：终端与测试连接共用。

mod connection;
mod latency;
mod tester;
mod transport;
pub mod vault;
mod worker;

pub use connection::{
    MissingCredential, SshConnectionConfig, SshConnector, SshHandle, SshPrompts, is_network_error,
};
pub use latency::{LATENCY_INTERVAL, round_trip};
pub use tester::{SshConnectionTester, describe_login_error};
pub use transport::SshTerminalTransportProvider;
pub use vault::{KnownHost, StoredKey};
pub(crate) use worker::run_terminal_worker;
