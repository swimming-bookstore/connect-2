//! Live laptop state.

use serde_json::Value;
use tokio::sync::watch;

#[derive(Clone)]
pub struct View {
    pub me: String,
    pub peers: Vec<Peer>,
    pub jpeg: Option<Vec<u8>>,
    pub url: String,
    pub tabs: Value,
    pub stdout: String,
    pub last: String,
    pub channel: String,
    pub dst: String,
    pub kind: String,
    pub video: String,
    pub width: u32,
    pub height: u32,
    pub log: Vec<String>,
    pub desk: Vec<String>,
    pub thread: Vec<Value>,
    pub answer: String,
    pub ice: Vec<Value>,
    pub ice_servers: Value,
    pub height_want: u32,
    pub want_answer: bool,
    pub jpeg_n: u64,
    pub seq: u64,
    pub busy: bool,
    pub authed: bool,
    pub cluster: String,
    pub auth_type: String,
    pub login_err: String,
    pub login_url: String,
    pub login_wait: bool,
    pub second_factor: String,
    pub connectors: Vec<(String, String, String)>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub tunnel: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            me: "demo".into(),
            peers: Vec::new(),
            jpeg: None,
            url: String::new(),
            tabs: serde_json::json!([]),
            stdout: String::new(),
            last: String::new(),
            channel: String::new(),
            dst: String::new(),
            kind: String::new(),
            video: String::new(),
            width: 0,
            height: 0,
            log: Vec::new(),
            desk: Vec::new(),
            thread: Vec::new(),
            answer: String::new(),
            ice: Vec::new(),
            ice_servers: serde_json::json!([]),
            height_want: 0,
            want_answer: false,
            jpeg_n: 0,
            seq: 1,
            busy: false,
            authed: false,
            cluster: String::new(),
            auth_type: String::new(),
            login_err: String::new(),
            login_url: String::new(),
            login_wait: false,
            second_factor: String::new(),
            connectors: Vec::new(),
        }
    }
}


pub fn patch(view: &watch::Sender<View>, f: impl FnOnce(&mut View)) {
    let mut g = view.borrow().clone();
    f(&mut g);
    g.seq = g.seq.wrapping_add(1);
    if view.send(g).is_err() {
        tracing::debug!("view closed");
    }
}
