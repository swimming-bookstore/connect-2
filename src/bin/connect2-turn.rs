//! Lab TURN (UDP :3478). Relays WebRTC when host ICE is not enough.
//!
//!     connect2-turn --public-ip 127.0.0.1 --user connect --pass connect-lab

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::net::UdpSocket;
use turn::auth::*;
use turn::relay::relay_static::*;
use turn::server::config::*;
use turn::server::*;
use webrtc_util::vnet::net::Net;

struct StaticAuth {
    creds: HashMap<String, Vec<u8>>,
}

impl AuthHandler for StaticAuth {
    fn auth_handle(&self, username: &str, _realm: &str, _src: SocketAddr) -> Result<Vec<u8>, turn::Error> {
        self.creds
            .get(username)
            .cloned()
            .ok_or(turn::Error::ErrFakeErr)
    }
}

#[derive(Parser)]
#[command(name = "connect2-turn", about = "UDP TURN for the Connect 2 lab")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1")]
    public_ip: String,
    #[arg(long, default_value_t = 3478)]
    port: u16,
    #[arg(long, default_value = "connect2")]
    realm: String,
    #[arg(long, default_value = "connect")]
    user: String,
    #[arg(long, default_value = "connect-lab")]
    pass: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "connect2_turn=info".into()),
        )
        .compact()
        .init();
    let cli = Cli::parse();
    let mut creds = HashMap::new();
    creds.insert(
        cli.user.clone(),
        generate_auth_key(&cli.user, &cli.realm, &cli.pass),
    );
    let conn = Arc::new(
        UdpSocket::bind(("0.0.0.0", cli.port))
            .await
            .with_context(|| format!("bind UDP :{}", cli.port))?,
    );
    tracing::info!(addr = %conn.local_addr()?, public = %cli.public_ip, user = %cli.user, "turn");
    let public_ip = IpAddr::from_str(&cli.public_ip).with_context(|| cli.public_ip.clone())?;
    let server = Server::new(ServerConfig {
        conn_configs: vec![ConnConfig {
            conn,
            relay_addr_generator: Box::new(RelayAddressGeneratorStatic {
                relay_address: public_ip,
                address: "0.0.0.0".into(),
                net: Arc::new(Net::new(None)),
            }),
        }],
        realm: cli.realm,
        auth_handler: Arc::new(StaticAuth { creds }),
        channel_bind_timeout: Duration::from_secs(0),
        alloc_close_notify: None,
    })
    .await
    .context("turn server")?;
    tokio::signal::ctrl_c().await.ok();
    server.close().await.ok();
    Ok(())
}
