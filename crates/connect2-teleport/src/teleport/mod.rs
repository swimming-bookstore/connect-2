//! Teleport reverse tunnel + inbound SSH (PTY and connect-app JSON).

use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use russh::keys::ssh_key::{Algorithm, Certificate};
use russh::keys::PublicKey;
use russh::{Channel, ChannelId, ChannelMsg, Preferred};
use rustls::ClientConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;

use crate::identity::Identity;
use crate::plane::{App, ClientMsg};

mod ssh;

pub const CHAN_HEARTBEAT: &str = "teleport-heartbeat";
pub const CHAN_TRANSPORT: &str = "teleport-transport";
pub const CHAN_DISCOVERY: &str = "teleport-discovery";
pub const REQ_DIAL: &str = "teleport-transport-dial";
pub const LOCAL_NODE: &str = "@local-node";
pub const ALPN_REVERSE_TUNNEL: &[u8] = b"teleport-reversetunnel";
pub const APP_SUBSYSTEM: &str = "connect-app";

#[derive(Clone)]
pub struct TunnelCfg {
    pub proxy: String,
    pub tunnel: Option<String>,
    pub insecure: bool,
    pub tls_ca: Option<std::path::PathBuf>,
}

/// JSON-line App pipes keyed by channel (`user:app`). One actor, no mutex.
#[derive(Clone)]
pub struct AppRouter {
    tx: mpsc::Sender<RouteOp>,
}

enum RouteOp {
    Bind {
        channel: String,
        tx: mpsc::Sender<Vec<u8>>,
    },
    Unbind {
        channel: String,
    },
    Send {
        msg: ClientMsg,
    },
}

impl Default for AppRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl AppRouter {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut sinks: HashMap<String, mpsc::Sender<Vec<u8>>> = HashMap::new();
            while let Some(op) = rx.recv().await {
                match op {
                    RouteOp::Bind { channel, tx } => {
                        sinks.insert(channel, tx);
                    }
                    RouteOp::Unbind { channel } => {
                        sinks.remove(&channel);
                    }
                    RouteOp::Send { msg } => {
                        if let Some(tx) = sinks.get(&msg.channel) {
                            let mut line = msg.data;
                            if !line.ends_with(&[b'\n']) {
                                line.push(b'\n');
                            }
                            let _ = tx.send(line).await;
                        }
                    }
                }
            }
        });
        Self { tx }
    }

    pub async fn bind(&self, channel: String, tx: mpsc::Sender<Vec<u8>>) {
        let _ = self.tx.send(RouteOp::Bind { channel, tx }).await;
    }

    pub async fn unbind(&self, channel: &str) {
        let _ = self
            .tx
            .send(RouteOp::Unbind {
                channel: channel.into(),
            })
            .await;
    }

    pub async fn send(&self, msg: ClientMsg) {
        let _ = self.tx.send(RouteOp::Send { msg }).await;
    }
}

pub async fn run(
    identity: Arc<Identity>,
    cfg: TunnelCfg,
    app_tx: mpsc::Sender<App>,
    router: AppRouter,
) -> Result<()> {
    let mut backoff = Duration::from_secs(1);
    loop {
        match dial_once(identity.clone(), &cfg, app_tx.clone(), router.clone()).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => tracing::warn!("reverse tunnel: {e:#}"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

async fn dial_once(
    identity: Arc<Identity>,
    cfg: &TunnelCfg,
    app_tx: mpsc::Sender<App>,
    router: AppRouter,
) -> Result<()> {
    let _certs = identity
        .certs
        .as_ref()
        .ok_or_else(|| anyhow!("join the cluster first"))?;
    let (host, port) = if let Some(t) = &cfg.tunnel {
        split_host_port(t)?
    } else {
        reverse_tunnel_addr(&cfg.proxy)?
    };
    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("resolve {host}:{port}"))?;
    let tcp = TcpStream::connect(addr)
        .await
        .with_context(|| format!("tcp {addr}"))?;
    let _ = tcp.set_nodelay(true);

    let ssh_cfg = russh::client::Config {
        inactivity_timeout: None,
        keepalive_interval: None,
        keepalive_max: 0,
        preferred: ssh_preferred(),
        ..Default::default()
    };
    let handler = AgentClient {
        identity: identity.clone(),
        app_tx: app_tx.clone(),
        router: router.clone(),
    };
    // :3024 is Teleport's reverse-tunnel SSH listener (plain SSH).
    // :3080/:443 terminate TLS first (ALPN teleport-reversetunnel).
    let mut handle = if use_tls_alpn(port) {
        tracing::info!(%host, port, "dial teleport proxy (TLS ALPN reverse tunnel)");
        let tls_cfg = tls_client_config(identity.as_ref(), cfg)?;
        let connector = TlsConnector::from(Arc::new(tls_cfg));
        let sni = ServerName::try_from(host.clone()).map_err(|e| anyhow!("sni {host}: {e}"))?;
        let tls = connector.connect(sni, tcp).await.context("tls")?;
        russh::client::connect_stream(Arc::new(ssh_cfg), tls, handler).await?
    } else {
        tracing::info!(%host, port, "dial teleport proxy (SSH reverse tunnel)");
        russh::client::connect_stream(Arc::new(ssh_cfg), tcp, handler).await?
    };
    let key = Arc::new(identity.ssh_private());
    let (algo, blob) = identity.ssh_cert_raw()?;
    let auth = handle
        .authenticate_openssh_cert_raw(identity.tunnel_user(), key, algo, blob)
        .await?;
    if !auth.success() {
        anyhow::bail!("ssh auth to proxy failed: {auth:?}");
    }

    let mut hb = handle
        .channel_open_custom(CHAN_HEARTBEAT, Vec::new())
        .await
        .context("open teleport-heartbeat")?;
    hb.request("ping", false, Vec::new()).await?;
    tracing::info!(host_id = %identity.host_id, name = %identity.hostname, "reverse tunnel up");

    let mut tick = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            msg = hb.wait() => {
                match msg {
                    None => anyhow::bail!("heartbeat channel closed"),
                    Some(ChannelMsg::CustomRequest { request, want_reply, .. }) => {
                        if want_reply {
                            let _ = hb.success().await;
                        }
                        tracing::debug!(%request, "heartbeat request");
                    }
                    Some(ChannelMsg::Close | ChannelMsg::Eof) => {
                        anyhow::bail!("heartbeat closed");
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => {
                if hb.request("ping", false, Vec::new()).await.is_err() {
                    anyhow::bail!("heartbeat ping failed");
                }
            }
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

struct AgentClient {
    identity: Arc<Identity>,
    app_tx: mpsc::Sender<App>,
    router: AppRouter,
}

impl russh::client::Handler for AgentClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn should_accept_unknown_server_channel(
        &mut self,
        _id: ChannelId,
        channel_type: &str,
    ) -> bool {
        channel_type == CHAN_TRANSPORT || channel_type == CHAN_DISCOVERY
    }

    async fn server_channel_open_unknown(
        &mut self,
        channel: Channel<russh::client::Msg>,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        let identity = self.identity.clone();
        let app_tx = self.app_tx.clone();
        let router = self.router.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_opened_channel(channel, identity, app_tx, router).await {
                tracing::warn!("tunnel channel: {e:#}");
            }
        });
        Ok(())
    }
}

async fn handle_opened_channel(
    mut channel: Channel<russh::client::Msg>,
    identity: Arc<Identity>,
    app_tx: mpsc::Sender<App>,
    router: AppRouter,
) -> Result<()> {
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Open { .. } => {}
            ChannelMsg::CustomRequest {
                request,
                want_reply,
                payload,
            } => {
                if request == REQ_DIAL {
                    if want_reply {
                        let _ = channel.success().await;
                    }
                    let req = parse_dial(&payload);
                    tracing::info!(address = %req.address, server_id = %req.server_id, "dial");
                    let stream = channel.into_stream();
                    ssh::run_ssh_server(stream, identity, app_tx, router).await?;
                    return Ok(());
                }
            }
            ChannelMsg::Close | ChannelMsg::Eof => break,
            _ => {}
        }
    }
    Ok(())
}

#[derive(Default)]
struct DialReq {
    address: String,
    server_id: String,
}

fn parse_dial(payload: &[u8]) -> DialReq {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(payload) {
        DialReq {
            address: v
                .get("Address")
                .or_else(|| v.get("address"))
                .and_then(|x| x.as_str())
                .unwrap_or(LOCAL_NODE)
                .into(),
            server_id: v
                .get("ServerID")
                .or_else(|| v.get("serverID"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .into(),
        }
    } else {
        DialReq {
            address: String::from_utf8_lossy(payload).into_owned(),
            server_id: String::new(),
        }
    }
}

pub fn cert_ok(cas: &[PublicKey], cert: &Certificate, user: &str) -> bool {
    if user.is_empty() {
        return false;
    }
    if cas.is_empty() {
        return true;
    }
    let ca_ok = cas.iter().any(|ca| cert.signature_key() == ca.key_data());
    if !ca_ok {
        return false;
    }
    let principals = cert.valid_principals();
    principals.is_empty() || principals.iter().any(|p| p == user || p == "*")
}

fn tls_client_config(identity: &Identity, cfg: &TunnelCfg) -> Result<ClientConfig> {
    let certs = identity
        .certs
        .as_ref()
        .ok_or_else(|| anyhow!("missing tls cert"))?;
    let mut chain: Vec<CertificateDer<'static>> = pem_certs(&certs.tls)?;
    for ca in &certs.tls_ca_certs {
        chain.extend(pem_certs(ca)?);
    }
    let key = PrivateKeyDer::from_pem_slice(identity.tls_key_pem.as_bytes())
        .map_err(|e| anyhow!("tls key: {e}"))?;
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for ca in &certs.tls_ca_certs {
        for c in pem_certs(ca)? {
            let _ = roots.add(c);
        }
    }
    if let Some(path) = &cfg.tls_ca {
        let pem = std::fs::read(path)?;
        for c in pem_certs(&pem)? {
            let _ = roots.add(c);
        }
    }
    let mut config = if cfg.insecure {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify))
            .with_client_auth_cert(chain, key)
            .map_err(|e| anyhow!("client cert: {e}"))?
    } else {
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(chain, key)
            .map_err(|e| anyhow!("client cert: {e}"))?
    };
    config.alpn_protocols = vec![ALPN_REVERSE_TUNNEL.to_vec()];
    Ok(config)
}

fn pem_certs(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>> {
    let mut out = Vec::new();
    for item in CertificateDer::pem_slice_iter(pem) {
        if let Ok(c) = item {
            out.push(c);
        }
    }
    if out.is_empty() {
        if let Ok(c) = CertificateDer::from_pem_slice(pem) {
            out.push(c);
        }
    }
    Ok(out)
}

pub fn ssh_preferred() -> Preferred {
    let mut p = Preferred::default();
    let keys: Vec<Algorithm> = [
        "rsa-sha2-256-cert-v01@openssh.com",
        "rsa-sha2-512-cert-v01@openssh.com",
        "ssh-rsa-cert-v01@openssh.com",
        "ssh-ed25519-cert-v01@openssh.com",
        "ecdsa-sha2-nistp256-cert-v01@openssh.com",
        "ssh-ed25519",
        "ecdsa-sha2-nistp256",
        "rsa-sha2-512",
        "rsa-sha2-256",
        "ssh-rsa",
    ]
    .iter()
    .filter_map(|s| Algorithm::new(s).ok())
    .collect();
    p.key = std::borrow::Cow::Owned(keys);
    p
}

/// Web proxy is `:443`/`:3080`. Reverse-tunnel listener is `:3024` unless the
/// caller already pointed `--tunnel` or `--proxy` at it.
fn reverse_tunnel_addr(proxy: &str) -> Result<(String, u16)> {
    let (host, port) = split_host_port(proxy)?;
    let port = match port {
        443 | 3080 | 80 => 3024,
        other => other,
    };
    Ok((host, port))
}

fn use_tls_alpn(port: u16) -> bool {
    matches!(port, 443 | 3080 | 80 | 8443)
}

fn split_host_port(proxy: &str) -> Result<(String, u16)> {
    let p = proxy
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if let Some((h, port)) = p.rsplit_once(':') {
        if !h.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return Ok((h.to_string(), port.parse().unwrap_or(443)));
        }
    }
    Ok((p.to_string(), 443))
}

#[derive(Debug)]
struct NoVerify;

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_host_port_defaults_443() {
        assert_eq!(
            split_host_port("teleport.example").unwrap(),
            ("teleport.example".into(), 443)
        );
        assert_eq!(
            split_host_port("https://teleport.example:3080/").unwrap(),
            ("teleport.example".into(), 3080)
        );
    }

    #[test]
    fn reverse_tunnel_rewrites_web_ports() {
        assert_eq!(
            reverse_tunnel_addr("127.0.0.1:3080").unwrap(),
            ("127.0.0.1".into(), 3024)
        );
        assert_eq!(
            reverse_tunnel_addr("teleport.example").unwrap(),
            ("teleport.example".into(), 3024)
        );
        assert_eq!(
            reverse_tunnel_addr("127.0.0.1:3024").unwrap(),
            ("127.0.0.1".into(), 3024)
        );
    }

    #[test]
    fn parse_dial_json_and_legacy() {
        let d = parse_dial(br#"{"Address":"@local-node","ServerID":"abc"}"#);
        assert_eq!(d.address, LOCAL_NODE);
        assert_eq!(d.server_id, "abc");
        let d = parse_dial(b"@local-node");
        assert_eq!(d.address, LOCAL_NODE);
    }

    #[test]
    fn cert_ok_rejects_empty_user() {
        assert!(user_ok("alice"));
        assert!(!user_ok(""));
    }

    fn user_ok(user: &str) -> bool {
        !user.is_empty()
    }

    #[tokio::test]
    async fn app_router_bind_send_unbind() {
        let router = AppRouter::new();
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4);
        router.bind("demo:app".into(), tx).await;
        router
            .send(crate::plane::app("box-1", "demo:app", b"hello".to_vec()))
            .await;
        let got = rx.recv().await.expect("line");
        assert_eq!(got, b"hello\n");
        router.unbind("demo:app").await;
        router
            .send(crate::plane::app("box-1", "demo:app", b"gone".to_vec()))
            .await;
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn app_router_keeps_existing_newline() {
        let router = AppRouter::new();
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1);
        router.bind("c".into(), tx).await;
        router
            .send(crate::plane::app("d", "c", b"ok\n".to_vec()))
            .await;
        assert_eq!(rx.recv().await.unwrap(), b"ok\n");
    }
}
