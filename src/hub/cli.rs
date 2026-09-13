//! CLI flags and Teleport hop config.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use crate::protocol::Kind;
use connect2_teleport::user::Cfg;

#[derive(Clone, Parser)]
#[command(
    name = "connect2",
    about = "Talk to connect2-agent through Teleport (no tsh)"
)]
pub struct Cli {
    #[arg(long, default_value = "127.0.0.1:3080")]
    pub proxy: String,
    /// Proxy SSH (`host:3023`). Default: proxy host on :3023.
    #[arg(long)]
    pub ssh: Option<String>,
    /// Auth gRPC (`host:3025`). Default: proxy host on :3025.
    #[arg(long)]
    pub auth: Option<String>,
    #[arg(long, default_value = "/tmp/tp/client/demo")]
    pub identity: PathBuf,
    /// Teleport username (certificate principal). First SSH hop.
    #[arg(long, default_value = "demo")]
    pub teleport_user: String,
    /// OS login on the node. Second SSH hop.
    #[arg(long, default_value = "packer")]
    pub user: String,
    /// Pre-select this node (hostname). Empty = directory only until you pick one.
    #[arg(long, default_value = "")]
    pub node: String,
    #[arg(long, default_value = "teleport.local")]
    pub cluster: String,
    #[arg(long, default_value = "default")]
    pub namespace: String,
    #[arg(long, default_value_t = true)]
    pub insecure: bool,
    #[arg(long, default_value = "127.0.0.1:3056")]
    pub http: String,
    /// Skip the native window (and any browser tab). HTTP still serves e2e.
    #[arg(long, default_value_t = false)]
    pub no_open: bool,
    #[arg(long, default_value_t = 3)]
    pub ls_secs: u64,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Clone, Subcommand)]
pub enum Cmd {
    /// Ping the proxy (`/webapi/ping`) and print cluster + auth connectors.
    Ping,
    /// Teleport login (`tsh login`): OIDC/SAML/GitHub SSO or local password.
    Login {
        /// `oidc`, `saml`, `github`, or `local` (password). Empty = password if no SSO, else first connector.
        #[arg(long)]
        auth: Option<String>,
        /// Connector name (`tsh login --auth=okta`).
        #[arg(long)]
        connector: Option<String>,
        /// Local Teleport username (`--auth=local`).
        #[arg(long)]
        user: Option<String>,
        /// Local password (prompt if omitted).
        #[arg(long)]
        password: Option<String>,
        /// OTP / TOTP token.
        #[arg(long)]
        otp: Option<String>,
    },
    /// List Teleport nodes (Auth inventory, no tsh).
    Ls,
    /// JSON App session on the box (no UI).
    Open {
        #[arg(value_enum)]
        kind: KindArg,
        #[arg(long)]
        jpeg_out: Option<PathBuf>,
        #[arg(long)]
        ask: Option<String>,
        #[arg(long)]
        stdin: Vec<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value_t = 12)]
        wait_secs: u64,
    },
    /// Interactive SSH on the node (`tsh ssh`, no `tsh`).
    Ssh {
        /// `user@host` or host. Default user is `--user`.
        target: String,
        /// Remote command. Empty = login shell.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum KindArg {
    Shell,
    Browser,
    Agent,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Shell => Kind::Shell,
            KindArg::Browser => Kind::Browser,
            KindArg::Agent => Kind::Agent,
        }
    }
}

pub fn cfg_of(cli: &Cli) -> Cfg {
    Cfg {
        proxy: cli.proxy.clone(),
        ssh: cli.ssh.clone(),
        auth: cli.auth.clone(),
        identity: cli.identity.clone(),
        teleport_user: cli.teleport_user.clone(),
        login: cli.user.clone(),
        cluster: cli.cluster.clone(),
        namespace: cli.namespace.clone(),
        insecure: cli.insecure,
    }
}

pub fn pick_login(
    auth: &str,
    connector: Option<&str>,
    info: &connect2_teleport::login::AuthInfo,
) -> Option<(String, String)> {
    if auth == "local" || auth == "password" {
        return None;
    }
    if let Some(name) = connector.filter(|s| !s.is_empty()) {
        if let Some(c) = info
            .connectors
            .iter()
            .find(|c| c.name == name || c.display == name)
        {
            return Some((c.kind.clone(), c.name.clone()));
        }
        let kind = if auth.is_empty() || auth == "sso" {
            "oidc"
        } else {
            auth
        };
        return Some((kind.into(), name.into()));
    }
    if auth == "oidc" || auth == "saml" || auth == "github" || auth == "sso" {
        let want = if auth == "sso" { "" } else { auth };
        let c = info
            .connectors
            .iter()
            .find(|c| want.is_empty() || c.kind == want)?;
        return Some((c.kind.clone(), c.name.clone()));
    }
    if !auth.is_empty() {
        let c = info
            .connectors
            .iter()
            .find(|c| c.name == auth || c.display.eq_ignore_ascii_case(auth))?;
        return Some((c.kind.clone(), c.name.clone()));
    }
    match info.auth_type.as_str() {
        "oidc" | "saml" | "github" => {
            let c = info.connectors.first()?;
            Some((c.kind.clone(), c.name.clone()))
        }
        _ => None,
    }
}

pub fn parse_ssh_target(target: &str, default_user: &str) -> (String, String) {
    if let Some((u, h)) = target.split_once('@') {
        if !u.is_empty() && !h.is_empty() {
            return (u.to_string(), h.to_string());
        }
    }
    (default_user.to_string(), target.to_string())
}
