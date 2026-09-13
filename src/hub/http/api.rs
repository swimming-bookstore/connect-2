use anyhow::Result;
use crate::ai;
use serde_json::Value;

use super::super::view::View;

pub(super) fn json_bytes(v: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(v)?)
}

fn put_str(o: &mut serde_json::Map<String, Value>, k: &str, v: &str) {
    if !v.is_empty() {
        o.insert(k.into(), Value::String(v.into()));
    }
}

pub(super) fn api_me(g: &View) -> Value {
    let connectors: Vec<_> = g
        .connectors
        .iter()
        .map(|(kind, name, display)| {
            serde_json::json!({"kind": kind, "name": name, "display": display})
        })
        .collect();
    serde_json::json!({
        "authed": g.authed,
        "me": g.me,
        "cluster": g.cluster,
        "auth_type": g.auth_type,
        "second_factor": g.second_factor,
        "login_err": g.login_err,
        "login_url": g.login_url,
        "login_wait": g.login_wait,
        "connectors": connectors,
        "grok": ai::info(),
    })
}

fn peer_name(g: &View, dst: &str) -> String {
    g.peers
        .iter()
        .find(|p| p.name == dst || p.id == dst)
        .map(|p| p.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| dst.to_string())
}

fn api_boxes(g: &View) -> Value {
    let peers: Vec<_> = g
        .peers
        .iter()
        .map(|p| {
            // Teleport reverse-tunnel hop is by hostname, not host UUID.
            serde_json::json!({"id": p.name, "name": p.name})
        })
        .collect();
    serde_json::json!({ "peers": peers })
}

fn api_session(g: &View) -> Value {
    let mut o = serde_json::Map::new();
    o.insert(
        "dst".into(),
        Value::String(if g.dst.is_empty() {
            String::new()
        } else {
            peer_name(g, &g.dst)
        }),
    );
    o.insert("kind".into(), Value::String(g.kind.clone()));
    o.insert("video".into(), Value::String(g.video.clone()));
    o.insert("tabs".into(), g.tabs.clone());
    if g.dst.is_empty() {
        return Value::Object(o);
    }
    if g.width != 0 {
        o.insert("width".into(), g.width.into());
    }
    if g.height != 0 {
        o.insert("height".into(), g.height.into());
    }
    put_str(&mut o, "url", &g.url);
    o.insert("stdout".into(), Value::String(g.stdout.clone()));
    put_str(&mut o, "answer", &g.answer);
    if g.tabs.as_array().is_some_and(|a| !a.is_empty()) {
        o.insert("tabs".into(), g.tabs.clone());
    }
    if !g.ice.is_empty() {
        o.insert("ice".into(), Value::Array(g.ice.clone()));
    }
    if g.kind == "browser" && g.video == "webrtc" {
        o.insert("ice_servers".into(), g.ice_servers.clone());
    }
    Value::Object(o)
}

fn api_chat(g: &View) -> Value {
    serde_json::json!({ "log": g.log, "desk": g.desk })
}

pub(super) fn state_json(g: &View) -> Result<Vec<u8>> {
    let mut o = match api_me(g) {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if let Value::Object(b) = api_boxes(g) {
        o.extend(b);
    }
    if let Value::Object(s) = api_session(g) {
        o.extend(s);
    }
    if let Value::Object(c) = api_chat(g) {
        o.extend(c);
    }
    o.insert("last".into(), Value::String(g.last.clone()));
    o.insert("jpeg_n".into(), Value::from(g.jpeg_n));
    o.insert("seq".into(), Value::from(g.seq));
    o.insert("busy".into(), Value::Bool(g.busy));
    Ok(serde_json::to_vec(&Value::Object(o))?)
}
