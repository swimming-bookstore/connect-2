//! Native Connect 2 window: exec `connect2-tauri`, which starts the hub itself.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, bail, Result};

fn tauri_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CONNECT2_TAURI") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("connect2-tauri");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "target/release/connect2-tauri",
        "src-tauri/target/release/connect2-tauri",
    ] {
        let p = here.join(rel);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn run() -> Result<()> {
    let bin = tauri_bin().ok_or_else(|| {
        anyhow!("connect2-tauri not found (build src-tauri; or set CONNECT2_TAURI=)")
    })?;
    tracing::info!("tauri {}", bin.display());
    let status = Command::new(&bin)
        .args(std::env::args().skip(1))
        .status()?;
    if !status.success() {
        bail!("connect2-tauri exited {status}");
    }
    Ok(())
}
