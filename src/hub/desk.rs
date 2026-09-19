//! Laptop Grok tools that drive Shell / Browser / Agent on a box.

use std::time::Duration;

use crate::ai;
use crate::protocol::{AiToolCall, In, Kind, Video};
use crate::tools;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use super::cli::Cli;
use super::pipe::PipeHub;
use super::session::{open_kind, resolve_dst, wait_kind, Front};
use super::view::{patch, View};

pub async fn agent_ask(
    cli: &Cli,
    pipe: &PipeHub,
    view: &watch::Sender<View>,
    cmd_tx: &mpsc::Sender<Front>,
    text: &str,
) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let dst = resolve_dst(view, "");
    if dst.is_empty() {
        patch(view, |g| g.log.push("error no boxes online".into()));
        return;
    }
    if view.borrow().kind != "agent" || view.borrow().dst != dst {
        if let Err(e) = open_kind(
            cli,
            pipe,
            view,
            cmd_tx,
            &dst,
            Kind::Agent,
            Video::None,
            0,
        )
        .await
        {
            patch(view, |g| g.log.push(format!("error {e:#}")));
            return;
        }
        if let Err(e) = wait_agent_ready(view, 8).await {
            tracing::debug!("agent wait: {e:#}");
        }
    }
    patch(view, |g| {
        let line = format!("you {text}");
        if g.log.last().map(|s| s.as_str()) != Some(line.as_str()) {
            g.log.push(line);
        }
    });
    if let Err(e) = pipe.write(In::AiUser { text: text.into() }).await {
        patch(view, |g| g.log.push(format!("error {e:#}")));
    }
}

pub async fn desk_ask(
    cli: &Cli,
    pipe: &PipeHub,
    view: &watch::Sender<View>,
    cmd_tx: &mpsc::Sender<Front>,
    text: &str,
) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    patch(view, |g| {
        let line = format!("you {text}");
        if g.desk.last().map(|s| s.as_str()) != Some(line.as_str()) {
            g.desk.push(line);
        }
    });
    let names: Vec<_> = view.borrow().peers.iter().map(|p| p.name.clone()).collect();
    let system = format!(
        "You operate Connect 2 from this laptop. Boxes: {}. \
         Use tools to run shell and drive the browser. Address boxes by name. \
         Call open/open_shell to show Shell. Call shell to type a command. \
         Call browser_navigate to go to a URL. \
         To talk to the box coding agent, call ask_agent with the message (do not use shell for that). \
         Be concise. Do not invent tool results.",
        if names.is_empty() {
            "(none online)".into()
        } else {
            names.join(", ")
        }
    );
    let mut messages = vec![
        serde_json::json!({"role":"system","content": system}),
        serde_json::json!({"role":"user","content": text}),
    ];
    let defs = tools::desk_tools();
    let mut reply = String::new();
    for _ in 0..8 {
        let c = match ai::complete_local(&Value::Array(messages.clone()), &defs).await {
            Ok(c) => c,
            Err(e) => {
                patch(view, |g| g.desk.push(format!("error {e:#}")));
                return;
            }
        };
        if c.tool_calls.is_empty() {
            reply = c.content;
            break;
        }
        if !c.content.trim().is_empty() {
            patch(view, |g| g.desk.push(format!("ai {}", c.content.trim())));
        }
        messages.push(serde_json::json!({
            "role":"assistant",
            "content": c.content,
            "tool_calls": c.tool_calls.iter().map(|call| serde_json::json!({
                "id": call.id,
                "type":"function",
                "function": {"name": call.name, "arguments": call.args.to_string()}
            })).collect::<Vec<_>>(),
        }));
        for call in c.tool_calls {
            let result = run_desk_tool(cli, pipe, view, cmd_tx, &call).await;
            patch(view, |g| {
                g.desk.push(format!("step {} {}: {result}", call.name, call.args))
            });
            messages.push(serde_json::json!({
                "role":"tool",
                "tool_call_id": call.id,
                "content": result,
            }));
        }
    }
    let reply = reply.trim();
    if !reply.is_empty() {
        patch(view, |g| g.desk.push(format!("ai {reply}")));
    }
}

async fn wait_browser(view: &watch::Sender<View>, secs: u64) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        {
            let g = view.borrow();
            if g.kind == "browser" && !g.channel.is_empty() && (!g.url.is_empty() || g.jpeg.is_some())
            {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("browser session did not start");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn wait_url(view: &watch::Sender<View>, want: &str, secs: u64) -> anyhow::Result<()> {
    let needle = want
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        {
            let g = view.borrow();
            if g.url.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()) {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("browser did not reach {want}");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn wait_jpeg(view: &watch::Sender<View>, secs: u64) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        {
            let g = view.borrow();
            if g.jpeg.is_some() || g.jpeg_n > 0 {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("browser produced no jpeg");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn wait_agent_ready(view: &watch::Sender<View>, secs: u64) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        {
            let g = view.borrow();
            if g.kind == "agent"
                && !g.channel.is_empty()
                && g.log.iter().any(|l| l.starts_with("chat "))
            {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("agent session did not start");
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

async fn wait_agent_reply(
    view: &watch::Sender<View>,
    before: usize,
    secs: u64,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        {
            let g = view.borrow();
            if g.log.iter().skip(before).any(|l| l.starts_with("ai ")) {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("agent produced no reply");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn wait_stdout_beyond(view: &watch::Sender<View>, before: &str, secs: u64) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        {
            let g = view.borrow();
            if g.stdout.len() > before.len() || (g.stdout != before && !g.stdout.is_empty()) {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("shell produced no output");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn run_desk_tool(
    cli: &Cli,
    pipe: &PipeHub,
    view: &watch::Sender<View>,
    cmd_tx: &mpsc::Sender<Front>,
    call: &AiToolCall,
) -> String {
    if call.name == "list_machines" {
        let g = view.borrow();
        let list: Vec<_> = g.peers.iter().map(|p| p.name.clone()).collect();
        return if list.is_empty() {
            "no boxes online".into()
        } else {
            list.join(", ")
        };
    }
    let machine = call
        .args
        .get("machine")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let dst = resolve_dst(view, machine);
    if dst.is_empty() {
        return "no boxes online".into();
    }
    match call.name.as_str() {
        "open" | "open_shell" => {
            match open_kind(
                cli,
                pipe,
                view,
                cmd_tx,
                &dst,
                Kind::Shell,
                Video::None,
                0,
            )
            .await
            {
                Ok(()) => {
                    if let Err(e) = wait_kind(view, "shell", 20).await {
                        tracing::debug!("shell wait: {e:#}");
                    }
                    "opened shell".into()
                }
                Err(e) => format!("{e:#}"),
            }
        }
        "shell" => {
            let cmd = call
                .args
                .get("command")
                .or_else(|| call.args.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if cmd.is_empty() {
                return "missing command".into();
            }
            if let Err(e) = open_kind(
                cli,
                pipe,
                view,
                cmd_tx,
                &dst,
                Kind::Shell,
                Video::None,
                0,
            )
            .await
            {
                return format!("{e:#}");
            }
            if let Err(e) = wait_kind(view, "shell", 20).await {
                tracing::debug!("shell wait: {e:#}");
            }
            let _ = wait_stdout_beyond(view, "", 15).await;
            let before = view.borrow().stdout.clone();
            let mut data = cmd.to_string();
            data.push('\n');
            if let Err(e) = pipe.write(In::Stdin { data }).await {
                return format!("{e:#}");
            }
            let _ = wait_stdout_beyond(view, &before, 8).await;
            format!("typed {cmd}")
        }
        "open_browser" | "browser_navigate" => {
            let url = call
                .args
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if let Err(e) = open_kind(
                cli,
                pipe,
                view,
                cmd_tx,
                &dst,
                Kind::Browser,
                Video::Jpeg,
                720,
            )
            .await
            {
                return format!("{e:#}");
            }
            if let Err(e) = wait_browser(view, 25).await {
                tracing::debug!("browser wait: {e:#}");
            }
            if url.is_empty() {
                let _ = wait_jpeg(view, 12).await;
                return "opened browser".into();
            }
            let url = crate::protocol::abs_url(&url);
            if let Err(e) = pipe.write(In::Navigate { url: url.clone() }).await {
                return format!("{e:#}");
            }
            let _ = wait_url(view, &url, 20).await;
            let _ = wait_jpeg(view, 12).await;
            format!("opened {url}")
        }
        "browser_eval" => {
            let expr = call
                .args
                .get("expression")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if expr.is_empty() {
                return "missing expression".into();
            }
            if let Err(e) = open_kind(
                cli,
                pipe,
                view,
                cmd_tx,
                &dst,
                Kind::Browser,
                Video::Jpeg,
                720,
            )
            .await
            {
                return format!("{e:#}");
            }
            if let Err(e) = pipe
                .write(In::Eval {
                    expression: expr.into(),
                })
                .await
            {
                return format!("{e:#}");
            }
            format!("eval {expr}")
        }
        "browser_tabs" => {
            if let Err(e) = open_kind(
                cli,
                pipe,
                view,
                cmd_tx,
                &dst,
                Kind::Browser,
                Video::Jpeg,
                720,
            )
            .await
            {
                return format!("{e:#}");
            }
            let g = view.borrow();
            if g.tabs.as_array().is_some_and(|a| !a.is_empty()) {
                g.tabs.to_string()
            } else if g.url.is_empty() {
                "[]".into()
            } else {
                g.url.clone()
            }
        }
        "open_agent" => {
            match open_kind(
                cli,
                pipe,
                view,
                cmd_tx,
                &dst,
                Kind::Agent,
                Video::None,
                0,
            )
            .await
            {
                Ok(()) => {
                    if let Err(e) = wait_agent_ready(view, 8).await {
                        tracing::debug!("agent wait: {e:#}");
                    }
                    "opened agent".into()
                }
                Err(e) => format!("{e:#}"),
            }
        }
        "ask_agent" => {
            let text = call
                .args
                .get("text")
                .or_else(|| call.args.get("message"))
                .or_else(|| call.args.get("prompt"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if text.is_empty() {
                return "missing text".into();
            }
            if view.borrow().kind != "agent" || view.borrow().dst != dst {
                if let Err(e) = open_kind(
                    cli,
                    pipe,
                    view,
                    cmd_tx,
                    &dst,
                    Kind::Agent,
                    Video::None,
                    0,
                )
                .await
                {
                    return format!("{e:#}");
                }
                if let Err(e) = wait_agent_ready(view, 8).await {
                    tracing::debug!("agent wait: {e:#}");
                }
            }
            patch(view, |g| {
                let line = format!("you {text}");
                if g.log.last().map(|s| s.as_str()) != Some(line.as_str()) {
                    g.log.push(line);
                }
            });
            let before = view.borrow().log.len();
            match pipe.write(In::AiUser { text: text.into() }).await {
                Ok(()) => {
                    let _ = wait_agent_reply(view, before, 20).await;
                    format!("sent to agent: {text}")
                }
                Err(e) => format!("{e:#}"),
            }
        }
        other => format!("unknown tool {other}"),
    }
}
