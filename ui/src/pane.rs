//! Session work + browser video preference. Native-testable (no WASM).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Shell,
    Browser,
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPref {
    Jpeg,
    Turn720,
    Turn1080,
}

impl Work {
    pub fn as_str(self) -> &'static str {
        match self {
            Work::Shell => "shell",
            Work::Browser => "browser",
            Work::Agent => "agent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "shell" => Some(Work::Shell),
            "browser" | "jpeg" | "turn" | "turn720" | "turn1080" => Some(Work::Browser),
            "agent" => Some(Work::Agent),
            _ => None,
        }
    }
}

impl VideoPref {
    pub fn as_str(self) -> &'static str {
        match self {
            VideoPref::Jpeg => "jpeg",
            VideoPref::Turn720 => "turn720",
            VideoPref::Turn1080 => "turn1080",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "webrtc" | "turn" | "turn720" => VideoPref::Turn720,
            "turn1080" | "1080" => VideoPref::Turn1080,
            _ => VideoPref::Jpeg,
        }
    }

    pub fn open_args(self) -> (&'static str, &'static str, u32) {
        match self {
            VideoPref::Jpeg => ("browser", "jpeg", 720),
            VideoPref::Turn720 => ("browser", "webrtc", 720),
            VideoPref::Turn1080 => ("browser", "webrtc", 1080),
        }
    }

    pub fn is_turn(self) -> bool {
        matches!(self, VideoPref::Turn720 | VideoPref::Turn1080)
    }
}

pub fn parse_pane(s: &str) -> Option<(Work, Option<VideoPref>)> {
    match s {
        "shell" => Some((Work::Shell, None)),
        "agent" => Some((Work::Agent, None)),
        "browser" | "jpeg" => Some((Work::Browser, Some(VideoPref::Jpeg))),
        "turn" | "turn720" | "webrtc" => Some((Work::Browser, Some(VideoPref::Turn720))),
        "turn1080" => Some((Work::Browser, Some(VideoPref::Turn1080))),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub box_name: Option<String>,
    pub work: Work,
    pub pref: Option<VideoPref>,
}

pub fn is_settings_path(raw: &str) -> bool {
    let mut p = raw.trim();
    if let Some(i) = p.find('?') {
        p = &p[..i];
    }
    if let Some(i) = p.find('#') {
        p = &p[..i];
    }
    p.trim_end_matches('/') == "/settings"
}

pub fn parse_route(raw: &str) -> Option<Route> {
    let mut h = raw.trim();
    if let Some(i) = h.find('#') {
        if i == 0 || h[..i].trim().trim_end_matches('/').is_empty() {
            h = &h[i..];
        }
    }
    h = h.trim_start_matches('#');
    h = h.trim_start_matches('/');
    if let Some(rest) = h.strip_prefix("boxes/") {
        h = rest;
    } else if h == "boxes" || h.is_empty() {
        return None;
    }
    if h.is_empty() {
        return None;
    }
    if let Some((box_part, pane)) = h.rsplit_once('/') {
        let (work, pref) = parse_pane(pane)?;
        let box_name = decode_box(box_part);
        if box_name.is_empty() {
            return None;
        }
        Some(Route {
            box_name: Some(box_name),
            work,
            pref,
        })
    } else {
        let (work, pref) = parse_pane(h)?;
        Some(Route {
            box_name: None,
            work,
            pref,
        })
    }
}

pub fn parse_hash(hash: &str) -> Option<(Work, Option<VideoPref>)> {
    parse_route(hash).map(|r| (r.work, r.pref))
}

pub fn pane_slug(work: Work, pref: VideoPref) -> &'static str {
    match work {
        Work::Shell => "shell",
        Work::Agent => "agent",
        Work::Browser => pref.as_str(),
    }
}

pub fn route_path(box_name: &str, work: Work, pref: VideoPref) -> String {
    let pane = pane_slug(work, pref);
    if box_name.is_empty() {
        format!("/{pane}")
    } else {
        format!("/boxes/{}/{}", encode_box(box_name), pane)
    }
}

fn encode_box(name: &str) -> String {
    let mut out = String::new();
    for b in name.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode_box(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = from_hex(bytes[i + 1]);
            let lo = from_hex(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn work_from_kind(kind: &str) -> Work {
    match kind {
        "browser" => Work::Browser,
        "agent" => Work::Agent,
        _ => Work::Shell,
    }
}

pub fn video_from_state(video: &str, height: u32) -> VideoPref {
    match video {
        "webrtc" if height >= 1080 => VideoPref::Turn1080,
        "webrtc" => VideoPref::Turn720,
        _ => VideoPref::Jpeg,
    }
}

pub fn work_open(work: Work, pref: VideoPref) -> (&'static str, &'static str, u32) {
    match work {
        Work::Shell => ("shell", "none", 0),
        Work::Agent => ("agent", "none", 0),
        Work::Browser => pref.open_args(),
    }
}

pub fn tab_label(title: &str, url: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return title.into();
    }
    let u = url.trim();
    if u.is_empty() || u == "about:blank" {
        "about:blank".into()
    } else {
        let rest = u.split("://").nth(1).unwrap_or(u);
        rest.split('/').next().unwrap_or(rest).to_string()
    }
}

pub fn hub_ws(http: &str) -> Option<String> {
    let http = http.trim_end_matches('/');
    if let Some(rest) = http.strip_prefix("https://") {
        Some(format!("wss://{rest}/live"))
    } else if let Some(rest) = http.strip_prefix("http://") {
        Some(format!("ws://{rest}/live"))
    } else {
        None
    }
}

pub fn hub_api(http: &str, path: &str) -> String {
    format!("{}{path}", http.trim_end_matches('/'))
}

/// Hub `seq` is monotonic (starts at 1). Drop frames that are not newer.
pub fn live_is_newer(prev_seq: u64, seq: u64) -> bool {
    seq > prev_seq
}

/// Room spinner stays until the hop has something to show (or it failed).
pub fn hop_ready(
    work: Work,
    turn: bool,
    busy: bool,
    stdout: &str,
    jpeg_n: u64,
    width: u32,
    answer: &str,
    kind: &str,
    last: &str,
    dst: &str,
    want: &str,
) -> bool {
    if !want.is_empty() && dst != want {
        return false;
    }
    if last.starts_with("open ") {
        return true;
    }
    if busy {
        return false;
    }
    match work {
        Work::Shell => !stdout.is_empty(),
        Work::Browser => {
            if turn {
                width != 0 || !answer.is_empty()
            } else {
                jpeg_n > 0
            }
        }
        Work::Agent => kind == "agent",
    }
}

pub fn chat_row(line: &str) -> Option<(&'static str, String)> {
    if let Some(t) = line.strip_prefix("you ") {
        Some(("me", t.to_string()))
    } else if let Some(t) = line.strip_prefix("ai ") {
        Some(("bot", t.to_string()))
    } else if line.starts_with("step ") || line.starts_with("error ") {
        Some(("tool", line.to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hash_all() {
        assert_eq!(parse_hash(""), None);
        assert_eq!(parse_hash("#shell"), Some((Work::Shell, None)));
        assert_eq!(parse_hash("#box-1/shell"), Some((Work::Shell, None)));
        assert_eq!(
            parse_route("/boxes/box-1/shell").and_then(|r| r.box_name),
            Some("box-1".into())
        );
        assert_eq!(
            route_path("box-1", Work::Shell, VideoPref::Jpeg),
            "/boxes/box-1/shell"
        );
    }

    #[test]
    fn work_parse_aliases() {
        assert_eq!(Work::parse("jpeg"), Some(Work::Browser));
        assert_eq!(VideoPref::Jpeg.open_args(), ("browser", "jpeg", 720));
        assert!(VideoPref::Turn720.is_turn());
        assert_eq!(work_open(Work::Shell, VideoPref::Turn1080), ("shell", "none", 0));
    }

    #[test]
    fn from_state() {
        assert_eq!(work_from_kind("browser"), Work::Browser);
        assert_eq!(video_from_state("webrtc", 1080), VideoPref::Turn1080);
        assert_eq!(tab_label(" Example ", "https://x"), "Example");
        assert_eq!(chat_row("you hi"), Some(("me", "hi".into())));
        assert_eq!(chat_row("ai hello"), Some(("bot", "hello".into())));
        assert_eq!(chat_row("step ran"), Some(("tool", "step ran".into())));
        assert_eq!(chat_row("noise"), None);
    }

    #[test]
    fn encode_decode_box_and_route() {
        assert_eq!(encode_box("box 1"), "box%201");
        assert_eq!(decode_box("box%201"), "box 1");
        assert_eq!(
            parse_route("/boxes/box-2/turn1080").unwrap(),
            Route {
                box_name: Some("box-2".into()),
                work: Work::Browser,
                pref: Some(VideoPref::Turn1080),
            }
        );
        assert_eq!(parse_route("/boxes"), None);
        assert!(is_settings_path("/settings"));
        assert!(is_settings_path("/settings/"));
        assert!(!is_settings_path("/boxes/box-1/shell"));
        assert_eq!(parse_hash("#agent"), Some((Work::Agent, None)));
        assert_eq!(
            route_path("box-3", Work::Browser, VideoPref::Jpeg),
            "/boxes/box-3/jpeg"
        );
        assert_eq!(pane_slug(Work::Browser, VideoPref::Turn720), "turn720");
        assert_eq!(VideoPref::parse("1080"), VideoPref::Turn1080);
        assert_eq!(VideoPref::parse("nope"), VideoPref::Jpeg);
    }

    #[test]
    fn tab_host_fallback() {
        assert_eq!(tab_label("", "about:blank"), "about:blank");
        assert_eq!(tab_label("", "https://x"), "x");
    }

    #[test]
    fn hub_urls_for_bundled_window() {
        assert_eq!(
            hub_ws("http://127.0.0.1:3056/"),
            Some("ws://127.0.0.1:3056/live".into())
        );
        assert_eq!(
            hub_ws("https://connect.example"),
            Some("wss://connect.example/live".into())
        );
        assert_eq!(hub_ws("tauri://localhost"), None);
        assert_eq!(
            hub_api("http://127.0.0.1:3056/", "/shot?3"),
            "http://127.0.0.1:3056/shot?3"
        );
        assert_eq!(
            hub_api("http://127.0.0.1:3056", "/cmd"),
            "http://127.0.0.1:3056/cmd"
        );
    }

    #[test]
    fn hop_ready_shell_waits_pty() {
        let r = |work, busy, stdout, jpeg, width, kind, last, dst, want| {
            hop_ready(work, false, busy, stdout, jpeg, width, "", kind, last, dst, want)
        };
        assert!(!r(Work::Shell, true, "", 0, 0, "shell", "", "box-1", "box-1"));
        assert!(!r(Work::Shell, false, "", 0, 0, "shell", "", "box-1", "box-1"));
        assert!(!r(Work::Shell, false, "packer@box-1$", 0, 0, "shell", "", "box-1", "box-2"));
        assert!(r(Work::Shell, false, "packer@box-1$", 0, 0, "shell", "", "box-1", "box-1"));
        assert!(r(Work::Shell, false, "", 0, 0, "shell", "open failed", "box-2", "box-2"));
        assert!(r(Work::Agent, false, "", 0, 0, "agent", "", "box-3", "box-3"));
        assert!(hop_ready(Work::Browser, false, false, "", 1, 0, "", "browser", "", "b", "b"));
        assert!(!hop_ready(Work::Browser, true, false, "", 1, 0, "", "browser", "", "b", "b"));
        assert!(hop_ready(Work::Browser, true, false, "", 0, 1280, "", "browser", "", "b", "b"));
    }

    #[test]
    fn live_seq_drops_stale() {
        assert!(!live_is_newer(0, 0));
        assert!(live_is_newer(0, 3));
        assert!(live_is_newer(2, 3));
        assert!(!live_is_newer(3, 3));
        assert!(!live_is_newer(4, 3));
    }
}
