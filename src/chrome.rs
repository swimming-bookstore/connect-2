//! Headless Chromium. CDP state lives in one task — no shared lock.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::plane::ClientMsg;
use crate::protocol::{Out, Tab};

static LIVE: AtomicUsize = AtomicUsize::new(0);

/// Hard cap on concurrent Chromium processes. Default 1 (OOM-safe).
pub fn max_live() -> usize {
    std::env::var("CONNECT2_MAX_CHROME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1)
}

pub fn live() -> usize {
    LIVE.load(Ordering::SeqCst)
}

fn try_begin() -> bool {
    let max = max_live();
    loop {
        let n = LIVE.load(Ordering::SeqCst);
        if n >= max {
            return false;
        }
        if LIVE
            .compare_exchange(n, n + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return true;
        }
    }
}

fn end_live() {
    LIVE.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1)))
        .ok();
}

/// Decrements `LIVE` if spawn is cancelled or fails before `Chromium` is built.
struct LiveGuard {
    active: bool,
}

impl LiveGuard {
    fn acquire() -> Option<Self> {
        if try_begin() {
            Some(Self { active: true })
        } else {
            None
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        if self.active {
            end_live();
        }
    }
}

pub struct Chromium {
    child: Child,
    cmd: mpsc::UnboundedSender<Cmd>,
    frames: broadcast::Sender<Vec<u8>>,
    last: tokio::sync::watch::Sender<Option<Vec<u8>>>,
    tabs: tokio::sync::watch::Sender<Snap>,
    user_data: PathBuf,
}

enum Cmd {
    Call {
        method: String,
        params: Value,
        page: bool,
        reply: Option<oneshot::Sender<Result<Value>>>,
    },
    Focus {
        id: String,
    },
    CloseTab {
        id: String,
    },
}

struct Page {
    url: String,
    title: String,
    session: Option<String>,
}

struct Front {
    /// Chromium targetId of the focused page.
    active: Option<String>,
    /// targetId → page. Chromium is the source of truth.
    pages: HashMap<String, Page>,
    order: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct Snap {
    pub tabs: Vec<Tab>,
    pub url: String,
}

impl Chromium {
    /// Only remove *orphaned* profile dirs (no live chrome using them).
    /// Never SIGKILL a running browser — that used to spawn-storm replacements.
    pub fn reap_stale() {
        let prefix = "connect2-agent-chrome-";
        let Ok(rd) = std::fs::read_dir("/var/tmp") else {
            return;
        };
        for ent in rd.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(prefix) {
                continue;
            }
            let path = ent.path();
            if chrome_using_dir(&path) {
                continue;
            }
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    pub async fn spawn(width: u32, height: u32) -> Result<Self> {
        let mut guard = LiveGuard::acquire().ok_or_else(|| {
            anyhow!(
                "chromium cap ({}) — close the other browser session",
                max_live()
            )
        })?;
        match Self::spawn_inner(width, height).await {
            Ok(v) => {
                guard.disarm();
                Ok(v)
            }
            Err(e) => Err(e),
        }
    }

    async fn spawn_inner(width: u32, height: u32) -> Result<Self> {
        Self::reap_stale();
        let bin = find_chrome()?;
        let port = free_port().await?;
        let user_data = PathBuf::from("/var/tmp").join(format!(
            "connect2-agent-chrome-{}-{}",
            std::process::id(),
            port
        ));
        let _ = std::fs::remove_dir_all(&user_data);
        std::fs::create_dir_all(&user_data).with_context(|| {
            format!(
                "create {}{}",
                user_data.display(),
                tmp_full_hint()
            )
        })?;

        let mut cmd = Command::new(&bin);
        cmd.args([
            "--headless=new",
            // required in this box/container; not a sandbox claim
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--no-first-run",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-default-apps",
            "--disable-extensions",
            "--disable-hang-monitor",
            "--disable-popup-blocking",
            "--disable-prompt-on-repost",
            "--metrics-recording-only",
            "--no-default-browser-check",
            "--password-store=basic",
            "--use-mock-keychain",
            "--hide-scrollbars",
            "--mute-audio",
            "--ozone-platform=headless",
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
            "--renderer-process-limit=1",
            "--disable-gpu",
            "--disable-gpu-compositing",
            "--js-flags=--max-old-space-size=128",
            "--remote-debugging-address=127.0.0.1",
            &format!("--remote-debugging-port={port}"),
            &format!("--user-data-dir={}", user_data.display()),
            &format!("--window-size={width},{height}"),
            "about:blank",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", bin.display()))?;
        let mut child = KillOnDrop {
            child: Some(child),
            user_data: user_data.clone(),
        };

        let ws = wait_browser_ws(port).await?;
        let (cmd, frames, last, tabs) = attach(ws, width, height).await?;
        let child = child.child.take().ok_or_else(|| anyhow!("chromium child"))?;
        Ok(Self {
            child,
            cmd,
            frames,
            last,
            tabs,
            user_data,
        })
    }

    /// Same as spawn, but Close (or a dropped channel) aborts and kills Chromium.
    pub async fn spawn_unless_close(
        width: u32,
        height: u32,
        cmds: &mut mpsc::Receiver<crate::protocol::In>,
    ) -> Result<Option<Self>> {
        let work = Self::spawn(width, height);
        tokio::pin!(work);
        loop {
            tokio::select! {
                biased;
                cmd = cmds.recv() => match cmd {
                    None | Some(crate::protocol::In::Close) => return Ok(None),
                    Some(_) => {}
                },
                r = &mut work => {
                    let chrome = r?;
                    loop {
                        match cmds.try_recv() {
                            Ok(crate::protocol::In::Close)
                            | Err(mpsc::error::TryRecvError::Disconnected) => return Ok(None),
                            Ok(_) => {}
                            Err(mpsc::error::TryRecvError::Empty) => return Ok(Some(chrome)),
                        }
                    }
                }
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.frames.subscribe()
    }

    pub fn last_jpeg(&self) -> Option<Vec<u8>> {
        self.last.borrow().clone()
    }

    pub fn click(&self, x: f64, y: f64, button: u8) {
        let (btn, mask) = match button {
            2 => ("right", 2),
            1 => ("middle", 4),
            _ => ("left", 1),
        };
        self.fire(
            "Input.dispatchMouseEvent",
            json!({"type":"mouseMoved","x":x,"y":y}),
        );
        self.fire(
            "Input.dispatchMouseEvent",
            json!({"type":"mousePressed","x":x,"y":y,"button":btn,"buttons":mask,"clickCount":1}),
        );
        self.fire(
            "Input.dispatchMouseEvent",
            json!({"type":"mouseReleased","x":x,"y":y,"button":btn,"buttons":0,"clickCount":1}),
        );
    }

    pub fn key(&self, key: &str, pressed: bool) {
        let typ = if pressed { "keyDown" } else { "keyUp" };
        let mut p = json!({"type": typ, "key": key});
        if key.len() == 1 {
            p["text"] = json!(key);
        }
        self.fire("Input.dispatchKeyEvent", p);
    }

    pub fn wheel(&self, x: f64, y: f64, dx: f64, dy: f64) {
        self.fire(
            "Input.dispatchMouseEvent",
            json!({"type":"mouseWheel","x":x,"y":y,"deltaX":dx,"deltaY":dy}),
        );
    }

    pub fn navigate(&self, url: &str) {
        self.fire("Page.navigate", json!({"url": url}));
    }

    pub fn new_tab(&self, url: &str) {
        let url = if url.is_empty() { "about:blank" } else { url };
        self.fire_browser("Target.createTarget", json!({"url": url}));
    }

    pub fn close_tab(&self, id: &str) {
        let _ = self.cmd.send(Cmd::CloseTab { id: id.into() });
    }

    pub fn go_back(&self) {
        self.fire("Runtime.evaluate", json!({"expression": "history.back()"}));
    }

    pub fn go_forward(&self) {
        self.fire("Runtime.evaluate", json!({"expression": "history.forward()"}));
    }

    pub fn focus(&self, id: &str) {
        let _ = self.cmd.send(Cmd::Focus { id: id.into() });
    }

    fn fire(&self, method: &str, params: Value) {
        let _ = self.cmd.send(Cmd::Call {
            method: method.into(),
            params,
            page: true,
            reply: None,
        });
    }

    fn fire_browser(&self, method: &str, params: Value) {
        let _ = self.cmd.send(Cmd::Call {
            method: method.into(),
            params,
            page: false,
            reply: None,
        });
    }

    pub fn push_tabs(&self, tx: mpsc::Sender<ClientMsg>, channel: String, dst: String) {
        let mut tabs = self.tabs.subscribe();
        let first = tabs.borrow().clone();
        tokio::spawn(async move {
            let send = |s: Snap| {
                serde_json::to_vec(&Out::Tabs {
                    tabs: s.tabs,
                    url: s.url,
                })
            };
            if let Ok(bytes) = send(first) {
                if tx.send(crate::plane::app(&dst, &channel, bytes)).await.is_err() {
                    return;
                }
            }
            loop {
                if tabs.changed().await.is_err() {
                    break;
                }
                let s = tabs.borrow_and_update().clone();
                let Ok(bytes) = send(s) else {
                    continue;
                };
                if tx.send(crate::plane::app(&dst, &channel, bytes)).await.is_err() {
                    break;
                }
            }
        });
    }

    pub async fn apply(&self, cmd: crate::protocol::In) -> Option<Out> {
        use crate::protocol::{abs_url, In};
        match cmd {
            In::Click { x, y, button } => self.click(x, y, button),
            In::Key { key, pressed } => self.key(&key, pressed),
            In::Navigate { url } => self.navigate(&abs_url(&url)),
            In::Wheel {
                x,
                y,
                delta_x,
                delta_y,
            } => self.wheel(x, y, delta_x, delta_y),
            In::Back => self.go_back(),
            In::Forward => self.go_forward(),
            In::NewTab { url } => self.new_tab(&if url.is_empty() {
                "about:blank".into()
            } else {
                abs_url(&url)
            }),
            In::CloseTab { id } => self.close_tab(&id),
            In::Focus { id } => self.focus(&id),
            In::Eval { expression } => {
                return Some(match self.eval(&expression).await {
                    Ok(result) => Out::Eval { result },
                    Err(e) => Out::Error {
                        message: e.to_string(),
                    },
                });
            }
            _ => {}
        }
        None
    }

    pub async fn eval(&self, expression: &str) -> Result<String> {
        let v = self
            .page(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
            )
            .await?;
        if v["result"]["exceptionDetails"].is_object() {
            bail!("{}", v["result"]["exceptionDetails"]);
        }
        let val = &v["result"]["result"]["value"];
        Ok(val.as_str().map(str::to_string).unwrap_or_else(|| val.to_string()))
    }

    async fn page(&self, method: &str, params: Value) -> Result<Value> {
        self.call(method, params, true).await
    }

    async fn call(&self, method: &str, params: Value, page: bool) -> Result<Value> {
        let (reply, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::Call {
                method: method.into(),
                params,
                page,
                reply: Some(reply),
            })
            .map_err(|_| anyhow!("cdp closed"))?;
        tokio::time::timeout(Duration::from_secs(8), rx)
            .await
            .map_err(|_| anyhow!("cdp timeout {method}"))?
            .map_err(|_| anyhow!("cdp dropped {method}"))?
    }
}

impl Drop for Chromium {
    fn drop(&mut self) {
        kill_chrome(&mut self.child, &self.user_data);
        end_live();
    }
}

/// Kills Chromium if spawn is cancelled before `Self` is built.
struct KillOnDrop {
    child: Option<Child>,
    user_data: PathBuf,
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            kill_chrome(&mut child, &self.user_data);
        }
    }
}

fn kill_chrome(child: &mut Child, user_data: &PathBuf) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.start_kill();
    for _ in 0..40 {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = std::fs::remove_dir_all(user_data);
}

fn chrome_url(url: &str) -> String {
    if url.is_empty() {
        "about:blank".into()
    } else {
        url.into()
    }
}

fn host_label(url: &str) -> String {
    let u = url.trim();
    if u.is_empty() || u == "about:blank" {
        return "about:blank".into();
    }
    let rest = u.split("://").nth(1).unwrap_or(u);
    rest.split('/').next().unwrap_or(rest).to_string()
}

fn chrome_title(title: &str, url: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        title.into()
    } else {
        host_label(url)
    }
}

fn snap_of(f: &Front) -> Snap {
    let tabs = f
        .order
        .iter()
        .filter_map(|id| {
            let p = f.pages.get(id)?;
            let url = chrome_url(&p.url);
            Some(Tab {
                id: id.clone(),
                title: chrome_title(&p.title, &url),
                url,
                active: f.active.as_deref() == Some(id.as_str()),
            })
        })
        .collect::<Vec<_>>();
    let url = f
        .active
        .as_ref()
        .and_then(|id| f.pages.get(id).map(|p| chrome_url(&p.url)))
        .unwrap_or_else(|| "about:blank".into());
    Snap { tabs, url }
}

fn upsert_target(front: &mut Front, target: &str, url: &str, title: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    if let Some(p) = front.pages.get_mut(target) {
        let url_changed = !url.is_empty() && url != p.url;
        if !url.is_empty() {
            p.url = url.into();
        }
        let title = title.trim();
        if !title.is_empty() {
            p.title = title.into();
        } else if url_changed {
            p.title.clear();
        }
        return false;
    }
    front.pages.insert(
        target.into(),
        Page {
            url: url.into(),
            title: title.into(),
            session: None,
        },
    );
    front.order.push(target.into());
    if front.active.is_none() {
        front.active = Some(target.into());
    }
    true
}

fn drop_target(front: &mut Front, target: &str) {
    front.pages.remove(target);
    front.order.retain(|id| id != target);
    if front.active.as_deref() == Some(target) {
        front.active = front.order.first().cloned();
    }
}

async fn attach(
    url: String,
    width: u32,
    height: u32,
) -> Result<(
    mpsc::UnboundedSender<Cmd>,
    broadcast::Sender<Vec<u8>>,
    tokio::sync::watch::Sender<Option<Vec<u8>>>,
    tokio::sync::watch::Sender<Snap>,
)> {
    let (ws, _) = connect_async(&url).await.context("cdp ws")?;
    let (mut sink, mut stream) = ws.split();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let (frames, _) = broadcast::channel::<Vec<u8>>(16);
    let frames_out = frames.clone();
    let (last, _) = tokio::sync::watch::channel(None);
    let last_out = last.clone();
    let (tabs, _) = tokio::sync::watch::channel(Snap {
        tabs: Vec::new(),
        url: String::new(),
    });
    let tabs_tx = tabs.clone();
    let (ready_tx, ready_rx) = oneshot::channel::<()>();
    let mut ready_tx = Some(ready_tx);

    tokio::spawn(async move {
        let mut next_id: u64 = 3;
        let mut pending: HashMap<u64, oneshot::Sender<Result<Value>>> = HashMap::new();
        let mut titles: HashMap<u64, String> = HashMap::new();
        let mut created: HashSet<u64> = HashSet::new();
        let mut front = Front {
            active: None,
            pages: HashMap::new(),
            order: Vec::new(),
        };
        let publish = |front: &Front| {
            let _ = tabs.send_replace(snap_of(front));
        };
        for msg in [
            json!({"id": 1, "method": "Target.setDiscoverTargets", "params": {"discover": true}}),
            json!({
                "id": 2,
                "method": "Target.setAutoAttach",
                "params": {"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true}
            }),
        ] {
            if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                return;
            }
        }

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        Cmd::Call { method, params, page, reply } => {
                            let id = next_id;
                            next_id += 1;
                            if method == "Target.createTarget" {
                                created.insert(id);
                            }
                            if let Some(reply) = reply {
                                pending.insert(id, reply);
                            }
                            let mut msg = json!({"id": id, "method": method, "params": params});
                            if page {
                                if let Some(sid) = front.active.as_ref().and_then(|tid| {
                                    front.pages.get(tid).and_then(|p| p.session.as_ref())
                                }) {
                                    msg["sessionId"] = json!(sid);
                                }
                            }
                            if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                                break;
                            }
                        }
                        Cmd::Focus { id } => {
                            if front.pages.contains_key(&id) {
                                front.active = Some(id.clone());
                                publish(&front);
                                let msg = json!({
                                    "id": next_id,
                                    "method": "Target.activateTarget",
                                    "params": {"targetId": id}
                                });
                                next_id += 1;
                                if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Cmd::CloseTab { id } => {
                            if front.pages.contains_key(&id) {
                                let msg = json!({
                                    "id": next_id,
                                    "method": "Target.closeTarget",
                                    "params": {"targetId": id}
                                });
                                next_id += 1;
                                if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                ev = stream.next() => {
                    let t = match ev {
                        Some(Ok(Message::Text(t))) => t,
                        Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_))) => continue,
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    };
                    let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                    if let Some(id) = v.get("id").and_then(Value::as_u64) {
                        if let Some(reply) = pending.remove(&id) {
                            let r = match v.get("error") {
                                Some(err) => Err(anyhow!("{err}")),
                                None => Ok(v.clone()),
                            };
                            let _ = reply.send(r);
                        }
                        if created.remove(&id) {
                            if let Some(tid) = v
                                .pointer("/result/targetId")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                            {
                                front.active = Some(tid.to_string());
                                publish(&front);
                                let msg = json!({
                                    "id": next_id,
                                    "method": "Target.activateTarget",
                                    "params": {"targetId": tid}
                                });
                                next_id += 1;
                                if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                        if let Some(tid) = titles.remove(&id) {
                            if let Some(t) = v
                                .pointer("/result/result/value")
                                .and_then(Value::as_str)
                            {
                                if let Some(p) = front.pages.get_mut(&tid) {
                                    p.title = t.to_string();
                                    publish(&front);
                                }
                            }
                        }
                        continue;
                    }
                    match v.get("method").and_then(Value::as_str) {
                        Some("Target.targetCreated") | Some("Target.targetInfoChanged") => {
                            let info = &v["params"]["targetInfo"];
                            if info["type"].as_str() != Some("page") {
                                continue;
                            }
                            let tid = info["targetId"].as_str().unwrap_or("");
                            let url = info["url"].as_str().unwrap_or("");
                            let title = info["title"].as_str().unwrap_or("");
                            let fresh = upsert_target(&mut front, tid, url, title);
                            if fresh && !created.is_empty() {
                                front.active = Some(tid.to_string());
                            }
                            publish(&front);
                        }
                        Some("Target.targetDestroyed") => {
                            let tid = v["params"]["targetId"].as_str().unwrap_or("");
                            drop_target(&mut front, tid);
                            publish(&front);
                        }
                        Some("Target.attachedToTarget") => {
                            let sid = v["params"]["sessionId"].as_str().unwrap_or("").to_string();
                            let typ = v["params"]["targetInfo"]["type"].as_str().unwrap_or("");
                            let url = v["params"]["targetInfo"]["url"].as_str().unwrap_or("");
                            let title = v["params"]["targetInfo"]["title"].as_str().unwrap_or("");
                            let target = v["params"]["targetInfo"]["targetId"].as_str().unwrap_or("").to_string();
                            if typ != "page" || sid.is_empty() || target.is_empty() {
                                continue;
                            }
                            upsert_target(&mut front, &target, url, title);
                            if let Some(p) = front.pages.get_mut(&target) {
                                p.session = Some(sid.clone());
                            }
                            if front.active.is_none() || !created.is_empty() {
                                front.active = Some(target);
                            }
                            publish(&front);
                            let en = [
                                json!({"id": next_id, "method": "Page.enable", "sessionId": sid}),
                                json!({"id": next_id + 1, "method": "Runtime.enable", "sessionId": sid}),
                                json!({
                                    "id": next_id + 2,
                                    "method": "Emulation.setDeviceMetricsOverride",
                                    "sessionId": sid,
                                    "params": {"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}
                                }),
                                json!({
                                    "id": next_id + 3,
                                    "method": "Page.startScreencast",
                                    "sessionId": sid,
                                    "params": {
                                        "format": "jpeg",
                                        "quality": 40,
                                        "maxWidth": width,
                                        "maxHeight": height,
                                        "everyNthFrame": 1
                                    }
                                }),
                            ];
                            next_id += 4;
                            for msg in en {
                                if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                                    return;
                                }
                            }
                            if let Some(tx) = ready_tx.take() {
                                let _ = tx.send(());
                            }
                        }
                        Some("Target.detachedFromTarget") => {
                            let sid = v["params"]["sessionId"].as_str().unwrap_or("");
                            let tid = front
                                .pages
                                .iter()
                                .find(|(_, p)| p.session.as_deref() == Some(sid))
                                .map(|(id, _)| id.clone());
                            if let Some(tid) = tid {
                                if let Some(p) = front.pages.get_mut(&tid) {
                                    p.session = None;
                                }
                            }
                            publish(&front);
                        }
                        Some("Page.frameNavigated") => {
                            let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or("");
                            let frame = &v["params"]["frame"];
                            if frame.get("parentId").and_then(Value::as_str).is_none() {
                                if let Some(u) = frame["url"].as_str() {
                                    if let Some(p) = front.pages.values_mut().find(|p| p.session.as_deref() == Some(sid)) {
                                        p.url = u.to_string();
                                        publish(&front);
                                    }
                                }
                            }
                        }
                        Some("Page.loadEventFired") | Some("Page.domContentEventFired") => {
                            let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or("");
                            let tid = front
                                .pages
                                .iter()
                                .find(|(_, p)| p.session.as_deref() == Some(sid))
                                .map(|(id, _)| id.clone());
                            if let Some(tid) = tid {
                                let id = next_id;
                                next_id += 1;
                                titles.insert(id, tid);
                                let msg = json!({
                                    "id": id,
                                    "method": "Runtime.evaluate",
                                    "sessionId": sid,
                                    "params": {"expression": "document.title", "returnByValue": true}
                                });
                                if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Some("Page.titleChanged") => {
                            let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or("");
                            if let Some(title) = v["params"]["title"].as_str() {
                                if let Some(p) = front.pages.values_mut().find(|p| p.session.as_deref() == Some(sid)) {
                                    if !title.trim().is_empty() {
                                        p.title = title.to_string();
                                        publish(&front);
                                    }
                                }
                            }
                        }
                        Some("Page.navigatedWithinDocument") => {
                            let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or("");
                            if let Some(u) = v["params"]["url"].as_str() {
                                if let Some(p) = front.pages.values_mut().find(|p| p.session.as_deref() == Some(sid)) {
                                    p.url = u.to_string();
                                    publish(&front);
                                }
                            }
                        }
                        Some("Page.screencastFrame") => {
                            let params = &v["params"];
                            let ack = params["sessionId"].clone();
                            let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or("").to_string();
                            if let Some(b64) = params["data"].as_str() {
                                if let Ok(jpeg) = STANDARD.decode(b64) {
                                    let on = sid.is_empty()
                                        || front.active.as_ref().and_then(|tid| {
                                            front.pages.get(tid)?.session.as_deref()
                                        }) == Some(sid.as_str());
                                    if on {
                                        let _ = last.send_replace(Some(jpeg.clone()));
                                        let _ = frames.send(jpeg);
                                    }
                                }
                            }
                            let mut ack_msg = json!({
                                "id": next_id,
                                "method": "Page.screencastFrameAck",
                                "params": {"sessionId": ack}
                            });
                            if !sid.is_empty() {
                                ack_msg["sessionId"] = json!(sid);
                            }
                            next_id += 1;
                            if sink.send(Message::Text(ack_msg.to_string().into())).await.is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    tokio::time::timeout(Duration::from_secs(15), ready_rx)
        .await
        .map_err(|_| anyhow!("no page target attached"))?
        .map_err(|_| anyhow!("cdp closed before page"))?;
    Ok((cmd_tx, frames_out, last_out, tabs_tx))
}
async fn wait_browser_ws(port: u16) -> Result<String> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(v) = client.get(&url).send().await {
            if let Ok(j) = v.json::<Value>().await {
                if let Some(ws) = j["webSocketDebuggerUrl"].as_str() {
                    return Ok(ws.to_string());
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "chromium debug port {port} never came up{}",
                tmp_full_hint()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn free_port() -> Result<u16> {
    let l = TcpListener::bind("127.0.0.1:0").await?;
    Ok(l.local_addr()?.port())
}

fn tmp_full_hint() -> String {
    match tmp_free_mb() {
        Some(mb) if mb < 64 => format!(" (/tmp is full, {mb} MiB free)"),
        _ => String::new(),
    }
}

fn tmp_free_mb() -> Option<u64> {
    let st = nix::sys::statvfs::statvfs("/tmp").ok()?;
    Some(st.blocks_available() * st.fragment_size() / (1024 * 1024))
}

fn find_chrome() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("CHROME") {
        return Ok(PathBuf::from(p));
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if let Ok(out) = std::process::Command::new("which").arg(name).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Ok(PathBuf::from(s));
                }
            }
        }
    }
    bail!("chromium/chrome not on PATH (set CHROME=)")
}

fn chrome_using_dir(dir: &std::path::Path) -> bool {
    let needle = format!("--user-data-dir={}", dir.display());
    let Ok(out) = std::process::Command::new("ps")
        .args(["-eo", "pid=,cmd="])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.contains(&needle) && !line.contains("--type="))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn front() -> Front {
        Front {
            active: None,
            pages: HashMap::new(),
            order: Vec::new(),
        }
    }

    #[test]
    fn live_cap_blocks_second() {
        std::env::set_var("CONNECT2_MAX_CHROME", "1");
        assert_eq!(max_live(), 1);
        assert!(try_begin());
        assert!(!try_begin());
        end_live();
        assert!(try_begin());
        end_live();
        std::env::remove_var("CONNECT2_MAX_CHROME");
    }

    #[test]
    fn chrome_title_rules() {
        assert_eq!(chrome_title(" Example ", "https://x"), "Example");
        assert_eq!(chrome_title("", "about:blank"), "about:blank");
        assert_eq!(chrome_title("  ", ""), "about:blank");
        assert_eq!(chrome_title("", "https://x"), "x");
    }

    #[test]
    fn upsert_and_drop_tabs() {
        let mut f = front();
        upsert_target(&mut f, "t1", "about:blank", "");
        upsert_target(&mut f, "t2", "https://example.com", "Example");
        assert_eq!(f.active.as_deref(), Some("t1"));
        assert_eq!(f.order, vec!["t1".to_string(), "t2".to_string()]);
        upsert_target(&mut f, "t1", "https://a", "A");
        assert_eq!(f.pages["t1"].url, "https://a");
        assert_eq!(f.pages["t1"].title, "A");
        let s = snap_of(&f);
        assert_eq!(s.tabs.len(), 2);
        assert!(s.tabs[0].active);
        assert!(!s.tabs[1].active);
        assert_eq!(s.url, "https://a");
        drop_target(&mut f, "t1");
        assert_eq!(f.active.as_deref(), Some("t2"));
        assert_eq!(snap_of(&f).tabs.len(), 1);
        drop_target(&mut f, "t2");
        assert!(f.active.is_none());
        assert!(snap_of(&f).tabs.is_empty());
    }

    #[test]
    fn upsert_ignores_empty_id() {
        let mut f = front();
        upsert_target(&mut f, "", "https://x", "x");
        assert!(f.pages.is_empty());
    }

    #[test]
    fn tmp_full_hint_shape() {
        let h = tmp_full_hint();
        assert!(h.is_empty() || h.contains("MiB free"));
    }
}
