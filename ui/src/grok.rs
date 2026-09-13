//! Grok OAuth tokens live in the browser (`localStorage`).

use serde::{Deserialize, Serialize};
use serde_json::json;

const KEY: &str = "connect2-grok";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Tokens {
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub refresh: String,
    #[serde(default)]
    pub expires: u64,
}

pub fn load() -> Option<Tokens> {
    let w = web_sys::window()?;
    let s = w.local_storage().ok()??;
    let raw = s.get_item(KEY).ok()??;
    let t: Tokens = serde_json::from_str(&raw).ok()?;
    if t.access.is_empty() || t.refresh.is_empty() {
        return None;
    }
    Some(t)
}

pub fn save(t: &Tokens) {
    if t.access.is_empty() || t.refresh.is_empty() {
        return;
    }
    if let Some(w) = web_sys::window() {
        if let Ok(Some(s)) = w.local_storage() {
            if let Ok(raw) = serde_json::to_string(t) {
                let _ = s.set_item(KEY, &raw);
            }
        }
    }
}

pub fn clear() {
    if let Some(w) = web_sys::window() {
        if let Ok(Some(s)) = w.local_storage() {
            let _ = s.remove_item(KEY);
        }
    }
}

pub fn restore_cmd() -> Option<serde_json::Value> {
    load().map(|t| json!({"type": "grok_tokens", "tokens": t}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_roundtrip_json() {
        let t = Tokens {
            access: "a".into(),
            refresh: "r".into(),
            expires: 1,
        };
        let v = serde_json::to_value(&t).unwrap();
        let back: Tokens = serde_json::from_value(v).unwrap();
        assert_eq!(back.access, "a");
        assert_eq!(back.refresh, "r");
        assert_eq!(back.expires, 1);
    }

    #[test]
    fn tokens_empty_access_is_not_usable() {
        let t: Tokens =
            serde_json::from_str(r#"{"access":"","refresh":"r","expires":1}"#).unwrap();
        assert!(t.access.is_empty());
        let t: Tokens = serde_json::from_value(json!({"access": "x"})).unwrap();
        assert_eq!(t.access, "x");
        assert!(t.refresh.is_empty());
    }
}
