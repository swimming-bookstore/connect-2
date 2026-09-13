#!/usr/bin/env python3
"""Rebuild Connect 2 (WASM + hub + Tauri) and open the native window.

    python3 scripts/lab.py reopen

This VM has no system gdk-3.0.pc; Tauri links against .sysroot.
"""
from __future__ import annotations

import os
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SYSROOT = ROOT / ".sysroot"
HUB = os.environ.get("CONNECT2_UI_HTTP", "http://127.0.0.1:3056")
PROXY = os.environ.get("TELEPORT_PROXY", "127.0.0.1:3080")
HTTP = os.environ.get("HTTP", "127.0.0.1:3056")


def run(cmd: list[str], **kw) -> None:
    kw.setdefault("cwd", ROOT)
    print("+", " ".join(cmd), flush=True)
    subprocess.check_call(cmd, **kw)


def kill_old() -> None:
    out = subprocess.check_output(["ps", "-eo", "pid,cmd"], text=True)
    mine = []
    for line in out.splitlines():
        if "grep" in line:
            continue
        if "connect2 --proxy" in line or "connect2-tauri" in line:
            try:
                mine.append(int(line.split(None, 1)[0]))
            except ValueError:
                pass
    for pid in mine:
        if pid == os.getpid():
            continue
        print(f"+ kill {pid}", flush=True)
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    if mine:
        time.sleep(0.4)


def load_dummy(env: dict[str, str]) -> bool:
    dummy = Path("/tmp/tp/dummy-sso.env")
    if not dummy.is_file():
        return False
    for line in dummy.read_text().splitlines():
        if "=" in line and not line.startswith("#"):
            k, v = line.split("=", 1)
            env.setdefault(k.strip(), v.strip())
    return bool(env.get("CONNECT2_DUMMY_SSO"))


def identity_path(env: dict[str, str]) -> str:
    # Dummy Okta only shows on Sign in. A live demo identity hides the button.
    if env.get("CONNECT2_DUMMY_SSO"):
        path = Path("/tmp/tp/client/session")
        if path.is_file():
            path.unlink()
        path.parent.mkdir(parents=True, exist_ok=True)
        return str(path)
    return os.environ.get("TELEPORT_IDENT", "/tmp/tp/client/demo")


def tauri_env() -> dict[str, str]:
    env = os.environ.copy()
    load_dummy(env)
    pc = SYSROOT / "usr/lib/x86_64-linux-gnu/pkgconfig"
    lib = SYSROOT / "usr/lib/x86_64-linux-gnu"
    if pc.is_dir():
        env["PKG_CONFIG_SYSROOT_DIR"] = str(SYSROOT)
        env["PKG_CONFIG_PATH"] = str(pc)
        syslib = "/usr/lib/x86_64-linux-gnu"
        env["LIBRARY_PATH"] = f"{syslib}:{lib}:{env.get('LIBRARY_PATH', '')}"
        env["LD_LIBRARY_PATH"] = f"{syslib}:{lib}:{env.get('LD_LIBRARY_PATH', '')}"
        env["RUSTFLAGS"] = f"-L {syslib} -L {lib} {env.get('RUSTFLAGS', '')}".strip()
    return env


def wait_hub(url: str, secs: float = 20) -> None:
    deadline = time.time() + secs
    last = ""
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as r:
                if r.status == 200:
                    print(f"hub {url} {r.status}", flush=True)
                    return
                last = str(r.status)
        except Exception as e:
            last = str(e)
        time.sleep(0.25)
    raise SystemExit(f"hub not up: {url} ({last})")


def main() -> int:
    kill_old()
    run(
        [
            "cargo",
            "build",
            "--release",
            "--bin",
            "connect2",
            "--features",
            "web,desktop",
        ]
    )
    run(["cargo", "build", "--release"], cwd=ROOT / "src-tauri", env=tauri_env())
    src = ROOT / "src-tauri/target/release/connect2-tauri"
    dst = ROOT / "target/release/connect2-tauri"
    if src.is_file():
        dst.parent.mkdir(parents=True, exist_ok=True)
        if dst.resolve() != src.resolve():
            dst.write_bytes(src.read_bytes())
            os.chmod(dst, 0o755)
    env = tauri_env()
    ident = identity_path(env)
    log = Path("/tmp/connect2-hub.log")
    hub = subprocess.Popen(
        [
            str(ROOT / "target/release/connect2"),
            "--proxy",
            PROXY,
            "--identity",
            ident,
            "--http",
            HTTP,
            "--insecure",
        ],
        cwd=ROOT,
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        env=env,
    )
    wait_hub(HUB)
    print(f"tauri pid={hub.pid}  {HUB}  identity={ident}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as e:
        raise SystemExit(e.returncode) from e
