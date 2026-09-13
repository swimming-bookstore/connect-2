//! User login without `tsh` (`tsh login` equivalent).
//!
//! - Local (`tsh login --user`): `POST /webapi/ssh/certs` with password + OTP.
//!   Not `/web/headless` — that is `tsh login --headless` (WebAuthn).
//! - SSO: `POST /webapi/{oidc,saml,github}/login/console` + localhost callback
//!
//! Writes a Teleport identity file (`tsh --identity`): SSH key + cert, TLS cert, CAs.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::pkcs8::{EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::user::{load_identity, ping_url};

const CERT_TTL_NS: u64 = 12 * 60 * 60 * 1_000_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Connector {
    pub kind: String,
    pub name: String,
    pub display: String,
}

#[derive(Clone, Debug, Default)]
pub struct AuthInfo {
    pub cluster: String,
    pub auth_type: String,
    pub second_factor: String,
    pub connectors: Vec<Connector>,
}

#[derive(Clone, Debug)]
pub struct LoginResult {
    pub username: String,
    pub identity_path: std::path::PathBuf,
}

pub fn parse_auth_ping(v: &Value) -> AuthInfo {
    let cluster = v
        .get("cluster_name")
        .and_then(Value::as_str)
        .unwrap_or("teleport")
        .to_string();
    let auth = v.get("auth").cloned().unwrap_or(json!({}));
    let auth_type = auth
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .to_string();
    let second_factor = auth
        .get("second_factor")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut connectors = Vec::new();
    for kind in ["oidc", "saml", "github"] {
        if let Some(block) = auth.get(kind) {
            push_connectors(&mut connectors, kind, block);
        }
    }
    if connectors.is_empty() {
        match auth_type.as_str() {
            "oidc" | "saml" | "github" => connectors.push(Connector {
                kind: auth_type.clone(),
                name: auth
                    .get(&auth_type)
                    .and_then(|b| b.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(&auth_type)
                    .to_string(),
                display: auth
                    .get(&auth_type)
                    .and_then(|b| b.get("display"))
                    .and_then(Value::as_str)
                    .unwrap_or(&auth_type)
                    .to_string(),
            }),
            _ => {}
        }
    }
    AuthInfo {
        cluster,
        auth_type,
        second_factor,
        connectors,
    }
}

fn push_connectors(out: &mut Vec<Connector>, kind: &str, block: &Value) {
    if let Some(arr) = block.get("connectors").and_then(Value::as_array) {
        for c in arr {
            let name = c
                .get("name")
                .or_else(|| c.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let display = c
                .get("display")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_string();
            out.push(Connector {
                kind: kind.into(),
                name,
                display,
            });
        }
        return;
    }
    if let Some(name) = block.get("name").and_then(Value::as_str) {
        if !name.is_empty() {
            out.push(Connector {
                kind: kind.into(),
                name: name.into(),
                display: block
                    .get("display")
                    .and_then(Value::as_str)
                    .unwrap_or(name)
                    .into(),
            });
        }
    }
}

pub async fn ping_auth(proxy: &str, insecure: bool) -> Result<AuthInfo> {
    let url = ping_url(proxy);
    let v = http(insecure)?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("ping {url}"))?
        .error_for_status()?
        .json::<Value>()
        .await?;
    Ok(parse_auth_ping(&v))
}

pub fn console_login_url(proxy: &str, kind: &str) -> String {
    let base = web_base(proxy);
    let path = match kind {
        "saml" => "saml",
        "github" => "github",
        _ => "oidc",
    };
    format!("{base}/webapi/{path}/login/console")
}

pub fn ssh_certs_url(proxy: &str) -> String {
    format!("{}/webapi/ssh/certs", web_base(proxy))
}

fn web_base(proxy: &str) -> String {
    let p = proxy.trim().trim_end_matches('/');
    if p.contains("://") {
        p.to_string()
    } else {
        format!("https://{p}")
    }
}

fn http(insecure: bool) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder().timeout(Duration::from_secs(30));
    if insecure {
        b = b.danger_accept_invalid_certs(true);
    }
    Ok(b.build()?)
}

struct KeyPair {
    pem: String,
    ssh_pub: Vec<u8>,
    tls_pub: Vec<u8>,
}

fn generate_keys() -> Result<KeyPair> {
    let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).context("rsa generate")?;
    let pem = key
        .to_pkcs1_pem(LineEnding::LF)
        .context("rsa pkcs1")?
        .to_string();
    let tls_pub = key
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .context("rsa public pem")?
        .into_bytes();
    Ok(KeyPair {
        pem: pem.clone(),
        ssh_pub: ssh_rsa_authorized(&key),
        tls_pub,
    })
}

fn ssh_string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

fn ssh_mpint(out: &mut Vec<u8>, n: &[u8]) {
    let mut n = n;
    while n.first() == Some(&0) {
        n = &n[1..];
    }
    if n.first().is_some_and(|b| *b & 0x80 != 0) {
        out.extend_from_slice(&((n.len() + 1) as u32).to_be_bytes());
        out.push(0);
        out.extend_from_slice(n);
    } else {
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
}

fn ssh_rsa_authorized(key: &RsaPrivateKey) -> Vec<u8> {
    let pubk = key.to_public_key();
    let mut blob = Vec::new();
    ssh_string(&mut blob, b"ssh-rsa");
    ssh_mpint(&mut blob, &pubk.e().to_bytes_be());
    ssh_mpint(&mut blob, &pubk.n().to_bytes_be());
    format!("ssh-rsa {}", STANDARD.encode(blob)).into_bytes()
}

/// Local user login (`tsh login --user` / password / OTP).
pub async fn login_password(
    proxy: &str,
    identity: &Path,
    user: &str,
    password: &str,
    otp: &str,
    insecure: bool,
) -> Result<LoginResult> {
    let keys = generate_keys()?;
    let url = ssh_certs_url(proxy);
    let body = json!({
        "user": user,
        "password": password,
        "otp_token": otp,
        "second_factor_token": otp,
        "pub_key": STANDARD.encode(&keys.ssh_pub),
        "tls_pub_key": STANDARD.encode(&keys.tls_pub),
        "ttl": CERT_TTL_NS,
    });
    let res = http(insecure)?
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("login {url}"))?;
    let status = res.status();
    let bytes = res.bytes().await?;
    if !status.is_success() {
        bail!("login {url} -> {status}: {}", String::from_utf8_lossy(&bytes));
    }
    let v: Value = serde_json::from_slice(&bytes).context("login json")?;
    write_login_identity(identity, &keys, &v)
}

/// SSO login (`tsh login --auth=oidc|saml|github`). Opens a browser.
pub async fn login_sso(
    proxy: &str,
    identity: &Path,
    kind: &str,
    connector: &str,
    insecure: bool,
) -> Result<LoginResult> {
    login_sso_notify(proxy, identity, kind, connector, insecure, |_| {}).await
}

pub async fn login_sso_notify(
    proxy: &str,
    identity: &Path,
    kind: &str,
    connector: &str,
    insecure: bool,
    mut on_url: impl FnMut(String),
) -> Result<LoginResult> {
    let keys = generate_keys()?;
    let listener = TcpListener::bind("127.0.0.1:0").await.context("bind sso callback")?;
    let addr = listener.local_addr()?;
    let redirect = format!("http://127.0.0.1:{}/callback", addr.port());
    let url = console_login_url(proxy, kind);
    let body = json!({
        "redirect_url": redirect,
        "public_key": STANDARD.encode(&keys.ssh_pub),
        "tls_pub_key": STANDARD.encode(&keys.tls_pub),
        "cert_ttl": CERT_TTL_NS,
        "connector_id": connector,
    });
    let res = http(insecure)?
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("sso {url}"))?;
    let status = res.status();
    let bytes = res.bytes().await?;
    if !status.is_success() {
        bail!("sso {url} -> {status}: {}", String::from_utf8_lossy(&bytes));
    }
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    let browse = v
        .get("redirect_url")
        .or_else(|| v.get("redirectUrl"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("sso: no redirect_url in {}", String::from_utf8_lossy(&bytes)))?;
    eprintln!("Teleport SSO  {browse}");
    on_url(browse.to_string());
    let wait = tokio::spawn(wait_callback(listener, Duration::from_secs(300)));
    open_sso_browser(browse);
    let payload = wait.await.context("sso callback")??;
    if payload.get("identity").and_then(Value::as_str).is_some() {
        write_sso_identity_file(identity, &payload)
    } else {
        write_login_identity(identity, &keys, &payload)
    }
}

/// One browser tab. Linux has both `xdg-open` and `open`; do not spawn both.
pub fn open_sso_browser(url: &str) {
    if let Ok(path) = std::env::var("CONNECT2_SSO_URL_FILE") {
        if !path.is_empty() {
            let _ = std::fs::write(&path, url);
        }
    }
    if std::env::var("CONNECT2_SSO_NO_OPEN").ok().as_deref() == Some("1") {
        return;
    }
    let bin = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(bin).arg(url).spawn();
}

async fn wait_callback(listener: TcpListener, timeout: Duration) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let (mut s, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| anyhow!("SSO timed out (open the URL printed above)"))?
            .context("sso callback accept")?;
        let mut buf = vec![0u8; 262144];
        let n = s.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        if req.starts_with("OPTIONS") {
            let _ = s
                .write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }
        match parse_callback(&req) {
            Ok(v) if sso_payload_has_certs(&v) => {
                let _ = s
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n<!doctype html><title>Connect 2</title><p>Signed in. You can close this window.</p>")
                    .await;
                return Ok(v);
            }
            _ => {
                let _ = s
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\nmissing certs")
                    .await;
            }
        }
    }
}

pub fn parse_callback(req: &str) -> Result<Value> {
    let line = req.lines().next().unwrap_or("");
    let path = line.split_whitespace().nth(1).unwrap_or("/");
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut response = query_param(query, "response");
    if response.is_empty() {
        if let Some(i) = req.find("\r\n\r\n") {
            let body = &req[i + 4..];
            response = query_param(body, "response");
            if response.is_empty() {
                let t = body.trim();
                if t.starts_with('{') {
                    return serde_json::from_str(t).context("callback json body");
                }
            }
        }
    }
    if response.is_empty() {
        bail!("SSO callback missing response");
    }
    let decoded = percent_decode(&response);
    let raw = STANDARD
        .decode(decoded.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or(decoded);
    serde_json::from_str(&raw).context("SSO callback JSON")
}

pub fn sso_payload_has_certs(v: &Value) -> bool {
    if v.get("cert").is_some() {
        return true;
    }
    v.get("identity")
        .and_then(Value::as_str)
        .is_some_and(identity_file_ok)
}

pub fn write_sso_identity_file(path: &Path, v: &Value) -> Result<LoginResult> {
    let text = v
        .get("identity")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("SSO callback missing identity"))?;
    if !identity_file_ok(text) {
        bail!("SSO callback is not a Teleport identity file");
    }
    let username = v
        .get("username")
        .or_else(|| v.get("user"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    std::fs::write(path, text.as_bytes()).with_context(|| format!("write {}", path.display()))?;
    Ok(LoginResult {
        username,
        identity_path: path.to_path_buf(),
    })
}

fn identity_file_ok(s: &str) -> bool {
    let key = s.contains("BEGIN RSA PRIVATE KEY")
        || s.contains("BEGIN OPENSSH PRIVATE KEY")
        || s.contains("BEGIN PRIVATE KEY");
    let cert = s.contains("-cert-v01@openssh.com") || s.contains("BEGIN CERTIFICATE");
    key && cert
}

fn query_param(q: &str, name: &str) -> String {
    for part in q.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            if k == name {
                return v.to_string();
            }
        }
    }
    String::new()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("00"), 16);
            if let Ok(c) = h {
                out.push(c);
                i += 3;
                continue;
            }
        } else if b[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write_login_identity(path: &Path, keys: &KeyPair, v: &Value) -> Result<LoginResult> {
    let username = v
        .get("username")
        .or_else(|| v.get("user"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let ssh_cert = json_bytes(v, "cert").context("login: cert")?;
    let tls_cert = json_bytes(v, "tls_cert")
        .or_else(|_| json_bytes(v, "tlsCert"))
        .unwrap_or_default();
    let mut out = String::new();
    if !keys.pem.ends_with('\n') {
        out.push_str(&keys.pem);
        out.push('\n');
    } else {
        out.push_str(&keys.pem);
    }
    out.push_str(&ssh_cert_line(&ssh_cert));
    out.push('\n');
    if !tls_cert.is_empty() {
        out.push_str(&to_pem("CERTIFICATE", &tls_cert));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if let Some(signers) = v.get("host_signers").and_then(Value::as_array) {
        for s in signers {
            let cluster = s
                .get("cluster_name")
                .or_else(|| s.get("domain_name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Some(keys) = s
                .get("authorized_keys")
                .or_else(|| s.get("checking_keys"))
                .and_then(Value::as_array)
            {
                for k in keys {
                    let line = json_key_line(k);
                    if !line.is_empty() {
                        if !cluster.is_empty() {
                            out.push_str(&format!("@cert-authority {cluster},{cluster},*.{cluster} "));
                        }
                        out.push_str(line.trim());
                        out.push('\n');
                    }
                }
            }
            if let Some(certs) = s
                .get("tls_certificates")
                .or_else(|| s.get("tls_certs"))
                .and_then(Value::as_array)
            {
                for c in certs {
                    if let Ok(b) = value_bytes(c) {
                        out.push_str(&to_pem("CERTIFICATE", &b));
                        if !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    std::fs::write(path, out.as_bytes()).with_context(|| format!("write {}", path.display()))?;
    Ok(LoginResult {
        username,
        identity_path: path.to_path_buf(),
    })
}

fn json_bytes(v: &Value, key: &str) -> Result<Vec<u8>> {
    v.get(key).ok_or_else(|| anyhow!("missing {key}")).and_then(value_bytes)
}

fn value_bytes(v: &Value) -> Result<Vec<u8>> {
    match v {
        Value::String(s) => {
            if s.contains("BEGIN") {
                Ok(s.as_bytes().to_vec())
            } else {
                STANDARD
                    .decode(s)
                    .or_else(|_| Ok(s.as_bytes().to_vec()))
            }
        }
        Value::Array(arr) => Ok(arr
            .iter()
            .filter_map(|x| x.as_u64().map(|n| n as u8))
            .collect()),
        _ => Err(anyhow!("not bytes")),
    }
}

fn json_key_line(v: &Value) -> String {
    match v {
        Value::String(s) => {
            if s.contains("ssh-") {
                s.clone()
            } else if let Ok(b) = STANDARD.decode(s) {
                if let Ok(t) = std::str::from_utf8(&b) {
                    if t.contains("ssh-") {
                        return t.trim().into();
                    }
                }
                ssh_cert_line(&b)
            } else {
                s.clone()
            }
        }
        _ => String::new(),
    }
}

fn ssh_cert_line(cert: &[u8]) -> String {
    let t = std::str::from_utf8(cert).unwrap_or("");
    if t.contains("-cert-v01@openssh.com") {
        return t.trim().to_string();
    }
    let algo = ssh_algo_from_blob(cert);
    format!("{algo} {}", STANDARD.encode(cert))
}

fn ssh_algo_from_blob(blob: &[u8]) -> &'static str {
    if blob.len() >= 4 {
        let n = u32::from_be_bytes(blob[0..4].try_into().unwrap_or([0; 4])) as usize;
        if 4 + n <= blob.len() {
            if let Ok(s) = std::str::from_utf8(&blob[4..4 + n]) {
                if s.contains("ed25519") {
                    return "ssh-ed25519-cert-v01@openssh.com";
                }
                if s.contains("rsa") {
                    return "ssh-rsa-cert-v01@openssh.com";
                }
                if s.contains("nistp256") {
                    return "ecdsa-sha2-nistp256-cert-v01@openssh.com";
                }
            }
        }
    }
    "ssh-ed25519-cert-v01@openssh.com"
}

fn to_pem(kind: &str, der: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(der) {
        if s.contains("BEGIN") {
            let mut t = s.trim().to_string();
            t.push('\n');
            return t;
        }
    }
    let b64 = STANDARD.encode(der);
    let mut out = format!("-----BEGIN {kind}-----\n");
    for c in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(c));
        out.push('\n');
    }
    out.push_str(&format!("-----END {kind}-----\n"));
    out
}

pub fn identity_ok(path: &Path) -> bool {
    load_identity(path).is_ok()
}

/// Drop the identity file (`tsh logout`).
pub fn logout(identity: &Path) -> Result<()> {
    if identity.is_dir() {
        for name in ["key", "key.pub", "key-cert.pub", "cas", "demo"] {
            let p = identity.join(name);
            if p.is_file() {
                std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
            }
        }
        return Ok(());
    }
    match std::fs::remove_file(identity) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {}", identity.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_pub_blob_is_wire_format() {
        let k = generate_keys().unwrap();
        let line = String::from_utf8(k.ssh_pub.clone()).unwrap();
        assert!(line.starts_with("ssh-rsa "), "{line}");
        let blob = STANDARD.decode(line.split_whitespace().nth(1).unwrap()).unwrap();
        assert_eq!(&blob[4..11], b"ssh-rsa");
        assert!(k.pem.contains("PRIVATE KEY"));
        assert!(k.tls_pub.starts_with(b"-----BEGIN PUBLIC KEY-----"));
    }

    #[test]
    fn parse_auth_local_otp_no_connectors() {
        let v = json!({
            "cluster_name": "teleport.local",
            "auth": { "type": "local", "second_factor": "otp" }
        });
        let a = parse_auth_ping(&v);
        assert_eq!(a.cluster, "teleport.local");
        assert_eq!(a.auth_type, "local");
        assert_eq!(a.second_factor, "otp");
        assert!(a.connectors.is_empty());
    }

    #[test]
    fn parse_auth_oidc() {
        let v = json!({
            "cluster_name": "teleport.example",
            "auth": {
                "type": "oidc",
                "second_factor": "off",
                "oidc": { "name": "okta", "display": "Okta" }
            }
        });
        let a = parse_auth_ping(&v);
        assert_eq!(a.cluster, "teleport.example");
        assert_eq!(a.auth_type, "oidc");
        assert_eq!(a.connectors.len(), 1);
        assert_eq!(a.connectors[0].name, "okta");
        assert_eq!(a.connectors[0].kind, "oidc");
    }

    #[test]
    fn parse_auth_connector_list() {
        let v = json!({
            "cluster_name": "c",
            "auth": {
                "type": "oidc",
                "oidc": { "connectors": [{ "name": "google", "display": "Google" }] }
            }
        });
        let a = parse_auth_ping(&v);
        assert_eq!(a.connectors[0].display, "Google");
    }

    #[test]
    fn console_urls() {
        assert_eq!(
            console_login_url("teleport.example:443", "oidc"),
            "https://teleport.example:443/webapi/oidc/login/console"
        );
        assert_eq!(
            ssh_certs_url("https://teleport.example"),
            "https://teleport.example/webapi/ssh/certs"
        );
    }

    #[test]
    fn callback_response_json() {
        let body = json!({"username":"ada","cert": STANDARD.encode(b"ssh-ed25519-cert-v01@openssh.com AAAA")});
        let enc = percent_encode(&body.to_string());
        let req = format!("GET /callback?response={enc} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        let v = parse_callback(&req).unwrap();
        assert_eq!(v["username"], "ada");
    }

    #[test]
    fn ssh_pub_is_authorized_keys_line() {
        let keys = generate_keys().unwrap();
        let line = String::from_utf8(keys.ssh_pub.clone()).unwrap();
        assert!(line.starts_with("ssh-rsa "), "{line}");
        let blob = STANDARD.decode(line.split_whitespace().nth(1).unwrap()).unwrap();
        assert_eq!(&blob[4..11], b"ssh-rsa");
        assert!(keys.pem.contains("BEGIN RSA PRIVATE KEY"));
    }

    #[test]
    fn ssh_mpint_strips_leading_zeros_and_pads_high_bit() {
        let mut out = Vec::new();
        ssh_mpint(&mut out, &[0, 0, 1]);
        assert_eq!(out, [0, 0, 0, 1, 1]);
        out.clear();
        ssh_mpint(&mut out, &[0x80, 0x01]);
        assert_eq!(out, [0, 0, 0, 3, 0, 0x80, 0x01]);
    }

    #[test]
    fn identity_ok_false_when_missing() {
        let p = std::env::temp_dir().join(format!("connect2-no-id-{}", uuid::Uuid::new_v4()));
        assert!(!identity_ok(&p));
        std::fs::write(&p, "not-a-key").unwrap();
        assert!(!identity_ok(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_callback_rejects_bad_request() {
        assert!(parse_callback("GET / HTTP/1.1\r\n\r\n").is_err());
        assert!(parse_callback("GET /callback HTTP/1.1\r\n\r\n").is_err());
        let v = parse_callback(
            "GET /callback?response=%7B%22username%22%3A%22x%22%7D HTTP/1.1\r\n\r\n",
        )
        .unwrap();
        assert!(!sso_payload_has_certs(&v));
        let post = "POST /callback HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"username\":\"okta-demo\"}";
        let v = parse_callback(post).unwrap();
        assert!(!sso_payload_has_certs(&v));
    }

    #[test]
    fn sso_identity_file_roundtrip() {
        let keys = generate_keys().unwrap();
        let ident = format!(
            "{}\nssh-rsa-cert-v01@openssh.com AAAATEST\n-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            keys.pem.trim()
        );
        let v = json!({"username": "okta-demo", "identity": ident});
        assert!(sso_payload_has_certs(&v));
        let dir = std::env::temp_dir().join(format!("connect2-sso-id-{}", std::process::id()));
        let path = dir.join("id");
        let r = write_sso_identity_file(&path, &v).unwrap();
        assert_eq!(r.username, "okta-demo");
        assert!(identity_ok(&path));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!sso_payload_has_certs(&json!({"username":"okta-demo"})));
    }

    #[test]
    fn write_identity_roundtrip() {
        let keys = generate_keys().unwrap();
        let cert_line = "ssh-ed25519-cert-v01@openssh.com AAAAC3NzaC1lZDI1NTE5LWNlcnQtdjAxQG9wZW5zc2guY29tAAAATEST";
        let v = json!({
            "username": "demo",
            "cert": STANDARD.encode(cert_line.as_bytes()),
            "tls_cert": "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            "host_signers": [{
                "cluster_name": "teleport.local",
                "authorized_keys": ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest"],
                "tls_certificates": ["-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----\n"]
            }]
        });
        let dir = std::env::temp_dir().join(format!("connect2-login-{}", std::process::id()));
        let path = dir.join("id");
        let r = write_login_identity(&path, &keys, &v).unwrap();
        assert_eq!(r.username, "demo");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("PRIVATE KEY"));
        assert!(text.contains("BEGIN OPENSSH PRIVATE KEY") || text.contains("BEGIN RSA PRIVATE KEY"));
        assert!(text.contains("ssh-ed25519-cert-v01@openssh.com") || text.contains("ssh-rsa-cert-v01@openssh.com"));
        let _ = std::fs::remove_file(&path);
        assert!(!identity_ok(&path));
        logout(&path).unwrap();
        std::fs::write(&path, "x").unwrap();
        logout(&path).unwrap();
        assert!(!path.exists());
        logout(&path).unwrap();
        let dir = std::env::temp_dir().join(format!("connect2-logout-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("key"), "k").unwrap();
        logout(&dir).unwrap();
        assert!(!dir.join("key").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn percent_encode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
}
