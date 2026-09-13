import { expect, test, type Page } from "@playwright/test";

async function home(page: Page) {
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
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
  if (await page.getByTestId("nav-shell").count()) {
    await expect(page.getByTestId("nav-shell")).toBeVisible();
    return;
  }
  await page.getByTestId("box-card").first().click();
  await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
}

test.describe.serial("ui chrome", () => {
  test("directory heading and cards", async ({ page }) => {
    await home(page);
    await expect(page.getByTestId("box-card").first()).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("chats-nav").getByTestId("nav-boxes")).toContainText("Connect 2");
    await expect(page.getByTestId("theme")).toBeVisible();
  });

  test("session rail has three work buttons not four video tabs", async ({ page }) => {
    await session(page);
    await expect(page.getByTestId("nav-shell")).toHaveAttribute("aria-label", "Shell");
    await expect(page.getByTestId("nav-browser")).toHaveAttribute("aria-label", "Browser");
    await expect(page.getByTestId("nav-agent")).toHaveAttribute("aria-label", "Agent");
    await expect(page.getByTestId("box-close")).toBeVisible();
    await expect(page.getByTestId("nav-jpeg")).toHaveCount(0);
    await expect(page.getByTestId("nav-turn720")).toHaveCount(0);
    await expect(page.getByTestId("nav-turn1080")).toHaveCount(0);
  });

  test("switching work toggles on class", async ({ page }) => {
    await session(page);
    await page.getByTestId("nav-agent").click();
    await expect(page.getByTestId("nav-agent")).toHaveClass(/on/);
    await expect(page.locator("#agent")).not.toHaveClass(/off/);
    await expect(page.locator("#term")).toHaveClass(/off/);
    await page.getByTestId("nav-shell").click();
    await expect(page.getByTestId("nav-shell")).toHaveClass(/on/);
    await expect(page.locator("#term")).not.toHaveClass(/off/);
  });

  test("browser chrome hidden on shell", async ({ page }) => {
    await session(page);
    await page.getByTestId("nav-shell").click();
    await expect(page.locator("#chrome")).toHaveClass(/off/);
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#chrome")).not.toHaveClass(/off/);
    await expect(page.locator("#back")).toBeVisible();
    await expect(page.locator("#fwd")).toBeVisible();
    await expect(page.locator("#go")).toBeVisible();
    await expect(page.locator("#url")).toBeVisible();
    await expect(page.locator("#video-bar")).toHaveCount(0);
    await page.getByTestId("nav-more").click();
    await expect(page.getByTestId("nav-settings")).toBeVisible();
  });

  test("agent send", async ({ page }) => {
    await session(page);
    await page.getByTestId("nav-agent").click();
    await expect(page.locator("#q")).toBeVisible();
    await expect(page.locator("#ask button")).toHaveAttribute("aria-label", "Send");
    const newchat = page.getByTestId("nav-col-room").locator("#newchat");
    await expect(newchat).toHaveAttribute("aria-label", "New chat");
    await expect(page.getByTestId("nav-col-ai").locator("#newchat")).toHaveCount(0);
    const name = await page.getByTestId("box-name").boundingBox();
    const nc = await newchat.boundingBox();
    const tabs = await page.locator("#nav").boundingBox();
    expect(name && nc && tabs).toBeTruthy();
    expect(nc!.x).toBeLessThan(name!.x);
    expect(nc!.x + nc!.width).toBeLessThanOrEqual(tabs!.x + 2);
  });

  test("boot splash is gone after leptos", async ({ page }) => {
    await home(page);
    await expect(page.locator("#boot")).toHaveCount(0);
  });

  test("static assets", async ({ page }) => {
    for (const [path, type] of [
      ["/style.css", "text/css"],
      ["/xterm.css", "text/css"],
      ["/xterm.js", "javascript"],
      ["/xterm-addon-fit.js", "javascript"],
      ["/pkg/connect2_ui.js", "javascript"],
      ["/ports.js", "javascript"],
    ] as const) {
      const r = await page.request.get(path);
      expect(r.ok(), path).toBeTruthy();
      expect(r.headers()["content-type"] || "").toMatch(new RegExp(type));
    }
    const wasm = await page.request.get("/pkg/connect2_ui_bg.wasm");
    expect(wasm.ok()).toBeTruthy();
    expect(wasm.headers()["content-type"]).toMatch(/wasm/);
  });

  test("404 unknown path", async ({ page }) => {
    const r = await page.request.get("/no-such");
    expect(r.status()).toBe(404);
  });

  test("state json shape", async ({ page }) => {
    const r = await page.request.get("/state");
    expect(r.ok()).toBeTruthy();
    const s = await r.json();
    expect(s).toHaveProperty("peers");
    expect(s).toHaveProperty("kind");
    expect(s).toHaveProperty("video");
    expect(s).toHaveProperty("tabs");
    expect(s).toHaveProperty("log");
    expect(s).toHaveProperty("grok");
    expect(s).toHaveProperty("me");
    expect(s).toHaveProperty("cluster");
    expect(s).toHaveProperty("authed");
    expect(s.grok).toHaveProperty("configured");
    expect(s.grok).toHaveProperty("model");
  });
});
