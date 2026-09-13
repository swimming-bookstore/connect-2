import { expect, test, type Page } from "@playwright/test";

async function state(page: Page) {
  const r = await page.request.get("/state");
  expect(r.ok()).toBeTruthy();
  return r.json();
}

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
  const s = await state(page);
  if (s.dst) {
    await page.goto("/");
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    return;
  }
  await home(page);
  await page.getByTestId("box-card").first().click();
  await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
}

test.describe.serial("console chrome", () => {
  test("index boots leptos", async ({ page }) => {
    const html = await page.request.get("/").then((r) => r.text());
    expect(html).toContain("connect2_ui.js");
    expect(html).not.toContain("fonts.googleapis.com");
    await home(page);
    await expect(page.locator("#boot")).toHaveCount(0);
  });

  test("three panes: chats, room, and AI", async ({ page }) => {
    await home(page);
    await expect(page.getByTestId("chats-nav").getByTestId("nav-boxes")).toContainText("Connect 2");
    await expect(page.locator(".chats > .navbar")).toHaveCount(0);
    await expect(page.locator(".chats")).toBeVisible();
    await expect(page.locator(".room")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toHaveClass(/ai-pane/);
    await expect(page.getByTestId("dir-title")).toBeVisible();
    await expect(page.locator(".menu").getByTestId("box-card").first()).toBeVisible();
    await expect(page.locator(".welcome")).toBeVisible();
    await expect(page.locator(".welcome h1")).toHaveCount(0);
    await expect(page.locator(".welcome")).toHaveText("");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("chats list has no user badge or Chats title", async ({ page }) => {
    await home(page);
    await expect(page.locator("#who")).toHaveCount(0);
    await expect(page.locator(".menu-title")).toHaveCount(0);
    await expect(page.getByTestId("theme")).toBeVisible();
  });

  test("directory lists every online box", async ({ page }) => {
    await home(page);
    const s = await state(page);
    expect(s.peers.length).toBeGreaterThanOrEqual(2);
    const cards = page.getByTestId("box-card");
    await expect(cards).toHaveCount(s.peers.length);
    for (const p of s.peers) {
      await expect(page.getByTestId("box-card-name").filter({ hasText: p.name })).toBeVisible();
    }
    await expect(page.getByTestId("box-card").first()).toBeVisible();
    await expect(page.getByTestId("empty-boxes")).toHaveCount(0);
  });

  test("desk is three columns; AI is a right pane", async ({ page }) => {
    await home(page);
    const desk = page.locator(".desk");
    await expect(desk.locator("> .chats")).toBeVisible();
    await expect(desk.locator("> .room")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toHaveClass(/ai-pane/);
    const cols = await desk.evaluate((el) => getComputedStyle(el).gridTemplateColumns);
    expect(cols.split(" ").filter(Boolean).length).toBe(3);
    const chats = await page.locator(".chats").boundingBox();
    const room = await page.locator(".room").boundingBox();
    const ai = await page.locator(".ai-pane").boundingBox();
    expect(chats && room && ai).toBeTruthy();
    expect(room!.x).toBeGreaterThan(chats!.x);
    expect(ai!.x).toBeGreaterThan(room!.x);
    const vp = page.viewportSize()!;
    const shell = await page.locator(".shell").boundingBox();
    expect(shell).toBeTruthy();
    expect(shell!.width).toBeGreaterThan(vp.width * 0.95);
    expect(shell!.height).toBeGreaterThan(vp.height * 0.95);
  });

  test("theme toggle light and dark persist", async ({ page }) => {
    await page.addInitScript(() => {});
    await page.request.post("/cmd", { data: { type: "home" } });
    await page.goto("/");
    await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
    const btn = page.getByTestId("theme");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await btn.click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    expect(await page.evaluate(() => localStorage.getItem("connect2-theme"))).toBe("dark");
    await page.reload();
    await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.getByTestId("theme").click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    expect(await page.evaluate(() => localStorage.getItem("connect2-theme"))).toBe("light");
  });

  test("session work tabs live in the top navbar", async ({ page }) => {
    await session(page);
    const nav = page.getByTestId("chats-nav");
    await expect(nav.getByTestId("nav-boxes")).toBeVisible();
    await expect(nav.getByTestId("nav-boxes")).toContainText("Connect 2");
    await expect(nav.getByTestId("box-name")).toHaveText("box-1");
    await expect(nav.getByTestId("nav-shell")).toBeVisible();
    await expect(nav.getByTestId("nav-browser")).toBeVisible();
    await expect(nav.getByTestId("nav-agent")).toBeVisible();
    await expect(nav.getByTestId("box-close")).toBeVisible();
    await expect(page.locator("#rail")).toHaveCount(0);
    await expect(page.locator(".chat-head")).toHaveCount(0);
    await expect(page.getByTestId("nav-more")).toBeVisible();
    await expect(page.getByTestId("theme")).toBeVisible();
    await expect(page.locator("#who")).toHaveCount(0);
    await page.getByTestId("nav-more").click();
    await expect(page.getByTestId("plane-logout")).toHaveText("Sign out");
  });

  test("session theme toggle still works", async ({ page }) => {
    await session(page);
    await page.getByTestId("theme").click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", /^(light|dark)$/);
    const theme = await page.locator("html").getAttribute("data-theme");
    await page.getByTestId("theme").click();
    await expect(page.locator("html")).not.toHaveAttribute("data-theme", theme || "missing");
  });

  test("narrow viewport is one pane with a phone dock", async ({ page }) => {
    await home(page);
    await page.setViewportSize({ width: 700, height: 800 });
    await expect(page.getByTestId("phone-dock")).toBeVisible();
    await expect(page.getByTestId("dir-title")).toBeVisible();
    await expect(page.locator(".room")).toBeHidden();
    await expect(page.locator(".ai-pane")).toBeHidden();
    await page.getByTestId("phone-ai").click();
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await expect(page.locator(".chats")).toBeHidden();
    await page.setViewportSize({ width: 1280, height: 800 });
  });
});
