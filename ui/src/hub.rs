//! Loopback hub: HTTP, live WS, theme, xterm/RTC ports.

use crate::grok;
use crate::pane::{self, VideoPref};
use crate::state::State;
use futures_util::StreamExt;
use leptos::prelude::*;
use serde_json::json;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

thread_local! {
    static LIVE_WS: RefCell<Option<(web_sys::WebSocket, Closure<dyn FnMut(web_sys::MessageEvent)>)>> =
        const { RefCell::new(None) };
    static HOP_WANT: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn hub() -> String {
    if let Some(w) = web_sys::window() {
        if let Ok(v) = js_sys::Reflect::get(&w, &"__CONNECT2_HUB".into()) {
            if let Some(s) = v.as_string() {
                let s = s.trim_end_matches('/').to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }
        if let Ok(origin) = w.location().origin() {
            if origin.starts_with("http://") || origin.starts_with("https://") {
                return origin;
            }
        }
    }
    String::new()
}

pub fn api(path: &str) -> String {
    pane::hub_api(&hub(), path)
}

pub async fn cmd(v: serde_json::Value) {
    if let Ok(req) = gloo_net::http::Request::post(&api("/cmd"))
        .header("content-type", "application/json")
        .body(v.to_string())
    {
        if let Err(e) = req.send().await {
            web_sys::console::error_1(&format!("cmd: {e}").into());
        }
    }
}

pub fn go_root() {
    if let Some(w) = web_sys::window() {
        let path = w.location().pathname().unwrap_or_default();
        if path != "/" && !path.is_empty() {
            let _ = w
                .history()
                .and_then(|h| h.replace_state_with_url(&JsValue::NULL, "", Some("/")));
        }
    }
}

pub fn path_now() -> String {
    web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_default()
}

pub fn push_path(path: &str) {
    if let Some(win) = web_sys::window() {
        let cur = win.location().pathname().unwrap_or_default();
        if cur == path {
            return;
        }
        let _ = win
            .history()
            .and_then(|h| h.push_state_with_url(&JsValue::NULL, "", Some(path)));
    }
}

pub fn theme_now() -> String {
    if let Some(w) = web_sys::window() {
        if let Ok(Some(s)) = w.local_storage() {
            if let Ok(Some(t)) = s.get_item("connect2-theme") {
                if t == "dark" || t == "light" {
                    return t;
                }
            }
        }
    }
    "light".into()
}

pub fn video_now() -> VideoPref {
    if let Some(w) = web_sys::window() {
        if let Ok(Some(s)) = w.local_storage() {
            if let Ok(Some(t)) = s.get_item("connect2-video") {
                return VideoPref::parse(&t);
            }
        }
    }
    VideoPref::Jpeg
}

pub fn save_video(pref: VideoPref) {
    if let Some(w) = web_sys::window() {
        if let Ok(Some(s)) = w.local_storage() {
            let _ = s.set_item("connect2-video", pref.as_str());
        }
    }
}

pub fn close_nav_menu() {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = el.blur();
    }
}

pub fn apply_theme(theme: &str) {
    let name = if theme == "dark" { "dark" } else { "light" };
    if let Some(w) = web_sys::window() {
        if let Some(doc) = w.document() {
            if let Some(el) = doc.document_element() {
                let _ = el.set_attribute("data-theme", name);
            }
        }
        if let Ok(Some(s)) = w.local_storage() {
            let _ = s.set_item("connect2-theme", theme);
        }
    }
}

pub fn js_call(name: &str, arg: &JsValue) {
    if let Some(w) = web_sys::window() {
        if let Ok(f) = js_sys::Reflect::get(&w, &name.into()) {
            if let Ok(f) = f.dyn_into::<js_sys::Function>() {
                let _ = f.call1(&w, arg);
            }
        }
    }
}

pub fn term_write(s: &str) {
    js_call("connectTermWrite", &JsValue::from_str(s));
}

fn paint_term(prev: &State, s: &State) {
    if prev.dst != s.dst {
        term_reset();
        if !s.stdout.is_empty() {
            term_write(&s.stdout);
        }
        return;
    }
    if prev.stdout == s.stdout {
        return;
    }
    if s.stdout.starts_with(&prev.stdout) && !prev.stdout.is_empty() {
        term_write(&s.stdout[prev.stdout.len()..]);
        return;
    }
    term_reset();
    if !s.stdout.is_empty() {
        term_write(&s.stdout);
    }
}

pub fn term_reset() {
    js_call("connectTermReset", &JsValue::UNDEFINED);
}

pub fn term_fit() {
    js_call("connectTermFit", &JsValue::UNDEFINED);
}

/// Until the hub session matches this dst (empty = home), ignore leftover PTY/JPEG from the previous box.
pub fn set_hop_want(dst: Option<String>) {
    HOP_WANT.with(|s| *s.borrow_mut() = dst);
}

pub fn rtc_close() {
    js_call("connectRtcClose", &JsValue::UNDEFINED);
}

pub fn rtc_apply(s: &State) {
    let v = json!({
        "width": s.width,
        "height": s.height,
        "answer": s.answer,
        "ice": s.ice.iter().map(|c| json!({
            "candidate": c.candidate,
            "sdp_mid": c.sdp_mid,
            "sdp_mline_index": c.sdp_mline_index,
        })).collect::<Vec<_>>(),
        "ice_servers": s.ice_servers.iter().map(|c| json!({
            "urls": c.urls,
            "username": c.username,
            "credential": c.credential,
        })).collect::<Vec<_>>(),
    });
    if let Ok(js) = js_sys::JSON::parse(&v.to_string()) {
        js_call("connectRtcApply", &js);
    }
}

async fn get_json<T: for<'de> serde::de::DeserializeOwned>(path: &str) -> Option<T> {
    gloo_net::http::Request::get(path)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()
}

fn apply_live(st: RwSignal<State>, mut s: State) {
    let prev = st.get_untracked();
    let newer = pane::live_is_newer(prev.seq, s.seq);
    if s.grok.configured {
        if let Some(t) = s.grok.tokens.clone() {
            grok::save(&t);
        }
    } else {
        grok::clear();
        s.grok.tokens = None;
    }
    let want = HOP_WANT.with(|h| h.borrow().clone());
    if let Some(ref w) = want {
        if &s.dst != w {
            if newer {
                st.update(|p| {
                    p.peers = s.peers;
                    p.me = s.me;
                    p.cluster = s.cluster;
                    p.authed = s.authed;
                    p.login_err = s.login_err;
                    p.login_url = s.login_url;
                    p.login_wait = s.login_wait;
                    p.second_factor = s.second_factor;
                    p.connectors = s.connectors;
                    p.grok = s.grok;
                    p.seq = s.seq;
                    p.busy = s.busy;
                    p.last = s.last;
                });
            }
            return;
        }
        HOP_WANT.with(|h| *h.borrow_mut() = None);
    }
    if newer {
        paint_term(&prev, &s);
        if s != prev {
            st.set(s);
        }
    }
}

pub async fn live(st: RwSignal<State>) {
    if let Some(v) = grok::restore_cmd() {
        cmd(v).await;
    }
    if let Some(s) = get_json::<State>(&api("/state")).await {
        apply_live(st, s);
    }
    let Some(url) = pane::hub_ws(&hub()) else {
        return;
    };
    let (tx, mut rx) = futures_channel::mpsc::unbounded::<String>();
    let Ok(ws) = web_sys::WebSocket::new(&url) else {
        return;
    };
    let cb = Closure::wrap(Box::new(move |ev: web_sys::MessageEvent| {
        if let Some(txt) = ev.data().as_string() {
            let _ = tx.unbounded_send(txt);
        }
    }) as Box<dyn FnMut(_)>);
    ws.set_onmessage(Some(cb.as_ref().unchecked_ref()));
    LIVE_WS.with(|slot| {
        if let Some((old, _)) = slot.borrow_mut().take() {
            let _ = old.close();
        }
        *slot.borrow_mut() = Some((ws, cb));
    });
    while let Some(txt) = rx.next().await {
        if let Ok(s) = serde_json::from_str::<State>(&txt) {
            apply_live(st, s);
        }
    }
}
