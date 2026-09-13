use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Meta {
    host_id: String,
    hostname: String,
}

pub struct Identity {
    pub host_id: String,
    pub hostname: String,
    pub ssh: PrivateKey,
    pub tls_key_pem: String,
    pub tls_pub_pem: String,
    pub certs: Option<Certs>,
}

#[derive(Clone, Debug)]
pub struct Certs {
    pub ssh: Vec<u8>,
    pub tls: Vec<u8>,
    pub tls_ca_certs: Vec<Vec<u8>>,
    pub ssh_ca_certs: Vec<Vec<u8>>,
}

impl Identity {
    pub fn load_or_create(dir: &Path, hostname: Option<String>) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let meta_path = dir.join("meta.json");
        let ssh_path = dir.join("ssh.key");
        let tls_path = dir.join("tls.key");
        let tls_pub_path = dir.join("tls.pub");

        let hostname = hostname.unwrap_or_else(|| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "connect2-agent".into())
        });

        let meta = if meta_path.exists() {
            serde_json::from_slice::<Meta>(&fs::read(&meta_path)?)?
        } else {
            let meta = Meta {
                host_id: uuid::Uuid::new_v4().to_string(),
                hostname: hostname.clone(),
            };
            write_private(&meta_path, serde_json::to_vec_pretty(&meta)?)?;
            meta
        };

        let ssh = if ssh_path.exists() {
            let pem = fs::read_to_string(&ssh_path)?;
            PrivateKey::from_openssh(&pem).or_else(|_| PrivateKey::from_bytes(&fs::read(&ssh_path)?))?
        } else {
            let key = PrivateKey::random(&mut rand::rngs::OsRng, Algorithm::Ed25519)?;
            write_private(
                &ssh_path,
                key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?
                    .as_bytes(),
            )?;
            key
        };

        let (tls_key_pem, tls_pub_pem) = if tls_path.exists() && tls_pub_path.exists() {
            (
                fs::read_to_string(&tls_path)?,
                fs::read_to_string(&tls_pub_path)?,
            )
        } else {
            let kp = rcgen::KeyPair::generate()?;
            let key_pem = kp.serialize_pem();
            let pub_pem = kp.public_key_pem();
            write_private(&tls_path, key_pem.as_bytes())?;
            write_private(&tls_pub_path, pub_pem.as_bytes())?;
            (key_pem, pub_pem)
        };

        let certs = load_certs(dir).ok();
        Ok(Self {
            host_id: meta.host_id,
            hostname: if hostname.is_empty() {
                meta.hostname
            } else {
                hostname
            },
            ssh,
            tls_key_pem,
            tls_pub_pem,
            certs,
        })
    }

    pub fn save_certs(&mut self, dir: &Path, certs: Certs) -> Result<()> {
        fs::create_dir_all(dir)?;
        fs::write(dir.join("ssh.cert"), &certs.ssh)?;
        fs::write(dir.join("tls.crt"), &certs.tls)?;
        let mut cas = Vec::new();
        for (i, ca) in certs.tls_ca_certs.iter().enumerate() {
            fs::write(dir.join(format!("tls-ca-{i}.crt")), ca)?;
            cas.push(ca.clone());
        }
        for (i, ca) in certs.ssh_ca_certs.iter().enumerate() {
            fs::write(dir.join(format!("ssh-ca-{i}.pub")), ca)?;
        }
        self.certs = Some(certs);
        let _ = cas;
        Ok(())
    }

    pub fn ssh_public_openssh(&self) -> Result<String> {
        Ok(self.ssh.public_key().to_openssh()?)
    }

    pub fn ssh_cert_raw(&self) -> Result<(String, Vec<u8>)> {
        let raw = self
            .certs
            .as_ref()
            .ok_or_else(|| anyhow!("not joined — missing ssh cert"))?;
        let s = String::from_utf8_lossy(&raw.ssh);
        let mut it = s.split_whitespace();
        let algo = it
            .next()
            .ok_or_else(|| anyhow!("empty ssh cert"))?
            .to_string();
        let b64 = it.next().ok_or_else(|| anyhow!("ssh cert missing blob"))?;
        let blob = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64))?;
        Ok((algo, blob))
    }

    pub fn ssh_private(&self) -> PrivateKey {
        self.ssh.clone()
    }

    /// Teleport reverse-tunnel SSH user is `hostUUID.cluster`.
    pub fn tunnel_user(&self) -> String {
        format!("{}.teleport.local", self.host_id)
    }

    pub fn ssh_ca_keys(&self) -> Vec<PublicKey> {
        let Some(certs) = &self.certs else {
            return Vec::new();
        };
        certs
            .ssh_ca_certs
            .iter()
            .filter_map(|raw| {
                let s = String::from_utf8_lossy(raw);
                PublicKey::from_openssh(s.trim()).ok()
            })
            .collect()
    }
}

fn load_certs(dir: &Path) -> Result<Certs> {
    let ssh = fs::read(dir.join("ssh.cert")).context("ssh.cert")?;
    let tls = fs::read(dir.join("tls.crt")).context("tls.crt")?;
    let mut tls_ca_certs = Vec::new();
    let mut ssh_ca_certs = Vec::new();
    let rd = fs::read_dir(dir)?;
    let mut names: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    names.sort();
    for p in names {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with("tls-ca-") {
            tls_ca_certs.push(fs::read(p)?);
        } else if name.starts_with("ssh-ca-") {
            ssh_ca_certs.push(fs::read(p)?);
        }
    }
    Ok(Certs {
        ssh,
        tls,
        tls_ca_certs,
        ssh_ca_certs,
    })
}

fn write_private(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
        .with_context(|| format!("write {}", path.display()))?
        .write_all(bytes.as_ref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_or_create_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "connect2-agent-id-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&dir);
        let a = Identity::load_or_create(&dir, Some("box".into())).unwrap();
        assert_eq!(a.hostname, "box");
        assert!(!a.host_id.is_empty());
        let b = Identity::load_or_create(&dir, Some("box".into())).unwrap();
        assert_eq!(a.host_id, b.host_id);
        assert_eq!(a.ssh_public_openssh().unwrap(), b.ssh_public_openssh().unwrap());
        let _ = fs::remove_dir_all(&dir);
    }
}
