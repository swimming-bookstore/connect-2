//! Teleport cluster login / logout for the laptop hub.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use connect2_teleport::login::LoginResult;
use connect2_teleport::user;
use tokio::sync::watch;

use super::cli::{self, pick_login, Cli};
use super::view::{patch, Peer, View};

pub async fn refresh_auth(cli: &Arc<Cli>, view: &watch::Sender<View>) {
    let proxy = cli.proxy.clone();
    let insecure = cli.insecure;
    let identity = cli.identity.clone();
    let fallback = cli.teleport_user.clone();
    let authed = connect2_teleport::login::identity_ok(&identity);
    match connect2_teleport::login::ping_auth(&proxy, insecure).await {
        Ok(mut info) => {
            super::lab::inject(&mut info);
            patch(view, |g| {
                g.authed = authed;
                g.cluster = info.cluster;
                g.auth_type = info.auth_type;
                g.second_factor = info.second_factor;
                g.connectors = info
                    .connectors
                    .into_iter()
                    .map(|c| (c.kind, c.name, c.display))
                    .collect();
                if authed && g.me.is_empty() {
                    g.me = fallback.clone();
                }
                if !authed {
                    g.me.clear();
                    g.peers.clear();
                }
            });
        }
        Err(e) => {
            tracing::debug!("auth ping: {e:#}");
            patch(view, |g| {
                g.authed = authed;
                if !authed {
                    g.me.clear();
                    g.peers.clear();
                }
            });
        }
    }
}

pub async fn cluster_logout(cli: &Arc<Cli>, view: &watch::Sender<View>) {
    let identity = cli.identity.clone();
    if let Err(e) = connect2_teleport::login::logout(&identity) {
        tracing::warn!("logout: {e:#}");
        patch(view, |g| g.login_err = format!("{e:#}"));
        return;
    }
    patch(view, |g| {
        g.authed = false;
        g.me.clear();
        g.peers.clear();
        g.login_err.clear();
        g.login_url.clear();
        g.login_wait = false;
        g.dst.clear();
    });
}

pub async fn cluster_login(
    cli: &Arc<Cli>,
    view: &watch::Sender<View>,
    auth: String,
    connector: String,
    user: String,
    password: String,
    otp: String,
) {
    patch(view, |g| {
        g.login_err.clear();
        g.login_url.clear();
        g.login_wait = false;
    });
    let proxy = cli.proxy.clone();
    let identity = cli.identity.clone();
    let insecure = cli.insecure;
    let fallback = cli.teleport_user.clone();
    let info = match connect2_teleport::login::ping_auth(&proxy, insecure).await {
        Ok(mut i) => {
            super::lab::inject(&mut i);
            i
        }
        Err(e) => {
            patch(view, |g| g.login_err = format!("{e:#}"));
            return;
        }
    };
    let r = if let Some((kind, name)) =
        pick_login(&auth, Some(connector.as_str()).filter(|s| !s.is_empty()), &info)
    {
        sso_login(view, &proxy, &identity, &kind, &name, insecure).await
    } else if !password.is_empty() {
        let user = if user.is_empty() {
            fallback.clone()
        } else {
            user
        };
        connect2_teleport::login::login_password(&proxy, &identity, &user, &password, &otp, insecure)
            .await
    } else if let Some((kind, name)) = pick_login("sso", None, &info) {
        sso_login(view, &proxy, &identity, &kind, &name, insecure).await
    } else {
        patch(view, |g| {
            g.login_err = "user, password, and OTP".into();
        });
        return;
    };
    match r {
        Ok(r) => {
            patch(view, |g| {
                g.authed = true;
                g.me = if r.username.is_empty() {
                    fallback
                } else {
                    r.username
                };
                g.login_err.clear();
                g.login_url.clear();
                g.login_wait = false;
            });
            let cfg = cli::cfg_of(cli);
            match user::list_nodes(&cfg).await {
                Ok(nodes) => {
                    patch(view, |g| {
                        g.peers = nodes
                            .into_iter()
                            .map(|n| Peer {
                                id: n.id,
                                name: n.name,
                                tunnel: n.tunnel,
                            })
                            .collect();
                    });
                }
                Err(e) => tracing::warn!("list nodes after login: {e:#}"),
            }
        }
        Err(e) => {
            patch(view, |g| {
                g.authed = false;
                g.login_err = format!("{e:#}");
                g.login_url.clear();
                g.login_wait = false;
            });
        }
    }
}

async fn sso_login(
    view: &watch::Sender<View>,
    proxy: &str,
    identity: &Path,
    kind: &str,
    name: &str,
    insecure: bool,
) -> Result<LoginResult> {
    patch(view, |g| g.login_wait = true);
    let view_url = view.clone();
    if super::lab::active() {
        let identity = identity.to_path_buf();
        let name = name.to_string();
        return tokio::task::spawn_blocking(move || {
            let on_url = |url: String| {
                patch(&view_url, |g| g.login_url = url);
            };
            match super::lab::login(&identity, &name, on_url) {
                Some(r) => r,
                None => anyhow::bail!("dummy sso not configured"),
            }
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
    }
    connect2_teleport::login::login_sso_notify(proxy, identity, kind, name, insecure, move |url| {
        patch(&view_url, |g| g.login_url = url);
    })
    .await
}
