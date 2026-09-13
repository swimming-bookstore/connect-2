//! Connect 2 laptop client.
//!
//! No subcommand: loopback hub. With a native window, `connect2-tauri` starts
//! that hub in-process (one binary). `--no-open` is hub only. Subcommands:
//! `ping`, `ls`, `open` for scripts and e2e.

#[cfg(feature = "desktop")]
mod window;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use clap::Parser;
use connect2::ai;
use connect2::hub::cli::{cfg_of, parse_ssh_target, pick_login, Cli, Cmd};
use connect2::hub::lab;
use connect2::hub::session;
use connect2::protocol::{In, Kind, Out, Video};
use connect2_teleport::user::{self, Cfg};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "connect2=info".into()),
        )
        .compact()
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    if cli.cmd.is_some() {
        return tokio::runtime::Runtime::new()?.block_on(run_cmd(cli));
    }
    run_ui(cli)
}

fn run_ui(cli: Cli) -> Result<()> {
    let http = cli.http.clone();
    let no_open = cli.no_open;
    #[cfg(feature = "desktop")]
    if !no_open {
        eprintln!("connect2  hub http://{http}  (inside connect2-tauri)");
        return window::run();
    }
    let _ = no_open;
    eprintln!("connect2  hub http://{http}  (API; --no-open)");
    eprintln!("  DaisyUI · Shell · Browser JPEG · Agent · laptop AI share");
    eprintln!("  inventory: Auth ListResources   session: russh connect-app");
    if session::spawn_hub(cli)?.join().is_err() {
        tracing::error!("hub thread panicked");
    }
    Ok(())
}

async fn run_cmd(cli: Cli) -> Result<()> {
    match cli.cmd.clone() {
        Some(Cmd::Ping) => {
            let mut info = connect2_teleport::login::ping_auth(&cli.proxy, cli.insecure).await?;
            lab::inject(&mut info);
            println!("cluster {}", info.cluster);
            println!("auth {}", info.auth_type);
            if !info.second_factor.is_empty() {
                println!("second_factor {}", info.second_factor);
            }
            for c in info.connectors {
                println!("connector {} {} {}", c.kind, c.name, c.display);
            }
            Ok(())
        }
        Some(Cmd::Login {
            auth,
            connector,
            user,
            password,
            otp,
        }) => {
            let auth = auth.unwrap_or_default();
            let mut info = connect2_teleport::login::ping_auth(&cli.proxy, cli.insecure).await?;
            lab::inject(&mut info);
            let r = if let Some((kind, name)) = pick_login(&auth, connector.as_deref(), &info) {
                if let Some(r) = lab::login(&cli.identity, &name, |_| {}) {
                    r?
                } else {
                    connect2_teleport::login::login_sso(
                        &cli.proxy,
                        &cli.identity,
                        &kind,
                        &name,
                        cli.insecure,
                    )
                    .await?
                }
            } else {
                let user = user.unwrap_or_else(|| cli.teleport_user.clone());
                let password = password.unwrap_or_else(read_password);
                let otp = match otp {
                    Some(o) => o,
                    None if info.second_factor == "off" || info.second_factor.is_empty() => {
                        String::new()
                    }
                    None => read_otp(),
                };
                connect2_teleport::login::login_password(
                    &cli.proxy,
                    &cli.identity,
                    &user,
                    &password,
                    &otp,
                    cli.insecure,
                )
                .await?
            };
            println!(
                "logged in as {} -> {}",
                r.username,
                r.identity_path.display()
            );
            Ok(())
        }
        Some(Cmd::Ls) => {
            let nodes = user::list_nodes(&cfg_of(&cli)).await?;
            if nodes.is_empty() {
                println!("(no nodes)");
            }
            for n in nodes {
                let via = if n.tunnel { "tunnel" } else { "direct" };
                println!("{}\t{}\t{via}", n.name, n.id);
            }
            Ok(())
        }
        Some(Cmd::Open {
            kind,
            jpeg_out,
            ask,
            stdin,
            url,
            wait_secs,
        }) => {
            if cli.node.is_empty() {
                bail!("open needs --node");
            }
            open_session(
                &cfg_of(&cli),
                &cli.node,
                kind.into(),
                jpeg_out,
                ask,
                stdin,
                url,
                Duration::from_secs(wait_secs),
            )
            .await
        }
        Some(Cmd::Ssh { target, command }) => run_ssh(&cli, &target, &command).await,
        None => Ok(()),
    }
}

fn read_password() -> String {
    prompt_secret("password: ")
}

fn read_otp() -> String {
    prompt_secret("OTP: ")
}

fn prompt_secret(label: &str) -> String {
    eprint!("{label}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim_end().to_string()
}

fn tty_size() -> (u32, u32) {
    let mut ws = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 {
            (u32::from(ws.ws_col.max(8)), u32::from(ws.ws_row.max(4)))
        } else {
            (80, 24)
        }
    }
}

async fn run_ssh(cli: &Cli, target: &str, command: &[String]) -> Result<()> {
    let (login, node) = parse_ssh_target(target, &cli.user);
    let mut cfg = cfg_of(cli);
    cfg.login = login;
    let (cols, rows) = tty_size();
    let remote = if command.is_empty() {
        None
    } else {
        Some(command.join(" "))
    };
    let sess = user::dial_ssh(&cfg, &node, cols, rows, remote.as_deref()).await?;
    let mut ch = sess.channel;

    let raw_ok = std::io::stdin().is_terminal() && std::io::stdout().is_terminal() && remote.is_none();
    let mut raw: Option<nix::sys::termios::Termios> = None;
    if raw_ok {
        let orig = nix::sys::termios::tcgetattr(std::io::stdin())?;
        let mut raw_mode = orig.clone();
        nix::sys::termios::cfmakeraw(&mut raw_mode);
        nix::sys::termios::tcsetattr(std::io::stdin(), nix::sys::termios::SetArg::TCSANOW, &raw_mode)?;
        raw = Some(orig);
    }

    struct Restore(Option<nix::sys::termios::Termios>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(orig) = self.0.take() {
                let _ = nix::sys::termios::tcsetattr(
                    std::io::stdin(),
                    nix::sys::termios::SetArg::TCSANOW,
                    &orig,
                );
            }
        }
    }
    let _restore = Restore(raw);

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut buf = [0u8; 8192];
    let mut stdin_open = true;
    loop {
        tokio::select! {
            n = stdin.read(&mut buf), if stdin_open => {
                match n {
                    Ok(0) | Err(_) => {
                        stdin_open = false;
                        let _ = ch.eof().await;
                    }
                    Ok(n) => {
                        ch.data(&buf[..n]).await?;
                    }
                }
            }
            msg = ch.wait() => {
                match msg {
                    Some(russh::ChannelMsg::Data { ref data }) => {
                        stdout.write_all(data).await?;
                        stdout.flush().await?;
                    }
                    Some(russh::ChannelMsg::ExtendedData { ref data, .. }) => {
                        stdout.write_all(data).await?;
                        stdout.flush().await?;
                    }
                    Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}


async fn open_session(
    cfg: &Cfg,
    node: &str,
    kind: Kind,
    jpeg_out: Option<PathBuf>,
    ask: Option<String>,
    stdin: Vec<String>,
    url: Option<String>,
    wait: Duration,
) -> Result<()> {
    let video = match kind {
        Kind::Browser => Video::Jpeg,
        _ => Video::None,
    };
    let sess = user::dial_app(cfg, node).await?;
    let mut stdio = sess.into_stdio();
    send_in(
        &mut stdio.stdin,
        &In::Open {
            kind,
            video,
            height: 720,
        },
    )
    .await?;

    let mut lines = BufReader::new(stdio.stdout).lines();
    let deadline = tokio::time::Instant::now() + wait;
    let mut saw_hello = false;
    let mut frames = 0u32;
    let mut last_url = String::new();
    let mut thread: Vec<serde_json::Value> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let line = tokio::time::timeout(left, lines.next_line()).await;
        let line = match line {
            Ok(Ok(Some(l))) => l,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(out) = serde_json::from_str::<Out>(line) else {
            eprintln!("raw: {line}");
            continue;
        };
        match &out {
            Out::Hello {
                kind,
                video,
                width,
                height,
                ..
            } => {
                println!("hello kind={kind:?} video={video:?} {width}x{height}");
                saw_hello = true;
                if kind == &Kind::Browser {
                    if let Some(u) = &url {
                        send_in(&mut stdio.stdin, &In::Navigate { url: u.clone() }).await?;
                    }
                }
                if kind == &Kind::Shell {
                    for s in &stdin {
                        send_in(
                            &mut stdio.stdin,
                            &In::Stdin {
                                data: format!("{s}\n"),
                            },
                        )
                        .await?;
                    }
                }
                if kind == &Kind::Agent {
                    if let Some(text) = &ask {
                        send_in(&mut stdio.stdin, &In::AiUser { text: text.clone() }).await?;
                    }
                }
            }
            Out::Stdout { data } => print!("{data}"),
            Out::Jpeg { data } => {
                frames += 1;
                if let Some(path) = &jpeg_out {
                    if let Ok(bytes) = STANDARD.decode(data) {
                        std::fs::write(path, bytes)?;
                    }
                }
                if frames == 1 {
                    println!("jpeg frame 1 ({} bytes b64)", data.len());
                }
            }
            Out::Tabs { url, tabs } => {
                last_url = url.clone();
                println!("tabs n={} url={url}", tabs.len());
            }
            Out::Eval { result } => println!("eval {result}"),
            Out::AiChat { id } => println!("ai chat {id}"),
            Out::AiAsk { id, reset, append } => {
                println!("ai ask {id} (complete on laptop)");
                if *reset {
                    thread = append.clone();
                } else {
                    thread.extend(append.clone());
                }
                let result = ai::share_result(id.clone(), thread.clone()).await;
                if let In::AiResult {
                    content,
                    tool_calls,
                    error: None,
                    ..
                } = &result
                {
                    if tool_calls.is_empty() && !content.is_empty() {
                        thread.push(serde_json::json!({"role":"assistant","content": content}));
                    }
                }
                send_in(&mut stdio.stdin, &result).await?;
            }
            Out::AiStep { tool, result, .. } => {
                println!("ai step {tool}: {}", clip(result, 400));
            }
            Out::AiReply { text } => println!("ai reply: {text}"),
            Out::Error { message } => eprintln!("error: {message}"),
            Out::Exit { code } => {
                println!("exit {code}");
                break;
            }
            _ => {}
        }
    }
    let _ = send_in(&mut stdio.stdin, &In::Close).await;
    if !saw_hello {
        return Err(anyhow!("no hello from box (tunnel/session failed)"));
    }
    if kind == Kind::Browser && frames == 0 {
        return Err(anyhow!("browser hello but no jpeg frames"));
    }
    if !last_url.is_empty() {
        println!("last url {last_url}");
    }
    println!("ok kind={kind:?} jpeg_frames={frames}");
    Ok(())
}

async fn send_in<W: AsyncWriteExt + Unpin>(w: &mut W, msg: &In) -> Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    w.write_all(&line).await?;
    w.flush().await?;
    Ok(())
}

fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect2::hub::http;
    use connect2_teleport::login::{parse_auth_ping, Connector};

    #[test]
    fn ssh_target_user_at_host() {
        assert_eq!(
            parse_ssh_target("packer@box-1", "demo"),
            ("packer".into(), "box-1".into())
        );
        assert_eq!(
            parse_ssh_target("box-2", "packer"),
            ("packer".into(), "box-2".into())
        );
        assert_eq!(
            parse_ssh_target("root@box-3", "packer"),
            ("root".into(), "box-3".into())
        );
    }

    #[test]
    fn pick_login_local_skips_sso() {
        let info = connect2_teleport::login::AuthInfo {
            cluster: "teleport.local".into(),
            auth_type: "local".into(),
            second_factor: "otp".into(),
            connectors: vec![Connector {
                kind: "oidc".into(),
                name: "okta".into(),
                display: "Okta".into(),
            }],
        };
        assert!(pick_login("local", None, &info).is_none());
        assert_eq!(
            pick_login("oidc", Some("okta"), &info),
            Some(("oidc".into(), "okta".into()))
        );
        let local_only = connect2_teleport::login::AuthInfo {
            cluster: "teleport.local".into(),
            auth_type: "local".into(),
            second_factor: "otp".into(),
            connectors: vec![],
        };
        assert!(pick_login("", None, &local_only).is_none());
        let v = serde_json::json!({
            "cluster_name": "teleport.local",
            "auth": { "type": "local", "second_factor": "otp" }
        });
        let ping = parse_auth_ping(&v);
        assert_eq!(ping.auth_type, "local");
        assert!(ping.connectors.is_empty());
    }

    #[test]
    fn ws_accept_rfc6455() {
        assert_eq!(
            http::ws_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn clip_and_pick_login_sso() {
        assert_eq!(clip("hi", 8), "hi");
        assert_eq!(clip("abcdefghij", 4), "abcd…");
        let info = connect2_teleport::login::AuthInfo {
            cluster: "c".into(),
            auth_type: "oidc".into(),
            second_factor: "off".into(),
            connectors: vec![
                Connector {
                    kind: "oidc".into(),
                    name: "okta".into(),
                    display: "Okta".into(),
                },
                Connector {
                    kind: "github".into(),
                    name: "gh".into(),
                    display: "GitHub".into(),
                },
            ],
        };
        assert_eq!(
            pick_login("", None, &info),
            Some(("oidc".into(), "okta".into()))
        );
        assert_eq!(
            pick_login("github", None, &info),
            Some(("github".into(), "gh".into()))
        );
        assert_eq!(
            pick_login("sso", None, &info),
            Some(("oidc".into(), "okta".into()))
        );
        assert_eq!(
            pick_login("", Some("GitHub"), &info),
            Some(("github".into(), "gh".into()))
        );
        assert_eq!(
            pick_login("okta", None, &info),
            Some(("oidc".into(), "okta".into()))
        );
        let local = connect2_teleport::login::AuthInfo {
            cluster: "c".into(),
            auth_type: "local".into(),
            second_factor: "otp".into(),
            connectors: info.connectors.clone(),
        };
        assert_eq!(pick_login("", None, &local), None);
        assert_eq!(
            pick_login("okta", None, &local),
            Some(("oidc".into(), "okta".into()))
        );
    }
}
