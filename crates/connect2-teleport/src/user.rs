//! Laptop Teleport client. No `tsh`.
//!
//! 1. SSH to the proxy (`:3023`) as the Teleport user, with the identity-file cert.
//! 2. Request subsystem `proxy:<node>:0@namespace@cluster` — Teleport dials the reverse tunnel.
//! 3. Nested SSH to `connect2-agent` as the OS login (`packer`), then `exec connect-app`.
//!
//! Inventory is Auth gRPC `ListResources` on `:3025` (mTLS from the same identity file).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use russh::client::{self, Handle, Msg};
use russh::keys::{PrivateKey, PublicKey};
use russh::{Channel, ChannelMsg, ChannelStream};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Clone)]
pub struct UserId {
    pub key: Arc<PrivateKey>,
    pub cert_algo: String,
    pub cert_blob: Vec<u8>,
}

#[derive(Clone)]
pub struct Cfg {
    pub proxy: String,
    pub ssh: Option<String>,
    /// Auth gRPC (`host:3025`). Default: proxy host on :3025.
    pub auth: Option<String>,
    pub identity: std::path::PathBuf,
    pub teleport_user: String,
    pub login: String,
    pub cluster: String,
    pub namespace: String,
    pub insecure: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub tunnel: bool,
}

pub struct AppSession {
    _proxy: Handle<AcceptAll>,
    _node: Handle<AcceptAll>,
    stream: ChannelStream<Msg>,
}

pub(crate) struct AcceptAll;

impl client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub fn load_identity(path: &Path) -> Result<UserId> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let key = private_key(&text)?;
    let (cert_algo, cert_blob) = ssh_cert(&text)?;
    Ok(UserId {
        key: Arc::new(key),
        cert_algo,
        cert_blob,
    })
}

fn private_key(text: &str) -> Result<PrivateKey> {
    let start = text
        .find("-----BEGIN")
        .ok_or_else(|| anyhow!("identity file: no private key"))?;
    let rest = &text[start..];
    let end = rest
        .find("-----END")
        .and_then(|i| rest[i..].find('\n').map(|j| i + j))
        .map(|i| i + 1)
        .unwrap_or(rest.len());
    let pem = &rest[..end];
    russh::keys::decode_secret_key(pem, None)
        .or_else(|_| PrivateKey::from_openssh(pem).map_err(|e| anyhow!("{e}")))
        .context("parse identity private key")
}

fn ssh_cert(text: &str) -> Result<(String, Vec<u8>)> {
    for line in text.lines() {
        let line = line.trim();
        if !line.contains("-cert-v01@openssh.com") {
            continue;
        }
        let mut it = line.split_whitespace();
        let algo = it.next().ok_or_else(|| anyhow!("empty cert line"))?;
        let b64 = it.next().ok_or_else(|| anyhow!("cert missing blob"))?;
        let blob = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .or_else(|_| {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD_NO_PAD, b64)
            })?;
        return Ok((algo.to_string(), blob));
    }
    bail!("identity file: no OpenSSH certificate line")
}

pub fn proxy_subsystem(node: &str, namespace: &str, cluster: &str) -> String {
    let ns = if namespace.is_empty() {
        "default"
    } else {
        namespace
    };
    if cluster.is_empty() {
        format!("proxy:{node}:0")
    } else {
        format!("proxy:{node}:0@{ns}@{cluster}")
    }
}

pub fn ssh_addr(proxy: &str, ssh: Option<&str>) -> Result<(String, u16)> {
    if let Some(s) = ssh.filter(|s| !s.is_empty()) {
        return split_host_port(s);
    }
    let (host, port) = split_host_port(proxy)?;
    let port = match port {
        443 | 3080 | 80 | 8443 => 3023,
        other => other,
    };
    Ok((host, port))
}

fn split_host_port(s: &str) -> Result<(String, u16)> {
    let p = s
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if let Some((h, port)) = p.rsplit_once(':') {
        if !h.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return Ok((h.to_string(), port.parse().unwrap_or(3023)));
        }
    }
    Ok((p.to_string(), 3023))
}

fn ssh_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: None,
        keepalive_max: 0,
        preferred: crate::teleport::ssh_preferred(),
        nodelay: true,
        ..Default::default()
    })
}

async fn auth_user(handle: &mut Handle<AcceptAll>, user: &str, id: &UserId) -> Result<()> {
    let auth = handle
        .authenticate_openssh_cert_raw(
            user,
            id.key.clone(),
            id.cert_algo.clone(),
            id.cert_blob.clone(),
        )
        .await?;
    if !auth.success() {
        bail!("ssh auth as {user} failed: {auth:?}");
    }
    Ok(())
}

async fn wait_ok(ch: &mut Channel<Msg>, what: &str) -> Result<()> {
    loop {
        match ch.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => bail!("{what} refused"),
            Some(ChannelMsg::Close) | Some(ChannelMsg::Eof) | None => {
                bail!("{what} closed before success")
            }
            _ => {}
        }
    }
}

struct Hop {
    proxy: Handle<AcceptAll>,
    node: Handle<AcceptAll>,
}

/// Proxy SSH hop + nested SSH to the node (same as `tsh ssh`, no `tsh`).
async fn hop(cfg: &Cfg, node: &str) -> Result<Hop> {
    let id = load_identity(&cfg.identity)?;
    let (host, port) = ssh_addr(&cfg.proxy, cfg.ssh.as_deref())?;
    let addr = (host.as_str(), port);
    tracing::info!(%host, port, node, "ssh proxy (no tsh)");
    let mut last_err = None;
    let hop_users = unique_users(&[&cfg.login, &cfg.teleport_user]);
    for hop_user in &hop_users {
        let tcp = TcpStream::connect(addr)
            .await
            .with_context(|| format!("tcp {host}:{port}"))?;
        let _ = tcp.set_nodelay(true);
        let mut proxy = match client::connect_stream(ssh_config(), tcp, AcceptAll).await {
            Ok(p) => p,
            Err(e) => {
                last_err = Some(anyhow!("ssh proxy handshake: {e}"));
                continue;
            }
        };
        if let Err(e) = auth_user(&mut proxy, hop_user, &id).await {
            last_err = Some(e);
            continue;
        }

        let mut jump = match proxy.channel_open_session().await {
            Ok(c) => c,
            Err(e) => {
                last_err = Some(anyhow!("proxy session: {e}"));
                continue;
            }
        };
        let sub = proxy_subsystem(node, &cfg.namespace, &cfg.cluster);
        tracing::info!(%sub, hop = hop_user, "proxy subsystem");
        if let Err(e) = jump.request_subsystem(true, sub.clone()).await {
            last_err = Some(anyhow!("request proxy subsystem: {e}"));
            continue;
        }
        if let Err(e) = wait_ok(&mut jump, &sub).await {
            last_err = Some(e);
            continue;
        }
        let tun = jump.into_stream();

        let mut node_ssh = match client::connect_stream(ssh_config(), tun, AcceptAll).await {
            Ok(s) => s,
            Err(e) => {
                last_err = Some(anyhow!("ssh node handshake: {e}"));
                continue;
            }
        };
        if let Err(e) = auth_user(&mut node_ssh, &cfg.login, &id).await {
            last_err = Some(e);
            continue;
        }
        return Ok(Hop {
            proxy,
            node: node_ssh,
        });
    }
    Err(last_err.unwrap_or_else(|| anyhow!("dial {node} failed")))
}

/// Dial `node` through the proxy reverse tunnel and start `connect-app`.
pub async fn dial_app(cfg: &Cfg, node: &str) -> Result<AppSession> {
    let hop = hop(cfg, node).await?;
    let app = hop
        .node
        .channel_open_session()
        .await
        .context("node session")?;
    if let Err(e) = app
        .request_subsystem(true, crate::teleport::APP_SUBSYSTEM)
        .await
    {
        bail!("connect-app subsystem: {e}");
    }
    // Do not wait_ok: JSON hello can race SSH CHANNEL_SUCCESS and would be dropped.
    Ok(AppSession {
        _proxy: hop.proxy,
        _node: hop.node,
        stream: app.into_stream(),
    })
}

/// Interactive / exec SSH on the node (`tsh ssh` without `tsh`).
pub struct SshSession {
    _proxy: Handle<AcceptAll>,
    _node: Handle<AcceptAll>,
    pub channel: Channel<Msg>,
}

pub async fn dial_ssh(
    cfg: &Cfg,
    node: &str,
    cols: u32,
    rows: u32,
    command: Option<&str>,
) -> Result<SshSession> {
    let hop = hop(cfg, node).await?;
    let mut ch = hop
        .node
        .channel_open_session()
        .await
        .context("ssh session")?;
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into());
    let interactive = command.is_none();
    if interactive {
        ch.request_pty(true, &term, cols, rows, 0, 0, &[])
            .await
            .context("pty")?;
        wait_ok(&mut ch, "pty").await?;
        ch.request_shell(false).await.context("shell")?;
    } else {
        let Some(cmd) = command else {
            bail!("exec without command");
        };
        ch.exec(false, cmd).await.context("exec")?;
    }
    Ok(SshSession {
        _proxy: hop.proxy,
        _node: hop.node,
        channel: ch,
    })
}

fn unique_users(users: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for u in users {
        if !u.is_empty() && !out.iter().any(|x: &String| x == u) {
            out.push((*u).to_string());
        }
    }
    out
}

impl AppSession {
    pub async fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.stream.write_all(data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.stream.read(buf).await?)
    }

    pub fn into_stdio(self) -> AppStdio {
        let (stdout, stdin) = tokio::io::split(self.stream);
        AppStdio {
            _proxy: self._proxy,
            _node: self._node,
            stdin,
            stdout,
        }
    }

    pub fn into_rw(self) -> AppRw {
        let io = self.into_stdio();
        AppRw {
            _proxy: io._proxy,
            _node: io._node,
            stdin: io.stdin,
            stdout: Some(io.stdout),
        }
    }
}

pub struct AppStdio {
    _proxy: Handle<AcceptAll>,
    _node: Handle<AcceptAll>,
    pub stdin: tokio::io::WriteHalf<ChannelStream<Msg>>,
    pub stdout: tokio::io::ReadHalf<ChannelStream<Msg>>,
}

/// Same as [`AppStdio`], but stdout can be taken by a reader task.
pub struct AppRw {
    _proxy: Handle<AcceptAll>,
    _node: Handle<AcceptAll>,
    pub stdin: tokio::io::WriteHalf<ChannelStream<Msg>>,
    pub stdout: Option<tokio::io::ReadHalf<ChannelStream<Msg>>>,
}

/// Cluster name from `/webapi/ping`.
pub async fn ping_cluster(proxy: &str, insecure: bool) -> Result<String> {
    let url = ping_url(proxy);
    let mut b = reqwest::Client::builder().timeout(Duration::from_secs(10));
    if insecure {
        b = b.danger_accept_invalid_certs(true);
    }
    let http = b.build()?;
    let v: Value = http
        .get(&url)
        .send()
        .await
        .with_context(|| format!("ping {url}"))?
        .error_for_status()?
        .json()
        .await?;
    Ok(v.get("cluster_name")
        .and_then(Value::as_str)
        .unwrap_or("teleport.local")
        .to_string())
}

pub fn ping_url(proxy: &str) -> String {
    let p = proxy.trim().trim_end_matches('/');
    if p.contains("://") {
        format!("{p}/webapi/ping")
    } else {
        format!("https://{p}/webapi/ping")
    }
}

pub fn auth_addr(proxy: &str, auth: Option<&str>) -> Result<(String, u16)> {
    if let Some(s) = auth.filter(|s| !s.is_empty()) {
        return split_host_port(s);
    }
    let (host, port) = split_host_port(proxy)?;
    let port = match port {
        443 | 3080 | 80 | 8443 | 3023 | 3024 => 3025,
        other => other,
    };
    Ok((host, port))
}

fn identity_tls(text: &str) -> Result<(reqwest::Identity, Option<reqwest::Certificate>)> {
    let mut pems = Vec::new();
    let mut cur = String::new();
    let mut in_pem = false;
    for line in text.lines() {
        if line.starts_with("-----BEGIN") {
            in_pem = true;
            cur.clear();
            cur.push_str(line);
            cur.push('\n');
        } else if in_pem {
            cur.push_str(line);
            cur.push('\n');
            if line.starts_with("-----END") {
                pems.push(cur.clone());
                in_pem = false;
            }
        }
    }
    let key = pems
        .iter()
        .find(|p| p.contains("BEGIN PRIVATE KEY") && !p.contains("OPENSSH"))
        .or_else(|| pems.iter().find(|p| p.contains("BEGIN RSA PRIVATE KEY")))
        .or_else(|| pems.iter().find(|p| p.contains("BEGIN EC PRIVATE KEY")))
        .ok_or_else(|| anyhow!("identity file: no TLS private key"))?;
    let certs: Vec<_> = pems
        .iter()
        .filter(|p| p.contains("BEGIN CERTIFICATE"))
        .cloned()
        .collect();
    if certs.is_empty() {
        bail!("identity file: no TLS certificate");
    }
    let mut chain = certs[0].clone();
    if !chain.ends_with('\n') {
        chain.push('\n');
    }
    chain.push_str(key);
    let id = reqwest::Identity::from_pem(chain.as_bytes()).context("identity TLS cert+key")?;
    let ca = certs.get(1).and_then(|pem| reqwest::Certificate::from_pem(pem.as_bytes()).ok());
    Ok((id, ca))
}

fn proto_varint(n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut n = n;
    while n >= 0x80 {
        out.push(((n as u8) & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

fn proto_str(field: u32, s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(proto_varint(((field as u64) << 3) | 2));
    out.extend(proto_varint(s.len() as u64));
    out.extend(s.as_bytes());
    out
}

fn proto_varint_field(field: u32, v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(proto_varint((field as u64) << 3));
    out.extend(proto_varint(v));
    out
}

fn grpc_frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(0);
    out.extend((payload.len() as u32).to_be_bytes());
    out.extend(payload);
    out
}

fn proto_strings(bytes: &[u8]) -> Vec<String> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let (tag, n) = match proto_read_varint(bytes, i) {
            Some(v) => v,
            None => break,
        };
        i = n;
        let wt = tag & 7;
        match wt {
            0 => {
                let Some((_, n)) = proto_read_varint(bytes, i) else { break };
                i = n;
            }
            1 => i = i.saturating_add(8),
            5 => i = i.saturating_add(4),
            2 => {
                let Some((len, n)) = proto_read_varint(bytes, i) else { break };
                i = n;
                let end = i.saturating_add(len as usize);
                if end > bytes.len() {
                    break;
                }
                let slice = &bytes[i..end];
                if let Ok(s) = std::str::from_utf8(slice) {
                    if !s.is_empty() && s.chars().all(|c| !c.is_control() || c == '\n') {
                        out.push(s.to_string());
                    }
                }
                out.extend(proto_strings(slice));
                i = end;
            }
            _ => break,
        }
    }
    out
}

fn proto_read_varint(bytes: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut n = 0u64;
    let mut shift = 0;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        n |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((n, i));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

fn parse_list_resources(body: &[u8]) -> Vec<Node> {
    let payload = if body.len() >= 5 && body[0] == 0 {
        let n = u32::from_be_bytes(body[1..5].try_into().unwrap_or([0; 4])) as usize;
        let end = 5usize.saturating_add(n).min(body.len());
        &body[5..end]
    } else {
        body
    };
    let strings = proto_strings(payload);
    let mut nodes = Vec::new();
    let mut i = 0;
    while i < strings.len() {
        if strings[i] == "hostname" && i + 1 < strings.len() {
            let name = strings[i + 1].clone();
            if !name.is_empty() && name != "hostname" {
                let id = strings
                    .get(i.saturating_sub(2))
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                let tunnel = strings.iter().any(|s| s.contains("use_tunnel")) || true;
                nodes.push(Node {
                    id,
                    name,
                    tunnel,
                });
            }
            i += 2;
            continue;
        }
        let name = &strings[i];
        if name.starts_with("box-")
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            let id = strings
                .iter()
                .take(i)
                .rev()
                .find(|s| s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4)
                .cloned()
                .unwrap_or_else(|| name.clone());
            nodes.push(Node {
                id,
                name: name.clone(),
                tunnel: true,
            });
        }
        i += 1;
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes.dedup_by(|a, b| a.name == b.name);
    nodes
}

/// Nodes from Auth `ListResources` (same inventory as `tsh ls`, no tsh).
pub async fn list_nodes(cfg: &Cfg) -> Result<Vec<Node>> {
    let text = std::fs::read_to_string(&cfg.identity)
        .with_context(|| format!("read {}", cfg.identity.display()))?;
    let (id, ca) = identity_tls(&text)?;
    let (host, port) = auth_addr(&cfg.proxy, cfg.auth.as_deref())?;
    let url = format!("https://{host}:{port}/proto.AuthService/ListResources");
    let mut payload = Vec::new();
    payload.extend(proto_str(1, "node"));
    if !cfg.namespace.is_empty() {
        payload.extend(proto_str(2, &cfg.namespace));
    }
    payload.extend(proto_varint_field(3, 200));
    let mut b = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .http2_prior_knowledge()
        .identity(id)
        .tls_sni(false);
    if cfg.insecure {
        b = b.danger_accept_invalid_certs(true);
    }
    if let Some(ca) = ca {
        b = b.add_root_certificate(ca);
    }
    let http = b.build()?;
    let res = http
        .post(&url)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(grpc_frame(&payload))
        .send()
        .await
        .with_context(|| format!("list nodes {url}"))?;
    if !res.status().is_success() {
        bail!("ListResources HTTP {}", res.status());
    }
    let headers = res.headers().clone();
    let bytes = res.bytes().await?;
    let status = headers
        .get("grpc-status")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0");
    if status != "0" {
        let msg = headers
            .get("grpc-message")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        bail!("ListResources grpc-status {status}: {msg}");
    }
    Ok(parse_list_resources(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_subsystem_full() {
        assert_eq!(
            proxy_subsystem("box-1", "default", "teleport.local"),
            "proxy:box-1:0@default@teleport.local"
        );
        assert_eq!(proxy_subsystem("box-1", "", ""), "proxy:box-1:0");
    }

    #[test]
    fn ssh_addr_web_to_3023() {
        let (h, p) = ssh_addr("127.0.0.1:3080", None).unwrap();
        assert_eq!((h, p), ("127.0.0.1".into(), 3023));
        let (h, p) = ssh_addr("https://teleport.example:443", None).unwrap();
        assert_eq!((h, p), ("teleport.example".into(), 3023));
        let (h, p) = ssh_addr("127.0.0.1:3080", Some("10.0.0.1:3023")).unwrap();
        assert_eq!((h, p), ("10.0.0.1".into(), 3023));
    }

    #[test]
    fn ping_url_https() {
        assert_eq!(
            ping_url("127.0.0.1:3080"),
            "https://127.0.0.1:3080/webapi/ping"
        );
    }

    #[test]
    fn auth_addr_web_to_3025() {
        let (h, p) = auth_addr("127.0.0.1:3080", None).unwrap();
        assert_eq!((h, p), ("127.0.0.1".into(), 3025));
        let (h, p) = auth_addr("https://teleport.example:443", None).unwrap();
        assert_eq!((h, p), ("teleport.example".into(), 3025));
        let (h, p) = auth_addr("127.0.0.1:3080", Some("10.0.0.1:3025")).unwrap();
        assert_eq!((h, p), ("10.0.0.1".into(), 3025));
    }

    #[test]
    fn parse_list_resources_hostname() {
        let hex = "00000000e30ae0011add010a046e6f64651a02763222680a2430376164656330632d643966352d343930352d393336372d333430343964623733626234120764656661756c742a110a08686f73746e616d651205626f782d31422464303935623062662d616638642d346132372d396639342d3230353564373063323534302a670a0e3132372e302e302e313a333032321a05626f782d312a432a0b088092b8c398feffffff013a0b088092b8c398feffffff0142270a0b088092b8c398feffffff01120b088092b8c398feffffff011a0b088092b8c398feffffff0130013a0731362e342e3132";
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let nodes = parse_list_resources(&bytes);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "box-1");
        assert!(nodes[0].tunnel);
    }

    #[test]
    fn unique_users_drops_empty_and_dupes() {
        assert_eq!(
            unique_users(&["packer", "", "packer", "root"]),
            vec!["packer".to_string(), "root".to_string()]
        );
        assert!(unique_users(&["", ""]).is_empty());
    }

    #[test]
    fn ping_url_keeps_https_scheme() {
        assert_eq!(
            ping_url("https://teleport.example:443"),
            "https://teleport.example:443/webapi/ping"
        );
    }

    #[test]
    fn identity_tls_matches_self_signed() {
        let kp = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec!["demo".into()]).unwrap();
        let cert = params.self_signed(&kp).unwrap();
        let text = format!("{}{}", cert.pem(), kp.serialize_pem());
        let (_id, ca) = identity_tls(&text).unwrap();
        assert!(ca.is_none());
    }

    #[test]
    fn identity_tls_skips_openssh_key() {
        let text = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
        assert!(identity_tls(text).is_err());
    }

    #[test]
    fn proto_helpers() {
        assert_eq!(proto_varint(0), vec![0]);
        assert_eq!(proto_varint(0x80), vec![0x80, 0x01]);
        let f = grpc_frame(b"hi");
        assert_eq!(f[0], 0);
        assert_eq!(&f[1..5], &(2u32).to_be_bytes());
        assert_eq!(&f[5..], b"hi");
    }
}
