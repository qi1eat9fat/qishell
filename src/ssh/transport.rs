use super::{
    connection::{SshConnectionConfig, SshConnector, SshPrompts},
    latency::{LATENCY_INTERVAL, round_trip},
    run_terminal_worker,
    tester::describe_login_error,
};
use crate::{
    connection::Latency,
    host::HostLogin,
    terminal::{
        RemoteTerminalTransportProvider, SharedTerminalTransportFactory, TerminalSize,
        TerminalTransport, TerminalTransportCommand, TerminalTransportEvent,
        TerminalTransportFactory,
    },
};
use anyhow::{Result, anyhow, bail};
use async_channel::Sender;
use russh::ChannelMsg;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, mpsc},
    time::Duration,
};
use tokio::sync::mpsc as tokio_mpsc;
use tokio::time::MissedTickBehavior;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Production remote-terminal adapter for the shared SSH connector.
pub struct SshTerminalTransportProvider {
    connector: SshConnector,
}
impl SshTerminalTransportProvider {
    pub fn with_connector(connector: SshConnector) -> Self {
        Self { connector }
    }
}
impl RemoteTerminalTransportProvider for SshTerminalTransportProvider {
    fn factory_for(&self, login: &HostLogin) -> SharedTerminalTransportFactory {
        Arc::new(SshTerminalTransport {
            config: SshConnectionConfig::from(login),
            connector: self.connector.clone(),
        })
    }
}

/// One host's connection settings. It is its own factory: every launch
/// and restart runs a fresh copy.
#[derive(Clone)]
struct SshTerminalTransport {
    config: SshConnectionConfig,
    connector: SshConnector,
}

impl TerminalTransportFactory for SshTerminalTransport {
    fn create(&self) -> Box<dyn TerminalTransport> {
        Box::new(self.clone())
    }
}

impl TerminalTransport for SshTerminalTransport {
    fn run(
        self: Box<Self>,
        initial_size: TerminalSize,
        commands: mpsc::Receiver<TerminalTransportCommand>,
        events: Sender<TerminalTransportEvent>,
    ) -> Result<()> {
        run_terminal_worker(commands, events.clone(), |broker, io_rx| {
            self.run_async(initial_size, broker, io_rx, events)
        })
    }
}

impl SshTerminalTransport {
    async fn run_async(
        &self,
        initial_size: TerminalSize,
        broker: Arc<SshPrompts>,
        mut io_rx: tokio_mpsc::UnboundedReceiver<TerminalTransportCommand>,
        events: Sender<TerminalTransportEvent>,
    ) -> Result<()> {
        // Said in the terminal: down to the cause, which says what to fix.
        let (handle, _fingerprint) = self
            .connector
            .connect(&self.config, broker.clone())
            .await
            .map_err(|error| anyhow!(describe_login_error(&error)))?;
        let mut shutdown = broker.shutdown_receiver();
        let mut channel = tokio::select! {
            result = handle.channel_open_session() => {
                result.map_err(|_| anyhow!("无法创建 SSH 会话通道".to_string()))?
            }
            _ = shutdown.changed() => bail!("连接已取消".to_string()),
        };
        tokio::select! {
            result = channel.request_pty(
                true,
                "xterm-256color",
                dimension(initial_size.columns()),
                dimension(initial_size.rows()),
                pixel_dimension(initial_size.columns(), initial_size.cell_width()),
                pixel_dimension(initial_size.rows(), initial_size.cell_height()),
                &[],
            ) => result.map_err(|_| anyhow!("服务器拒绝创建终端".to_string()))?,
            _ = shutdown.changed() => bail!("连接已取消".to_string()),
        }
        tokio::select! {
            result = channel.request_shell(true) => {
                result.map_err(|_| anyhow!("服务器拒绝启动 Shell".to_string()))?
            }
            _ = shutdown.changed() => bail!("连接已取消".to_string()),
        }
        events
            .send(TerminalTransportEvent::Started)
            .await
            .map_err(|_| anyhow!("终端标签页已关闭".to_string()))?;

        // Round trips are measured with `keepalive@openssh.com`, which every
        // server answers (a refusal is an answer too), so this is the SSH-level
        // delay a keystroke's echo sees. The ping queues behind output on the
        // same connection: during a flood of output the reading rises, which is
        // the responsiveness the user actually gets. It doubles as a keepalive.
        // The first tick fires at once, so a reading shows up right after
        // connecting; a new ping only goes out once the last one has settled.
        let mut latency_ticker = tokio::time::interval(LATENCY_INTERVAL);
        latency_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut ping: Option<Pin<Box<dyn Future<Output = Latency> + Send + '_>>> = None;

        let mut exit_code = 0;
        let mut exit_signal = None;
        loop {
            tokio::select! {
                command = io_rx.recv() => match command {
                    Some(TerminalTransportCommand::Write(bytes)) => {
                        channel.data_bytes(bytes).await.map_err(|_| anyhow!("向远程终端写入失败".to_string()))?;
                    }
                    Some(TerminalTransportCommand::Resize(size)) => {
                        channel.window_change(
                            dimension(size.columns()), dimension(size.rows()),
                            pixel_dimension(size.columns(), size.cell_width()),
                            pixel_dimension(size.rows(), size.cell_height()),
                        ).await.map_err(|_| anyhow!("调整远程终端尺寸失败".to_string()))?;
                    }
                    Some(TerminalTransportCommand::Shutdown) | None => {
                        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
                            let _ = channel.eof().await;
                            let _ = channel.close().await;
                            let _ = handle.disconnect(
                                russh::Disconnect::ByApplication,
                                "Qi Shell closed the terminal",
                                "zh-CN",
                            ).await;
                        }).await;
                        return Ok(());
                    }
                    Some(TerminalTransportCommand::PromptReply { .. }) => {}
                },
                _ = latency_ticker.tick(), if ping.is_none() => {
                    ping = Some(Box::pin(round_trip(&handle)));
                },
                latency = async { ping.as_mut().expect("guarded").await }, if ping.is_some() => {
                    ping = None;
                    let _ = events.send(TerminalTransportEvent::Latency(latency)).await;
                },
                message = channel.wait() => match message {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if events.send(TerminalTransportEvent::Output(data.to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => exit_code = exit_status,
                    Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                        exit_signal = Some(format!("{signal_name:?}"));
                    }
                    Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
        }
        let _ = events
            .send(TerminalTransportEvent::Exited {
                code: exit_code,
                signal: exit_signal,
            })
            .await;
        Ok(())
    }
}

fn dimension(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

fn pixel_dimension(cells: usize, cell_size: u16) -> u32 {
    cells
        .saturating_mul(usize::from(cell_size))
        .min(u32::MAX as usize) as u32
}
