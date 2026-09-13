#!/usr/bin/env python3
"""Dummy Okta for the local lab.

Teleport OSS cannot host OIDC/SAML. Same shape as `tsh login --auth=oidc`:
Sign in with Okta opens this page; the page hits Connect 2's localhost
callback with `?response=…`. Certs come from `tctl auth sign`.

    python3 scripts/lab.py sso
"""
from __future__ import annotations

import json
import os
import ssl
import subprocess
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DIR = Path(os.environ.get("TP_DIR", "/tmp/tp"))
PORT = int(os.environ.get("OIDC_PORT", "3059"))
ISSUER = os.environ.get("OIDC_ISSUER", f"http://127.0.0.1:{PORT}")
USER = os.environ.get("OIDC_USER", "okta-demo")
TCTL = os.environ.get("TCTL", str(DIR / "teleport/tctl"))
CFG = Path(os.environ.get("TELEPORT_CFG", str(DIR / "teleport.yaml")))
PROXY = os.environ.get("TELEPORT_PROXY", "127.0.0.1:3080")
IDENT = Path(os.environ.get("CONNECT2_DUMMY_IDENTITY", str(DIR / "client" / "okta-ident")))


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt: str, *args) -> None:
        print("oidc", fmt % args, flush=True)

    def _send(self, code: int, body: bytes, ctype: str) -> None:
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        q = urllib.parse.parse_qs(parsed.query)
        path = parsed.path.rstrip("/") or "/"
        if path in ("/", "/.well-known/openid-configuration"):
            body = json.dumps(
                {
                    "issuer": ISSUER,
                    "authorization_endpoint": f"{ISSUER}/authorize",
                    "dummy": "okta",
                }
            ).encode()
            self._send(200, body, "application/json")
            return
        if path == "/authorize":
            redirect = (q.get("redirect_uri") or [""])[0]
            ident = mint_identity()
            if not redirect:
                self._send(400, b"missing redirect_uri", "text/plain")
                return
            body = json.dumps({"username": USER, "identity": ident.read_text()})
            html = f"""<!doctype html><title>Dummy Okta</title>
<p>Signed in as <b>{USER}</b>. You can close this window.</p>
<script>
const url = {json.dumps(redirect)};
const body = {json.dumps(body)};
(async () => {{
  for (let i = 0; i < 40; i++) {{
    try {{
      const r = await fetch(url, {{
        method: "POST",
        mode: "cors",
        cache: "no-store",
        headers: {{"content-type": "application/json"}},
        body,
      }});
      if (r.ok) return;
    }} catch (e) {{}}
    await new Promise((ok) => setTimeout(ok, 150));
  }}
  document.body.insertAdjacentHTML("beforeend", "<p>Could not reach Connect 2 callback.</p>");
}})();
</script>
""".encode()
            self._send(200, html, "text/html; charset=utf-8")
            return
        self._send(404, b"not found", "text/plain")


def serve() -> ThreadingHTTPServer:
    httpd = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def ensure_user() -> None:
    subprocess.run(
        [
            TCTL,
            "--config",
            str(CFG),
            "users",
            "add",
            USER,
            "--roles=access,editor",
            "--logins=packer,root",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )


def mint_identity() -> Path:
    ensure_user()
    IDENT.parent.mkdir(parents=True, exist_ok=True)
    subprocess.check_call(
        [
            TCTL,
            "--config",
            str(CFG),
            "auth",
            "sign",
            f"--user={USER}",
            f"--out={IDENT}",
            "--overwrite",
            "--ttl=12h",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return IDENT


def wait_oidc(secs: float = 3) -> None:
    deadline = time.time() + secs
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{ISSUER}/.well-known/openid-configuration", timeout=1) as r:
                if r.status == 200:
                    return
        except Exception:
            time.sleep(0.1)
    raise SystemExit(f"dummy okta did not start on {ISSUER}")


def insecure_opener() -> urllib.request.OpenerDirector:
    ctx = ssl._create_unverified_context()
    https = urllib.request.HTTPSHandler(context=ctx)
    return urllib.request.build_opener(https, urllib.request.HTTPCookieProcessor())


def drive_sso_url(url: str) -> None:
    opener = insecure_opener()
    req = urllib.request.Request(url, method="GET")
    try:
        with opener.open(req, timeout=30) as r:
            r.read()
    except urllib.error.HTTPError as e:
        e.read()


def write_env() -> None:
    path = DIR / "dummy-sso.env"
    path.write_text(
        f"CONNECT2_DUMMY_SSO={ISSUER}\n"
        f"CONNECT2_DUMMY_IDENTITY={IDENT}\n"
        f"CONNECT2_DUMMY_USER={USER}\n"
    )


def apply_connector() -> None:
    """OSS Teleport cannot create OIDC. Dummy SSO is env + this HTTP server."""
    mint_identity()
    write_env()


def main() -> int:
    httpd = serve()
    wait_oidc()
    apply_connector()
    print(f"dummy okta  {ISSUER}")
    print(f"identity    {IDENT}")
    print("app env     CONNECT2_DUMMY_SSO  CONNECT2_DUMMY_IDENTITY")
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        httpd.shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
