//! Box dials Teleport. App is control. Video is JPEG on App or WebRTC. SSH PTY is native.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use tokio::sync::{mpsc, oneshot};

use connect2::chrome::Chromium;
use connect2::code;
use connect2::identity::Identity;
use connect2::jpeg;
use connect2::plane::{self, App, ClientMsg};
use connect2::protocol::{In, Kind, Out, Video};
use connect2::rtc::{Ice, Rtc};
use connect2::shell::Shell;
use connect2::teleport::{self, AppRouter, TunnelCfg};
use connect2::video;

#[derive(Parser)]
#[command(name = "connect2-agent", about = "Chromium or shell on a private box, via Teleport")]
struct Cli {
    /// Teleport proxy (`host:3080` or `https://teleport.example:443`)
    #[arg(long)]
    proxy: String,
    /// Reverse-tunnel listener (`host:3024`). Defaults to proxy host on :3024.
    #[arg(long)]
    tunnel: Option<String>,
    /// Join token (`tctl tokens add --type=node`). Reused after first join if identity dir has certs.
    #[arg(long)]
    token: Option<String>,
    #[arg(long, default_value = "identity")]
    identity: std::path::PathBuf,
    #[arg(long)]
    hostname: Option<String>,
    #[arg(long)]
    tls_ca: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = false)]
    insecure: bool,
    #[arg(long, default_value_t = 4)]
    max_sessions: usize,
    #[arg(long, default_value_t = 180)]
    idle_secs: u64,
    #[arg(long, default_value = "stun:stun.l.google.com:19302")]
    stun: Vec<String>,
    #[arg(long)]
    turn: Option<String>,
    #[arg(long)]
    turn_user: Option<String>,
    #[arg(long)]
    turn_pass: Option<String>,
}

struct Slot {
    gen: u64,
    tx: mpsc::Sender<In>,
    gone: Option<oneshot::Receiver<()>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "connect2=info".into()),
        )
        .compact()
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    connect2::chrome::Chromium::reap_stale();
    run(Cli::parse()).await
}

async fn run(cli: Cli) -> Result<()> {
    let mut identity = Identity::load_or_create(&cli.identity, cli.hostname.clone())?;
    if identity.certs.is_none() {
        let token = cli
            .token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("--token required for first join"))?;
        tracing::info!(proxy = %cli.proxy, host_id = %identity.host_id, "joining teleport");
        let certs = connect2::join::register(
            &cli.proxy,
            &token,
            &identity,
            cli.tls_ca.as_deref(),
            cli.insecure,
        )
        .await?;
        identity.save_certs(&cli.identity, certs)?;
        tracing::info!("joined; certs written to {}", cli.identity.display());
    }
    let identity = Arc::new(identity);

    let ice = Ice::new(
        &cli.stun,
        cli.turn.as_deref(),
        cli.turn_user.as_deref(),
        cli.turn_pass.as_deref(),
    );
    let max = cli.max_sessions.max(1);
    let idle = Duration::from_secs(cli.idle_secs);
    let mut sessions: HashMap<String, Slot> = HashMap::new();
    let mut next_gen: u64 = 1;
    let (done_tx, mut done_rx) = mpsc::channel::<(String, u64)>(32);
    let (app_tx, mut app_rx) = mpsc::channel::<App>(64);
    let (out_tx, mut out_rx) = mpsc::channel::<ClientMsg>(64);
    let router = AppRouter::new();
    let tunnel_cfg = TunnelCfg {
        proxy: cli.proxy.clone(),
        tunnel: cli.tunnel.clone(),
        insecure: cli.insecure,
        tls_ca: cli.tls_ca.clone(),
    };

    let tunnel_id = identity.clone();
    let tunnel_router = router.clone();
    tokio::spawn(async move {
        if let Err(e) = teleport::run(tunnel_id, tunnel_cfg, app_tx, tunnel_router).await {
            tracing::error!("tunnel: {e:#}");
        }
    });

    tracing::info!(
        host_id = %identity.host_id,
        name = %identity.hostname,
        "tsh ssh {user}@{name}  —  or  tsh ssh {user}@{name} -s connect-app",
        user = "${USER}",
        name = identity.hostname,
    );

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            ended = done_rx.recv() => {
                if let Some((ch, gen)) = ended {
                    if sessions.get(&ch).is_some_and(|s| s.gen == gen) {
                        sessions.remove(&ch);
                        tracing::info!(channel = %ch, n = sessions.len(), "session gone");
                    }
                }
            }
            msg = out_rx.recv() => {
                if let Some(msg) = msg {
                    router.send(msg).await;
                }
            }
            msg = app_rx.recv() => {
                let Some(a) = msg else { break };
                on_app(&mut sessions, &mut next_gen, &done_tx, &out_tx, a, max, idle, ice.clone()).await;
            }
        }
    }
    Ok(())
}

async fn on_app(
    sessions: &mut HashMap<String, Slot>,
    next_gen: &mut u64,
    done_tx: &mpsc::Sender<(String, u64)>,
    tx: &mpsc::Sender<ClientMsg>,
    a: App,
    max: usize,
    idle: Duration,
    ice: Ice,
) {
    if a.channel.is_empty() {
        return;
    }
    let Ok(raw) = serde_json::from_slice::<In>(&a.data) else {
        return;
    };
    let ch = a.channel;
    let dst = a.src;
    if matches!(raw, In::Close) {
        if let Some(s) = sessions.get(&ch) {
            let _ = s.tx.try_send(In::Close);
            tracing::info!(channel = %ch, "close");
        }
        return;
    }
    sessions.retain(|_, s| !s.tx.is_closed());
    if matches!(raw, In::Open { .. }) {
        let prev = sessions.remove(&ch).map(|s| {
            let _ = s.tx.try_send(In::Close);
            s.gone
        });
        if sessions.len() >= max {
            tracing::warn!(channel = %ch, n = sessions.len(), max, "cap");
            let _ = plane::send_out(tx, &dst, &ch, &Out::Error { message: format!("cap ({max})") }).await;
            return;
        }
        let Some((cmd, kind, video, height)) = raw.start_session() else {
            return;
        };
        let (in_tx, in_rx) = mpsc::channel(32);
        let gen = *next_gen;
        *next_gen += 1;
        let _ = in_tx.send(cmd).await;
        let (gone_tx, gone_rx) = oneshot::channel();
        sessions.insert(ch.clone(), Slot { gen, tx: in_tx, gone: Some(gone_rx) });
        let done = done_tx.clone();
        let plane_tx = tx.clone();
        tokio::spawn(async move {
            if let Some(Some(prev)) = prev {
                let _ = prev.await;
            }
            if kind == Kind::Browser {
                Chromium::reap_stale();
                for _ in 0..40 {
                    if connect2::chrome::live() < connect2::chrome::max_live() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                if connect2::chrome::live() >= connect2::chrome::max_live() {
                    tracing::warn!(channel = %ch, "chromium cap");
                    let _ = plane::send_out(
                        &plane_tx,
                        &dst,
                        &ch,
                        &Out::Error {
                            message: format!(
                                "chromium cap ({}) — close the other browser session",
                                connect2::chrome::max_live()
                            ),
                        },
                    )
                    .await;
                    let _ = gone_tx.send(());
                    let _ = done.send((ch, gen)).await;
                    return;
                }
            }
            match (kind, video) {
                (Kind::Shell, _) => shell_session(ch.clone(), dst, plane_tx, in_rx, idle).await,
                (Kind::Agent, _) => code::session(ch.clone(), dst, plane_tx, in_rx, idle).await,
                (Kind::Browser, Video::Webrtc) => {
                    let (w, h) = video::size(height);
                    webrtc_session(ch.clone(), dst, plane_tx, in_rx, idle, ice.clone(), w, h).await
                }
                (Kind::Browser, _) => {
                    jpeg::session(ch.clone(), dst, plane_tx, in_rx, idle, height).await
                }
            }
            let _ = gone_tx.send(());
            let _ = done.send((ch, gen)).await;
        });
        return;
    }
    if let Some(s) = sessions.get(&ch) {
        let _ = s.tx.send(raw).await;
    }
}

async fn webrtc_session(
    channel: String,
    dst: String,
    tx: mpsc::Sender<ClientMsg>,
    mut cmds: mpsc::Receiver<In>,
    idle: Duration,
    ice: Ice,
    w: u32,
    h: u32,
) {
    tracing::info!(%channel, width = w, height = h, ice = ice.browser_servers().len(), "webrtc");
    let chrome = match Chromium::spawn_unless_close(w, h, &mut cmds).await {
        Ok(Some(c)) => c,
        Ok(None) => return,
        Err(e) => {
            tracing::error!("chrome: {e:#}");
            let _ = plane::send_out(&tx, &dst, &channel, &Out::Error { message: e.to_string() }).await;
            return;
        }
    };
    chrome.push_tabs(tx.clone(), channel.clone(), dst.clone());
    if plane::send_out(
        &tx,
        &dst,
        &channel,
        &Out::Hello {
            kind: Kind::Browser,
            video: Video::Webrtc,
            width: w,
            height: h,
            fps: video::FPS,
            ice_servers: ice.browser_servers(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let mut pending_ice = Vec::new();
    let mut deadline = tokio::time::Instant::now() + idle;
    let rtc = loop {
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                if matches!(cmd, In::Close) { return; }
                if !idle.is_zero() { deadline = tokio::time::Instant::now() + idle; }
                match cmd {
                    In::Offer { sdp } => {
                        match answer_until_close(ice.clone(), w, h, sdp, &mut cmds, &mut pending_ice).await {
                            Ok(Some(v)) => break v,
                            Ok(None) => return,
                            Err(e) => {
                                tracing::error!("webrtc answer: {e:#}");
                                let _ = plane::send_out(&tx, &dst, &channel, &Out::Error { message: format!("webrtc: {e}") }).await;
                                return;
                            }
                        }
                    }
                    In::Ice { candidate, sdp_mid, sdp_mline_index } => {
                        pending_ice.push((candidate, sdp_mid, sdp_mline_index));
                    }
                    cmd => {
                        if let Some(out) = chrome.apply(cmd).await {
                            if plane::send_out(&tx, &dst, &channel, &out).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline), if !idle.is_zero() => return,
        }
    };
    let (rtc, sdp, mut ice_rx) = rtc;
    let mut rtc = attach_rtc(rtc, &chrome);
    if plane::send_out(&tx, &dst, &channel, &Out::Answer { sdp }).await.is_err() {
        return;
    }
    for (candidate, sdp_mid, sdp_mline_index) in pending_ice.drain(..) {
        let _ = rtc.add_ice(candidate, sdp_mid, sdp_mline_index).await;
    }
    loop {
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { break };
                if matches!(cmd, In::Close) { break; }
                if !idle.is_zero() { deadline = tokio::time::Instant::now() + idle; }
                match cmd {
                    In::Offer { sdp } => {
                        tracing::info!("renegotiate");
                        match answer_until_close(ice.clone(), w, h, sdp, &mut cmds, &mut pending_ice).await {
                            Ok(Some((new_rtc, answer, new_ice))) => {
                                ice_rx = new_ice;
                                rtc = attach_rtc(new_rtc, &chrome);
                                if plane::send_out(&tx, &dst, &channel, &Out::Answer { sdp: answer }).await.is_err() {
                                    break;
                                }
                            }
                            Ok(None) => break,
                            Err(e) => tracing::warn!("renegotiate: {e:#}"),
                        }
                    }
                    In::Ice { candidate, sdp_mid, sdp_mline_index } => {
                        let _ = rtc.add_ice(candidate, sdp_mid, sdp_mline_index).await;
                    }
                    cmd => {
                        if let Some(out) = chrome.apply(cmd).await {
                            if plane::send_out(&tx, &dst, &channel, &out).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
            msg = ice_rx.recv() => {
                let Some(msg) = msg else { continue };
                if plane::send_out(&tx, &dst, &channel, &msg).await.is_err() {
                    break;
                }
            }
            _ = tokio::time::sleep_until(deadline), if !idle.is_zero() => {
                tracing::info!(%channel, "idle");
                break;
            }
        }
    }
}

async fn answer_until_close(
    ice: Ice,
    w: u32,
    h: u32,
    sdp: String,
    cmds: &mut mpsc::Receiver<In>,
    pending_ice: &mut Vec<(String, Option<String>, Option<u16>)>,
) -> Result<Option<(Rtc, String, mpsc::Receiver<Out>)>> {
    let work = Rtc::answer(ice, w, h, &sdp);
    tokio::pin!(work);
    loop {
        tokio::select! {
            biased;
            cmd = cmds.recv() => match cmd {
                None | Some(In::Close) => return Ok(None),
                Some(In::Ice { candidate, sdp_mid, sdp_mline_index }) => {
                    pending_ice.push((candidate, sdp_mid, sdp_mline_index));
                }
                Some(_) => {}
            },
            r = &mut work => {
                loop {
                    match cmds.try_recv() {
                        Ok(In::Close) | Err(mpsc::error::TryRecvError::Disconnected) => {
                            return Ok(None);
                        }
                        Ok(In::Ice {
                            candidate,
                            sdp_mid,
                            sdp_mline_index,
                        }) => pending_ice.push((candidate, sdp_mid, sdp_mline_index)),
                        Ok(_) => {}
                        Err(mpsc::error::TryRecvError::Empty) => break,
                    }
                }
                return Ok(Some(r?));
            }
        }
    }
}

fn attach_rtc(rtc: Rtc, chrome: &Chromium) -> std::sync::Arc<Rtc> {
    if let Some(jpeg) = chrome.last_jpeg() {
        rtc.push_jpeg(jpeg);
    }
    let rtc = std::sync::Arc::new(rtc);
    let pump = rtc.clone();
    let mut frames = chrome.subscribe();
    tokio::spawn(async move {
        loop {
            match frames.recv().await {
                Ok(jpeg) => pump.push_jpeg(jpeg),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    rtc
}

async fn shell_session(
    channel: String,
    dst: String,
    tx: mpsc::Sender<ClientMsg>,
    mut cmds: mpsc::Receiver<In>,
    idle: Duration,
) {
    tracing::info!(%channel, "shell");
    let (mut sh, mut out) = match Shell::spawn() {
        Ok(v) => v,
        Err(e) => {
            let _ = plane::send_out(&tx, &dst, &channel, &Out::Error { message: e.to_string() }).await;
            return;
        }
    };
    if plane::send_out(
        &tx,
        &dst,
        &channel,
        &Out::Hello {
            kind: Kind::Shell,
            video: Video::None,
            width: 0,
            height: 0,
            fps: 0,
            ice_servers: Vec::new(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let mut deadline = tokio::time::Instant::now() + idle;
    loop {
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { break };
                if matches!(cmd, In::Close) { break; }
                if !idle.is_zero() { deadline = tokio::time::Instant::now() + idle; }
                match cmd {
                    In::Stdin { data } => {
                        if sh.write(data.as_bytes()).await.is_err() { break; }
                    }
                    In::Resize { cols, rows } => {
                        let _ = sh.resize(cols, rows);
                    }
                    _ => {}
                }
            }
            chunk = out.recv() => {
                let Some(chunk) = chunk else { break };
                let data = String::from_utf8_lossy(&chunk).into_owned();
                if plane::send_out(&tx, &dst, &channel, &Out::Stdout { data }).await.is_err() {
                    break;
                }
            }
            _ = tokio::time::sleep_until(deadline), if !idle.is_zero() => break,
        }
    }
    let _ = plane::send_out(&tx, &dst, &channel, &Out::Exit { code: 0 }).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect2::protocol::In;
    use std::time::Duration;

    fn ice() -> Ice {
        Ice::new(&["stun:stun.l.google.com:19302".into()], None, None, None)
    }

    #[tokio::test]
    async fn on_app_ignores_empty_channel() {
        let mut sessions = HashMap::new();
        let mut next = 1u64;
        let (done_tx, _done_rx) = mpsc::channel(1);
        let (tx, _rx) = mpsc::channel(1);
        on_app(
            &mut sessions,
            &mut next,
            &done_tx,
            &tx,
            App {
                src: "a".into(),
                dst: "b".into(),
                data: serde_json::to_vec(&In::Open {
                    kind: Kind::Shell,
                    video: Video::None,
                    height: 0,
                })
                .unwrap(),
                channel: String::new(),
            },
            4,
            Duration::from_secs(30),
            ice(),
        )
        .await;
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn on_app_open_shell_and_close() {
        let mut sessions = HashMap::new();
        let mut next = 1u64;
        let (done_tx, mut done_rx) = mpsc::channel(4);
        let (tx, mut rx) = mpsc::channel(16);
        on_app(
            &mut sessions,
            &mut next,
            &done_tx,
            &tx,
            App {
                src: "alice".into(),
                dst: "box".into(),
                data: serde_json::to_vec(&In::Open {
                    kind: Kind::Shell,
                    video: Video::None,
                    height: 0,
                })
                .unwrap(),
                channel: "alice:box".into(),
            },
            4,
            Duration::from_secs(30),
            ice(),
        )
        .await;
        assert_eq!(sessions.len(), 1);
        assert!(sessions.contains_key("alice:box"));
        let hello = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("hello")
            .expect("msg");
        let out: Out = serde_json::from_slice(&hello.data).unwrap();
        assert!(matches!(out, Out::Hello { kind: Kind::Shell, .. }));

        on_app(
            &mut sessions,
            &mut next,
            &done_tx,
            &tx,
            App {
                src: "alice".into(),
                dst: "box".into(),
                data: serde_json::to_vec(&In::Close).unwrap(),
                channel: "alice:box".into(),
            },
            4,
            Duration::from_secs(30),
            ice(),
        )
        .await;
        let ended = tokio::time::timeout(Duration::from_secs(3), done_rx.recv())
            .await
            .expect("done")
            .expect("ended");
        if sessions.get(&ended.0).is_some_and(|s| s.gen == ended.1) {
            sessions.remove(&ended.0);
        }
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn on_app_cap() {
        let mut sessions = HashMap::new();
        let mut next = 1u64;
        let (done_tx, _done_rx) = mpsc::channel(4);
        let (tx, mut rx) = mpsc::channel(16);
        for i in 0..2 {
            on_app(
                &mut sessions,
                &mut next,
                &done_tx,
                &tx,
                App {
                    src: "alice".into(),
                    dst: "box".into(),
                    data: serde_json::to_vec(&In::Open {
                        kind: Kind::Shell,
                        video: Video::None,
                        height: 0,
                    })
                    .unwrap(),
                    channel: format!("ch-{i}"),
                },
                1,
                Duration::from_secs(30),
                ice(),
            )
            .await;
        }
        assert_eq!(sessions.len(), 1);
        let mut saw_cap = false;
        while let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(800), rx.recv()).await {
            if let Ok(Out::Error { message }) = serde_json::from_slice::<Out>(&m.data) {
                if message.contains("cap") {
                    saw_cap = true;
                    break;
                }
            }
        }
        assert!(saw_cap);
    }
}
