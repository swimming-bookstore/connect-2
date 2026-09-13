//! Hub sessions: connect-app pipe, inventory, Grok tools.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crate::ai;
use crate::protocol::{In, Kind, Out, Video};
use connect2_teleport::user;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::sync::{mpsc, watch};

use super::cli::{self, Cli};
use super::cluster;
use super::desk;
use super::pipe::PipeHub;
use super::view::{patch, Peer, View};

pub enum Front {
    Pick(String),
    Home,
    ClusterLogout,
    ClusterLogin {
        auth: String,
        connector: String,
        user: String,
        password: String,
        otp: String,
    },
    Open {
        dst: String,
        kind: Kind,
        video: Video,
        height: u32,
    },
    Msg(In),
    Desk(String),
    AgentAsk(String),
}

pub fn spawn_hub(cli: Cli) -> Result<std::thread::JoinHandle<()>> {
    let http = cli.http.clone();
    let hub = std::thread::Builder::new()
        .name("connect2-hub".into())
        .spawn(move || match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => {
                if let Err(e) = rt.block_on(run_hub(cli)) {
                    tracing::error!("hub: {e:#}");
                }
            }
            Err(e) => tracing::error!("tokio: {e}"),
        })?;
    for _ in 0..80 {
        if std::net::TcpStream::connect(&http).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(hub)
}

pub async fn run_hub(cli: Cli) -> Result<()> {
    let mut init = View::default();
    init.me = cli.teleport_user.clone();
    init.authed = connect2_teleport::login::identity_ok(&cli.identity);
    if !init.authed {
        init.me.clear();
    }
    if !cli.node.is_empty() && init.authed {
        init.peers.push(Peer {
            id: cli.node.clone(),
            name: cli.node.clone(),
            tunnel: true,
        });
    }
    let (view_tx, _view_rx) = watch::channel(init);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Front>(64);
    let pipe = PipeHub::start();
    let cli = Arc::new(cli);

    {
        let bind = cli.http.clone();
        let v = view_tx.clone();
        let tx = cmd_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = super::http::serve(bind, v, tx).await {
                tracing::error!("http: {e:#}");
            }
        });
    }

    {
        let cli = cli.clone();
        let view = view_tx.clone();
        tokio::spawn(async move {
            poll_nodes(cli, view).await;
        });
    }

    {
        let cli = cli.clone();
        let view = view_tx.clone();
        tokio::spawn(async move {
            cluster::refresh_auth(&cli, &view).await;
        });
    }

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Front::Home => {
                        if let Err(e) = show_directory(&view_tx).await {
                            tracing::warn!("home: {e:#}");
                        }
                    }
                    Front::ClusterLogout => {
                        if let Err(e) = hangup(&pipe, &view_tx).await {
                            tracing::warn!("home: {e:#}");
                        }
                        cluster::cluster_logout(&cli, &view_tx).await;
                    }
                    Front::ClusterLogin { auth, connector, user, password, otp } => {
                        let cli = cli.clone();
                        let view = view_tx.clone();
                        tokio::spawn(async move {
                            cluster::cluster_login(&cli, &view, auth, connector, user, password, otp).await;
                        });
                    }
                    Front::Pick(dst) => {
                        if let Err(e) = pick(&cli, &pipe, &view_tx, &dst).await {
                            tracing::warn!("pick {dst}: {e:#}");
                        }
                    }
                    Front::Open { dst, kind, video, height } => {
                        match tokio::time::timeout(
                            Duration::from_secs(20),
                            open_kind(&cli, &pipe, &view_tx, &cmd_tx, &dst, kind, video, height),
                        )
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                tracing::warn!("open {dst}: {e:#}");
                                patch(&view_tx, |g| {
                                    g.busy = false;
                                    g.last = format!("open {e:#}");
                                });
                            }
                            Err(_) => {
                                tracing::warn!("open {dst}: timed out");
                                patch(&view_tx, |g| {
                                    g.busy = false;
                                    g.last = format!("open {dst}: timed out");
                                });
                            }
                        }
                    }
                    Front::Desk(text) => {
                        desk::desk_ask(&cli, &pipe, &view_tx, &cmd_tx, &text).await;
                    }
                    Front::AgentAsk(text) => {
                        desk::agent_ask(&cli, &pipe, &view_tx, &cmd_tx, &text).await;
                    }
                    Front::Msg(msg) => {
                        if let In::Resize { cols, rows } = &msg {
                            if *cols < 8 || *rows < 4 {
                                continue;
                            }
                        }
                        let dst = view_tx.borrow().dst.clone();
                        let kind = view_tx.borrow().kind.clone();
                        if matches!(msg, In::Stdin { .. } | In::Resize { .. }) && dst.is_empty() {
                            continue;
                        }
                        if matches!(msg, In::Stdin { .. } | In::Resize { .. }) && kind != "shell"
                        {
                            continue;
                        }
                        if matches!(msg, In::Stdin { .. } | In::Resize { .. })
                            && !pipe.same_node(&dst).await
                        {
                            continue;
                        }
                        if matches!(msg, In::Offer { .. } | In::Ice { .. }) {
                            if let Err(e) = wait_kind(&view_tx, "browser", 25).await {
                                tracing::debug!("browser wait: {e:#}");
                            }
                        }
                        if let Err(e) = pipe.write(msg).await {
                            tracing::warn!("send: {e:#}");
                            pipe.close().await;
                            continue;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn attach_reader(pipe: PipeHub, view: watch::Sender<View>, cmd_tx: mpsc::Sender<Front>) {
    let Some((stdout, gen)) = pipe.take_stdout().await else {
        return;
    };
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !pipe.is_gen(gen).await {
                break;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(msg) = serde_json::from_str::<Out>(line) else {
                tracing::debug!(%line, "raw");
                continue;
            };
            on_out(&view, &cmd_tx, msg).await;
        }
        tracing::warn!(gen, "connect-app stdout closed");
        pipe.drop_if(gen).await;
    });
}

async fn pick(cli: &Cli, pipe: &PipeHub, view: &watch::Sender<View>, dst: &str) -> Result<()> {
    let dst = resolve_dst(view, dst);
    if dst.is_empty() {
        bail!("no node selected");
    }
    let prev = view.borrow().dst.clone();
    if prev == dst {
        return Ok(());
    }
    let _ = pipe;
    patch(view, |g| {
        g.dst = dst.clone();
        g.channel.clear();
        g.jpeg = None;
        g.stdout.clear();
        g.log.clear();
        g.kind.clear();
        g.video.clear();
        g.url.clear();
        g.tabs = serde_json::json!([]);
        g.width = 0;
        g.height = 0;
        g.height_want = 0;
        g.answer.clear();
        g.ice.clear();
        g.want_answer = false;
        g.last.clear();
    });
    let _ = cli;
    tracing::info!(%dst, "picked node");
    Ok(())
}

async fn show_directory(view: &watch::Sender<View>) -> Result<()> {
    patch(view, |g| {
        g.dst.clear();
        g.kind.clear();
        g.video.clear();
        g.last.clear();
        g.busy = false;
    });
    Ok(())
}

async fn hangup(pipe: &PipeHub, view: &watch::Sender<View>) -> Result<()> {
    pipe.close().await;
    patch(view, |g| {
        g.dst.clear();
        g.channel.clear();
        g.jpeg = None;
        g.stdout.clear();
        g.log.clear();
        g.kind.clear();
        g.video.clear();
        g.url.clear();
        g.tabs = serde_json::json!([]);
        g.width = 0;
        g.height = 0;
        g.height_want = 0;
        g.answer.clear();
        g.ice.clear();
        g.want_answer = false;
        g.last.clear();
        g.busy = false;
    });
    Ok(())
}

async fn ensure_pipe(cli: &Cli, pipe: &PipeHub, view: &watch::Sender<View>) -> Result<()> {
    let dst = view.borrow().dst.clone();
    if dst.is_empty() {
        bail!("no node selected");
    }
    let opened = pipe.ensure(cli::cfg_of(cli), dst.clone()).await?;
    if opened {
        patch(view, |v| {
            v.channel = format!("{dst}:app");
            v.dst = dst.clone();
        });
        tracing::info!(%dst, "connect-app up");
    }
    Ok(())
}

pub fn resolve_dst(view: &watch::Sender<View>, machine: &str) -> String {
    let g = view.borrow();
    let m = machine.trim();
    if !m.is_empty() {
        if let Some(p) = g.peers.iter().find(|p| p.name == m || p.id == m) {
            return p.name.clone();
        }
        return m.to_string();
    }
    if !g.dst.is_empty() {
        return g.dst.clone();
    }
    g.peers.first().map(|p| p.name.clone()).unwrap_or_default()
}

pub async fn open_kind(
    cli: &Cli,
    pipe: &PipeHub,
    view: &watch::Sender<View>,
    cmd_tx: &mpsc::Sender<Front>,
    dst: &str,
    kind: Kind,
    video: Video,
    height: u32,
) -> Result<()> {
    let kind_s = match kind {
        Kind::Browser => "browser",
        Kind::Shell => "shell",
        Kind::Agent => "agent",
    };
    let video_s = match video {
        Video::Webrtc => "webrtc",
        Video::Jpeg => "jpeg",
        Video::None => "none",
    };
    patch(view, |g| g.busy = true);
    if view.borrow().dst != dst {
        pipe.close().await;
        pick(cli, pipe, view, dst).await?;
    }
    let same = pipe.same_node(dst).await;
    let alive = same && pipe.alive().await;
    let have_shell = kind == Kind::Shell && !view.borrow().stdout.is_empty();
    // A second Open on the same connect-app channel kills a live PTY.
    if alive
        && view.borrow().kind == kind_s
        && (kind != Kind::Browser || view.borrow().video == video_s)
        && !view.borrow().channel.is_empty()
        && (kind != Kind::Shell || have_shell)
    {
        patch(view, |g| g.busy = false);
        return Ok(());
    }
    if same
        && kind == Kind::Shell
        && view.borrow().kind == "shell"
        && !view.borrow().channel.is_empty()
        && have_shell
    {
        patch(view, |g| g.busy = false);
        return Ok(());
    }
    patch(view, |g| {
        g.busy = true;
        g.kind = kind_s.into();
        g.video = video_s.into();
        g.width = 0;
        g.height = 0;
        g.height_want = height;
        g.answer.clear();
        g.ice.clear();
        g.ice_servers = serde_json::json!([]);
        g.want_answer = matches!(video, Video::Webrtc);
        g.last.clear();
        if kind == Kind::Shell {
            g.stdout.clear();
        }
        if kind == Kind::Agent {
            g.log.clear();
        }
        if kind != Kind::Browser {
            g.jpeg = None;
        }
    });
    if !same {
        pipe.close().await;
        ensure_pipe(cli, pipe, view).await?;
        attach_reader(pipe.clone(), view.clone(), cmd_tx.clone()).await;
    } else if !alive {
        attach_reader(pipe.clone(), view.clone(), cmd_tx.clone()).await;
    }
    pipe.write(In::Open {
        kind,
        video,
        height,
    })
    .await?;
    patch(view, |g| g.busy = false);
    Ok(())
}

pub async fn wait_kind(view: &watch::Sender<View>, kind: &str, secs: u64) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        {
            let g = view.borrow();
            if g.kind == kind && !g.channel.is_empty() {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("{kind} session did not start");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn poll_nodes(cli: Arc<Cli>, view: watch::Sender<View>) {
    loop {
        let wait = Duration::from_secs(cli.ls_secs.max(1));
        if !connect2_teleport::login::identity_ok(&cli.identity) {
            if view.borrow().authed {
                patch(&view, |g| {
                    g.authed = false;
                    g.me.clear();
                    g.peers.clear();
                    g.dst.clear();
                });
            }
            tokio::time::sleep(wait).await;
            continue;
        }
        if !view.borrow().authed {
            tokio::time::sleep(wait).await;
            continue;
        }
        let cfg = cli::cfg_of(&cli);
        match user::list_nodes(&cfg).await {
            Ok(nodes) => {
                let peers: Vec<Peer> = nodes
                    .into_iter()
                    .map(|n| Peer {
                        id: n.id,
                        name: n.name,
                        tunnel: n.tunnel,
                    })
                    .collect();
                let mut g = view.borrow().clone();
                let changed = g.peers != peers;
                if changed {
                    g.peers = peers;
                }
                if changed {
                    g.seq = g.seq.wrapping_add(1);
                    if view.send(g).is_err() {
                        tracing::debug!("view closed");
                    }
                }
            }
            Err(e) => tracing::debug!("list nodes: {e:#}"),
        }
        tokio::time::sleep(wait).await;
    }
}

async fn on_out(view: &watch::Sender<View>, cmd_tx: &mpsc::Sender<Front>, msg: Out) {
    if let Out::AiAsk { id, reset, append } = &msg {
        let id = id.clone();
        let tx = cmd_tx.clone();
        let view = view.clone();
        let reset = *reset;
        let append = append.clone();
        tokio::spawn(async move {
            patch(&view, |g| {
                if reset {
                    g.thread = append.clone();
                } else {
                    g.thread.extend(append.clone());
                }
            });
            let thread = view.borrow().thread.clone();
            let result = ai::share_result(id, thread).await;
            if let In::AiResult {
                content,
                tool_calls,
                error: None,
                ..
            } = &result
            {
                if tool_calls.is_empty() && !content.is_empty() {
                    patch(&view, |g| {
                        g.thread
                            .push(serde_json::json!({"role":"assistant","content": content}));
                    });
                }
            }
            if tx.send(Front::Msg(result)).await.is_err() {
                tracing::debug!("ai result dropped");
            }
        });
        return;
    }
    patch(view, |g| match msg {
        Out::Jpeg { data } => {
            if let Ok(j) = STANDARD.decode(data) {
                g.jpeg = Some(j);
                g.jpeg_n = g.jpeg_n.wrapping_add(1);
            }
        }
        Out::Tabs { tabs, url } => {
            g.url = tabs
                .iter()
                .find(|t| t.active)
                .map(|t| t.url.clone())
                .unwrap_or(url);
            g.tabs = serde_json::to_value(&tabs).unwrap_or(serde_json::json!([]));
        }
        Out::Stdout { data } => {
            if g.kind == "shell" {
                g.stdout.push_str(&data);
                if g.stdout.len() > 80_000 {
                    g.stdout = g.stdout[g.stdout.len() - 40_000..].into();
                }
            }
        }
        Out::Hello {
            kind,
            video,
            width,
            height,
            ice_servers,
            ..
        } => {
            g.kind = match kind {
                Kind::Browser => "browser",
                Kind::Shell => "shell",
                Kind::Agent => "agent",
            }
            .into();
            g.video = match video {
                Video::Webrtc => "webrtc",
                Video::Jpeg => "jpeg",
                Video::None => "none",
            }
            .into();
            g.width = width;
            g.height = height;
            if !ice_servers.is_empty() {
                g.ice_servers = serde_json::to_value(&ice_servers).unwrap_or(serde_json::json!([]));
            }
            if kind != Kind::Browser {
                g.jpeg = None;
            }
        }
        Out::Answer { sdp } => {
            if g.want_answer {
                g.answer = sdp;
                g.want_answer = false;
            }
        }
        Out::Ice {
            candidate,
            sdp_mid,
            sdp_mline_index,
        } => {
            if !g.answer.is_empty() {
                g.ice.push(serde_json::json!({
                    "type": "ice",
                    "candidate": candidate,
                    "sdp_mid": sdp_mid,
                    "sdp_mline_index": sdp_mline_index,
                }));
            }
        }
        Out::Eval { result } => g.last = result,
        Out::Error { message } => {
            g.last = format!("error {message}");
            g.log.push(format!("error {message}"));
        }
        Out::AiChat { id } => {
            g.log.push(format!("chat {id}"));
            g.kind = "agent".into();
        }
        Out::AiReply { text } => g.log.push(format!("ai {text}")),
        Out::AiStep { tool, result, .. } => {
            g.log.push(format!("step {tool}: {result}"));
        }
        Out::Exit { code } => g.last = format!("exit {code}"),
        Out::AiAsk { .. } => {}
    });
}
