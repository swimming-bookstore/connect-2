use anyhow::{bail, Result};
use crate::ai;
use crate::protocol::{In, Kind, Video};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use super::super::session::Front;
use super::super::view::{patch, View};

pub(super) async fn on_cmd(view: &watch::Sender<View>, tx: &mpsc::Sender<Front>, body: String) {
    let v: Value = serde_json::from_str(&body).unwrap_or(serde_json::json!({}));
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "home" => {
            if tx.send(Front::Home).await.is_err() {
                tracing::debug!("cmd home dropped");
            }
        }
        "plane_logout" | "cluster_logout" => {
            if tx.send(Front::ClusterLogout).await.is_err() {
                tracing::debug!("cmd logout dropped");
            }
        }
        "cluster_login" => {
            send_front(
                tx,
                Front::ClusterLogin {
                    auth: v
                        .get("auth")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    connector: v
                        .get("connector")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    user: v.get("user").and_then(Value::as_str).unwrap_or("").to_string(),
                    password: v
                        .get("password")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    otp: v.get("otp").and_then(Value::as_str).unwrap_or("").to_string(),
                },
            )
            .await;
        }
        "pick" => {
            if let Some(dst) = v.get("dst").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                send_front(tx, Front::Pick(dst.into())).await;
            }
        }
        "open" => {
            let dst = v
                .get("dst")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| {
                    let g = view.borrow();
                    if !g.dst.is_empty() {
                        Some(g.dst.clone())
                    } else {
                        g.peers.first().map(|p| p.name.clone())
                    }
                });
            let Some(dst) = dst else {
                return;
            };
            let Ok(kind) = parse_kind(v.get("kind").and_then(Value::as_str).unwrap_or("browser")) else {
                return;
            };
            let Ok(video) = parse_video(v.get("video").and_then(Value::as_str).unwrap_or("jpeg")) else {
                return;
            };
            let height = v.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
            send_front(
                tx,
                Front::Open {
                    dst,
                    kind,
                    video,
                    height,
                },
            )
            .await;
        }
        "ask" => {
            let text = v.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if !text.is_empty() {
                send_front(tx, Front::Desk(text.into())).await;
            }
        }
        "agent_ask" => {
            let text = v.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if !text.is_empty() {
                send_front(tx, Front::AgentAsk(text.into())).await;
            }
        }
        "new_desk" => {
            patch(view, |g| g.desk.clear());
        }
        "new_chat" => {
            patch(view, |g| {
                g.log.clear();
                g.thread.clear();
            });
            send_front(tx, Front::Msg(In::AiNew)).await;
        }
        "grok_login" => {
            if let Err(e) = ai::login_start().await {
                patch(view, |g| g.desk.push(format!("error {e:#}")));
            } else {
                patch(view, |_| {});
            }
        }
        "logout" => {
            if let Err(e) = ai::logout().await {
                tracing::debug!("grok logout: {e:#}");
            }
            patch(view, |_| {});
        }
        "grok_tokens" => {
            if let Some(tok) = v.get("tokens") {
                if let Err(e) = ai::adopt_tokens(tok) {
                    tracing::warn!("grok tokens: {e:#}");
                } else {
                    patch(view, |_| {});
                }
            }
        }
        _ => {
            if let Ok(cmd) = serde_json::from_value::<In>(v) {
                if matches!(cmd, In::Offer { .. }) {
                    patch(view, |g| g.want_answer = true);
                }
                send_front(tx, Front::Msg(cmd)).await;
            }
        }
    }
}

async fn send_front(tx: &mpsc::Sender<Front>, msg: Front) {
    if tx.send(msg).await.is_err() {
        tracing::debug!("hub cmd dropped");
    }
}

fn parse_kind(s: &str) -> Result<Kind> {
    match s {
        "browser" => Ok(Kind::Browser),
        "shell" => Ok(Kind::Shell),
        "agent" => Ok(Kind::Agent),
        _ => bail!("kind browser|shell|agent"),
    }
}

fn parse_video(s: &str) -> Result<Video> {
    match s {
        "webrtc" => Ok(Video::Webrtc),
        "jpeg" => Ok(Video::Jpeg),
        "none" => Ok(Video::None),
        _ => bail!("video jpeg|webrtc|none"),
    }
}
