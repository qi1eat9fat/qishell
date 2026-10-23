//! 临时联调程序：用真实服务器验证登录与终端传输。
//!
//! 凭据从环境变量读取：
//! `QISHELL_SSH_ADDRESS` / `QISHELL_SSH_PORT` / `QISHELL_SSH_USER` / `QISHELL_SSH_PASSWORD`

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use qishell::connection::{
    ConnectionPromptKind, ConnectionPromptReply, ConnectionSecret, ConnectionTester, LoginTest,
};
use qishell::host::{HostId, HostLogin, LoginMethod};
use qishell::secrets::{InMemorySecretStore, SecretStore};
use qishell::ssh::{SshConnectionTester, SshConnector, SshTerminalTransportProvider};
use qishell::terminal::{
    RemoteTerminalTransportProvider, TerminalSize, TerminalTransportCommand, TerminalTransportEvent,
};

fn main() -> anyhow::Result<()> {
    let address = std::env::var("QISHELL_SSH_ADDRESS")?;
    let port: u16 = std::env::var("QISHELL_SSH_PORT")?.parse()?;
    let username = std::env::var("QISHELL_SSH_USER")?;
    let password = std::env::var("QISHELL_SSH_PASSWORD")?;

    let known_hosts = std::env::temp_dir().join("qishell-live-known_hosts");
    let _ = std::fs::remove_file(&known_hosts);

    let secrets = Arc::new(InMemorySecretStore::new());
    let login = HostLogin {
        host: HostId(1),
        name: "live".into(),
        address: address.clone(),
        port,
        username: username.clone(),
        method: LoginMethod::Password,
        key_path: None,
        key_id: None,
    };
    secrets.set(&login.password_secret(), &password)?;
    let connector = SshConnector::new(&known_hosts, secrets);

    // 1. 「测试连接」同款登录器。
    let tester = SshConnectionTester::new(connector.clone());
    let request = LoginTest {
        host: HostId(1),
        name: "live".into(),
        address: address.clone(),
        port,
        username: username.clone(),
        method: LoginMethod::Password,
        key_path: None,
        password: Some(password.clone()),
        passphrase: None,
    };
    match tester.test(request, Box::new(|_| true)) {
        Ok(()) => println!("TEST_CONNECTION_OK"),
        Err(reason) => {
            println!("TEST_CONNECTION_FAILED: {reason}");
            return Err(anyhow::anyhow!(reason));
        }
    }

    // 2. 完整终端：pty、shell、命令回显。
    let provider = SshTerminalTransportProvider::with_connector(connector);
    let factory = provider.factory_for(&login);
    let transport = factory.create();
    let (commands, command_rx) = mpsc::channel();
    let (events, event_rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        if let Err(error) = transport.run(TerminalSize::DEFAULT, command_rx, events.clone()) {
            let _ = events.send_blocking(TerminalTransportEvent::Failed(error.to_string()));
        }
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut output = Vec::new();
    let mut sent_command = false;
    let mut seen_started = false;
    while std::time::Instant::now() < deadline {
        let Ok(event) = event_rx.recv_blocking() else {
            break;
        };
        match event {
            TerminalTransportEvent::Started => {
                seen_started = true;
                println!("TERMINAL_STARTED");
                commands.send(TerminalTransportCommand::Write(
                    b"printf 'QISHELL_LIVE_OK\\n'; uname -a; id\r".to_vec(),
                ))?;
                sent_command = true;
            }
            TerminalTransportEvent::Prompt(prompt) => {
                let trusted = matches!(prompt.kind(), ConnectionPromptKind::UnknownHost(_));
                commands.send(TerminalTransportCommand::PromptReply {
                    request_id: prompt.request_id(),
                    reply: if trusted {
                        ConnectionPromptReply::TrustAndSave
                    } else {
                        ConnectionPromptReply::Answers(vec![ConnectionSecret::new(
                            password.clone(),
                        )])
                    },
                })?;
            }
            TerminalTransportEvent::Output(bytes) => {
                output.extend_from_slice(&bytes);
                let text = String::from_utf8_lossy(&output);
                if text.contains("QISHELL_LIVE_OK") && text.contains("Linux") {
                    println!("TERMINAL_OUTPUT_OK");
                    println!("--- terminal output ---");
                    println!("{}", text.trim_end());
                    println!("--- end ---");
                    break;
                }
            }
            TerminalTransportEvent::Latency(_) => {}
            TerminalTransportEvent::Exited { code, signal } => {
                println!("TERMINAL_EXITED: code={code} signal={signal:?}");
                break;
            }
            TerminalTransportEvent::Failed(error) => {
                println!("TERMINAL_FAILED: {error}");
                return Err(anyhow::anyhow!(error));
            }
        }
    }

    let _ = commands.send(TerminalTransportCommand::Shutdown);
    let _ = std::fs::remove_file(&known_hosts);
    if !seen_started {
        return Err(anyhow::anyhow!("终端没有启动"));
    }
    if !sent_command {
        return Err(anyhow::anyhow!("没有发出测试命令"));
    }
    if !String::from_utf8_lossy(&output).contains("QISHELL_LIVE_OK") {
        return Err(anyhow::anyhow!("终端没有收到命令输出"));
    }
    println!("ALL_OK");
    Ok(())
}
