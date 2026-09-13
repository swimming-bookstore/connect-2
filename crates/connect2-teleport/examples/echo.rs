//! Join as a Node, keep the reverse tunnel open, echo JSON on `connect-app`.
//!
//! ```
//! tctl tokens add --type=node --ttl=15m
//! TELEPORT_TOKEN=... cargo run -p connect2-teleport --example box
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use connect2_teleport::identity::Identity;
use connect2_teleport::join;
use connect2_teleport::plane::{self, App};
use connect2_teleport::teleport::{self, AppRouter, TunnelCfg};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let dir = PathBuf::from("./identity");
    let proxy = "127.0.0.1:3080";
    let insecure = true;

    let mut id = Identity::load_or_create(&dir, Some("box-1".into()))?;
    if id.certs.is_none() {
        let token = std::env::var("TELEPORT_TOKEN")?;
        let certs = join::register(proxy, &token, &id, None, insecure).await?;
        id.save_certs(&dir, certs)?;
    }

    let (app_tx, mut app_rx) = mpsc::channel::<App>(64);
    let router = AppRouter::new();
    let cfg = TunnelCfg {
        proxy: proxy.into(),
        tunnel: Some("127.0.0.1:3024".into()),
        insecure,
        tls_ca: None,
    };

    let tunnel_id = Arc::new(id);
    let tunnel_router = router.clone();
    tokio::spawn(async move {
        let _ = teleport::run(tunnel_id, cfg, app_tx, tunnel_router).await;
    });

    while let Some(a) = app_rx.recv().await {
        router.send(plane::app(&a.src, &a.channel, a.data)).await;
    }
    Ok(())
}
