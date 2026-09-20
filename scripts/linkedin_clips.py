#!/usr/bin/env python3
"""Record three ~10s LinkedIn 1080×1080 clips.

Each take starts mid-type in Ask AI (~1–2s before Enter), then Enter and the reply.
The full Connect 2 window is scaled into the square (no crop). Demo CSS + xterm
fonts are bumped so the UI stays readable. Files use a ~320 kb/s floor
(LinkedIn rejects videos under 75 KB).
"""

from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import record as rec

ROOT = rec.ROOT
CLIPS = (
    (
        "shell",
        "Shell",
        "say hello to 수영 책방 Swimming Bookstore",
        lambda s: (
            s.get("kind") == "shell"
            and "box-1" in str(s.get("dst") or "")
            and "Swimming Bookstore" in (s.get("stdout") or "")
        ),
        "ask-shell",
        40.0,
        6.0,
    ),
    (
        "browser",
        "Browser",
        "open swimming bookstore in the browser",
        lambda s: (
            s.get("kind") == "browser"
            and "box-2" in str(s.get("dst") or "")
            and "swimming-bookstore" in (s.get("url") or "").lower()
            and int(s.get("jpeg_n") or 0) > 0
        ),
        "ask-browser",
        55.0,
        6.0,
    ),
    (
        "agent",
        "Agent",
        "ask box-3 agent to say hello",
        lambda s: (
            s.get("kind") == "agent"
            and "box-3" in str(s.get("dst") or "")
            and any(
                (x or "").startswith("ai ") and "Swimming Bookstore" in (x or "")
                for x in (s.get("log") or [])
            )
        ),
        "ask-agent",
        50.0,
        6.0,
    ),
)


def bar_png(path: Path, title: str) -> None:
    import cairo

    w, h = 1080, 1080
    cream = (0xFF / 255, 0xF9 / 255, 0xEF / 255)
    navy = (0x1D / 255, 0x35 / 255, 0x50 / 255)
    muted = (0x6A / 255, 0x5C / 255, 0x4E / 255)
    surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
    cr = cairo.Context(surf)
    cr.set_source_rgb(*cream)
    cr.paint()
    cr.select_font_face("Liberation Serif", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(20)
    cr.set_source_rgb(*muted)
    t = "CONNECT 2"
    xb, _yb, tw, _th, _xa, _ya = cr.text_extents(t)
    cr.move_to((w - tw) / 2 - xb, 64)
    cr.show_text(t)
    cr.select_font_face("Liberation Serif", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)
    cr.set_font_size(64)
    cr.set_source_rgb(*navy)
    xb, _yb, tw, _th, _xa, _ya = cr.text_extents(title)
    cr.move_to((w - tw) / 2 - xb, 138)
    cr.show_text(title)
    cr.select_font_face("Liberation Serif", cairo.FONT_SLANT_ITALIC, cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(28)
    cr.set_source_rgb(*muted)
    t = "Use AI to control your boxes."
    xb, _yb, tw, _th, _xa, _ya = cr.text_extents(t)
    cr.move_to((w - tw) / 2 - xb, 972)
    cr.show_text(t)
    cr.select_font_face("Noto Serif CJK KR", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(22)
    cr.set_source_rgb(*muted)
    t = "© 수영 책방 Swimming Bookstore"
    xb, _yb, tw, _th, _xa, _ya = cr.text_extents(t)
    cr.move_to((w - tw) / 2 - xb, 1048)
    cr.show_text(t)
    surf.write_to_png(str(path))


VIDEO_W, VIDEO_H = 1080, 675
# LinkedIn native video rejects files under 75 KB.
MIN_BPS = 320_000
MIN_BYTES = 80_000
DEMO_FONT_PX = 16
TERM_FONT_PX = 16
DEMO_ZOOM = 1
WIN_W, WIN_H = 1280, 800


def probe_dur(path: Path) -> float:
    out = subprocess.check_output(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            str(path),
        ],
        text=True,
    ).strip()
    return float(out)


def probe_wh(path: Path) -> tuple[int, int]:
    out = subprocess.check_output(
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
            str(path),
        ],
        text=True,
    ).strip()
    w, h = out.split(",")
    return int(w), int(h)


def fit_vf(src_w: int, src_h: int, out_w: int = VIDEO_W, out_h: int = VIDEO_H) -> str:
    """Letterbox the full Connect 2 window into the square slot (no crop)."""
    s = min(out_w / max(src_w, 1), out_h / max(src_h, 1))
    w = max(2, int(src_w * s)) & ~1
    h = max(2, int(src_h * s)) & ~1
    w = min(w, out_w - (out_w % 2))
    h = min(h, out_h - (out_h % 2))
    return (
        f"scale={w}:{h}:flags=lanczos,setsar=1,"
        f"pad={out_w}:{out_h}:({out_w}-iw)/2:({out_h}-ih)/2:color=0xFFF9EF"
    )


def encode_v() -> list[str]:
    return [
        "-an",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-profile:v",
        "high",
        "-b:v",
        str(MIN_BPS),
        "-minrate",
        str(MIN_BPS),
        "-maxrate",
        str(MIN_BPS),
        "-bufsize",
        str(MIN_BPS * 2),
        "-x264-params",
        "nal-hrd=cbr:force-cfr=1",
        "-preset",
        "slow",
        "-r",
        str(rec.FPS),
        "-movflags",
        "+faststart",
    ]


def fit_clip(
    raw: Path,
    dest: Path,
    keep_s: float,
    head_end: float,
    tail_start: float,
) -> None:
    """Keep type+Enter, a short loading beat, then the reply. Skip the long wait."""
    dur = probe_dur(raw)
    ff = rec.ffmpeg_bin()
    keep_s = max(keep_s, 1.0)
    head_end = min(max(head_end, 0.4), dur)
    load_flash = 0.35
    head = min(head_end + load_flash, keep_s - 4.0, dur)
    head = max(head, 0.6)
    result_at = min(max(tail_start, head_end), max(dur - 0.05, 0.0))
    if result_at < head + 0.2:
        result_at = min(dur, head)
    tail_need = max(keep_s - head, 0.4)
    avail = max(dur - result_at, 0.0)
    tail_take = min(tail_need, avail)
    pad = max(0.0, tail_need - tail_take)

    def encode(args: list[str]) -> None:
        subprocess.check_call(args)

    if result_at <= head + 0.12 or tail_take < 0.2:
        pad_all = max(0.0, keep_s - dur)
        vf = f"tpad=stop_mode=clone:stop_duration={pad_all:.3f}" if pad_all > 0.05 else "null"
        encode(
            [
                ff,
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                str(raw),
                "-vf",
                vf,
                "-t",
                f"{keep_s:.3f}",
                "-an",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "18",
                "-preset",
                "fast",
                str(dest),
            ]
        )
        return
    pad_f = f",tpad=stop_mode=clone:stop_duration={pad:.3f}" if pad > 0.05 else ""
    encode(
        [
            ff,
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            str(raw),
            "-filter_complex",
            f"[0:v]trim=0:{head:.3f},setpts=PTS-STARTPTS[h];"
            f"[0:v]trim=start={result_at:.3f}:end={result_at + tail_take:.3f},setpts=PTS-STARTPTS{pad_f}[t];"
            f"[h][t]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
            "-t",
            f"{keep_s:.3f}",
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "18",
            "-preset",
            "fast",
            str(dest),
        ]
    )


def square(
    raw: Path,
    title: str,
    out: Path,
    keep_s: float,
    head_end: float,
    tail_start: float,
) -> None:
    with tempfile.TemporaryDirectory() as d:
        bg = Path(d) / "bg.png"
        clip = Path(d) / "clip.mp4"
        bar_png(bg, title)
        fit_clip(raw, clip, keep_s, head_end, tail_start)
        sw, sh = probe_wh(clip)
        inner = fit_vf(sw, sh)
        subprocess.check_call(
            [
                rec.ffmpeg_bin(),
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-loop",
                "1",
                "-i",
                str(bg),
                "-i",
                str(clip),
                "-filter_complex",
                f"[1:v]{inner}[v];[0:v][v]overlay=(W-w)/2:178:shortest=1",
                *encode_v(),
                str(out),
            ]
        )
        if out.stat().st_size < MIN_BYTES:
            rec.die(f"{out} is {out.stat().st_size} bytes, LinkedIn needs >75KB")


def reframe_existing() -> None:
    """Fit the already-exported square's inner 1080×675 (full window, no extra crop)."""
    for name, title, *_rest in CLIPS:
        src = ROOT / "docs" / f"linkedin-{name}.mp4"
        if not src.exists():
            rec.die(f"missing {src}")
        with tempfile.TemporaryDirectory() as d:
            inner = Path(d) / "inner.mp4"
            bg = Path(d) / "bg.png"
            out = Path(d) / "out.mp4"
            subprocess.check_call(
                [
                    rec.ffmpeg_bin(),
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-i",
                    str(src),
                    "-vf",
                    "crop=1080:675:0:178",
                    "-an",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-crf",
                    "12",
                    "-preset",
                    "fast",
                    str(inner),
                ]
            )
            bar_png(bg, title)
            sw, sh = probe_wh(inner)
            vf = fit_vf(sw, sh)
            subprocess.check_call(
                [
                    rec.ffmpeg_bin(),
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-loop",
                    "1",
                    "-i",
                    str(bg),
                    "-i",
                    str(inner),
                    "-filter_complex",
                    f"[1:v]{vf}[v];[0:v][v]overlay=(W-w)/2:178:shortest=1",
                    *encode_v(),
                    str(out),
                ]
            )
            if out.stat().st_size < MIN_BYTES:
                rec.die(f"{out} is {out.stat().st_size} bytes, LinkedIn needs >75KB")
            dest = ROOT / "docs" / f"linkedin-{name}.mp4"
            dest.write_bytes(out.read_bytes())
            print(f"reframed {dest} ({dest.stat().st_size} bytes)")


def demo_scale(ws: rec.Ws) -> None:
    """Bigger UI type for the square clip. Full window stays in frame."""
    rec.js(
        ws,
        f"""(() => {{
          const root = document.documentElement;
          root.style.zoom = '1';
          root.style.fontSize = '{DEMO_FONT_PX}px';
          let st = document.getElementById('demo-type');
          if (!st) {{
            st = document.createElement('style');
            st.id = 'demo-type';
            document.head.appendChild(st);
          }}
          st.textContent = `
            html {{ zoom: 1; }}
            html, body {{ font-size: {DEMO_FONT_PX}px !important; }}
            .text-xs, .text-sm, .chat-bubble, .input, textarea, button, .btn, .menu, .label, .navbar {{
              font-size: {DEMO_FONT_PX}px !important;
              line-height: 1.35 !important;
            }}
            #ai-q, [data-testid="ai-q"], #q, textarea, .input {{
              font-size: {DEMO_FONT_PX}px !important;
              min-height: 2.4rem !important;
            }}
            .chat-bubble {{ padding: 0.7rem 0.9rem !important; }}
          `;
          if (typeof connectTermFontSize === 'function') connectTermFontSize({TERM_FONT_PX});
        }})()""",
    )
    rec.hold(0.25)


def size_window(ws: rec.Ws) -> None:
    try:
        win = ws.call("Browser.getWindowForTarget")
        wid = win.get("windowId")
        if wid is None:
            return
        ws.call(
            "Browser.setWindowBounds",
            {
                "windowId": wid,
                "bounds": {
                    "left": 40,
                    "top": 40,
                    "width": WIN_W,
                    "height": WIN_H,
                    "windowState": "normal",
                },
            },
        )
    except Exception as e:
        print(f"  window bounds: {e}")
    rec.hold(0.2)


def empty_ai(ws: rec.Ws) -> None:
    rec.post_cmd({"type": "home"})
    rec.wait_pred(lambda s: not s.get("dst"), 15, "home")
    rec.wait_js(ws, """!!document.querySelector('[data-testid="ai-q"]')""", 15, "ai input")
    rec.js(
        ws,
        """(() => {
          const b = document.querySelector('[data-testid="ai-new"]');
          if (b) b.click();
        })()""",
    )
    rec.post_cmd({"type": "new_desk"})
    rec.wait_js(
        ws,
        """(() => {
          const log = document.querySelector('[data-testid="ai-log"]');
          const q = document.querySelector('#ai-q');
          if (!log || !q) return false;
          if ((q.value || '').trim()) return false;
          return !log.querySelector('.chat-bubble');
        })()""",
        12,
        "empty ask ai",
    )
    rec.hold(0.4)


def focus_ai(ws: rec.Ws) -> None:
    rec.wait_js(ws, "!!document.querySelector('#ai-q')", 15, "ai input")
    rec.js(
        ws,
        """(() => {
          if (typeof connectTermBlur === 'function') connectTermBlur();
          const i = document.querySelector('#ai-q');
          if (i) i.focus();
        })()""",
    )
    rec.hold(0.12)


def split_prompt(text: str, delay: float = 0.11, pre_enter_s: float = 1.7) -> tuple[str, str]:
    n_tail = max(14, int(round(pre_enter_s / delay)))
    if len(text) <= n_tail:
        return "", text
    return text[:-n_tail], text[-n_tail:]


def wait_frames(nframes: dict, n: int, timeout: float = 3.0) -> None:
    t0 = time.monotonic()
    while nframes["n"] < n and time.monotonic() - t0 < timeout:
        time.sleep(0.04)


def caret_end(ws: rec.Ws) -> None:
    rec.js(
        ws,
        """(() => {
          if (typeof connectTermBlur === 'function') connectTermBlur();
          const i = document.querySelector('#ai-q');
          if (!i) return;
          i.focus();
          const n = (i.value || '').length;
          if (i.setSelectionRange) i.setSelectionRange(n, n);
          i.scrollLeft = i.scrollWidth;
        })()""",
    )
    rec.hold(0.08)


def submit_ai(ws: rec.Ws) -> None:
    rec.type_keys(ws, "\n", delay=0.05)
    rec.js(
        ws,
        """(() => {
          const i = document.querySelector('#ai-q');
          if (!i) return;
          i.dispatchEvent(new Event('input', { bubbles: true }));
          const form = i.closest('form');
          if (form && (i.value || '').trim()) form.requestSubmit();
        })()""",
    )
    rec.hold(0.2)


def start_cap(dpy, wid, w, h, redirected, out: Path):
    ff = rec.spawn(
        [
            rec.ffmpeg_bin(),
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgr0",
            "-s",
            f"{w}x{h}",
            "-r",
            str(rec.FPS),
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
            str(out),
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    assert ff.stdin is not None
    stop = threading.Event()
    nframes = {"n": 0}

    def pump() -> None:
        last = None
        tick = 1.0 / rec.FPS
        t0 = time.monotonic()
        while not stop.is_set():
            if time.monotonic() - t0 > 90.0:
                break
            frame = rec.grab_bgr(dpy, wid, w, h, redirected)
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
    return ff, stop, cap, nframes


def login_cli() -> None:
    rec.DEMO_IDENT.parent.mkdir(parents=True, exist_ok=True)
    if rec.DEMO_IDENT.exists():
        rec.DEMO_IDENT.unlink()
    rec.totp_wait_unused(rec.TOTP_USED, min_left=8.0)
    otp = rec.totp_fresh(rec.TOTP_USED)
    subprocess.check_call(
        [
            str(rec.BIN / "connect2"),
            "--proxy",
            rec.PROXY,
            "--identity",
            str(rec.DEMO_IDENT),
            "--insecure",
            "login",
            "--auth",
            "local",
            "--user",
            rec.DEMO_USER,
            "--password",
            rec.DEMO_PASSWORD,
            "--otp",
            otp,
        ],
        cwd=ROOT,
    )


def boot_hub() -> None:
    env = {**os.environ, "CONNECT2_DEMO_AGENT": "1"}
    rec.spawn(
        [
            str(rec.BIN / "connect2"),
            "--proxy",
            rec.PROXY,
            "--identity",
            str(rec.DEMO_IDENT),
            "--http",
            rec.HTTP,
            "--insecure",
            "--no-open",
        ],
        env=env,
    )
    host, port = rec.HTTP.rsplit(":", 1)
    rec.wait_tcp(host, int(port), 25)


def boot() -> rec.Ws:
    rec.x11.XSetErrorHandler(rec._KEEP_HANDLER)
    for line in subprocess.check_output(["ps", "-eo", "pid,cmd"], text=True).splitlines():
        if "connect2 --proxy" in line or "connect2 --http" in line or "connect2-tauri" in line:
            try:
                os.kill(int(line.split(None, 1)[0]), signal.SIGTERM)
            except (ValueError, ProcessLookupError):
                pass
    time.sleep(0.4)
    login_cli()
    boot_hub()
    rec.wait_pred(
        lambda s: s.get("authed") and len(s.get("peers") or []) >= 3,
        40,
        "connect2 boxes",
    )
    ws = rec.attach_chrome(f"{rec.HTTP_URL}/")
    rec.wait_js(
        ws,
        """!!document.querySelector('[data-testid="box-card"]')""",
        20,
        "connect2 directory",
    )
    rec.wait_js(ws, """!!document.querySelector('#ai-q')""", 15, "ask ai")
    size_window(ws)
    demo_scale(ws)
    return ws


def find_app(dpy) -> tuple[int, int, int, bool]:
    import ctypes
    from ctypes import byref, c_int

    wid = 0
    for _ in range(80):
        wid = rec.find_window(dpy, rec.TITLE)
        if wid:
            break
        time.sleep(0.15)
    if not wid:
        rec.die("demo window not found")
    ev = c_int()
    er = c_int()
    redirected = bool(rec.xcomp.XCompositeQueryExtension(dpy, byref(ev), byref(er)))
    if redirected:
        rec.xcomp.XCompositeRedirectWindow(dpy, wid, rec.CompositeRedirectAutomatic)
        rec.x11.XSync(dpy, 0)
    w, h = rec.geometry(dpy, wid)
    for _ in range(40):
        if w >= 640 and h >= 400:
            break
        time.sleep(0.1)
        w, h = rec.geometry(dpy, wid)
    if w < 640 or h < 400:
        rec.die(f"bad window size {w}x{h}")
    w -= w % 2
    h -= h % 2
    rec.x11.XMapRaised(dpy, wid)
    rec.x11.XRaiseWindow(dpy, wid)
    rec.x11.XFlush(dpy)
    return wid, w, h, redirected


def take(ws: rec.Ws, dpy, wid, w, h, redirected, name, title, prompt, pred, label, wait_s, hold_s):
    print(f"take {name}")
    empty_ai(ws)
    demo_scale(ws)
    delay = 0.12
    head, tail = split_prompt(prompt, delay=delay, pre_enter_s=1.8)
    focus_ai(ws)
    if head:
        rec.type_input(ws, "#ai-q", head, submit=False, delay=0.02)
    rec.focus_sel(ws, "#ai-q")
    caret_end(ws)
    raw = ROOT / "docs" / f".linkedin-{name}-raw.mp4"
    out = ROOT / "docs" / f"linkedin-{name}.mp4"
    ff, stop, cap, nframes = start_cap(dpy, wid, w, h, redirected, raw)
    wait_frames(nframes, 8)
    caret_end(ws)
    rec.hold(0.12)
    if tail:
        rec.type_keys(ws, tail, delay=delay)
        caret_end(ws)
    rec.hold(0.22)
    submit_ai(ws)
    rec.hold(0.3)
    n_typed = nframes["n"]
    got = rec.wait_pred(pred, wait_s, label)
    if not pred(got):
        rec.die(f"{label} never arrived")
    n_done = nframes["n"]
    rec.hold(max(hold_s, 6.0))
    stop.set()
    cap.join(timeout=3)
    err = ff.stderr.read().decode("utf-8", "replace") if ff.stderr else ""
    rc = ff.wait(timeout=30)
    if rc != 0:
        rec.die(f"ffmpeg failed: {err[-800:]}")
    if nframes["n"] < rec.FPS * 6:
        rec.die(f"only {nframes['n']} window frames")
    fps = float(rec.FPS)
    head_end = n_typed / fps
    tail_start = max(0.0, n_done / fps - 0.2)
    print(f"  cut head={head_end:.2f}s tail={tail_start:.2f}s frames={nframes['n']}")
    square(raw, title, out, 10.0, head_end, tail_start)
    raw.unlink(missing_ok=True)
    print(f"wrote {out} ({nframes['n']} frames)")


def main() -> int:
    rec.OUT.parent.mkdir(parents=True, exist_ok=True)
    flags = {a.lower() for a in sys.argv[1:]}
    if "--reframe" in flags or "reframe" in flags:
        reframe_existing()
        return 0
    ws = boot()
    demo_scale(ws)
    dpy = rec.x11.XOpenDisplay(None)
    if not dpy:
        rec.die("cannot open X display")
    from ctypes import byref, c_int

    wid, w, h, redirected = find_app(dpy)
    root = rec.x11.XDefaultRootWindow(dpy)
    hidden = False
    evb = c_int()
    erb = c_int()
    if rec.xfixes.XFixesQueryExtension(dpy, byref(evb), byref(erb)):
        rec.xfixes.XFixesHideCursor(dpy, root)
        hidden = True
    print(f"capturing {w}x{h} window {hex(wid)}", file=sys.stderr)
    want = {a.lower() for a in sys.argv[1:] if not a.startswith("-")} - {"reframe"}
    try:
        for name, title, prompt, pred, label, wait_s, hold_s in CLIPS:
            if want and name not in want:
                continue
            take(ws, dpy, wid, w, h, redirected, name, title, prompt, pred, label, wait_s, hold_s)
    finally:
        ws.close()
        if hidden:
            rec.xfixes.XFixesShowCursor(dpy, root)
            rec.x11.XFlush(dpy)
        rec.x11.XCloseDisplay(dpy)
    return 0


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    raise SystemExit(main())
