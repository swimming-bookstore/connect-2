#!/usr/bin/env python3
"""Keep lab user `demo` able to log in (password + TOTP).

`tctl users reset` does not clear Teleport's login lockout, and running it on
every `lab.py` mints a new TOTP so the console code is always stale.

This script:

  1. Tries the password + TOTP already on disk.
  2. If that works, does nothing.
  3. If the account is missing, locked, or the secrets are wrong, deletes and
     recreates `demo`, completes the invite, and writes:

       /tmp/tp/demo-password.txt
       /tmp/tp/totp-secret.txt
       /tmp/tp/qr.png
"""
from __future__ import annotations

import base64
import ctypes
import ctypes.util
import hashlib
import hmac
import json
import os
import re
import struct
import subprocess
import sys
import time
from pathlib import Path
from urllib.parse import parse_qs, urlparse

TCTL = os.environ.get("TCTL", "/tmp/tp/teleport/tctl")
CFG = os.environ.get("TELEPORT_CFG", "/tmp/tp/teleport.yaml")
PROXY = os.environ.get("TELEPORT_WEB", "https://127.0.0.1:3080")
USER = os.environ.get("DEMO_USER", "demo")
PASSWORD = os.environ.get("DEMO_PASSWORD", "demo-pass-2026")
ROLES = os.environ.get("DEMO_ROLES", "access,editor")
LOGINS = os.environ.get("DEMO_LOGINS", "packer,root")
DIR = Path(os.environ.get("TP_DIR", "/tmp/tp"))
COOKIE = DIR / "reset.cookies"
PASSWORD_FILE = DIR / "demo-password.txt"
SECRET_FILE = DIR / "totp-secret.txt"


def totp(secret: str, now: float | None = None) -> str:
    pad = "=" * ((8 - len(secret) % 8) % 8)
    key = base64.b32decode(secret.upper() + pad, casefold=True)
    t = int((now if now is not None else time.time()) // 30)
    digest = hmac.new(key, struct.pack(">Q", t), hashlib.sha1).digest()
    off = digest[-1] & 0x0F
    code = (struct.unpack(">I", digest[off : off + 4])[0] & 0x7FFFFFFF) % 1_000_000
    return f"{code:06d}"


def totp_fresh(secret: str) -> str:
    left = 30 - int(time.time() % 30)
    if left < 3:
        time.sleep(left + 0.05)
    return totp(secret)


def decode_qr_png(png: bytes) -> str:
    tmp = DIR / "qr-boot.png"
    gray = DIR / "qr-boot.gray"
    tmp.write_bytes(png)
    r = subprocess.run(
        ["ffmpeg", "-y", "-i", str(tmp), "-f", "rawvideo", "-pix_fmt", "gray", str(gray)],
        capture_output=True,
    )
    if r.returncode:
        raise RuntimeError(r.stderr[-400:].decode("utf-8", "replace"))
    probe = subprocess.check_output(
        [
            "ffprobe",
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
            str(tmp),
        ],
        text=True,
    ).strip()
    w, h = map(int, probe.split(","))
    raw = gray.read_bytes()
    z = ctypes.CDLL(ctypes.util.find_library("zbar") or "libzbar.so.0")
    z.zbar_image_scanner_create.restype = ctypes.c_void_p
    z.zbar_image_scanner_destroy.argtypes = [ctypes.c_void_p]
    z.zbar_image_scanner_set_config.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
    ]
    z.zbar_image_create.restype = ctypes.c_void_p
    z.zbar_image_destroy.argtypes = [ctypes.c_void_p]
    z.zbar_image_set_format.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    z.zbar_image_set_size.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint]
    z.zbar_image_set_data.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.c_void_p,
    ]
    z.zbar_scan_image.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    z.zbar_scan_image.restype = ctypes.c_int
    z.zbar_image_first_symbol.argtypes = [ctypes.c_void_p]
    z.zbar_image_first_symbol.restype = ctypes.c_void_p
    z.zbar_symbol_get_data.argtypes = [ctypes.c_void_p]
    z.zbar_symbol_get_data.restype = ctypes.c_char_p

    def fourcc(s: str) -> int:
        b = s.encode()
        return b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24)

    scanner = z.zbar_image_scanner_create()
    z.zbar_image_scanner_set_config(scanner, 0, 0, 1)
    image = z.zbar_image_create()
    z.zbar_image_set_format(image, fourcc("Y800"))
    z.zbar_image_set_size(image, w, h)
    buf = ctypes.create_string_buffer(raw, len(raw))
    z.zbar_image_set_data(image, ctypes.addressof(buf), len(raw), None)
    if z.zbar_scan_image(scanner, image) < 1:
        raise RuntimeError("QR decode failed")
    data = z.zbar_symbol_get_data(z.zbar_image_first_symbol(image)).decode()
    z.zbar_image_destroy(image)
    z.zbar_image_scanner_destroy(scanner)
    return data


def curl(*args: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(["curl", "-sk", *args], capture_output=True, check=True)


def csrf_from_jar(path: Path) -> str:
    for line in path.read_text().splitlines():
        if "__Host-grv_csrf" in line:
            return line.split("\t")[-1].strip()
    raise RuntimeError("no CSRF cookie")


def tctl(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [TCTL, "--config", CFG, *args],
        capture_output=True,
        text=True,
    )


def web_login(code: str) -> tuple[int, str]:
    COOKIE.write_text("")
    curl("-c", str(COOKIE), "-b", str(COOKIE), "-o", str(DIR / "login.html"), f"{PROXY}/web/login")
    body = json.dumps(
        {
            "user": USER,
            "pass": PASSWORD,
            "second_factor_token": code,
        }
    )
    out = DIR / "session.out"
    r = subprocess.run(
        [
            "curl",
            "-sk",
            "-c",
            str(COOKIE),
            "-b",
            str(COOKIE),
            "-H",
            "Content-Type: application/json",
            "-H",
            f"X-CSRF-Token: {csrf_from_jar(COOKIE)}",
            "-d",
            body,
            "-w",
            "%{http_code}",
            "-o",
            str(out),
            f"{PROXY}/webapi/sessions/web",
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    return int(r.stdout.strip() or "0"), out.read_text() if out.is_file() else ""


def ensure_logins() -> None:
    tctl("users", "update", USER, f"--set-logins={LOGINS}")


def login_ok() -> bool:
    if not SECRET_FILE.is_file():
        return False
    secret = SECRET_FILE.read_text().strip().replace(" ", "")
    if not secret:
        return False
    code = totp_fresh(secret)
    status, body = web_login(code)
    if status == 200:
        return True
    if "too many incorrect attempts" in body:
        return False
    return False


def token_from(text: str) -> str:
    m = re.search(r"/web/(?:invite|reset)/([0-9a-f]+)", text)
    if not m:
        raise RuntimeError(text.strip() or "no invite token")
    return m.group(1)


def complete_invite(token: str) -> str:
    COOKIE.write_text("")
    curl(
        "-c",
        str(COOKIE),
        "-b",
        str(COOKIE),
        "-o",
        str(DIR / "reset.html"),
        f"{PROXY}/web/invite/{token}",
    )
    js = curl(
        "-c",
        str(COOKIE),
        "-b",
        str(COOKIE),
        f"{PROXY}/webapi/users/password/token/{token}",
    ).stdout
    data = json.loads(js)
    png = base64.b64decode(data["qrCode"])
    (DIR / "qr.png").write_bytes(png)
    otpauth = decode_qr_png(png)
    secret = parse_qs(urlparse(otpauth).query)["secret"][0]
    code = totp_fresh(secret)
    body = json.dumps(
        {
            "token": token,
            "password": base64.b64encode(PASSWORD.encode()).decode(),
            "second_factor_token": code,
        }
    )
    r = subprocess.run(
        [
            "curl",
            "-sk",
            "-c",
            str(COOKIE),
            "-b",
            str(COOKIE),
            "-H",
            "Content-Type: application/json",
            "-H",
            f"X-CSRF-Token: {csrf_from_jar(COOKIE)}",
            "-X",
            "PUT",
            "-d",
            body,
            "-w",
            "%{http_code}",
            "-o",
            str(DIR / "put.out"),
            f"{PROXY}/webapi/users/password/token",
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    if r.stdout.strip() != "200":
        raise RuntimeError(f"{r.stdout} {(DIR / 'put.out').read_text()}")
    return secret


def recreate_user() -> str:
    tctl("users", "rm", USER)
    added = tctl("users", "add", USER, f"--roles={ROLES}", f"--logins={LOGINS}")
    text = added.stdout + added.stderr
    if added.returncode:
        raise RuntimeError(text.strip() or "tctl users add failed")
    return complete_invite(token_from(text))


def main() -> int:
    DIR.mkdir(parents=True, exist_ok=True)
    if not Path(TCTL).exists():
        print(f"missing tctl at {TCTL}", file=sys.stderr)
        return 1
    try:
        if login_ok():
            ensure_logins()
            PASSWORD_FILE.write_text(PASSWORD + "\n")
            print(f"user {USER} ok")
            print(f"password {PASSWORD}")
            print(f"totp {SECRET_FILE.read_text().strip()}")
            print("web https://127.0.0.1:3080")
            print("mfa  http://127.0.0.1:3057")
            return 0
        secret = recreate_user()
        ensure_logins()
        PASSWORD_FILE.write_text(PASSWORD + "\n")
        SECRET_FILE.write_text(secret + "\n")
        if not login_ok():
            print("demo recreated but web login still failed", file=sys.stderr)
            return 1
    except Exception as e:
        print(e, file=sys.stderr)
        return 1
    print(f"user {USER}")
    print(f"password {PASSWORD}")
    print(f"totp {secret}")
    print("web https://127.0.0.1:3080")
    print("mfa  http://127.0.0.1:3057")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
