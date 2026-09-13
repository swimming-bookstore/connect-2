//! Join a Teleport cluster (`POST /webapi/host/credentials`).

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};

use crate::identity::{Certs, Identity};

pub async fn register(
    proxy: &str,
    token: &str,
    identity: &Identity,
    tls_ca: Option<&std::path::Path>,
    insecure: bool,
) -> Result<Certs> {
    let url = host_credentials_url(proxy);
    let body = json!({
        "hostID": identity.host_id,
        "node_name": identity.hostname,
        "role": "Node",
        "token": token,
        "public_tls_key": identity.tls_pub_pem.as_bytes(),
        "public_ssh_key": identity.ssh_public_openssh()?.into_bytes(),
        "additional_principals": [identity.hostname, identity.host_id],
        "dns_names": [identity.hostname],
    });
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(ca) = tls_ca {
        let pem = std::fs::read(ca).with_context(|| format!("read {}", ca.display()))?;
        let cert = reqwest::Certificate::from_pem(&pem)?;
        builder = builder.add_root_certificate(cert);
    }
    let http = builder.build()?;
    let res = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("join {url}"))?;
    let status = res.status();
    let bytes = res.bytes().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "join {url} -> {status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    parse_certs(&bytes).context("parse join certs")
}

pub fn host_credentials_url(proxy: &str) -> String {
    let proxy = proxy.trim().trim_end_matches('/');
    if proxy.contains("://") {
        format!("{proxy}/webapi/host/credentials")
    } else {
        format!("https://{proxy}/webapi/host/credentials")
    }
}

pub fn parse_certs(bytes: &[u8]) -> Result<Certs> {
    let v: Value = serde_json::from_slice(bytes)?;
    Ok(Certs {
        ssh: json_bytes(&v, "ssh")?,
        tls: json_bytes(&v, "tls")?,
        tls_ca_certs: json_bytes_array(&v, "tls_ca_certs")?,
        ssh_ca_certs: json_bytes_array(&v, "ssh_ca_certs")?,
    })
}

fn json_bytes(v: &Value, key: &str) -> Result<Vec<u8>> {
    match v.get(key) {
        Some(Value::String(s)) => STANDARD.decode(s).or_else(|_| Ok(s.as_bytes().to_vec())),
        Some(Value::Array(arr)) => Ok(arr
            .iter()
            .filter_map(|x| x.as_u64().map(|n| n as u8))
            .collect()),
        _ => Err(anyhow!("missing {key}")),
    }
}

fn json_bytes_array(v: &Value, key: &str) -> Result<Vec<Vec<u8>>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|item| match item {
                Value::String(s) => STANDARD.decode(s).or_else(|_| Ok(s.as_bytes().to_vec())),
                other => json_bytes(&json!({ "x": other }), "x"),
            })
            .collect(),
        Some(Value::String(s)) => Ok(vec![STANDARD.decode(s).unwrap_or_else(|_| s.as_bytes().to_vec())]),
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_credentials_url_adds_https() {
        assert_eq!(
            host_credentials_url("teleport.example:443"),
            "https://teleport.example:443/webapi/host/credentials"
        );
        assert_eq!(
            host_credentials_url("https://teleport.example"),
            "https://teleport.example/webapi/host/credentials"
        );
    }

    #[test]
    fn parse_certs_base64() {
        let ssh = STANDARD.encode(b"ssh-cert");
        let tls = STANDARD.encode(b"-----BEGIN CERTIFICATE-----\nM\n-----END CERTIFICATE-----\n");
        let ca = STANDARD.encode(b"ca");
        let v = json!({"ssh": ssh, "tls": tls, "tls_ca_certs": [ca], "ssh_ca_certs": []});
        let c = parse_certs(serde_json::to_vec(&v).unwrap().as_slice()).unwrap();
        assert_eq!(c.ssh, b"ssh-cert");
        assert!(c.tls.starts_with(b"-----BEGIN"));
        assert_eq!(c.tls_ca_certs.len(), 1);
    }
}
