#!/usr/bin/env python3
"""Live TOTP for the lab `demo` user. Reads /tmp/tp/totp-secret.txt. Loopback only."""
from __future__ import annotations

import base64
import hashlib
import hmac
import os
import struct
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

USER = os.environ.get("DEMO_USER", "demo")
PASSWORD = os.environ.get("DEMO_PASSWORD", "demo-pass-2026")
ISSUER = os.environ.get("DEMO_ISSUER", "teleport.local")
HOST = os.environ.get("DEMO_TOTP_HOST", "0.0.0.0")
PORT = int(os.environ.get("DEMO_TOTP_PORT", "3057"))
SECRET_PATH = Path(os.environ.get("DEMO_TOTP_SECRET", "/tmp/tp/totp-secret.txt"))


def secret() -> str:
    return SECRET_PATH.read_text().strip().replace(" ", "")


def totp(sec: str, now: float | None = None) -> tuple[str, int]:
    pad = "=" * ((8 - len(sec) % 8) % 8)
    key = base64.b32decode(sec.upper() + pad, casefold=True)
    t = int((now if now is not None else time.time()) // 30)
    left = 30 - int(time.time() % 30)
    digest = hmac.new(key, struct.pack(">Q", t), hashlib.sha1).digest()
    off = digest[-1] & 0x0F
    code = (struct.unpack(">I", digest[off : off + 4])[0] & 0x7FFFFFFF) % 1_000_000
    return f"{code:06d}", left


class H(BaseHTTPRequestHandler):
    def log_message(self, *_a):
        pass

    def do_GET(self):
        try:
            code, left = totp(secret())
        except Exception as e:
            body = f"no TOTP secret ({e}). run python3 scripts/lab.py password".encode()
            self.send_response(503)
            self.send_header("content-type", "text/plain; charset=utf-8")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path.startswith("/code"):
            body = f'{{"code":"{code}","left":{left},"user":"{USER}","issuer":"{ISSUER}"}}'.encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("cache-control", "no-store")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        html = f"""<!doctype html>
<html><meta charset="utf-8"><title>demo TOTP</title>
<style>
  html,body{{margin:0;height:100%;font-family:ui-sans-serif,system-ui,sans-serif;background:#111;color:#f5f5f5}}
  .box{{min-height:100%;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px}}
  .k{{letter-spacing:.2em;text-transform:uppercase;opacity:.6;font-size:13px}}
  #c{{font:800 72px/1 ui-monospace,Menlo,Consolas,monospace;letter-spacing:.12em}}
  #l{{opacity:.7}}
</style>
<div class="box">
  <div class="k">{ISSUER} · {USER}</div>
  <div id="c">{code}</div>
  <div id="l">{left}s left</div>
  <div class="k">password {PASSWORD} · https://127.0.0.1:3080</div>
</div>
<script>
async function tick(){{
  const r = await fetch("/code", {{cache:"no-store"}});
  const j = await r.json();
  document.getElementById("c").textContent = j.code;
  document.getElementById("l").textContent = j.left + "s left";
}}
setInterval(tick, 1000);
</script>
"""
        body = html.encode()
        self.send_response(200)
        self.send_header("content-type", "text/html; charset=utf-8")
        self.send_header("cache-control", "no-store")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    HTTPServer((HOST, PORT), H).serve_forever()
