#!/usr/bin/env python3
"""Record docs/demo.mp4 by capturing the live connect2 Chromium window.

XGetImage of that window only. CDP/HTTP drive Shell / box Chromium JPEG /
Agent over Teleport; they never become the video.
"""

from __future__ import annotations

import atexit
import base64
import ctypes
import hashlib
import hmac
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from ctypes import (
    CFUNCTYPE,
    POINTER,
    Structure,
    byref,
    c_char_p,
    c_int,
    c_long,
    c_uint,
    c_ulong,
    c_void_p,
    cast,
)
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "debug"
OUT = ROOT / "docs" / "demo.mp4"
FPS = 15
TITLE = "Connect 2 demo"
CDP_PORT = 9223
MAX_SEC = 170.0
IDENT = Path(os.environ.get("TELEPORT_IDENT", "/tmp/tp/client/demo"))
DEMO_IDENT = Path(os.environ.get("DEMO_IDENT", "/tmp/tp/client/demo-record"))
PROXY = os.environ.get("TELEPORT_PROXY", "127.0.0.1:3080")
WEB = os.environ.get("TELEPORT_WEB", "https://127.0.0.1:3080")
HTTP = os.environ.get("DEMO_HTTP", "127.0.0.1:3056")
HTTP_URL = f"http://{HTTP}"
DEMO_USER = os.environ.get("DEMO_USER", "demo")
DEMO_PASSWORD = os.environ.get("DEMO_PASSWORD", "demo-pass-2026")
TOTP_SECRET = Path(os.environ.get("DEMO_TOTP_SECRET", "/tmp/tp/totp-secret.txt"))
TOTP_USED: set[str] = set()

os.environ.setdefault("DISPLAY", ":0.0")
os.environ.setdefault("XAUTHORITY", str(Path.home() / ".Xauthority"))

CHILDREN: list[subprocess.Popen] = []

x11 = ctypes.CDLL("libX11.so.6")
xcomp = ctypes.CDLL("libXcomposite.so.1")
xfixes = ctypes.CDLL("libXfixes.so.3")

ZPixmap = 2
AllPlanes = c_ulong(~0)
CompositeRedirectAutomatic = 0


class XImage(Structure):
    _fields_ = [
        ("width", c_int),
        ("height", c_int),
        ("xoffset", c_int),
        ("format", c_int),
        ("data", c_void_p),
        ("byte_order", c_int),
        ("bitmap_unit", c_int),
        ("bitmap_bit_order", c_int),
        ("bitmap_pad", c_int),
        ("depth", c_int),
        ("bytes_per_line", c_int),
        ("bits_per_pixel", c_int),
        ("red_mask", c_ulong),
        ("green_mask", c_ulong),
        ("blue_mask", c_ulong),
        ("obdata", c_void_p),
        ("f", c_void_p * 6),
    ]


x11.XOpenDisplay.restype = c_void_p
x11.XOpenDisplay.argtypes = [c_char_p]
x11.XCloseDisplay.argtypes = [c_void_p]
x11.XDefaultRootWindow.restype = c_ulong
x11.XDefaultRootWindow.argtypes = [c_void_p]
x11.XQueryTree.restype = c_int
x11.XQueryTree.argtypes = [
    c_void_p,
    c_ulong,
    POINTER(c_ulong),
    POINTER(c_ulong),
    POINTER(POINTER(c_ulong)),
    POINTER(c_uint),
]
x11.XFetchName.restype = c_int
x11.XFetchName.argtypes = [c_void_p, c_ulong, POINTER(c_char_p)]
x11.XGetGeometry.restype = c_int
x11.XGetGeometry.argtypes = [
    c_void_p,
    c_ulong,
    POINTER(c_ulong),
    POINTER(c_int),
    POINTER(c_int),
    POINTER(c_uint),
    POINTER(c_uint),
    POINTER(c_uint),
    POINTER(c_uint),
]
x11.XGetImage.restype = POINTER(XImage)
x11.XGetImage.argtypes = [
    c_void_p,
    c_ulong,
    c_int,
    c_int,
    c_uint,
    c_uint,
    c_ulong,
    c_int,
]
x11.XDestroyImage.restype = c_int
x11.XDestroyImage.argtypes = [POINTER(XImage)]
x11.XFree.argtypes = [c_void_p]
x11.XFreePixmap.argtypes = [c_void_p, c_ulong]
x11.XFlush.argtypes = [c_void_p]
x11.XSync.argtypes = [c_void_p, c_int]
x11.XSetErrorHandler.restype = c_void_p
x11.XSetErrorHandler.argtypes = [c_void_p]
x11.XInternAtom.restype = c_ulong
x11.XInternAtom.argtypes = [c_void_p, c_char_p, c_int]
x11.XGetWindowProperty.restype = c_int
x11.XGetWindowProperty.argtypes = [
    c_void_p,
    c_ulong,
    c_ulong,
    c_long,
    c_long,
    c_int,
    c_ulong,
    POINTER(c_ulong),
    POINTER(c_int),
    POINTER(c_ulong),
    POINTER(c_ulong),
    POINTER(c_void_p),
]
x11.XRaiseWindow.argtypes = [c_void_p, c_ulong]
x11.XMapRaised.argtypes = [c_void_p, c_ulong]

xcomp.XCompositeQueryExtension.restype = c_int
xcomp.XCompositeQueryExtension.argtypes = [c_void_p, POINTER(c_int), POINTER(c_int)]
xcomp.XCompositeRedirectWindow.argtypes = [c_void_p, c_ulong, c_int]
xcomp.XCompositeNameWindowPixmap.restype = c_ulong
xcomp.XCompositeNameWindowPixmap.argtypes = [c_void_p, c_ulong]
xfixes.XFixesQueryExtension.restype = c_int
xfixes.XFixesQueryExtension.argtypes = [c_void_p, POINTER(c_int), POINTER(c_int)]
xfixes.XFixesHideCursor.argtypes = [c_void_p, c_ulong]
xfixes.XFixesShowCursor.argtypes = [c_void_p, c_ulong]


@CFUNCTYPE(c_int, c_void_p, c_void_p)
def _xerr(_dpy, _ev):
    return 0


_KEEP_HANDLER = _xerr


def die(msg: str, code: int = 1) -> None:
    print(msg, file=sys.stderr)
    sys.exit(code)


def kill_all() -> None:
    for p in reversed(CHILDREN):
        if p.poll() is None:
            p.send_signal(signal.SIGTERM)
    time.sleep(0.2)
    for p in reversed(CHILDREN):
        if p.poll() is None:
            p.kill()


atexit.register(kill_all)


def spawn(args: list[str], **kw) -> subprocess.Popen:
    kw.setdefault("stdin", subprocess.DEVNULL)
    p = subprocess.Popen(args, cwd=ROOT, **kw)
    CHILDREN.append(p)
    return p


def port_up(host: str, port: int) -> bool:
    s = socket.socket()
    s.settimeout(0.3)
    try:
        s.connect((host, port))
        return True
    except OSError:
        return False
    finally:
        s.close()


def wait_tcp(host: str, port: int, secs: float = 20) -> None:
    deadline = time.time() + secs
    while time.time() < deadline:
        if port_up(host, port):
            return
        time.sleep(0.15)
    die(f"timeout waiting {host}:{port}")


def http_get(url: str, timeout: float = 8) -> tuple[int, bytes, str]:
    req = urllib.request.Request(url, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read(), r.headers.get("content-type", "")
    except urllib.error.HTTPError as e:
        return e.code, e.read(), e.headers.get("content-type", "")


def post_cmd(obj: dict) -> None:
    req = urllib.request.Request(
        f"{HTTP_URL}/cmd",
        data=json.dumps(obj).encode(),
        method="POST",
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=20) as r:
        r.read()


def state() -> dict:
    _, body, _ = http_get(f"{HTTP_URL}/state")
    return json.loads(body.decode())


def wait_pred(pred, secs: float, label: str) -> dict:
    deadline = time.time() + secs
    last: dict = {}
    while time.time() < deadline:
        last = state()
        if pred(last):
            print(f"  ok {label}")
            return last
        time.sleep(0.25)
    print(
        f"  timeout {label} kind={last.get('kind')} video={last.get('video')} "
        f"h={last.get('height')} url={last.get('url')!r} last={last.get('last')!r}"
    )
    return last


def children_of(dpy, win: int) -> list[int]:
    root = c_ulong()
    parent = c_ulong()
    kids = POINTER(c_ulong)()
    n = c_uint()
    if not x11.XQueryTree(dpy, win, byref(root), byref(parent), byref(kids), byref(n)):
        return []
    out = [kids[i] for i in range(n.value)]
    if kids:
        x11.XFree(cast(kids, c_void_p))
    return out


def net_wm_name(dpy, win: int) -> str:
    atom = x11.XInternAtom(dpy, b"_NET_WM_NAME", 0)
    utf8 = x11.XInternAtom(dpy, b"UTF8_STRING", 0)
    actual_type = c_ulong()
    actual_fmt = c_int()
    nitems = c_ulong()
    bytes_after = c_ulong()
    prop = c_void_p()
    status = x11.XGetWindowProperty(
        dpy,
        win,
        atom,
        0,
        1024,
        0,
        utf8,
        byref(actual_type),
        byref(actual_fmt),
        byref(nitems),
        byref(bytes_after),
        byref(prop),
    )
    if status != 0 or not prop:
        return ""
    s = ctypes.string_at(prop, nitems.value).decode("utf-8", "replace")
    x11.XFree(prop)
    return s


def window_name(dpy, win: int) -> str:
    n = net_wm_name(dpy, win)
    if n:
        return n
    name = c_char_p()
    if x11.XFetchName(dpy, win, byref(name)) and name.value:
        s = name.value.decode("utf-8", "replace")
        x11.XFree(name)
        return s
    return ""


def walk_windows(dpy, win: int) -> list[int]:
    out = [win]
    for c in children_of(dpy, win):
        out.extend(walk_windows(dpy, c))
    return out


def find_window(dpy, title: str) -> int:
    root = x11.XDefaultRootWindow(dpy)
    needles = (title.lower(), "connectagentfold", "connect 2 demo")
    best = 0
    best_area = 0
    for win in walk_windows(dpy, root):
        name = window_name(dpy, win).lower()
        if not any(n in name for n in needles):
            continue
        w, h = geometry(dpy, win)
        if w < 640 or h < 400:
            continue
        # Prefer the 1280x800 app window, not a full-screen leftover.
        area = w * h
        score = area
        if 1200 <= w <= 1400 and 700 <= h <= 900:
            score += 10_000_000
        if score > best_area:
            best = win
            best_area = score
    if best:
        return best
    try:
        out = subprocess.check_output(
            ["xwininfo", "-root", "-tree"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError:
        return 0
    for line in out.splitlines():
        low = line.lower()
        if not any(n in low for n in needles):
            continue
        tok = line.strip().split()[0]
        if not tok.startswith("0x"):
            continue
        wid = int(tok, 16)
        w, h = geometry(dpy, wid)
        if w >= 640 and h >= 400:
            return wid
    return 0


def geometry(dpy, win: int) -> tuple[int, int]:
    root = c_ulong()
    x = c_int()
    y = c_int()
    w = c_uint()
    h = c_uint()
    bw = c_uint()
    depth = c_uint()
    if not x11.XGetGeometry(
        dpy, win, byref(root), byref(x), byref(y), byref(w), byref(h), byref(bw), byref(depth)
    ):
        return 0, 0
    return int(w.value), int(h.value)


def grab_bgr(dpy, win: int, w: int, h: int, redirected: bool) -> bytes | None:
    drawable = win
    pix = 0
    if redirected:
        pix = xcomp.XCompositeNameWindowPixmap(dpy, win)
        if pix:
            drawable = pix
    img_p = x11.XGetImage(dpy, drawable, 0, 0, w, h, AllPlanes, ZPixmap)
    if pix:
        x11.XFreePixmap(dpy, pix)
    if not img_p:
        return None
    img = img_p.contents
    if img.bits_per_pixel != 32 or not img.data:
        x11.XDestroyImage(img_p)
        return None
    raw = ctypes.string_at(img.data, img.bytes_per_line * h)
    stride = img.bytes_per_line
    x11.XDestroyImage(img_p)
    row = w * 4
    if stride == row:
        return raw
    packed = bytearray(h * row)
    for y in range(h):
        packed[y * row : (y + 1) * row] = raw[y * stride : y * stride + row]
    return bytes(packed)


def ffmpeg_bin() -> str:
    ff = os.environ.get("FF") or str(Path.home() / ".local/bin/ffmpeg")
    if os.access(ff, os.X_OK):
        return ff
    return shutil.which("ffmpeg") or die("ffmpeg not found")


def wait_js(ws: Ws, expr: str, secs: float, label: str):
    deadline = time.time() + secs
    last = None
    while time.time() < deadline:
        last = js(ws, expr)
        if last:
            print(f"  ok {label}")
            return last
        time.sleep(0.25)
    die(f"timeout {label} last={last!r}")


def totp_code(secret: str, now: float | None = None) -> str:
    pad = "=" * ((8 - len(secret) % 8) % 8)
    key = base64.b32decode(secret.upper() + pad, casefold=True)
    t = int((now if now is not None else time.time()) // 30)
    digest = hmac.new(key, struct.pack(">Q", t), hashlib.sha1).digest()
    off = digest[-1] & 0x0F
    code = (struct.unpack(">I", digest[off : off + 4])[0] & 0x7FFFFFFF) % 1_000_000
    return f"{code:06d}"


def totp_fresh(used: set[str] | None = None) -> str:
    if not TOTP_SECRET.exists():
        die(f"TOTP secret missing at {TOTP_SECRET}")
    secret = TOTP_SECRET.read_text().strip().replace(" ", "")
    while True:
        left = 30 - int(time.time() % 30)
        if left < 4:
            time.sleep(left + 0.05)
        code = totp_code(secret)
        if used is None or code not in used:
            if used is not None:
                used.add(code)
            return code
        time.sleep(left + 0.05)


def boot_client(identity: Path) -> None:
    env = {**os.environ, "CONNECT2_DEMO_AGENT": "1"}
    spawn(
        [
            str(BIN / "connect2"),
            "--proxy",
            PROXY,
            "--identity",
            str(identity),
            "--http",
            HTTP,
            "--no-open",
        ],
        env=env,
    )
    host, port = HTTP.rsplit(":", 1)
    wait_tcp(host, int(port), 25)


class Ws:
    def __init__(self, url: str):
        from urllib.parse import urlparse
        import base64
        import struct

        u = urlparse(url)
        self.sock = socket.create_connection((u.hostname, u.port or 80), timeout=10)
        self.sock.settimeout(10)
        key = base64.b64encode(os.urandom(16)).decode()
        path = u.path or "/"
        if u.query:
            path += "?" + u.query
        req = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {u.hostname}:{u.port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        )
        self.sock.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("cdp websocket handshake closed")
            buf += chunk
        if b"101" not in buf.split(b"\r\n", 1)[0]:
            raise RuntimeError(f"cdp handshake failed: {buf[:200]!r}")
        extra = buf.split(b"\r\n\r\n", 1)[1]
        self._rest = extra
        self._id = 0

    def _send_frame(self, payload: bytes) -> None:
        import struct

        mask = os.urandom(4)
        header = bytearray()
        header.append(0x81)
        n = len(payload)
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", n))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", n))
        header.extend(mask)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(header + masked)

    def _read_exact(self, n: int) -> bytes:
        out = bytearray()
        if self._rest:
            take = self._rest[:n]
            self._rest = self._rest[n:]
            out.extend(take)
        while len(out) < n:
            chunk = self.sock.recv(n - len(out))
            if not chunk:
                raise RuntimeError("cdp socket closed")
            out.extend(chunk)
        return bytes(out)

    def _recv_frame(self) -> bytes:
        import struct

        while True:
            b1, b2 = self._read_exact(2)
            opcode = b1 & 0x0F
            masked = b2 & 0x80
            n = b2 & 0x7F
            if n == 126:
                n = struct.unpack("!H", self._read_exact(2))[0]
            elif n == 127:
                n = struct.unpack("!Q", self._read_exact(8))[0]
            mask = self._read_exact(4) if masked else b""
            payload = self._read_exact(n)
            if masked:
                payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
            if opcode == 0x8:
                raise RuntimeError("cdp websocket closed")
            if opcode == 0x9:
                self.sock.sendall(bytes([0x8A, 0x80, 0, 0, 0, 0]))
                continue
            if opcode in (0x1, 0x2, 0x0):
                return payload

    def call(self, method: str, params: dict | None = None, timeout: float = 20) -> dict:
        self._id += 1
        mid = self._id
        msg = {"id": mid, "method": method}
        if params:
            msg["params"] = params
        self._send_frame(json.dumps(msg).encode())
        deadline = time.time() + timeout
        while time.time() < deadline:
            self.sock.settimeout(max(0.2, deadline - time.time()))
            try:
                raw = self._recv_frame()
            except socket.timeout:
                continue
            data = json.loads(raw.decode())
            if data.get("id") == mid:
                if "error" in data:
                    raise RuntimeError(f"{method}: {data['error']}")
                return data.get("result") or {}
        raise TimeoutError(method)

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


def js(ws: Ws, expr: str) -> object:
    r = ws.call(
        "Runtime.evaluate",
        {"expression": expr, "returnByValue": True},
    )
    return (r.get("result") or {}).get("value")


def cdp_pages() -> list[dict]:
    try:
        _, body, _ = http_get(f"http://127.0.0.1:{CDP_PORT}/json/list")
        tabs = json.loads(body.decode())
    except Exception:
        return []
    return [t for t in tabs if t.get("type") == "page"]


def attach_page(page: dict) -> Ws:
    ws = Ws(page["webSocketDebuggerUrl"])
    ws.call("Page.enable")
    ws.call("Runtime.enable")
    ws.call("Page.bringToFront")
    try:
        win = ws.call("Browser.getWindowForTarget")
        wid = win.get("windowId")
        if wid is not None:
            ws.call(
                "Browser.setWindowBounds",
                {
                    "windowId": wid,
                    "bounds": {
                        "left": 40,
                        "top": 40,
                        "width": 1280,
                        "height": 800,
                        "windowState": "normal",
                    },
                },
            )
    except Exception as e:
        print(f"  window bounds: {e}")
    js(ws, f"document.title = {json.dumps(TITLE)}")
    time.sleep(0.4)
    return ws


def wait_page(pred, secs: float, label: str) -> dict:
    deadline = time.time() + secs
    last = None
    while time.time() < deadline:
        pages = cdp_pages()
        last = pages
        for t in pages:
            if pred(t):
                print(f"  ok {label}")
                return t
        time.sleep(0.25)
    die(f"timeout {label} last={last!r}")


def attach_chrome(url: str) -> Ws:
    chrome = shutil.which("chromium") or shutil.which("google-chrome") or os.environ.get("CHROME")
    if not chrome:
        die("need chromium")
    udd = ROOT / "target" / "fold-ui-chrome"
    if udd.exists():
        shutil.rmtree(udd, ignore_errors=True)
    default = udd / "Default"
    default.mkdir(parents=True)
    (default / "Preferences").write_text(
        json.dumps(
            {
                "credentials_enable_service": False,
                "profile": {"password_manager_enabled": False},
                "autofill": {"profile_enabled": False, "credit_card_enabled": False},
            }
        )
    )
    spawn(
        [
            chrome,
            "--ozone-platform=x11",
            "--class=ConnectAgentFold",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-save-password-bubble",
            "--disable-password-manager-reauthentication",
            "--password-store=basic",
            "--disable-features=PasswordManagerOnboarding,PasswordCheck,PasswordLeakDetection,AutofillServerCommunication",
            "--ignore-certificate-errors",
            "--allow-insecure-localhost",
            "--autoplay-policy=no-user-gesture-required",
            "--no-sandbox",
            # hide “You are using an unsupported command-line flag: --no-sandbox”
            "--test-type",
            "--disable-infobars",
            f"--remote-debugging-port={CDP_PORT}",
            f"--user-data-dir={udd}",
            "--window-size=1280,800",
            "--window-position=40,40",
            f"--app={url}",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env={**os.environ, "DISPLAY": os.environ.get("DISPLAY", ":0.0")},
    )
    wait_tcp("127.0.0.1", CDP_PORT, 15)
    page = wait_page(lambda t: True, 10, "chrome page")
    return attach_page(page)


def focus_sel(ws: Ws, selector: str) -> None:
    got = js(
        ws,
        f"""(() => {{
          const el = document.querySelector({json.dumps(selector)});
          if (!el) return null;
          el.focus();
          el.click();
          const r = el.getBoundingClientRect();
          return {{x: r.left + Math.min(24, r.width / 2), y: r.top + r.height / 2}};
        }})()""",
    )
    if not isinstance(got, dict):
        die(f"no {selector}")
    x, y = float(got["x"]), float(got["y"])
    for typ in ("mousePressed", "mouseReleased"):
        ws.call(
            "Input.dispatchMouseEvent",
            {"type": typ, "x": x, "y": y, "button": "left", "clickCount": 1},
        )
    time.sleep(0.15)


def key_event(ws: Ws, typ: str, **params) -> None:
    ws.call("Input.dispatchKeyEvent", {"type": typ, **params})


def type_keys(ws: Ws, text: str, delay: float = 0.08) -> None:
    for ch in text:
        if ch == "\n":
            key_event(
                ws,
                "keyDown",
                key="Enter",
                code="Enter",
                windowsVirtualKeyCode=13,
            )
            key_event(
                ws,
                "keyUp",
                key="Enter",
                code="Enter",
                windowsVirtualKeyCode=13,
            )
        else:
            key_event(ws, "keyDown", text=ch, unmodifiedText=ch, key=ch)
        time.sleep(delay)


def clear_input(ws: Ws, selector: str) -> None:
    js(
        ws,
        f"""(() => {{
          const i = document.querySelector({json.dumps(selector)});
          if (!i) return;
          i.focus();
          const proto = Object.getOwnPropertyDescriptor(
            i.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype,
            'value'
          );
          proto.set.call(i, '');
          i.dispatchEvent(new Event('input', {{bubbles: true}}));
        }})()""",
    )


def click_sel(ws: Ws, selector: str) -> None:
    got = js(
        ws,
        f"""(() => {{
          const el = document.querySelector({json.dumps(selector)});
          if (!el) return null;
          const r = el.getBoundingClientRect();
          return {{x: r.left + r.width / 2, y: r.top + r.height / 2}};
        }})()""",
    )
    if not isinstance(got, dict):
        die(f"no {selector}")
    x, y = float(got["x"]), float(got["y"])
    for typ in ("mousePressed", "mouseReleased"):
        ws.call(
            "Input.dispatchMouseEvent",
            {"type": typ, "x": x, "y": y, "button": "left", "clickCount": 1},
        )
    time.sleep(0.2)


def type_input(ws: Ws, selector: str, text: str, submit: bool = True) -> None:
    focus_sel(ws, selector)
    clear_input(ws, selector)
    time.sleep(0.2)
    for ch in text:
        js(
            ws,
            f"""(() => {{
              const i = document.querySelector({json.dumps(selector)});
              if (!i) return;
              const proto = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
              proto.set.call(i, (i.value || '') + {json.dumps(ch)});
              i.dispatchEvent(new Event('input', {{bubbles: true}}));
            }})()""",
        )
        time.sleep(0.07)
    time.sleep(0.25)
    if submit:
        if selector == "#q":
            click_sel(ws, "#ask button")
        elif selector == "#ai-q":
            click_sel(ws, '[data-testid="ai-send"]')
        else:
            type_keys(ws, "\n", delay=0.05)
        time.sleep(0.2)


def type_ai(ws: Ws, text: str) -> None:
    wait_js(ws, "!!document.querySelector('#ai-q')", 15, "ai input")
    js(
        ws,
        """(() => {
          if (typeof connectTermBlur === 'function') connectTermBlur();
          const i = document.querySelector('#ai-q');
          if (i) i.focus();
        })()""",
    )
    time.sleep(0.2)
    type_input(ws, "#ai-q", text, submit=True)


def type_placeholder(ws: Ws, placeholder: str, text: str) -> None:
    sel = f'input[placeholder={json.dumps(placeholder)}]'
    wait_js(ws, f"!!document.querySelector({json.dumps(sel)})", 20, placeholder)
    type_input(ws, sel, text, submit=False)


def teleport_web_login(ws: Ws) -> None:
    js(
        ws,
        """localStorage.setItem('grv_teleport_license_acknowledged', 'true');""",
    )
    got = js(
        ws,
        """!!document.querySelector('input[placeholder="Username"]')""",
    )
    if not got:
        js(
            ws,
            """(() => {
              const box = document.querySelector('input[type="checkbox"]');
              if (box && !box.checked) box.click();
              const b = [...document.querySelectorAll('button')].find(
                (el) => (el.textContent || '').trim() === 'Continue'
              );
              if (b && !b.disabled) b.click();
            })()""",
        )
        hold(0.8)
    wait_js(
        ws,
        """!!document.querySelector('input[placeholder="Username"]')""",
        25,
        "teleport username",
    )
    print("teleport web login")
    hold(0.8)
    type_placeholder(ws, "Username", DEMO_USER)
    type_placeholder(ws, "Password", DEMO_PASSWORD)
    wait_js(
        ws,
        """!!document.querySelector('input[placeholder="123 456"]')""",
        10,
        "teleport otp",
    )
    type_placeholder(ws, "123 456", totp_fresh(TOTP_USED))
    hold(0.3)
    js(
        ws,
        """(() => {
          const b = [...document.querySelectorAll('button')].find(
            (el) => (el.textContent || '').trim() === 'Sign In'
          );
          if (b) b.click();
        })()""",
    )
    wait_js(
        ws,
        """(() => {
          const t = document.body ? document.body.innerText : '';
          return t.includes('box-1') && t.includes('box-2') && t.includes('box-3');
        })()""",
        35,
        "teleport boxes",
    )
    print("teleport dashboard")
    hold(3.0)


def navigate(ws: Ws, url: str) -> None:
    ws.call("Page.navigate", {"url": url})
    time.sleep(0.6)


def hold(secs: float) -> None:
    time.sleep(secs)


def main() -> int:
    x11.XSetErrorHandler(_KEEP_HANDLER)
    ffmpeg = ffmpeg_bin()
    OUT.parent.mkdir(parents=True, exist_ok=True)

    host, port = HTTP.rsplit(":", 1)
    subprocess.check_call(["cargo", "build", "--bins", "--features", "web"], cwd=ROOT)
    for line in subprocess.check_output(["ps", "-eo", "pid,cmd"], text=True).splitlines():
        if "connect2 --proxy" in line or "connect2 --http" in line or "connect2-tauri" in line:
            try:
                os.kill(int(line.split(None, 1)[0]), signal.SIGTERM)
            except (ValueError, ProcessLookupError):
                pass
    time.sleep(0.6)
    if DEMO_IDENT.exists():
        DEMO_IDENT.unlink()
    boot_client(DEMO_IDENT)

    wait_pred(lambda s: not s.get("authed"), 20, "signed-out")
    ws = attach_chrome(f"{WEB}/web/login")
    js(
        ws,
        """localStorage.setItem('grv_teleport_license_acknowledged', 'true');""",
    )
    ws.call("Page.reload")
    time.sleep(1.0)

    dpy = x11.XOpenDisplay(None)
    if not dpy:
        die("cannot open X display")

    wid = 0
    for _ in range(80):
        wid = find_window(dpy, TITLE)
        if wid:
            break
        time.sleep(0.15)
    if not wid:
        die("demo window not found")

    ev = c_int()
    er = c_int()
    redirected = bool(xcomp.XCompositeQueryExtension(dpy, byref(ev), byref(er)))
    if redirected:
        xcomp.XCompositeRedirectWindow(dpy, wid, CompositeRedirectAutomatic)
        x11.XSync(dpy, 0)

    root = x11.XDefaultRootWindow(dpy)
    hidden = False
    evb = c_int()
    erb = c_int()
    if xfixes.XFixesQueryExtension(dpy, byref(evb), byref(erb)):
        xfixes.XFixesHideCursor(dpy, root)
        hidden = True

    w, h = geometry(dpy, wid)
    for _ in range(40):
        if w >= 640 and h >= 400:
            break
        time.sleep(0.1)
        w, h = geometry(dpy, wid)
    if w < 640 or h < 400:
        die(f"bad window size {w}x{h}")
    w -= w % 2
    h -= h % 2
    x11.XMapRaised(dpy, wid)
    x11.XRaiseWindow(dpy, wid)
    x11.XFlush(dpy)
    print(f"capturing {w}x{h} window {hex(wid)}", file=sys.stderr)

    ff = subprocess.Popen(
        [
            ffmpeg,
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgr0",
            "-s",
            f"{w}x{h}",
            "-r",
            str(FPS),
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "18",
            "-preset",
            "fast",
            "-movflags",
            "+faststart",
            str(OUT),
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    CHILDREN.append(ff)
    assert ff.stdin is not None

    stop = threading.Event()
    nframes = {"n": 0}

    def pump() -> None:
        last = None
        tick = 1.0 / FPS
        t0 = time.monotonic()
        while not stop.is_set():
            if time.monotonic() - t0 > MAX_SEC:
                break
            frame = grab_bgr(dpy, wid, w, h, redirected)
            if frame is None:
                frame = last
            if frame is None:
                time.sleep(tick)
                continue
            last = frame
            try:
                ff.stdin.write(frame)
            except BrokenPipeError:
                break
            nframes["n"] += 1
            time.sleep(tick)
        try:
            ff.stdin.close()
        except OSError:
            pass

    cap = threading.Thread(target=pump, daemon=True)
    cap.start()

    try:
        teleport_web_login(ws)
        navigate(ws, f"{HTTP_URL}/")
        wait_js(
            ws,
            """!!document.querySelector('[data-testid="cluster-login-form"]')""",
            20,
            "connect2 login",
        )
        print("connect2 login")
        hold(1.0)
        type_input(ws, '[data-testid="cluster-user"]', DEMO_USER, submit=False)
        type_input(ws, '[data-testid="cluster-password"]', DEMO_PASSWORD, submit=False)
        type_input(ws, '[data-testid="cluster-otp"]', totp_fresh(TOTP_USED), submit=False)
        hold(0.4)
        click_sel(ws, '[data-testid="cluster-login"]')
        wait_pred(
            lambda s: s.get("authed") and len(s.get("peers") or []) >= 3,
            30,
            "connect2 boxes",
        )
        wait_js(
            ws,
            """!!document.querySelector('[data-testid="box-card"]')""",
            20,
            "connect2 directory",
        )
        print("directory")
        hold(2.0)
        post_cmd({"type": "home"})
        wait_pred(lambda s: not s.get("dst"), 15, "home")
        hold(1.5)

        print("ask ai shell")
        type_ai(ws, "open box-1 shell and say hello to 수영 책방 Swimming Bookstore")
        wait_pred(
            lambda s: s.get("kind") == "shell"
            and "box-1" in str(s.get("dst") or "")
            and "Swimming Bookstore" in (s.get("stdout") or ""),
            40,
            "ask-shell",
        )
        hold(2.5)

        print("ask ai browser")
        type_ai(ws, "open box-2 browser and go to https://swimming-bookstore.github.io/homepage/")
        wait_pred(
            lambda s: s.get("kind") == "browser"
            and "box-2" in str(s.get("dst") or "")
            and "swimming-bookstore" in (s.get("url") or "").lower()
            and int(s.get("jpeg_n") or 0) > 0,
            55,
            "ask-browser",
        )
        hold(3.5)

        print("ask ai agent")
        type_ai(ws, "ask box-3 agent to list the Rust modules in src/")
        wait_pred(
            lambda s: s.get("kind") == "agent"
            and "box-3" in str(s.get("dst") or "")
            and any((x or "").startswith("ai ") for x in (s.get("log") or [])),
            50,
            "ask-agent",
        )
        hold(4.0)
    finally:
        stop.set()
        cap.join(timeout=3)
        ws.close()
        if hidden:
            xfixes.XFixesShowCursor(dpy, root)
            x11.XFlush(dpy)
        x11.XCloseDisplay(dpy)

    err = ff.stderr.read().decode("utf-8", "replace") if ff.stderr else ""
    rc = ff.wait(timeout=30)
    if rc != 0:
        die(f"ffmpeg failed: {err[-800:]}")
    if nframes["n"] < FPS * 8:
        die(f"only {nframes['n']} window frames")
    print(f"wrote {OUT} ({nframes['n']} frames, window capture)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
