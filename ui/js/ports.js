/* Connect 2 ports: xterm + WebRTC. Hub live state is a WebSocket in the Leptos WASM. */

class ConnectTerm extends HTMLElement {}
class ConnectLive extends HTMLElement {}
if (!customElements.get("connect-term")) customElements.define("connect-term", ConnectTerm);
if (!customElements.get("connect-live")) customElements.define("connect-live", ConnectLive);

function hub() {
  const h = window.__CONNECT2_HUB;
  return typeof h === "string" && h ? h.replace(/\/$/, "") : "";
}

function post(v) {
  fetch(hub() + "/cmd", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(v),
  }).catch(() => {});
}

let term = null;
let fit = null;
let wrap = null;
let pc = null;
let termBound = false;
let pendingWrite = "";
let resizeTimer = 0;

function mountTerm() {
  const el = document.getElementById("term");
  if (!el || !window.Terminal) return false;
  if (!wrap) {
    wrap = document.createElement("div");
    wrap.style.height = "100%";
    wrap.style.width = "100%";
    term = new window.Terminal({
      cursorBlink: true,
      convertEol: true,
      fontFamily: "ui-monospace, Menlo, Consolas, monospace",
      fontSize: 13,
      theme: { background: "#111111", foreground: "#f5f5f5", cursor: "#f5f5f5" },
    });
    const Fit = window.FitAddon?.FitAddon;
    if (Fit) {
      fit = new Fit();
      term.loadAddon(fit);
    }
    term.open(wrap);
    term.onData((data) => post({ type: "stdin", data }));
    wrap.addEventListener("mousedown", (ev) => {
      if (typingElsewhere(ev)) return;
      term.focus();
    });
    wrap.addEventListener("click", (ev) => {
      if (typingElsewhere(ev)) return;
      term.focus();
    });
  }
  if (wrap.parentNode !== el) el.appendChild(wrap);
  if (!termBound) {
    termBound = true;
    el.addEventListener("mousedown", (ev) => {
      if (typingElsewhere(ev)) return;
      term?.focus();
    });
    el.addEventListener("click", (ev) => {
      if (typingElsewhere(ev)) return;
      term?.focus();
    });
    document.addEventListener(
      "keydown",
      (ev) => {
        if (typingElsewhere(ev) || !termOn()) return;
        if (!termOwnsFocus()) return;
        term.focus();
      },
      true,
    );
  }
  if (pendingWrite) {
    try {
      term.write(pendingWrite);
    } catch (_) {}
    pendingWrite = "";
  }
  requestFit();
  for (const ms of [16, 50, 200, 500]) setTimeout(requestFit, ms);
  return true;
}

function typingElsewhere(ev) {
  const t = ev?.target;
  if (t && t !== document && t !== document.body) {
    const tag = (t.tagName || "").toLowerCase();
    if (tag === "input" || tag === "textarea" || tag === "select" || t.isContentEditable) {
      return true;
    }
    if (typeof t.closest === "function" && t.closest(".ai-pane, .chat-in, #ask, #url, #q")) {
      return true;
    }
  }
  const a = document.activeElement;
  if (!a || a === document.body || a === document.documentElement) return false;
  const tag = (a.tagName || "").toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select" || a.isContentEditable) return true;
  return typeof a.closest === "function" && !!a.closest(".ai-pane, .chat-in, #ask");
}

function termOn() {
  const host = document.getElementById("term");
  return !!(host && !host.classList.contains("off") && term);
}

function termOwnsFocus() {
  const host = document.getElementById("term");
  const a = document.activeElement;
  if (!host || !a) return false;
  return host.contains(a);
}

function blurTerm() {
  try {
    term?.blur();
  } catch (_) {}
  const a = document.activeElement;
  if (a && a.classList && a.classList.contains("xterm-helper-textarea")) {
    a.blur();
  }
}

function requestFit() {
  const el = document.getElementById("term");
  if (!el || el.classList.contains("off") || !term) return;
  try {
    fit?.fit();
  } catch (_) {}
  const cols = term.cols || 0;
  const rows = term.rows || 0;
  if (cols < 8 || rows < 4) return;
  if (termOwnsFocus() && !typingElsewhere({ target: document.activeElement })) {
    term.focus();
  }
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => post({ type: "resize", cols, rows }), 120);
}

window.connectTermBlur = blurTerm;

window.connectTermWrite = (s) => {
  const chunk = String(s).replace(/\u007f/g, "\u0008");
  if (!mountTerm() || !term) {
    pendingWrite += chunk;
    return;
  }
  term.write(chunk);
};
window.connectTermReset = () => {
  pendingWrite = "";
  try {
    term?.reset();
  } catch (_) {}
};
window.connectTermFit = () => {
  mountTerm();
  requestFit();
};

const mo = new MutationObserver(() => {
  const el = document.getElementById("term");
  if (el && !el.classList.contains("off")) mountTerm();
});
mo.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
window.addEventListener("resize", () => requestFit());
document.addEventListener(
  "pointerdown",
  (ev) => {
    if (ev.target && typeof ev.target.closest === "function" && ev.target.closest(".ai-pane")) {
      blurTerm();
    }
  },
  true,
);

function mapXY(el, clientX, clientY) {
  const r = el.getBoundingClientRect();
  const fw = document.documentElement.dataset.fw ? Number(document.documentElement.dataset.fw) : 1280;
  const fh = document.documentElement.dataset.fh ? Number(document.documentElement.dataset.fh) : 720;
  return {
    x: (clientX - r.left) * (fw / Math.max(1, r.width)),
    y: (clientY - r.top) * (fh / Math.max(1, r.height)),
  };
}

document.addEventListener(
  "mousedown",
  (ev) => {
    const el = ev.target.closest("#view, #rtc");
    if (!el) return;
    el.focus();
    const { x, y } = mapXY(el, ev.clientX, ev.clientY);
    post({ type: "click", x, y });
  },
  true,
);
document.addEventListener(
  "wheel",
  (ev) => {
    const el = ev.target.closest("#view, #rtc");
    if (!el) return;
    ev.preventDefault();
    let dx = ev.deltaX;
    let dy = ev.deltaY;
    if (ev.deltaMode === 1) {
      dx *= 16;
      dy *= 16;
    } else if (ev.deltaMode === 2) {
      dx *= 720;
      dy *= 720;
    }
    post({ type: "wheel", x: ev.clientX, y: ev.clientY, deltaX: dx, deltaY: dy });
  },
  { passive: false, capture: true },
);
document.addEventListener(
  "keydown",
  (ev) => {
    const el = ev.target.closest("#view, #rtc");
    if (!el) return;
    if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
    ev.preventDefault();
    post({ type: "key", key: ev.key, pressed: true });
  },
  true,
);
document.addEventListener(
  "keyup",
  (ev) => {
    const el = ev.target.closest("#view, #rtc");
    if (!el) return;
    if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
    ev.preventDefault();
    post({ type: "key", key: ev.key, pressed: false });
  },
  true,
);

let rtcEl = null;
function ensureRtc() {
  const host = document.getElementById("live");
  if (!host) return null;
  if (!rtcEl) {
    rtcEl = document.createElement("video");
    rtcEl.id = "rtc";
    rtcEl.autoplay = true;
    rtcEl.muted = true;
    rtcEl.playsInline = true;
    rtcEl.tabIndex = 0;
  }
  if (rtcEl.parentNode !== host) host.appendChild(rtcEl);
  return rtcEl;
}

let hello = [0, 0];
let iceN = 0;
let gen = 0;
let badAnswer = "";
let starting = false;

function rtcClose() {
  gen += 1;
  try {
    pc?.close();
  } catch (_) {}
  pc = null;
  hello = [0, 0];
  iceN = 0;
  badAnswer = "";
  starting = false;
  const v = ensureRtc();
  if (v) {
    try {
      v.pause();
    } catch (_) {}
    v.srcObject = null;
  }
}

function hasUfrag(sdp) {
  const s = (sdp || "").toLowerCase();
  return s.includes("a=ice-ufrag") && s.includes("a=ice-pwd");
}
function hasFingerprint(sdp) {
  return (sdp || "").toLowerCase().includes("a=fingerprint");
}

async function rtcStart(servers, videoEl, myGen) {
  const cfg = { iceServers: [], iceTransportPolicy: "all" };
  for (const s of servers || []) {
    const ice = { urls: s.urls || [] };
    if (s.username) {
      ice.username = s.username;
      ice.credential = s.credential;
    }
    cfg.iceServers.push(ice);
  }
  if (!cfg.iceServers.length) cfg.iceServers.push({ urls: "stun:stun.l.google.com:19302" });
  const peer = new RTCPeerConnection(cfg);
  peer.addTransceiver("video", { direction: "recvonly" });
  peer.ontrack = (ev) => {
    if (gen !== myGen) return;
    const stream = ev.streams[0] || new MediaStream([ev.track]);
    videoEl.srcObject = stream;
    videoEl.muted = true;
    videoEl.autoplay = true;
    videoEl.play?.().catch(() => {});
  };
  peer.onicecandidate = (ev) => {
    if (gen !== myGen || !ev.candidate) return;
    const candidate = ev.candidate.candidate;
    if (!candidate) return;
    if (candidate.split(/\s+/)[1] === "2") return;
    post({
      type: "ice",
      candidate,
      sdp_mid: ev.candidate.sdpMid || "0",
      sdp_mline_index: ev.candidate.sdpMLineIndex ?? 0,
    });
  };
  const offer = await peer.createOffer();
  await peer.setLocalDescription(offer);
  let sdp = peer.localDescription?.sdp || offer.sdp || "";
  for (let i = 0; i < 80 && !hasUfrag(sdp); i++) {
    if (gen !== myGen) {
      peer.close();
      throw new Error("stale");
    }
    await new Promise((r) => setTimeout(r, 50));
    sdp = peer.localDescription?.sdp || offer.sdp || "";
  }
  if (!hasUfrag(sdp)) throw new Error("offer missing ice-ufrag");
  post({ type: "offer", sdp });
  return peer;
}

function addRemoteIce(peer, c) {
  if (!c?.candidate) return;
  const candidate = c.candidate.startsWith("candidate:") || c.candidate === "null" ? c.candidate : `candidate:${c.candidate}`;
  peer
    .addIceCandidate({
      candidate,
      sdpMid: c.sdp_mid || "0",
      sdpMLineIndex: c.sdp_mline_index ?? 0,
    })
    .catch(() => {});
}

async function rtcApply(msg) {
  const videoEl = ensureRtc();
  if (!videoEl) return;
  const width = msg.width || 0;
  const height = msg.height || 0;
  if (hello[0] !== width || hello[1] !== height) {
    if (starting) return;
    gen += 1;
    const myGen = gen;
    try {
      pc?.close();
    } catch (_) {}
    pc = null;
    iceN = 0;
    hello = [width, height];
    badAnswer = "";
    starting = true;
    try {
      pc = await rtcStart(msg.ice_servers || [], videoEl, myGen);
      if (gen !== myGen) {
        pc.close();
        pc = null;
        return;
      }
    } catch (e) {
      console.error("webrtc", e);
      hello = [0, 0];
      return;
    } finally {
      starting = false;
    }
  }
  const answer = msg.answer || "";
  if (answer && hasFingerprint(answer) && badAnswer !== answer && pc && !pc.remoteDescription) {
    try {
      await pc.setRemoteDescription({ type: "answer", sdp: answer });
    } catch (e) {
      console.error("answer", e);
      badAnswer = answer;
      return;
    }
  }
  const ice = msg.ice || [];
  if (pc?.remoteDescription && iceN < ice.length) {
    for (const c of ice.slice(iceN)) addRemoteIce(pc, c);
    iceN = ice.length;
  }
}

window.connectRtcApply = (msg) => rtcApply(msg);
window.connectRtcClose = () => rtcClose();
new MutationObserver(() => ensureRtc()).observe(document.documentElement, { childList: true, subtree: true });
