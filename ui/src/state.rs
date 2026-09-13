//! Hub `/state` JSON.

use serde::Deserialize;

use crate::grok;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Peer {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct IceCand {
    #[serde(default)]
    pub candidate: String,
    #[serde(default)]
    pub sdp_mid: Option<String>,
    #[serde(default)]
    pub sdp_mline_index: Option<u16>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct IceServer {
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub credential: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Tab {
    pub id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub active: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Grok {
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub user_code: Option<String>,
    #[serde(default)]
    pub verification_uri: Option<String>,
    #[serde(default)]
    pub tokens: Option<grok::Tokens>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Connector {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct State {
    #[serde(default)]
    pub me: String,
    #[serde(default)]
    pub cluster: String,
    #[serde(default)]
    pub authed: bool,
    #[serde(default)]
    pub login_err: String,
    #[serde(default)]
    pub login_url: String,
    #[serde(default)]
    pub login_wait: bool,
    #[serde(default)]
    pub second_factor: String,
    #[serde(default)]
    pub connectors: Vec<Connector>,
    #[serde(default)]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub dst: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub video: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub ice: Vec<IceCand>,
    #[serde(default)]
    pub ice_servers: Vec<IceServer>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub log: Vec<String>,
    #[serde(default)]
    pub desk: Vec<String>,
    #[serde(default)]
    pub grok: Grok,
    #[serde(default)]
    pub jpeg_n: u64,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub last: String,
}
