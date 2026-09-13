use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if env::var("CARGO_FEATURE_WEB").is_ok() {
        build_ui()?;
    }
    Ok(())
}

fn build_ui() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let ui_manifest = manifest.join("ui/Cargo.toml");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/js");
    println!("cargo:rerun-if-changed=ui/css");
    println!("cargo:rerun-if-changed=ui/public/style.css");
    println!("cargo:rerun-if-changed=ui/Cargo.toml");
    println!("cargo:rerun-if-changed=package.json");

    if env::var_os("CONNECT2_SKIP_CSS").is_none() {
        let css = Command::new("npm")
            .arg("run")
            .arg("css")
            .current_dir(&manifest)
            .status()?;
        if !css.success() {
            return Err("ui css build failed (npm run css)".into());
        }
    }

    let ui_target = env::var_os("CONNECT2_UI_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("ui/target"));

    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.arg("build")
        .arg("--manifest-path")
        .arg(&ui_manifest)
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--release")
        .env("CARGO_TARGET_DIR", &ui_target);
    for (k, _) in env::vars() {
        if k.starts_with("CARGO") && k != "CARGO" && k != "CARGO_HOME" && k != "CARGO_TERM_COLOR" {
            cmd.env_remove(&k);
        }
    }
    if env::var_os("CONNECT2_KEEP_RUSTC").is_none() {
        cmd.env_remove("RUSTC");
        cmd.env_remove("RUSTC_WRAPPER");
    }
    cmd.env_remove("RUSTFLAGS");
    if env::var_os("CONNECT2_KEEP_RUSTC").is_some() {
        cmd.arg("--offline");
    }

    let status = cmd.status()?;
    if !status.success() {
        return Err("ui wasm build failed".into());
    }

    let wasm = ui_target.join("wasm32-unknown-unknown/release/connect2_ui.wasm");
    let out = PathBuf::from(env::var("OUT_DIR")?).join("webui");
    std::fs::create_dir_all(&out)?;
    wasm_bindgen_cli_support::Bindgen::new()
        .input_path(&wasm)
        .web(true)?
        .debug(false)
        .generate(&out)?;
    emit_snippets(&out)?;
    copy_tauri_assets(&manifest, &out)?;
    Ok(())
}

fn copy_file(src: &std::path::Path, dst: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    Ok(())
}

fn copy_tauri_assets(
    manifest: &std::path::Path,
    webui: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let public = manifest.join("ui/public");
    let pkg = public.join("pkg");
    std::fs::create_dir_all(&pkg)?;
    copy_file(
        &webui.join("connect2_ui.js"),
        &pkg.join("connect2_ui.js"),
    )?;
    copy_file(
        &webui.join("connect2_ui_bg.wasm"),
        &pkg.join("connect2_ui_bg.wasm"),
    )?;
    let snippets = webui.join("snippets");
    if snippets.is_dir() {
        copy_dir(&snippets, &pkg.join("snippets"))?;
    }
    copy_file(&manifest.join("ui/js/ports.js"), &public.join("ports.js"))?;
    std::fs::write(public.join("index.html"), TAURI_INDEX)?;
    Ok(())
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        let to = dst.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &to)?;
        } else {
            std::fs::copy(&p, &to)?;
        }
    }
    Ok(())
}

const TAURI_INDEX: &str = r#"<!DOCTYPE html>
<html data-theme="light">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Connect 2</title>
<script>
try {
  var t = localStorage.getItem("connect2-theme");
  document.documentElement.setAttribute("data-theme", t === "dark" ? "dark" : "light");
} catch (e) {}
</script>
<link rel="stylesheet" href="style.css">
<link rel="stylesheet" href="xterm.css">
<body>
<p id="boot">loading…</p>
<div id="main"></div>
<script src="xterm.js"></script>
<script src="xterm-addon-fit.js"></script>
<script src="ports.js"></script>
<script type="module">
  import init from "./pkg/connect2_ui.js";
  try {
    await init({ module_or_path: "./pkg/connect2_ui_bg.wasm" });
  } catch (e) {
    document.getElementById("boot").textContent = String(e && e.message ? e.message : e);
  }
</script>
"#;

fn emit_snippets(out: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let root = out.join("snippets");
    let mut files = Vec::new();
    if root.is_dir() {
        collect_files(&root, &root, &mut files);
    }
    files.sort();
    let mut code = String::from("pub static UI_SNIPPETS: &[(&str, &[u8])] = &[\n");
    for rel in &files {
        let key = rel.to_string_lossy().replace('\\', "/");
        let include = format!("snippets/{key}");
        code.push_str(&format!(
            "    ({key:?}, include_bytes!(concat!(env!(\"OUT_DIR\"), \"/webui/{include}\"))),\n"
        ));
    }
    code.push_str("];\n");
    std::fs::write(out.join("ui_snippets.rs"), code)?;
    Ok(())
}

fn collect_files(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_path_buf());
        }
    }
}
