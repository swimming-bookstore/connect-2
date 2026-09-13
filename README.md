# Connect 2

Reach a private box from your laptop. Neither side opens inbound ports.

Teleport is the middle. A box agent keeps a reverse tunnel. The laptop app is a directory of boxes — Shell, Browser, Agent — in the browser or a desktop window. No `tsh` required.

```
laptop  →  Teleport proxy  →  box (connect2-agent)
```

## What you run

| Where | Binary | What it does |
|---|---|---|
| Each box | `connect2-agent` | Joins the cluster, tunnels, runs Chromium / PTY / coding agent |
| Your laptop | `connect2` | Lists boxes, opens sessions. `--no-open` is the web hub only |
| Your laptop | `connect2-tauri` | Native window around the same UI (starts the hub itself) |

The UI talks only to a loopback hub (`http://127.0.0.1:3056`). The hub talks to Teleport.

## Laptop

```bash
# first time — same as tsh login
connect2 --proxy teleport.example:443 --identity ./user login --auth local --user demo
# Okta / OIDC / SAML
connect2 --proxy teleport.example:443 --identity ./user login --auth oidc --connector okta

connect2 --proxy teleport.example:443 --identity ./user ls
connect2 --proxy teleport.example:443 --identity ./user --node box-1 open shell
connect2 --proxy teleport.example:443 --identity ./user
```

No flags after login: desktop window if `connect2-tauri` is next to `connect2`, otherwise print usage. `--no-open` serves the UI at **http://127.0.0.1:3056**.

**Sign in in the app**

- Local cluster: user, password, 6-digit OTP (same as `tsh login`).
- Okta (or any OIDC/SAML/GitHub): **Sign in with …** opens the system browser, then a localhost callback. Same path as `tsh login --auth=okta`.

`--insecure` skips proxy TLS (lab). `--tls-ca` pins a PEM CA.

## Box

```bash
tctl tokens add --type=node --ttl=15m
connect2-agent --proxy teleport.example:443 --token TOKEN --identity ./identity
# later — certs already on disk
connect2-agent --proxy teleport.example:443 --identity ./identity
```

Optional TURN for WebRTC video: `--turn turn:host:3478 --turn-user … --turn-pass …`. Browser needs `chromium` (`CHROME=`). WebRTC needs `ffmpeg` (`FFMPEG=`).

## Build

Rust throughout. Toolchain: `rust-toolchain.toml` (1.98 + `wasm32-unknown-unknown`).

```bash
cargo build --release --bins --features web,desktop   # connect2, connect2-agent, connect2-turn
cargo build --release --manifest-path src-tauri/Cargo.toml   # window (needs GTK / WebKit)
```

Nix:

```bash
nix build .#connect2            # hub (static musl on Linux)
nix build .#connect2-tauri      # window (dynamic GTK/WebKit)
nix build .#connect2-desktop    # both in bin/
nix develop
```

The window cannot be a static binary (WebKit). CLI/agent/TURN can.

## This repo’s lab

```bash
python3 scripts/lab.py            # Teleport + demo user + TURN + TOTP page
python3 scripts/lab.py fold       # then the web hub
python3 scripts/lab.py reopen     # rebuild hub + window
python3 scripts/lab.py e2e        # boxes + Playwright
python3 scripts/lab.py sso        # dummy Okta (OSS Teleport has no real OIDC)
```

| | |
|---|---|
| Console | https://127.0.0.1:3080 |
| Connect 2 | http://127.0.0.1:3056 |
| OTP codes | http://127.0.0.1:3057 |
| Password | `demo` / `demo-pass-2026` — **always** the 6-digit code too |

This cluster is password + OTP. Password alone fails. Dummy Okta is a lab shim so the browser-callback path can be clicked; it is not a real IdP.

## Layout

| Path | |
|---|---|
| `crates/connect2-teleport` | Join, reverse tunnel, inventory, `connect-app` (reusable, no UI) |
| `src/` | Box agent: Chromium, shell, coding agent |
| `src/hub/` | Laptop hub: login, `/cmd`, `/live`, JPEG |
| `ui/` | Leptos + DaisyUI (WASM) — same bundle in browser and Tauri |
| `src-tauri/` | Native window only |

Examples for the crate: `cargo run -p connect2-teleport --example echo` (box) and `--example laptop`.
