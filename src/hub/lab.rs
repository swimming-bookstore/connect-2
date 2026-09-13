//! Lab-only dummy Okta. OSS Teleport cannot host OIDC.
//!
//! Same browser shape as `tsh login --auth=oidc`: open IdP, then a localhost
//! callback that carries a Teleport identity (private key + certs). Username
//! alone is not enough. Unset `CONNECT2_DUMMY_SSO` outside the lab.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

use anyhow::{Context, Result};
use connect2_teleport::login::{
    parse_callback, sso_payload_has_certs, write_sso_identity_file, AuthInfo, Connector,
    LoginResult,
};

pub fn active() -> bool {
    issuer().is_some()
}

pub fn inject(info: &mut AuthInfo) {
    if issuer().is_none() {
        return;
    }
    if info.connectors.iter().any(|c| c.name == "okta") {
        return;
    }
    info.connectors.push(Connector {
        kind: "oidc".into(),
        name: "okta".into(),
        display: "Okta".into(),
    });
}

/// Open Dummy Okta in the browser and wait for the localhost callback.
/// `None` = use real Teleport SSO.
pub fn login(
    identity: &Path,
    connector: &str,
    mut on_url: impl FnMut(String),
) -> Option<Result<LoginResult>> {
    let issuer = issuer()?;
    Some(run(identity, connector, &issuer, &mut on_url))
}

fn issuer() -> Option<String> {
    std::env::var("CONNECT2_DUMMY_SSO")
        .ok()
        .filter(|s| !s.is_empty())
}

fn run(
    identity: &Path,
    connector: &str,
    issuer: &str,
    on_url: &mut impl FnMut(String),
) -> Result<LoginResult> {
    let listener = TcpListener::bind("127.0.0.1:0").context("bind sso callback")?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let browse = format!(
        "{}/authorize?redirect_uri={redirect}&connector_id={connector}",
        issuer.trim_end_matches('/')
    );
    eprintln!("Dummy SSO  {browse}");
    on_url(browse.clone());
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = tx.send(wait_callback(listener));
    });
    connect2_teleport::login::open_sso_browser(&browse);
    let payload = rx
        .recv_timeout(std::time::Duration::from_secs(300))
        .context("SSO timed out (open the URL printed above)")??;
    write_sso_identity_file(identity, &payload)
}

fn wait_callback(listener: TcpListener) -> Result<serde_json::Value> {
    let _ = listener.set_nonblocking(false);
    loop {
        let (mut s, _) = listener.accept().context("sso callback accept")?;
        let mut buf = vec![0u8; 262144];
        let n = s.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        if req.starts_with("OPTIONS") {
            let _ = s.write_all(
                b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type\r\nConnection: close\r\n\r\n",
            );
            continue;
        }
        match parse_callback(&req) {
            Ok(v) if sso_payload_has_certs(&v) => {
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n<!doctype html><title>Connect 2</title><p>Signed in. You can close this window.</p>",
                );
                return Ok(v);
            }
            _ => {
                let _ = s.write_all(
                    b"HTTP/1.1 400 Bad Request\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\nmissing certs",
                );
            }
        }
    }
}


