//! List nodes, dial `connect-app`, send one JSON line.
//!
//! Identity file is a Teleport user cert (same as `tsh --identity`).
//!
//! ```
//! cargo run -p connect2-teleport --example laptop
//! ```

use std::path::PathBuf;

use anyhow::Result;
use connect2_teleport::user::{self, Cfg};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cfg = Cfg {
        proxy: "127.0.0.1:3080".into(),
        ssh: None,
        auth: None,
        identity: PathBuf::from("./user"),
        teleport_user: "demo".into(),
        login: "packer".into(),
        cluster: "teleport.local".into(),
        namespace: "default".into(),
        insecure: true,
    };

    let nodes = user::list_nodes(&cfg).await?;
    let node = nodes
        .iter()
        .find(|n| n.tunnel)
        .ok_or_else(|| anyhow::anyhow!("no online node"))?;

    let mut sess = user::dial_app(&cfg, &node.name).await?;
    sess.write_all(b"{\"hello\":true}\n").await?;

    let mut buf = [0u8; 4096];
    let n = sess.read(&mut buf).await?;
    print!("{}", String::from_utf8_lossy(&buf[..n]));
    Ok(())
}
