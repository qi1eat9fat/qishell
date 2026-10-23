//! 活动连接的往返耗时，给状态栏显示。

use std::time::{Duration, Instant};

use crate::connection::Latency;

use super::SshHandle;

/// 多久量一次。
pub const LATENCY_INTERVAL: Duration = Duration::from_secs(5);
/// 一次 ping 等多久算超时。
const LATENCY_TIMEOUT: Duration = Duration::from_secs(5);

/// 用 `keepalive@openssh.com` 量一次往返；服务器拒绝也算回答。
///
/// ping 排在连接上已有数据后面，输出刷屏或传文件时读数会升高——那正是
/// 用户实际感受到的响应速度。
pub async fn round_trip(handle: &SshHandle) -> Latency {
    let sent = Instant::now();
    match tokio::time::timeout(LATENCY_TIMEOUT, handle.send_ping()).await {
        Ok(Ok(())) => Latency::Measured(sent.elapsed()),
        _ => Latency::TimedOut,
    }
}
