//! Inbound SSH on the reverse tunnel (PTY + connect-app).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use russh::keys::ssh_key::Certificate;
use russh::keys::PublicKey;
use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Pty};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use crate::identity::Identity;
use crate::plane::App;
use crate::shell::Shell;

use super::{ssh_preferred, AppRouter, APP_SUBSYSTEM};

pub(super) async fn run_ssh_server<S>(
    stream: S,
    identity: Arc<Identity>,
    app_tx: mpsc::Sender<App>,
    router: AppRouter,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let host_key = identity.ssh_private();
    let mut preferred = ssh_preferred();
    let (host_cert, host_cert_algo) = match identity.ssh_cert_raw() {
        Ok((algo, blob)) => {
            if let Ok(a) = russh::keys::Algorithm::new(&algo) {
                preferred.key = std::borrow::Cow::Owned(vec![a.clone()]);
                tracing::info!(%algo, blob = blob.len(), "ssh host cert");
                (Some(blob), Some(a))
            } else {
                (Some(blob), None)
            }
        }
        Err(_) => (None, None),
    };
    let config = russh::server::Config {
        inactivity_timeout: None,
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        keys: vec![host_key],
        host_cert,
        host_cert_algo,
        methods: MethodSet::from([MethodKind::PublicKey].as_slice()),
        preferred,
        ..Default::default()
    };
    let mut server = NodeServer {
        app_tx,
        router,
    };
    let handler = server.new_client(None);
    russh::server::run_stream(Arc::new(config), stream, handler)
        .await
        .map_err(|e| anyhow!("ssh session: {e:?}"))?
        .await
        .map_err(|e| anyhow!("ssh session join: {e:?}"))?;
    Ok(())
}

struct NodeServer {
    app_tx: mpsc::Sender<App>,
    router: AppRouter,
}

impl russh::server::Server for NodeServer {
    type Handler = NodeHandler;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        NodeHandler {
            app_tx: self.app_tx.clone(),
            router: self.router.clone(),
            user: None,
            shell_in: None,
            app_ch: None,
        }
    }
}

struct NodeHandler {
    app_tx: mpsc::Sender<App>,
    router: AppRouter,
    user: Option<String>,
    shell_in: Option<mpsc::UnboundedSender<Vec<u8>>>,
    app_ch: Option<String>,
}

impl russh::server::Handler for NodeHandler {
    type Error = russh::Error;

    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        certificate: &Certificate,
    ) -> Result<Auth, Self::Error> {
        if user.is_empty() {
            return Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        // Proxy already issued this user cert. Host join only stores the *host* CA,
        // while tsh presents a *user* CA cert — accept named logins.
        self.user = Some(user.to_string());
        tracing::info!(%user, principals = ?certificate.valid_principals(), "user cert ok");
        Ok(Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        _public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        // Teleport user certs are offered as publickey+cert. If russh already
        // verified the signature, accept named users so tsh can log in.
        if user.is_empty() || user.starts_with('-') {
            return Ok(Auth::Reject {
                proceed_with_methods: Some(MethodSet::from([MethodKind::PublicKey].as_slice())),
                partial_success: false,
            });
        }
        self.user = Some(user.to_string());
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        tracing::info!(user = ?self.user, "session channel");
        Ok(true)
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        _variable_value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::debug!(%variable_name, "env");
        session.channel_success(channel)?;
        Ok(())
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single_connection: bool,
        _x11_auth_protocol: &str,
        _x11_auth_cookie: &str,
        _x11_screen_number: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn agent_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        // Access role has forward_agent: true. Accept so Teleport/web continues;
        // we do not actually proxy an agent socket.
        session.channel_success(channel)?;
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!("pty");
        self.spawn_shell(channel, session, col_width as u16, row_height as u16);
        session.channel_success(channel)?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(tx) = &self.shell_in {
            let _ = tx.send(format!("\x00resize:{col_width}:{row_height}").into_bytes());
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.shell_in.is_none() {
            self.spawn_shell(channel, session, 80, 24);
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data);
        tracing::info!(%cmd, "exec");
        if cmd.trim() == APP_SUBSYSTEM {
            self.start_app(channel, session);
            session.channel_success(channel)?;
            return Ok(());
        }
        self.spawn_exec(channel, session, cmd.into_owned());
        session.channel_success(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == APP_SUBSYSTEM {
            self.start_app(channel, session);
            session.channel_success(channel)?;
        } else {
            session.channel_failure(channel)?;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(tx) = &self.shell_in {
            let _ = tx.send(data.to_vec());
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shell_in = None;
        if let Some(ch) = self.app_ch.take() {
            let router = self.router.clone();
            tokio::spawn(async move {
                router.unbind(&ch).await;
            });
        }
        Ok(())
    }
}

impl NodeHandler {
    fn spawn_shell(&mut self, channel: ChannelId, session: &mut Session, cols: u16, rows: u16) {
        if self.shell_in.is_some() {
            return;
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        self.shell_in = Some(tx);
        let handle = session.handle();
        tokio::spawn(async move {
            let (mut sh, mut out) = match Shell::spawn() {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("pty: {e:#}");
                    let _ = handle.close(channel).await;
                    return;
                }
            };
            let _ = sh.resize(cols, rows);
            loop {
                tokio::select! {
                    chunk = rx.recv() => {
                        let Some(chunk) = chunk else { break };
                        if let Some(rest) = chunk.strip_prefix(b"\x00resize:") {
                            if let Ok(s) = std::str::from_utf8(rest) {
                                let mut it = s.split(':');
                                let c = it.next().and_then(|x| x.parse().ok()).unwrap_or(80);
                                let r = it.next().and_then(|x| x.parse().ok()).unwrap_or(24);
                                let _ = sh.resize(c, r);
                            }
                            continue;
                        }
                        if sh.write(&chunk).await.is_err() { break; }
                    }
                    chunk = out.recv() => {
                        let Some(chunk) = chunk else { break };
                        if handle.data(channel, chunk.into()).await.is_err() { break; }
                    }
                }
            }
            let _ = handle.exit_status_request(channel, 0).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
    }

    fn spawn_exec(&mut self, channel: ChannelId, session: &mut Session, cmd: String) {
        let handle = session.handle();
        tokio::spawn(async move {
            let mut child = match tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = handle
                        .data(channel, format!("{e}\n").into_bytes().into())
                        .await;
                    let _ = handle.exit_status_request(channel, 1).await;
                    let _ = handle.close(channel).await;
                    return;
                }
            };
            let mut stdout = child.stdout.take();
            let mut stderr = child.stderr.take();
            let mut out_buf = vec![0u8; 4096];
            let mut err_buf = vec![0u8; 4096];
            loop {
                tokio::select! {
                    n = async {
                        if let Some(s) = stdout.as_mut() { s.read(&mut out_buf).await } else { Ok(0) }
                    } => {
                        match n {
                            Ok(0) => break,
                            Err(_) => break,
                            Ok(n) => {
                                if handle.data(channel, out_buf[..n].to_vec().into()).await.is_err() { break; }
                            }
                        }
                    }
                    n = async {
                        if let Some(s) = stderr.as_mut() { s.read(&mut err_buf).await } else { Ok(0) }
                    } => {
                        match n {
                            Ok(0) | Err(_) => {}
                            Ok(n) => {
                                let _ = handle.extended_data(channel, 1, err_buf[..n].to_vec().into()).await;
                            }
                        }
                    }
                }
            }
            let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(1) as u32;
            let _ = handle.exit_status_request(channel, code).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
    }

    fn start_app(&mut self, channel: ChannelId, session: &mut Session) {
        if self.app_ch.is_some() {
            return;
        }
        let user = self.user.clone().unwrap_or_else(|| "user".into());
        let ch_name = format!("{user}:app");
        self.app_ch = Some(ch_name.clone());
        let (bytes_tx, mut bytes_rx) = mpsc::channel::<Vec<u8>>(64);
        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        self.shell_in = Some(in_tx);
        let app_tx = self.app_tx.clone();
        let handle = session.handle();
        let router = self.router.clone();
        tokio::spawn(async move {
            router.bind(ch_name.clone(), bytes_tx).await;
            let mut buf = Vec::new();
            loop {
                tokio::select! {
                    chunk = in_rx.recv() => {
                        let Some(chunk) = chunk else { break };
                        buf.extend_from_slice(&chunk);
                        while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                            let mut line: Vec<u8> = buf.drain(..=i).collect();
                            if line.last() == Some(&b'\n') {
                                line.pop();
                            }
                            if line.last() == Some(&b'\r') {
                                line.pop();
                            }
                            if line.is_empty() {
                                continue;
                            }
                            if app_tx.send(App {
                                src: user.clone(),
                                dst: String::new(),
                                data: line,
                                channel: ch_name.clone(),
                            }).await.is_err() {
                                break;
                            }
                        }
                    }
                    line = bytes_rx.recv() => {
                        let Some(line) = line else { break };
                        if handle.data(channel, line.into()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            router.unbind(&ch_name).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
    }
}

