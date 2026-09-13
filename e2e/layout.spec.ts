import { expect, test, type Page } from "@playwright/test";

async function home(page: Page) {
  await page.addInitScript(() => localStorage.setItem("connect2-theme", "light"));
  await page.request.post("/cmd", { data: { type: "home" } });
  for (let i = 0; i < 50; i++) {
    const s = await page.request.get("/state").then((r) => r.json());
    if (!s.dst) break;
    await page.waitForTimeout(100);
  }
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
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

test.describe.serial("layout chrome", () => {
  test("AI pane is always on the right and cannot be closed", async ({ page }) => {
    await home(page);
    await expect(page.getByTestId("nav-ai")).toHaveCount(0);
    await expect(page.getByTestId("ai-close")).toHaveCount(0);
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toHaveClass(/ai-pane/);
    const desk = page.locator(".desk");
    await expect(desk).not.toHaveClass(/card/);
    const cols = await desk.evaluate((el) => getComputedStyle(el).gridTemplateColumns);
    expect(cols.split(" ").filter(Boolean).length).toBe(3);
    const chats = await page.locator(".chats").boundingBox();
    const room = await page.locator(".room").boundingBox();
    const ai = await page.locator(".ai-pane").boundingBox();
    expect(chats && room && ai).toBeTruthy();
    expect(room!.x).toBeGreaterThan(chats!.x);
    expect(ai!.x).toBeGreaterThan(room!.x);
    expect(chats!.x).toBeLessThan(8);
    const radius = await page.locator(".chats").evaluate((el) => getComputedStyle(el).borderRadius);
    expect(parseFloat(radius) || 0).toBe(0);
  });

  test("one top bar holds logo, work tabs, and AI actions", async ({ page }) => {
    await session(page);
    const top = page.getByTestId("chats-nav");
    const brand = page.getByTestId("nav-boxes");
    await expect(top).toBeVisible();
    await expect(brand).toBeVisible();
    await expect(brand).toHaveClass(/brand/);
    await expect(brand).toContainText("Connect 2");
    await expect(page.locator(".chats > .navbar")).toHaveCount(0);
    await expect(page.locator("#rail")).toHaveCount(0);
    await expect(page.locator(".chat-head")).toHaveCount(0);
    await expect(page.locator(".room-bar")).toHaveCount(0);
    await expect(top).toHaveClass(/navbar/);
    await expect(top).toHaveClass(/top-bar/);
    await expect(top.getByTestId("nav-shell")).toBeVisible();
    await expect(top.getByTestId("nav-browser")).toBeVisible();
    await expect(top.getByTestId("nav-agent")).toBeVisible();
    await expect(top.getByTestId("box-close")).toBeVisible();
    await expect(top.locator(".nav-brand")).toBeVisible();
    await expect(top.locator(".nav-room")).toBeVisible();
    await expect(top.locator(".nav-ai")).toBeAttached();
    await expect(top.getByTestId("ai-title")).toHaveText("Ask AI");
    await expect(page.locator(".chat-model")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "New", exact: true })).toHaveCount(0);
    const grok = await page.request.get("/state").then((r) => r.json());
    if (grok.grok?.configured) {
      await expect(top.getByTestId("nav-col-ai").getByTestId("ai-new")).toHaveAttribute("aria-label", "New chat");
    } else {
      await expect(page.getByTestId("ai-new")).toHaveCount(0);
    }
    const th = await top.evaluate((el) => (el as HTMLElement).getBoundingClientRect().height);
    expect(th).toBeGreaterThanOrEqual(40);
    const bar = await top.boundingBox();
    const chats = await page.locator(".chats").boundingBox();
    expect(bar && chats).toBeTruthy();
    expect(bar!.y).toBeLessThan(chats!.y);
    const pairs: [string, string][] = [
      ["nav-col-chats", ".chats"],
      ["nav-col-room", ".room"],
      ["nav-col-ai", ".ai-pane"],
    ];
    for (const [navId, pane] of pairs) {
      const a = await page.getByTestId(navId).boundingBox();
      const b = await page.locator(pane).boundingBox();
      expect(a && b, navId).toBeTruthy();
      expect(Math.abs(a!.x - b!.x), navId).toBeLessThan(2);
      expect(Math.abs(a!.width - b!.width), navId).toBeLessThan(2);
    }
  });

  test("box rows are the name only — no avatar, status dot, or live label", async ({
    page,
  }) => {
    await home(page);
    const row = page.getByTestId("box-card").first();
    await expect(row).toBeVisible();
    await expect(row.locator(".avatar")).toHaveCount(0);
    await expect(row.locator(".status")).toHaveCount(0);
    await expect(row).not.toContainText("live");
    await expect(row.getByTestId("box-card-name")).toHaveText(/box-/);
    await expect(row.locator("[data-testid=box-card-name]")).toHaveCount(1);
    await expect(row).not.toHaveClass(/btn/);
    await expect(page.locator("ul.chats-list")).toBeVisible();
    await expect(page.locator("#who")).toHaveCount(0);
    await expect(page.locator(".menu-title")).toHaveCount(0);
    const nav = page.getByTestId("chats-nav");
    await expect(nav).toHaveClass(/navbar/);
    await expect(nav).toHaveClass(/top-bar/);
    await expect(nav.getByTestId("nav-boxes")).toBeVisible();
    await expect(nav.getByTestId("nav-boxes")).toContainText("Connect 2");
    const chats = nav.getByTestId("nav-col-chats");
    await expect(chats.getByTestId("nav-more")).toBeVisible();
    await expect(chats.getByTestId("theme")).toBeVisible();
    const theme = await chats.getByTestId("theme").boundingBox();
    const more = await chats.getByTestId("nav-more").boundingBox();
    const brand = await nav.getByTestId("nav-boxes").boundingBox();
    const col = await chats.boundingBox();
    expect(theme && more && brand && col).toBeTruthy();
    expect(theme!.x).toBeLessThan(more!.x);
    expect(brand!.x).toBeLessThan(theme!.x);
    expect(more!.x + more!.width).toBeGreaterThan(col!.x + col!.width - 56);
    expect(theme!.x - (brand!.x + brand!.width)).toBeGreaterThan(8);
    await nav.getByTestId("nav-more").click();
    await expect(nav.getByTestId("nav-settings")).toBeVisible();
    await expect(nav.getByTestId("plane-logout")).toBeVisible();
    const pane = await page.locator(".chats").boundingBox();
    const bar = await nav.boundingBox();
    expect(bar && pane).toBeTruthy();
    expect(bar!.y).toBeLessThan(pane!.y);
  });

  test("box highlight is instant — no DaisyUI button transition", async ({ page }) => {
    await home(page);
    const rows = page.getByTestId("box-card");
    await expect(rows).toHaveCount(3);
    const a = rows.nth(0);
    const b = rows.nth(1);
    await b.click();
    await expect(b).toHaveClass(/menu-active/, { timeout: 250 });
    await expect(a).not.toHaveClass(/menu-active/);
    const tr = await b.evaluate((el) => getComputedStyle(el).transitionDuration);
    expect(tr.split(",").every((p) => p.trim() === "0s" || p.trim() === "")).toBeTruthy();
  });

  test("room bar shows the box name as text, not a select", async ({ page }) => {
    await session(page);
    const name = page.getByTestId("box-name");
    await expect(name).toBeVisible();
    await expect(name).toHaveText(/box-/);
    await expect(name).toHaveJSProperty("tagName", "P");
    await expect(page.locator("select#box-name")).toHaveCount(0);
    await expect(page.locator("select[data-testid=box-name]")).toHaveCount(0);
    const tabs = page.locator("#nav");
    const close = page.getByTestId("box-close");
    const col = page.getByTestId("nav-col-room");
    const nb = await name.boundingBox();
    const tb = await tabs.boundingBox();
    const cb = await close.boundingBox();
    const rb = await col.boundingBox();
    expect(nb && tb && cb && rb).toBeTruthy();
    expect(tb!.x).toBeGreaterThan(nb!.x);
    expect(cb!.x).toBeGreaterThan(tb!.x);
    const mid = rb!.x + rb!.width / 2;
    expect(Math.abs(nb!.x + nb!.width / 2 - mid)).toBeLessThan(24);
  });

  test("Connect 2 brand is in the top bar and goes home", async ({ page }) => {
    await home(page);
    const brand = page.getByTestId("nav-boxes");
    await expect(brand).toHaveText("Connect 2");
    await expect(brand).toHaveClass(/brand/);
    await expect(page.getByTestId("chats-nav").getByTestId("nav-boxes")).toBeVisible();
    await expect(page.getByTestId("box-close")).toHaveCount(0);
  });

  test("close button returns home", async ({ page }) => {
    await session(page);
    await expect(page.getByTestId("box-close")).toBeVisible();
    await page.getByTestId("box-close").click();
    await expect(page.getByTestId("nav-shell")).toHaveCount(0);
    await expect(page.getByTestId("box-close")).toHaveCount(0);
    await expect(page.getByTestId("box-card").first()).not.toHaveClass(/menu-active/);
  });

  test("room stage uses DaisyUI base, not a custom wash", async ({ page }) => {
    await session(page);
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#chrome")).toBeVisible();
    const wrap = page.locator("#stage-wrap");
    await expect(wrap).toBeVisible();
    const bg = await wrap.evaluate((n) => getComputedStyle(n).backgroundColor);
    expect(bg).not.toBe("rgba(0, 0, 0, 0)");
    expect(bg).not.toBe("transparent");
  });

  test("dark mode work buttons stay square icon buttons", async ({ page }) => {
    await session(page);
    const btn = page.getByTestId("theme").first();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await btn.click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const shell = page.getByTestId("nav-shell");
    const browser = page.getByTestId("nav-browser");
    await expect(shell).toHaveClass(/on/);
    await expect(shell).toHaveClass(/btn-square/);
    await expect(browser).toHaveClass(/btn-square/);
    await expect(page.locator("#nav")).toHaveClass(/nav-work/);
  });

  test("logged-out AI gate is Login Grok only, no extra AI heading or Grok badge", async ({
    page,
  }) => {
    await home(page);
    const s = await page.request.get("/state").then((r) => r.json());
    await expect(page.locator(".chat-head")).toHaveCount(0);
    await expect(page.locator(".chat-model")).toHaveCount(0);
    await expect(page.locator("p.gate-kicker")).toHaveCount(0);
    if (s.grok?.configured) {
      await expect(page.getByTestId("chats-nav").getByTestId("ai-new")).toHaveAttribute("aria-label", "New chat");
      await expect(page.getByTestId("ai-login")).toHaveCount(0);
      return;
    }
    await expect(page.getByTestId("ai-new")).toHaveCount(0);
    await expect(page.getByTestId("ai-gate")).toBeVisible();
    await expect(page.getByTestId("ai-login")).toHaveText("Login Grok");
    await expect(page.getByTestId("ai-gate").locator("p.text-lg")).toHaveCount(0);
  });
});
