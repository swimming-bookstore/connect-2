pub(super) const APP_CSS: &str = include_str!("../../../ui/public/style.css");
pub(super) const PORTS_JS: &str = include_str!("../../../ui/js/ports.js");
pub(super) const XTERM_CSS: &str = include_str!("../../../ui/public/xterm.css");
pub(super) const XTERM_JS: &str = include_str!("../../../ui/public/xterm.js");
pub(super) const XTERM_FIT: &str = include_str!("../../../ui/public/xterm-addon-fit.js");

#[cfg(feature = "web")]
pub(super) const UI_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/webui/connect2_ui.js"));
#[cfg(not(feature = "web"))]
pub(super) const UI_JS: &str = "";

#[cfg(feature = "web")]
pub(super) const UI_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/webui/connect2_ui_bg.wasm"));
#[cfg(not(feature = "web"))]
pub(super) const UI_WASM: &[u8] = b"";

#[cfg(feature = "web")]
mod ui_snippets {
    include!(concat!(env!("OUT_DIR"), "/webui/ui_snippets.rs"));
}

pub(super) fn ui_snippet(path: &str) -> Option<&'static [u8]> {
    #[cfg(feature = "web")]
    {
        let rel = path.strip_prefix("/pkg/snippets/")?;
        if rel.contains("..") {
            return None;
        }
        ui_snippets::UI_SNIPPETS
            .iter()
            .find(|(k, _)| *k == rel)
            .map(|(_, b)| *b)
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = path;
        None
    }
}

pub(super) fn app_css() -> Vec<u8> {
    let live = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/public/style.css");
    if let Ok(b) = std::fs::read(&live) {
        return b;
    }
    APP_CSS.as_bytes().to_vec()
}

fn asset_tag(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub(super) fn index_html() -> String {
    let app_css = app_css();
    let acss = asset_tag(&app_css);
    let xcss = asset_tag(XTERM_CSS.as_bytes());
    let xjs = asset_tag(XTERM_JS.as_bytes());
    let xfit = asset_tag(XTERM_FIT.as_bytes());
    let ui_js = asset_tag(UI_JS.as_bytes());
    let ui_wasm = asset_tag(UI_WASM);
    let ports = asset_tag(PORTS_JS.as_bytes());
    format!(
        r#"<!DOCTYPE html>
<html data-theme="light">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Connect 2</title>
<script>
try {{
  var t = localStorage.getItem("connect2-theme");
  document.documentElement.setAttribute("data-theme", t === "dark" ? "dark" : "light");
}} catch (e) {{}}
</script>
<link rel="stylesheet" href="/style.css?{acss}">
<link rel="stylesheet" href="/xterm.css?{xcss}">
<body>
<p id="boot">loading…</p>
<div id="main"></div>
<script src="/xterm.js?{xjs}"></script>
<script src="/xterm-addon-fit.js?{xfit}"></script>
<script src="/ports.js?{ports}"></script>
<script type="module">
  import init from "/pkg/connect2_ui.js?{ui_js}";
  try {{
    await init({{ module_or_path: "/pkg/connect2_ui_bg.wasm?{ui_wasm}" }});
  }} catch (e) {{
    document.getElementById("boot").textContent = String(e && e.message ? e.message : e);
  }}
</script>
"#
    )
}

