//! Laptop AI share. Completions run here. Tokens stay in memory (or env key).

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::watch;

use crate::protocol::{AiToolCall, In};
use crate::tools;

const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const REFRESH_SKEW_MS: u64 = 5 * 60 * 1000;
const MIN_TTL_MS: u64 = 30 * 1000;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct GrokTokens {
    pub access: String,
    pub refresh: String,
    pub expires: u64,
}

#[derive(Clone, Serialize)]
pub struct ChatInfo {
    pub configured: bool,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<GrokTokens>,
}

fn pending() -> &'static watch::Sender<Option<(String, String)>> {
    static P: OnceLock<watch::Sender<Option<(String, String)>>> = OnceLock::new();
    P.get_or_init(|| watch::channel(None).0)
}

fn tokens() -> &'static watch::Sender<Option<GrokTokens>> {
    static T: OnceLock<watch::Sender<Option<GrokTokens>>> = OnceLock::new();
    T.get_or_init(|| watch::channel(None).0)
}

fn set_pending(v: Option<(String, String)>) {
    let _ = pending().send_replace(v);
}

fn set_tokens(v: Option<GrokTokens>) {
    let _ = tokens().send_replace(v);
}

fn demo_agent() -> bool {
    std::env::var("CONNECT2_DEMO_AGENT").ok().as_deref() == Some("1")
}

fn live_tokens() -> Option<GrokTokens> {
    let t = tokens().borrow().clone()?;
    if t.access.starts_with("e2e-") || t.refresh.starts_with("e2e-") {
        return None;
    }
    Some(t)
}

pub fn info() -> ChatInfo {
    let p = pending().borrow().clone();
    let t = live_tokens();
    ChatInfo {
        configured: env_key().is_some() || t.is_some() || demo_agent(),
        model: model_name(),
        user_code: p.as_ref().map(|(c, _)| c.clone()),
        verification_uri: p.as_ref().map(|(_, u)| u.clone()),
        tokens: t,
    }
}

pub fn adopt_tokens(v: &Value) -> Result<()> {
    let t: GrokTokens = serde_json::from_value(v.clone()).context("grok tokens")?;
    if t.access.is_empty() || t.refresh.is_empty() {
        return Err(anyhow!("empty grok tokens"));
    }
    if t.access.starts_with("e2e-") || t.refresh.starts_with("e2e-") {
        return Err(anyhow!("demo/e2e grok tokens"));
    }
    set_tokens(Some(t));
    set_pending(None);
    Ok(())
}

pub async fn login_start() -> Result<ChatInfo> {
    let d = provider_grok::start_login().await?;
    let _ = open::that(&d.verification_uri_complete);
    set_pending(Some((
        d.user_code.clone(),
        d.verification_uri_complete.clone(),
    )));
    tokio::spawn(async move {
        let mut interval = d.interval;
        loop {
            if provider_grok::now_ms() >= d.deadline_ms {
                set_pending(None);
                break;
            }
            match poll_device(&d.device_code).await {
                Ok(Poll::Done(t)) => {
                    set_tokens(Some(t));
                    set_pending(None);
                    break;
                }
                Ok(Poll::Pending) => {}
                Ok(Poll::SlowDown(n)) => interval = n,
                Ok(Poll::Denied | Poll::Expired) | Err(_) => {
                    set_pending(None);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    });
    Ok(info())
}

pub async fn logout() -> Result<ChatInfo> {
    set_tokens(None);
    set_pending(None);
    Ok(info())
}

enum Poll {
    Pending,
    SlowDown(u64),
    Done(GrokTokens),
    Denied,
    Expired,
}

#[derive(Debug, Deserialize)]
struct TokenBody {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    interval: Option<u64>,
}

async fn poll_device(device_code: &str) -> Result<Poll> {
    let (st, tok) = post_token(&[
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("client_id", XAI_CLIENT_ID),
        ("device_code", device_code),
    ])
    .await?;
    if (200..300).contains(&st) {
        return Ok(Poll::Done(tokens_from_body(&tok, None)?));
    }
    Ok(match tok.error.as_deref() {
        Some("authorization_pending") => Poll::Pending,
        Some("slow_down") => Poll::SlowDown(tok.interval.filter(|n| *n > 0).unwrap_or(5)),
        Some("access_denied") | Some("authorization_denied") => Poll::Denied,
        Some("expired_token") => Poll::Expired,
        _ => return Err(anyhow!("xAI token poll failed (HTTP {st})")),
    })
}

async fn refresh_tokens(refresh: &str) -> Result<GrokTokens> {
    let (status, body) = post_token(&[
        ("grant_type", "refresh_token"),
        ("client_id", XAI_CLIENT_ID),
        ("refresh_token", refresh),
    ])
    .await?;
    if !(200..300).contains(&status) {
        return Err(anyhow!("xAI refresh failed (HTTP {status})"));
    }
    tokens_from_body(&body, Some(refresh))
}

async fn post_token(fields: &[(&str, &str)]) -> Result<(u16, TokenBody)> {
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .post(XAI_TOKEN_URL)
        .header("Accept", "application/json")
        .form(fields)
        .send()
        .await
        .context("oauth request")?;
    let status = resp.status().as_u16();
    let text = resp.text().await.context("oauth body")?;
    let body = serde_json::from_str(&text).context("oauth json")?;
    Ok((status, body))
}

fn tokens_from_body(body: &TokenBody, prev_refresh: Option<&str>) -> Result<GrokTokens> {
    let access = body
        .access_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("oauth missing access_token"))?;
    let refresh = body
        .refresh_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(prev_refresh)
        .ok_or_else(|| anyhow!("oauth missing refresh_token"))?;
    let lifetime_ms = body.expires_in.unwrap_or(3600).saturating_mul(1000);
    let skew = REFRESH_SKEW_MS.min(lifetime_ms.saturating_sub(MIN_TTL_MS));
    Ok(GrokTokens {
        access: access.into(),
        refresh: refresh.into(),
        expires: provider_grok::now_ms()
            .saturating_add(lifetime_ms)
            .saturating_sub(skew),
    })
}

fn env_key() -> Option<String> {
    std::env::var("CONNECT2_AI_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .ok()
        .filter(|s| !s.is_empty())
}

fn model_name() -> String {
    std::env::var("CONNECT2_AI_MODEL").unwrap_or_else(|_| {
        if env_key().is_some() {
            "gpt-4o-mini".into()
        } else {
            "grok-4.6".into()
        }
    })
}

fn base_url(grok: bool) -> String {
    let mut base = std::env::var("CONNECT2_AI_BASE")
        .or_else(|_| std::env::var("OPENAI_BASE_URL"))
        .unwrap_or_else(|_| {
            if grok {
                "https://api.x.ai/v1".into()
            } else {
                "https://api.openai.com/v1".into()
            }
        });
    while base.ends_with('/') {
        base.pop();
    }
    base
}

pub struct Completion {
    pub content: String,
    pub tool_calls: Vec<AiToolCall>,
}

/// Completions on the laptop. Tokens never leave this process.
pub async fn complete_local(messages: &Value, tools: &Value) -> Result<Completion> {
    if let Some(c) = demo_completion(messages, tools) {
        return Ok(c);
    }
    let (token, base, model) = if let Some(key) = env_key() {
        (key, base_url(false), model_name())
    } else {
        let token = bearer()
            .await
            .context("not logged in — Login Grok in the client")?;
        (token, base_url(true), model_name())
    };
    complete(&token, &base, &model, messages, tools).await
}

fn demo_call(name: &str, args: Value) -> AiToolCall {
    AiToolCall {
        id: format!("demo-{name}"),
        name: name.into(),
        args,
    }
}

fn tool_named(tools: &Value, name: &str) -> bool {
    tools
        .as_array()
        .into_iter()
        .flatten()
        .any(|v| v.pointer("/function/name").and_then(|n| n.as_str()) == Some(name))
}

fn last_user(messages: &Value) -> Option<String> {
    messages.as_array()?.iter().rev().find_map(|m| {
        if m.get("role")?.as_str()? != "user" {
            return None;
        }
        Some(m.get("content")?.as_str().unwrap_or("").to_string())
    })
}

fn demo_completion(messages: &Value, tools: &Value) -> Option<Completion> {
    if !demo_agent() {
        return None;
    }
    let arr = messages.as_array()?;
    let last = arr.last()?;
    let role = last.get("role")?.as_str()?;
    if tool_named(tools, "bash") || tool_named(tools, "read") {
        let user = last_user(messages).unwrap_or_default();
        let low_user = user.to_ascii_lowercase();
        if low_user.contains("hello") || low_user.contains("swimming") {
            return Some(Completion {
                content: "hello 수영 책방 Swimming Bookstore".into(),
                tool_calls: Vec::new(),
            });
        }
        let content = if role == "tool" {
            last.get("content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("chrome.rs  code.rs  lib.rs  main.rs")
                .into()
        } else {
            "src/ has chrome.rs, code.rs, lib.rs, main.rs.".into()
        };
        return Some(Completion {
            content,
            tool_calls: Vec::new(),
        });
    }
    let user = last_user(messages)?;
    let low = user.to_ascii_lowercase();
    if role == "tool" {
        let content = if low.contains("github.io") || low.contains("browser") || low.contains("go to")
        {
            "Opened box-2 in the browser.".into()
        } else if low.contains("agent") || low.contains("src/") {
            "Asked the box-3 agent.".into()
        } else {
            "Opened the shell.".into()
        };
        return Some(Completion {
            content,
            tool_calls: Vec::new(),
        });
    }
    if role != "user" {
        return Some(Completion {
            content: String::new(),
            tool_calls: Vec::new(),
        });
    }
    if low.contains("github.io") || low.contains("browser") || low.contains("go to") {
        return Some(Completion {
            content: String::new(),
            tool_calls: vec![demo_call(
                "browser_navigate",
                json!({"machine":"box-2","url":"https://swimming-bookstore.github.io/homepage/"}),
            )],
        });
    }
    if low.contains("agent") || low.contains("src/") || low.contains("rust") || low.contains("box-3") {
        let text = if low.contains("hello") || low.contains("swimming") {
            "say hello to 수영 책방 Swimming Bookstore"
        } else {
            "list the Rust modules in src/"
        };
        return Some(Completion {
            content: String::new(),
            tool_calls: vec![demo_call("ask_agent", json!({"machine":"box-3","text": text}))],
        });
    }
    Some(Completion {
        content: String::new(),
        tool_calls: vec![demo_call(
            "shell",
            json!({
                "machine":"box-1",
                "command":"echo 'hello 수영 책방 Swimming Bookstore'"
            }),
        )],
    })
}

async fn bearer() -> Result<String> {
    let mut t = live_tokens().ok_or_else(|| anyhow!("not logged in — Login Grok in the client"))?;
    if provider_grok::now_ms() >= t.expires {
        t = refresh_tokens(&t.refresh)
            .await
            .context("refresh xAI token")?;
        set_tokens(Some(t.clone()));
    }
    Ok(t.access)
}

async fn complete(
    token: &str,
    base: &str,
    model: &str,
    messages: &Value,
    tools: &Value,
) -> Result<Completion> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()?;
    let v: Value = http
        .post(format!("{base}/chat/completions"))
        .bearer_auth(token)
        .json(&json!({
            "model": model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto"
        }))
        .send()
        .await
        .context("ai http")?
        .error_for_status()
        .context("ai status")?
        .json()
        .await?;
    let msg = v
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| anyhow!("ai: no message ({v})"))?;
    let content = msg["content"].as_str().unwrap_or("").to_string();
    let tool_calls = msg
        .get("tool_calls")
        .and_then(|c| c.as_array())
        .map(|calls| calls.iter().filter_map(parse_call).collect())
        .unwrap_or_default();
    Ok(Completion {
        content,
        tool_calls,
    })
}

fn parse_call(call: &Value) -> Option<AiToolCall> {
    let id = call.get("id")?.as_str()?.to_string();
    let name = call.pointer("/function/name")?.as_str()?.to_string();
    let raw = call.pointer("/function/arguments")?.as_str().unwrap_or("{}");
    let args = serde_json::from_str(raw).unwrap_or_else(|_| json!({}));
    Some(AiToolCall { id, name, args })
}

/// Box coding agent asked the laptop to complete. Tokens stay here.
pub async fn share_result(id: String, thread: Vec<Value>) -> In {
    let tools = tools::coding_tools();
    match complete_local(&Value::Array(thread), &tools).await {
        Ok(c) => {
            let remote: Vec<_> = c
                .tool_calls
                .into_iter()
                .filter(|c| !tools::is_desk_tool(&c.name))
                .collect();
            In::AiResult {
                id,
                content: c.content,
                tool_calls: remote,
                error: None,
            }
        }
        Err(e) => In::AiResult {
            id,
            content: String::new(),
            tool_calls: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_call_ok() {
        let v = json!({
            "id": "c1",
            "function": {"name": "read", "arguments": "{\"path\":\"a.rs\"}"}
        });
        let c = parse_call(&v).expect("call");
        assert_eq!(c.id, "c1");
        assert_eq!(c.name, "read");
        assert_eq!(c.args["path"], "a.rs");
    }
}
