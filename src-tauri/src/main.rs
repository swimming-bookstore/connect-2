#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use connect2::hub::{spawn_hub, Cli};
use tauri::{WebviewUrl, WebviewWindowBuilder};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "connect2=info".into()),
        )
        .compact()
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut cli = Cli::try_parse().unwrap_or_else(|_| Cli::parse_from(["connect2-tauri"]));
    cli.no_open = true;
    let hub_url = std::env::var("CONNECT2_UI_HTTP").unwrap_or_else(|_| format!("http://{}", cli.http));
    if let Err(e) = spawn_hub(cli) {
        eprintln!("connect2-tauri hub: {e:#}");
        std::process::exit(1);
    }

    tauri::Builder::default()
        .setup(move |app| {
            let boot = format!("window.__CONNECT2_HUB={};", serde_json::to_string(&hub_url)?);
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Connect 2")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 620.0)
                .maximized(true)
                .resizable(true)
                .visible(true)
                .focused(true)
                .initialization_script(&boot)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("connect2-tauri: {e}");
            std::process::exit(1);
        });
}
