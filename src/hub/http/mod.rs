//! Loopback HTTP + WebSocket hub.

mod api;
mod assets;
mod cmd;

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use super::session::Front;
use super::view::View;

use self::api::{api_me, json_bytes, state_json};
use self::assets::{app_css, index_html, ui_snippet, PORTS_JS, UI_JS, UI_WASM, XTERM_CSS, XTERM_FIT, XTERM_JS};
use self::cmd::on_cmd;

pub async fn serve(bind: String, view: watch::Sender<View>, tx: mpsc::Sender<Front>) -> Result<()> {
    if !loopback_bind(&bind) {
        bail!("http bind {bind} is not loopback");
    }
    let lis = TcpListener::bind(&bind).await?;
    loop {
        let (s, _) = lis.accept().await?;
        let view = view.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(s, view, tx).await {
                tracing::debug!("http conn: {e:#}");
            }
        });
    }
}

fn loopback_bind(bind: &str) -> bool {
    if let Ok(a) = bind.parse::<std::net::SocketAddr>() {
        return a.ip().is_loopback();
    }
    let host = bind.rsplit_once(':').map(|(h, _)| h).unwrap_or(bind);
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

fn spa_path(path: &str) -> bool {
    let p = path.trim_end_matches('/');
    p == "/boxes"
        || p == "/settings"
        || p.starts_with("/boxes/")
        || matches!(
            p,
            "/shell" | "/agent" | "/browser" | "/jpeg" | "/turn" | "/turn720" | "/turn1080"
        )
}

async fn read_http(s: &mut TcpStream) -> Result<(String, String, String, Vec<u8>)> {
    let mut raw = Vec::new();
    let mut tmp = [0u8; 8192];
    let hdr_end = loop {
        let n = s.read(&mut tmp).await?;
        if n == 0 {
            break None;
        }
        raw.extend_from_slice(&tmp[..n]);
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(i);
        }
        if raw.len() > 1024 * 1024 {
            bail!("headers too large");
        }
    };
    let Some(hdr_end) = hdr_end else {
        bail!("no headers");
    };
    let head = String::from_utf8_lossy(&raw[..hdr_end]).into_owned();
    let mut body = raw.split_off(hdr_end + 4);
    let want = head.lines().find_map(|l| {
        l.split_once(':').and_then(|(k, v)| {
            if k.eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
    });
    if let Some(want) = want {
        while body.len() < want {
            let n = s.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(want);
    }
    let line = head.lines().next().unwrap_or("");
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let path = it
        .next()
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_string();
    Ok((method, path, head, body))
}

async fn handle(
    mut s: TcpStream,
    view: watch::Sender<View>,
    tx: mpsc::Sender<Front>,
) -> Result<()> {
    let (method, path, head, body) = read_http(&mut s).await?;
    if method == "OPTIONS" {
        let h = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Allow-Methods: GET, POST, HEAD, OPTIONS\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        s.write_all(h.as_bytes()).await?;
        return Ok(());
    }
    if method == "GET" && path == "/live" {
        if header_value(&head, "Upgrade")
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        {
            return live_ws(s, view, &head).await;
        }
        let h = "HTTP/1.1 426 Upgrade Required\r\nUpgrade: websocket\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nwebsocket";
        s.write_all(h.as_bytes()).await?;
        return Ok(());
    }
    let g = view.borrow().clone();
    let verb = if method == "HEAD" { "GET" } else { method.as_str() };
    let (code, ctype, body) = match (verb, path.as_str()) {
        ("GET", "/" | "") => (200, "text/html; charset=utf-8", index_html().into_bytes()),
        ("GET", "/favicon.ico") => (204, "image/x-icon", Vec::new()),
        ("GET", "/pkg/connect2_ui.js") => {
            #[cfg(feature = "web")]
            {
                (
                    200,
                    "text/javascript; charset=utf-8",
                    UI_JS.as_bytes().to_vec(),
                )
            }
            #[cfg(not(feature = "web"))]
            {
                (404, "text/plain", b"no web ui".to_vec())
            }
        }
        ("GET", "/pkg/connect2_ui_bg.wasm") => {
            #[cfg(feature = "web")]
            {
                (200, "application/wasm", UI_WASM.to_vec())
            }
            #[cfg(not(feature = "web"))]
            {
                (404, "text/plain", b"no web ui".to_vec())
            }
        }
        ("GET", p) if p.starts_with("/pkg/snippets/") => match ui_snippet(p) {
            Some(b) => (200, "text/javascript; charset=utf-8", b.to_vec()),
            None => (404, "text/plain", b"no".to_vec()),
        },
        ("GET", "/ports.js") => (200, "text/javascript; charset=utf-8", PORTS_JS.as_bytes().to_vec()),
        ("GET", "/style.css") => (200, "text/css; charset=utf-8", app_css()),
        ("GET", "/xterm.css") => (200, "text/css; charset=utf-8", XTERM_CSS.as_bytes().to_vec()),
        ("GET", "/xterm.js") => (
            200,
            "text/javascript; charset=utf-8",
            XTERM_JS.as_bytes().to_vec(),
        ),
        ("GET", "/xterm-addon-fit.js") => (
            200,
            "text/javascript; charset=utf-8",
            XTERM_FIT.as_bytes().to_vec(),
        ),
        ("GET", "/shot") => match g.jpeg {
            Some(j) => (200, "image/jpeg", j),
            None => (204, "text/plain", Vec::new()),
        },
        ("GET", "/api/me") => (200, "application/json", json_bytes(&api_me(&g))?),
        ("GET", "/state") => (200, "application/json", state_json(&g)?),
        ("POST", "/cmd") => {
            on_cmd(&view, &tx, String::from_utf8_lossy(&body).into_owned()).await;
            (200, "text/plain", b"ok".to_vec())
        }
        _ if method == "GET" && spa_path(path.as_str()) => {
            (200, "text/html; charset=utf-8", index_html().into_bytes())
        }
        _ => (404, "text/plain", b"no".to_vec()),
    };
    let cache = if path.starts_with("/pkg/")
        || path == "/ports.js"
        || path.starts_with("/xterm")
        || path == "/style.css"
    {
        "Cache-Control: no-store\r\n"
    } else {
        ""
    };
    let reason = match code {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "OK",
    };
    let h = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: content-type\r\n{cache}Connection: close\r\n\r\n",
        body.len()
    );
    s.write_all(h.as_bytes()).await?;
    if method != "HEAD" {
        s.write_all(&body).await?;
    }
    Ok(())
}

fn header_value(head: &str, name: &str) -> Option<String> {
    for line in head.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.eq_ignore_ascii_case(name) {
            return Some(v.trim().to_string());
        }
    }
    None
}

pub fn ws_accept_key(key: &str) -> String {
    const MAGIC: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(key.as_bytes());
    h.update(MAGIC);
    STANDARD.encode(h.finalize())
}

fn ws_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + payload.len());
    out.push(0x80 | opcode);
    let n = payload.len();
    if n < 126 {
        out.push(n as u8);
    } else if n < 65536 {
        out.push(126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

async fn live_ws(mut s: TcpStream, view: watch::Sender<View>, head: &str) -> Result<()> {
    let key = header_value(head, "Sec-WebSocket-Key").ok_or_else(|| anyhow!("no websocket key"))?;
    let accept = ws_accept_key(&key);
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nAccess-Control-Allow-Origin: *\r\n\r\n"
    );
    s.write_all(resp.as_bytes()).await?;
    s.flush().await?;
    let (mut rd, mut wr) = s.into_split();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });
    let mut rx = view.subscribe();
    loop {
        let body = state_json(&rx.borrow_and_update())?;
        wr.write_all(&ws_frame(0x1, &body)).await?;
        wr.flush().await?;
        tokio::select! {
            r = rx.changed() => {
                r?;
            }
            _ = tokio::time::sleep(Duration::from_secs(20)) => {
                wr.write_all(&ws_frame(0x9, b"ping")).await?;
                wr.flush().await?;
            }
        }
    }
}

