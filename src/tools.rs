use serde_json::{json, Value};

/// Box coding agent (Agent tab): files + bash on the box.
pub fn coding_tools() -> Value {
    json!([
        fn_tool(
            "read",
            "Read a file. A directory lists names.",
            json!({
                "path": {"type":"string"},
                "offset": {"type":"integer"},
                "limit": {"type":"integer"}
            }),
            &["path"]
        ),
        fn_tool(
            "write",
            "Write a file (overwrite).",
            json!({
                "path": {"type":"string"},
                "content": {"type":"string"}
            }),
            &["path", "content"]
        ),
        fn_tool(
            "edit",
            "Replace old with new. old must appear exactly once.",
            json!({
                "path": {"type":"string"},
                "old": {"type":"string"},
                "new": {"type":"string"}
            }),
            &["path", "old", "new"]
        ),
        fn_tool(
            "bash",
            "Run a shell command in the workspace (no UI).",
            json!({ "cmd": {"type":"string"} }),
            &["cmd"]
        ),
    ])
}

/// Laptop sidebar: open Shell/Browser, type, go to URLs. Not the box coding agent.
pub fn desk_tools() -> Value {
    json!([
        fn_tool(
            "list_machines",
            "List boxes this laptop can see.",
            json!({}),
            &[]
        ),
        fn_tool(
            "open",
            "Show this box's Shell in the laptop UI.",
            json!({ "machine": {"type":"string"} }),
            &[]
        ),
        fn_tool(
            "open_shell",
            "Show this box's Shell in the laptop UI.",
            json!({ "machine": {"type":"string"} }),
            &[]
        ),
        fn_tool(
            "shell",
            "Show Shell and type a command, as if you typed it.",
            json!({
                "machine": {"type":"string"},
                "command": {"type":"string"}
            }),
            &["command"]
        ),
        fn_tool(
            "open_browser",
            "Show this box's Browser. Optional url.",
            json!({
                "machine": {"type":"string"},
                "url": {"type":"string"}
            }),
            &[]
        ),
        fn_tool(
            "browser_navigate",
            "Show Browser and go to a URL.",
            json!({
                "machine": {"type":"string"},
                "url": {"type":"string"}
            }),
            &["url"]
        ),
        fn_tool(
            "browser_eval",
            "Show Browser and run JavaScript in the active tab.",
            json!({
                "machine": {"type":"string"},
                "expression": {"type":"string"}
            }),
            &["expression"]
        ),
        fn_tool(
            "browser_tabs",
            "Show Browser and list tabs.",
            json!({ "machine": {"type":"string"} }),
            &[]
        ),
        fn_tool(
            "open_agent",
            "Show this box's Agent tab (coding agent on the box).",
            json!({ "machine": {"type":"string"} }),
            &[]
        ),
        fn_tool(
            "ask_agent",
            "Open the Agent tab and type this message to the box coding agent. Use this whenever the user wants to talk to the agent.",
            json!({
                "machine": {"type":"string", "description":"Box name. Default: current box."},
                "text": {"type":"string", "description":"Message to type into the Agent chat."}
            }),
            &["text"]
        ),
    ])
}

pub fn is_desk_tool(name: &str) -> bool {
    matches!(
        name,
        "open"
            | "open_shell"
            | "open_browser"
            | "browser_navigate"
            | "shell"
            | "browser_eval"
            | "browser_tabs"
            | "list_machines"
            | "open_agent"
            | "ask_agent"
    )
}

fn fn_tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_tools() {
        let t = coding_tools();
        let names: Vec<_> = t
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.pointer("/function/name")?.as_str())
            .collect();
        assert_eq!(names, ["read", "write", "edit", "bash"]);
        assert!(is_desk_tool("ask_agent"));
        assert!(!is_desk_tool("bash"));
    }
}
