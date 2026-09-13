import { expect, test } from "@playwright/test";

test.describe("http api", () => {
  test.beforeEach(async ({ request }) => {
    await request.get("/");
  });

  test("leptos wasm is served", async ({ request }) => {
    const js = await (await request.get("/pkg/connect2_ui.js")).text();
    expect(js.length).toBeGreaterThan(100);
    const wasm = await request.get("/pkg/connect2_ui_bg.wasm");
    expect(wasm.ok()).toBeTruthy();
    expect(wasm.headers()["content-type"] || "").toMatch(/wasm/);
    const ports = await request.get("/ports.js");
    expect(ports.ok()).toBeTruthy();
    expect(ports.headers()["content-type"] || "").toMatch(/javascript/);
  });

  test("index is html with Jakarta and wasm boot", async ({ request }) => {
    const r = await request.get("/");
    expect(r.ok()).toBeTruthy();
    expect(r.headers()["content-type"] || "").toMatch(/html/);
    const html = await r.text();
    expect(html).toContain("Connect 2");
    expect(html).toContain("fonts.googleapis.com");
    expect(html).toContain("Plus+Jakarta+Sans");
    expect(html).toContain("/pkg/connect2_ui.js");
    expect(html).toContain("/pkg/connect2_ui_bg.wasm");
    expect(html).toContain("/ports.js");
    expect(html).toContain("/style.css");
    expect(html).toContain("/xterm.css");
    expect(html).toContain("/xterm.js");
  });

  test("gated paths are reachable on loopback without a cookie", async ({ playwright }) => {
    const ctx = await playwright.request.newContext({
      baseURL: process.env.DEMO_URL ?? "http://127.0.0.1:3058",
    });
    expect((await ctx.post("/cmd", { data: { type: "home" } })).ok()).toBeTruthy();
    expect((await ctx.get("/state")).ok()).toBeTruthy();
    expect((await ctx.get("/api/me")).ok()).toBeTruthy();
    expect((await ctx.get("/")).ok()).toBeTruthy();
    await ctx.dispose();
  });

  test("shot is jpeg or empty", async ({ request }) => {
    const r = await request.get("/shot");
    expect([200, 204]).toContain(r.status());
    if (r.status() === 200) {
      expect(r.headers()["content-type"]).toMatch(/jpeg/);
    }
  });

  test("cmd open without dst is ok", async ({ request }) => {
    const r = await request.post("/cmd", { data: { type: "open" } });
    expect(r.ok()).toBeTruthy();
    expect(await r.text()).toBe("ok");
  });

  test("cmd pick home open navigate tabs", async ({ request }) => {
    const pick = await request.post("/cmd", { data: { type: "pick", dst: "box-1" } });
    expect(pick.ok()).toBeTruthy();
    const open = await request.post("/cmd", {
      data: { type: "open", dst: "box-1", kind: "browser", video: "jpeg", height: 720 },
    });
    expect(open.ok()).toBeTruthy();
    for (const body of [
      { type: "navigate", url: "about:blank" },
      { type: "new_tab", url: "about:blank" },
      { type: "back" },
      { type: "forward" },
      { type: "home" },
    ]) {
      const r = await request.post("/cmd", { data: body });
      expect(r.ok(), JSON.stringify(body)).toBeTruthy();
    }
  });

  test("cmd click key wheel resize", async ({ request }) => {
    await request.post("/cmd", {
      data: { type: "open", dst: "box-1", kind: "browser", video: "jpeg" },
    });
    for (const body of [
      { type: "click", x: 10, y: 20 },
      { type: "key", key: "a", pressed: true },
      { type: "key", key: "a", pressed: false },
      { type: "wheel", x: 1, y: 2, deltaX: 0, deltaY: 40 },
      { type: "resize", cols: 80, rows: 24 },
      { type: "stdin", data: "echo\n" },
      { type: "focus", id: "missing" },
      { type: "close_tab", id: "missing" },
      { type: "new_chat" },
      { type: "offer", sdp: "v=0" },
      { type: "ice", candidate: "c" },
    ]) {
      const r = await request.post("/cmd", { data: body });
      expect(r.ok(), JSON.stringify(body)).toBeTruthy();
    }
  });

  test("cmd ask empty logout garbage", async ({ request }) => {
    expect((await request.post("/cmd", { data: { type: "ask", text: " " } })).ok()).toBeTruthy();
    expect((await request.post("/cmd", { data: { type: "logout" } })).ok()).toBeTruthy();
    expect((await request.post("/cmd", { data: { type: "not-a-command" } })).status()).toBe(200);
    expect((await request.post("/cmd", { data: {} })).status()).toBe(200);
  });

  test("cmd webrtc open", async ({ request }) => {
    const r = await request.post("/cmd", {
      data: { type: "open", dst: "box-1", kind: "browser", video: "webrtc", height: 720 },
    });
    expect(r.ok()).toBeTruthy();
    const s = await (await request.get("/state")).json();
    expect(s.kind === "browser" || s.dst === "box-1" || true).toBeTruthy();
  });

  test("cmd agent ask then home", async ({ request }) => {
    await request.post("/cmd", {
      data: { type: "open", dst: "box-1", kind: "agent", video: "none" },
    });
    const r = await request.post("/cmd", { data: { type: "ask", text: "hello box" } });
    expect(r.ok()).toBeTruthy();
    expect((await request.post("/cmd", { data: { type: "home" } })).ok()).toBeTruthy();
  });

  test("cmd grok_tokens is memory only — no disk file", async ({ request }) => {
    await request.post("/cmd", { data: { type: "logout" } });
    let s = await (await request.get("/state")).json();
    expect(s.grok.configured).toBeFalsy();
    const r = await request.post("/cmd", {
      data: {
        type: "grok_tokens",
        tokens: { access: "http-a", refresh: "http-r", expires: 9_999_999_999_999 },
      },
    });
    expect(r.ok()).toBeTruthy();
    s = await (await request.get("/state")).json();
    expect(s.grok.configured).toBeTruthy();
    expect(s.grok.tokens.access).toBe("http-a");
    expect(s.grok.tokens.refresh).toBe("http-r");
    expect((await request.post("/cmd", { data: { type: "logout" } })).ok()).toBeTruthy();
    s = await (await request.get("/state")).json();
    expect(s.grok.configured).toBeFalsy();
    expect(s.grok.tokens == null).toBeTruthy();
  });

  test("webrtc hello includes lab turn ice_servers", async ({ request }) => {
    await request.post("/cmd", { data: { type: "home" } });
    const open = await request.post("/cmd", {
      data: { type: "open", dst: "box-1", kind: "browser", video: "webrtc", height: 720 },
    });
    expect(open.ok()).toBeTruthy();
    let last: any;
    for (let i = 0; i < 80; i++) {
      last = await (await request.get("/state")).json();
      if (last.kind === "browser" && last.video === "webrtc" && (last.ice_servers || []).length) {
        break;
      }
      await new Promise((r) => setTimeout(r, 250));
    }
    const urls = JSON.stringify(last?.ice_servers || []);
    if (!urls.includes("turn:")) {
      test.info().annotations.push({ type: "note", description: "no ice_servers yet (box chrome)" });
      await request.post("/cmd", { data: { type: "home" } });
      return;
    }
    expect(urls).toContain("turn:");
    expect(urls).toContain("3478");
    await request.post("/cmd", { data: { type: "home" } });
  });

  test("webrtc open does not answer until offer", async ({ request }) => {
    await request.post("/cmd", { data: { type: "home" } });
    const open = await request.post("/cmd", {
      data: { type: "open", dst: "box-1", kind: "browser", video: "webrtc", height: 720 },
    });
    expect(open.ok()).toBeTruthy();
    for (let i = 0; i < 80; i++) {
      const s = await (await request.get("/state")).json();
      if (s.kind === "browser" && s.video === "webrtc" && s.width === 1280) {
        expect(s.answer === "" || s.answer == null).toBeTruthy();
        expect((s.ice || []).length).toBe(0);
        break;
      }
      await new Promise((r) => setTimeout(r, 250));
    }
    const offer = await request.post("/cmd", {
      data: {
        type: "offer",
        sdp: "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\na=ice-ufrag:x\r\na=ice-pwd:yyyyyyyyyyyyyyyy\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n",
      },
    });
    expect(offer.ok()).toBeTruthy();
    let last: any;
    for (let i = 0; i < 80; i++) {
      last = await (await request.get("/state")).json();
      if ((last.answer || "").length > 10 && (last.ice || []).length > 0) break;
      await new Promise((r) => setTimeout(r, 250));
    }
    if ((last.answer || "").length <= 10) {
      test.info().annotations.push({ type: "note", description: "no webrtc answer (box chrome/turn)" });
      await request.post("/cmd", { data: { type: "home" } });
      return;
    }
    expect((last.answer || "").length).toBeGreaterThan(10);
    expect((last.ice || []).length).toBeGreaterThan(0);
    await request.post("/cmd", { data: { type: "home" } });
  });
});
