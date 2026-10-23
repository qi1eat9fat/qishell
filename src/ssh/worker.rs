//! What a remote terminal's transport runs on, an SSH terminal's and a
//! Telnet one's alike: a tokio runtime of its own on the transport's
//! thread, a thread carrying the engine's commands into it, and the broker
//! that puts a connection's questions in the terminal and its answers back.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_channel::Sender;
use tokio::sync::{mpsc as tokio_mpsc, watch};

use super::SshPrompts;
use crate::terminal::{TerminalTransportCommand, TerminalTransportEvent};

/// Run `body` until it returns, on this thread. It gets the broker and the
/// commands other than prompt answers, which go to the broker; `Shutdown`
/// also cancels whatever the broker is waiting for, and comes through last.
pub(crate) fn run_terminal_worker<F, Fut>(
    commands: mpsc::Receiver<TerminalTransportCommand>,
    events: Sender<TerminalTransportEvent>,
    body: F,
) -> Result<()>
where
    F: FnOnce(Arc<SshPrompts>, tokio_mpsc::UnboundedReceiver<TerminalTransportCommand>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("无法启动 SSH 运行时".to_string())?;
    let (command_tx, mut bridge_commands) = tokio_mpsc::unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let bridge_stop = stop.clone();
    let bridge = thread::Builder::new()
        .name("qishell-ssh-command-bridge".into())
        .spawn(move || {
            while !bridge_stop.load(Ordering::Acquire) {
                match commands.recv_timeout(Duration::from_millis(20)) {
                    Ok(command) => {
                        if command_tx.send(command).is_err() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .context("无法启动 SSH 命令桥接线程".to_string())?;

    let result = runtime.block_on(async move {
        let (io_tx, io_rx) = tokio_mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let broker = Arc::new(SshPrompts::new(
            Arc::new(move |prompt| {
                events
                    .try_send(TerminalTransportEvent::Prompt(prompt))
                    .is_ok()
            }),
            shutdown_rx,
        ));
        let router_broker = broker.clone();
        let router = tokio::spawn(async move {
            while let Some(command) = bridge_commands.recv().await {
                match command {
                    TerminalTransportCommand::PromptReply { request_id, reply } => {
                        router_broker.respond(request_id, reply);
                    }
                    TerminalTransportCommand::Shutdown => {
                        let _ = shutdown_tx.send(true);
                        router_broker.cancel_all();
                        let _ = io_tx.send(TerminalTransportCommand::Shutdown);
                        break;
                    }
                    command => {
                        if io_tx.send(command).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let result = body(broker, io_rx).await;
        router.abort();
        result
    });
    stop.store(true, Ordering::Release);
    let _ = bridge.join();
    result
}
