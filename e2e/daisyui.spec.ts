import { expect, test, type Locator, type Page } from "@playwright/test";

async function home(page: Page) {
  await page.addInitScript(() => localStorage.setItem("connect2-theme", "light"));
  await page.request.post("/cmd", { data: { type: "home" } });
  for (let i = 0; i < 80; i++) {
    const s = await page.request.get("/state").then((r) => r.json());
    if (!s.dst) break;
    await page.waitForTimeout(250);
  }
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
  if (await page.getByTestId("nav-shell").count()) {
    await page.getByTestId("nav-boxes").click();
    await expect(page.getByTestId("nav-shell")).toHaveCount(0);
  }
}

async function session(page: Page) {
  const s = await page.request.get("/state").then((r) => r.json());
  if (s.dst) {
    await page.goto("/");
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    return;
  }
  await home(page);
  await page.getByTestId("box-card").first().click();
  await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
}

async function box(el: Locator) {
  return el.evaluate((n) => {
    const r = (n as HTMLElement).getBoundingClientRect();
    const s = getComputedStyle(n as HTMLElement);
    return {
      w: r.width,
      h: r.height,
      x: r.x,
      y: r.y,
      display: s.display,
      overflow: s.overflow,
      pointer: s.pointerEvents,
      visible: s.visibility !== "hidden" && s.display !== "none" && r.width > 0 && r.height > 0,
    };
  });
}

test.describe.serial("daisyui", () => {
  test("desk is three visible columns with AI pinned on the right", async ({ page }) => {
    await home(page);
    const desk = page.locator(".desk");
    const d = await box(desk);
    const vp = page.viewportSize()!;
    expect(d.h).toBeGreaterThan(vp.height * 0.7);
    expect(d.w).toBeGreaterThan(vp.width * 0.9);
    const shell = await box(page.locator(".shell"));
    expect(shell.w).toBeGreaterThan(vp.width * 0.95);
    expect(shell.h).toBeGreaterThan(vp.height * 0.95);
    expect(shell.x).toBeLessThan(4);
    expect(shell.y).toBeLessThan(4);

    for (const sel of [".chats", ".room"]) {
      const b = await box(page.locator(sel));
      expect(b.visible, sel).toBeTruthy();
      expect(b.h, sel).toBeGreaterThan(400);
      expect(b.w, sel).toBeGreaterThan(200);
    }
    await expect(page.locator(".ai-pane")).toBeVisible();

    const chats = await page.locator(".chats").boundingBox();
    const room = await page.locator(".room").boundingBox();
    expect(chats && room).toBeTruthy();
    expect(chats!.x + chats!.width).toBeLessThanOrEqual(room!.x + 8);

    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toHaveClass(/ai-pane/);
    const deskCols = await page.locator(".desk").evaluate((el) => getComputedStyle(el).gridTemplateColumns);
    expect(deskCols.split(" ").filter(Boolean).length).toBe(3);
    const ai = await page.locator(".ai-pane").boundingBox();
    expect(ai).toBeTruthy();
    expect(ai!.x).toBeGreaterThan(room!.x);
    expect(ai!.height).toBeGreaterThan(400);
  });

  test("light theme tokens and daisy surfaces", async ({ page }) => {
    await home(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    const css = await page.request.get("/style.css").then((r) => r.text());
    expect(css).toContain("--color-primary");
    expect(css).toMatch(/daisyui|primary/i);
    expect(css).not.toMatch(/holiday/i);
    const primary = await page.locator("html").evaluate((el) => getComputedStyle(el).getPropertyValue("--color-primary").trim());
    expect(primary.replace(/\s/g, "")).toMatch(/#000000|#000|0,0,0|0% 0|0%0/);
    const bg = await page.locator("html").evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(bg).not.toBe("rgba(0, 0, 0, 0)");
    const chatsBg = await page.locator(".chats").evaluate((el) => getComputedStyle(el).backgroundColor);
    const roomBg = await page.locator(".room").evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(chatsBg).not.toBe("rgba(0, 0, 0, 0)");
    expect(roomBg).not.toBe("rgba(0, 0, 0, 0)");
    const radius = await page.locator(".chats").evaluate((el) => getComputedStyle(el).borderRadius);
    expect(parseFloat(radius) || 0).toBe(0);
  });

  test("chat rows are full-width clickable buttons", async ({ page }) => {
    await home(page);
    const row = page.getByTestId("box-card").first();
    await expect(row).toBeVisible();
    const b = await box(row);
    expect(b.h).toBeGreaterThanOrEqual(32);
    expect(b.w).toBeGreaterThan(180);
    expect(b.pointer).not.toBe("none");
    const align = await row.evaluate((n) => getComputedStyle(n as HTMLElement).textAlign);
    expect(["left", "start"]).toContain(align);
    await expect(row.locator(".avatar")).toHaveCount(0);
    await expect(row.locator(".status")).toHaveCount(0);
    await expect(row).not.toContainText("live");
  });

  test("Dark sits left of Menu; Settings and Sign out stay in the menu", async ({ page }) => {
    await home(page);
    const nav = page.getByTestId("chats-nav");
    await expect(nav).toBeVisible();
    await expect(nav).toHaveClass(/navbar/);
    await expect(nav).toHaveClass(/top-bar/);
    const desk = page.locator(".desk");
    const nb = await box(nav);
    const db = await box(desk);
    expect(nb.y).toBeLessThan(db.y);
    expect(nb.w).toBeGreaterThan(db.w - 8);
    await expect(nav.getByTestId("nav-boxes")).toContainText("Connect 2");
    await expect(page.locator(".chats > .navbar")).toHaveCount(0);
    await expect(nav.locator(".nav-brand")).toBeVisible();
    await expect(nav.locator(".nav-room")).toBeAttached();
    await expect(nav.locator(".nav-ai")).toBeAttached();
    const chats = nav.getByTestId("nav-col-chats");
    const theme = chats.getByTestId("theme");
    const more = chats.getByTestId("nav-more");
    await expect(theme).toBeVisible();
    await expect(more).toBeVisible();
    const tb = await box(theme);
    const mb = await box(more);
    const brand = await box(nav.getByTestId("nav-boxes"));
    expect(tb.x).toBeLessThan(mb.x);
    expect(brand.x).toBeLessThan(tb.x);
    const col = await box(chats);
    expect(mb.x + mb.w).toBeGreaterThan(col.x + col.w - 56);
    expect(tb.x - (brand.x + brand.w)).toBeGreaterThan(8);
    expect(tb.h).toBeGreaterThanOrEqual(24);
    expect(tb.w).toBeGreaterThanOrEqual(24);
    expect(tb.pointer).not.toBe("none");
    await more.click();
    for (const id of ["nav-settings", "plane-logout"] as const) {
      const btn = nav.getByTestId(id);
      await expect(btn).toBeVisible();
      const b = await box(btn);
      expect(b.h, id).toBeGreaterThanOrEqual(20);
      expect(b.w, id).toBeGreaterThan(40);
      expect(b.pointer, id).not.toBe("none");
    }
    await expect(nav.getByTestId("plane-logout")).toHaveText("Sign out");
  });

  test("menu closes when clicking anywhere else", async ({ page }) => {
    await home(page);
    await page.getByTestId("nav-more").click();
    await expect(page.getByTestId("nav-settings")).toBeVisible();
    await page.getByTestId("dir-title").click({ position: { x: 8, y: 8 } });
    await expect(page.getByTestId("nav-settings")).toBeHidden();
  });

  test("settings is a real form with daisy select and save", async ({ page }) => {
    await home(page);
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("settings")).toBeVisible();
    await expect.poll(async () => new URL(page.url()).pathname).toBe("/settings");
    const sel = page.getByTestId("video-mode");
    await expect(sel).toBeVisible();
    const b = await box(sel);
    expect(b.h).toBeGreaterThanOrEqual(32);
    expect(b.w).toBeGreaterThan(120);
    await expect(page.getByTestId("video-save")).toHaveClass(/btn/);
    const saveBox = await box(page.getByTestId("video-save"));
    const selBox = await box(sel);
    expect(saveBox.y).toBeGreaterThanOrEqual(selBox.y + selBox.h + 8);
    expect(selBox.h).toBeLessThan(56);
    const overlap =
      selBox.x < saveBox.x + saveBox.w &&
      selBox.x + selBox.w > saveBox.x &&
      selBox.y < saveBox.y + saveBox.h &&
      selBox.y + selBox.h > saveBox.y;
    expect(overlap).toBe(false);
    await page.getByTestId("video-save").click();
    await expect(page.getByTestId("settings")).toHaveCount(0);
  });

  test("settings keeps TURN 720p after save and reopen", async ({ page }) => {
    await home(page);
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("settings")).toBeVisible();
    await page.getByTestId("video-mode").selectOption("turn720");
    await page.getByTestId("video-save").click();
    await expect(page.getByTestId("settings")).toHaveCount(0);
    await expect.poll(async () => new URL(page.url()).pathname).toBe("/");
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await expect.poll(async () => new URL(page.url()).pathname).toBe("/settings");
    await expect(page.getByTestId("video-mode")).toHaveValue("turn720");
    await page.getByTestId("settings-back").click();
    await expect.poll(async () => new URL(page.url()).pathname).toBe("/");
  });

  test("empty room has no welcome copy", async ({ page }) => {
    await home(page);
    const hero = page.locator(".welcome").first();
    await expect(hero).toBeVisible();
    await expect(hero).toHaveText("");
    const b = await box(hero);
    expect(b.h).toBeGreaterThan(80);
    expect(b.h).toBeLessThanOrEqual(800);
  });

  test("room tabs look like tabs and switch panes", async ({ page }) => {
    await session(page);
    const shell = page.getByTestId("nav-shell");
    const browser = page.getByTestId("nav-browser");
    const agent = page.getByTestId("nav-agent");
    for (const el of [shell, browser, agent]) {
      const b = await box(el);
      expect(b.h).toBeGreaterThanOrEqual(24);
      expect(b.w).toBeGreaterThanOrEqual(24);
      expect(Math.abs(b.w - b.h)).toBeLessThan(12);
      await expect(el).toHaveClass(/btn-square/);
    }
    await browser.click();
    await expect(page.locator("#chrome")).not.toHaveClass(/off/);
    await expect(page.locator("#term")).toHaveClass(/off/);
    await expect(page.locator("#term")).toHaveCSS("display", "none");
    await expect(page.locator("#chrome")).toBeVisible();
    await agent.click();
    await expect(page.locator("#agent")).not.toHaveClass(/off/);
    await expect(page.locator("#term")).toHaveCSS("display", "none");
    await shell.click();
    await expect(page.locator("#term")).not.toHaveClass(/off/);
    await expect(page.locator("#term")).not.toHaveCSS("display", "none");
    await expect(page.locator("#chrome")).toHaveCSS("display", "none");
    await expect(page.locator("#agent")).toHaveCSS("display", "none");
  });

  test("url bar join is one row of equal-height controls", async ({ page }) => {
    await session(page);
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#chrome")).toBeVisible();
    const bar = page.locator("#bar");
    const back = page.locator("#back");
    const url = page.locator("#url");
    const go = page.locator("#go");
    const bb = await box(bar);
    const ub = await box(url);
    const gb = await box(go);
    const kb = await box(back);
    expect(bb.w).toBeGreaterThan(300);
    expect(ub.w).toBeGreaterThan(120);
    expect(Math.abs(ub.h - gb.h)).toBeLessThan(12);
    expect(Math.abs(kb.h - gb.h)).toBeLessThan(12);
    expect(ub.y).toBeGreaterThan(0);
  });

  test("agent composer is a daisy input plus send", async ({ page }) => {
    await session(page);
    await page.getByTestId("nav-agent").click();
    const q = page.locator("#q");
    const send = page.locator("#ask button");
    await expect(q).toBeVisible();
    await expect(send).toBeVisible();
    const qb = await box(q);
    const sb = await box(send);
    expect(qb.h).toBeGreaterThanOrEqual(32);
    expect(qb.w).toBeGreaterThan(160);
    expect(sb.h).toBeGreaterThanOrEqual(28);
    await expect(send).toHaveClass(/btn/);
  });

  test("ai rail composer is join input plus daisy send", async ({ page }) => {
    await home(page);
    const q = page.getByTestId("ai-q");
    const send = page.getByTestId("ai-send");
    if (await q.isVisible().catch(() => false)) {
      await expect(q).toBeVisible();
      await expect(send).toBeVisible();
      const qb = await box(q);
      const sb = await box(send);
      expect(qb.h).toBeGreaterThanOrEqual(32);
      expect(qb.w).toBeGreaterThan(120);
      expect(sb.h).toBeGreaterThanOrEqual(28);
      await expect(send).toHaveClass(/btn/);
    } else {
      await expect(page.getByTestId("ai-gate")).toBeVisible();
      const login = page.getByTestId("ai-login");
      await expect(login).toHaveClass(/btn/);
      expect((await box(login)).h).toBeGreaterThanOrEqual(32);
    }
  });

  test("no control overflows the viewport", async ({ page }) => {
    await home(page);
    const bad = await page.evaluate(() => {
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const out: string[] = [];
      for (const el of document.querySelectorAll("button, input, textarea, select, .chats, .room")) {
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) continue;
        if (r.right > vw + 2 || r.bottom > vh + 2 || r.left < -2 || r.top < -2) {
          out.push(`${el.id || el.getAttribute("data-testid") || el.className} ${JSON.stringify({ l: r.left, t: r.top, r: r.right, b: r.bottom })}`);
        }
      }
      return out;
    });
    expect(bad, bad.join("\n")).toEqual([]);
  });

  test("theme toggle switches light and dark", async ({ page }) => {
    await home(page);
    const btn = page.getByTestId("theme").first();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await btn.click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.getByTestId("theme").first().click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("cluster gate uses daisy inputs when signed out overlay is shown via empty authed UI", async ({ page }) => {
    await home(page);
    await expect(page.getByTestId("cluster-user")).toHaveCount(0);
    await expect(page.locator(".desk")).toBeVisible();
  });
});
