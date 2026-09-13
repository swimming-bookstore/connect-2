import { expect, test, type Page } from "@playwright/test";

async function state(page: Page) {
  const r = await page.request.get("/state");
  expect(r.ok()).toBeTruthy();
  return r.json();
}

async function waitState(page: Page, pred: (s: any) => boolean, ms = 25_000) {
  const t0 = Date.now();
  let last: any;
  while (Date.now() - t0 < ms) {
    last = await state(page);
    if (pred(last)) return last;
    await page.waitForTimeout(200);
  }
  throw new Error(`state wait failed: ${JSON.stringify(last)}`);
}

async function home(page: Page) {
  await page.request.post("/cmd", { data: { type: "home" } });
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
  if (await page.getByTestId("nav-shell").count()) {
    await page.getByTestId("nav-boxes").click();
  }
}

async function openBox(page: Page) {
  const s = await state(page);
  if (s.dst) {
    await page.goto("/");
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    return;
  }
  await home(page);
  await page.getByTestId("box-card").first().click();
  await waitState(page, (x) => !!x.dst);
  await expect(page.getByTestId("nav-shell")).toBeVisible();
}

async function openBrowser(page: Page) {
  await openBox(page);
  await page.getByTestId("nav-browser").click();
  await expect(page.getByTestId("nav-browser")).toHaveClass(/on/);
  await expect(page.locator("#chrome")).not.toHaveClass(/off/);
  await waitState(
    page,
    (x) => x.kind === "browser" && x.video === "jpeg" && x.tabs?.length >= 1,
  );
  await expect(page.locator("#chrome")).not.toHaveClass(/off/);
  await expect(page.getByTestId("tab").first()).toBeVisible({ timeout: 25_000 });
}

test.describe.serial("connect2", () => {
  test("directory lists boxes", async ({ page }) => {
    await home(page);
    await expect(page.getByTestId("box-card").first()).toBeVisible({ timeout: 20_000 });
    const s = await waitState(page, (x) => x.peers?.length >= 1);
    expect(s.peers.length).toBeGreaterThan(0);
    expect(s.me).toBeTruthy();
  });

  test("open box goes to shell", async ({ page }) => {
    await openBox(page);
    await expect(page.getByTestId("nav-browser")).toBeVisible();
    await expect(page.getByTestId("nav-agent")).toBeVisible();
    await expect(page.locator("#term")).toBeVisible();
    const s = await waitState(page, (x) => x.kind === "shell");
    expect(s.kind).toBe("shell");
    expect(s.dst).toBeTruthy();
  });

  test("one click opens, one click returns to boxes", async ({ page }) => {
    await home(page);
    await expect(page.getByTestId("box-card").first()).toBeVisible();
    const before = await state(page);
    expect(before.dst).toBeFalsy();

    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("dir-title")).toBeVisible();
    const opened = await waitState(page, (s) => !!s.dst && s.kind === "shell", 15_000);
    expect(opened.dst).toBeTruthy();
    expect(page.url()).toMatch(/\/shell/);
    await expect(page.locator("#term")).toBeVisible();
    await page.waitForTimeout(600);
    await expect(page.getByTestId("nav-shell")).toBeVisible();
    await expect(page.getByTestId("dir-title")).toBeVisible();

    await page.getByTestId("nav-boxes").click({ clickCount: 1 });
    await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("nav-shell")).toHaveCount(0);
    const homed = await waitState(page, (s) => !s.dst, 15_000);
    expect(homed.dst).toBeFalsy();
    await page.waitForTimeout(600);
    await expect(page.getByTestId("dir-title")).toBeVisible();
    await expect(page.getByTestId("box-card").first()).toBeVisible();
  });

  test("boxes button returns home", async ({ page }) => {
    await openBox(page);
    await page.getByTestId("nav-boxes").click();
    await waitState(page, (x) => !x.dst);
    await page.goto("/");
    await expect(page.getByTestId("dir-title")).toBeVisible();
  });

  test("jpeg: first about:blank tab", async ({ page }) => {
    await openBrowser(page);
    await expect(page.locator("#chrome")).toBeVisible();
    await expect(page.getByTestId("tab-new")).toBeVisible();
    await expect(page.getByTestId("tab-label")).toHaveText(/about:blank/i);
    await expect(page.locator("#video-bar")).toHaveCount(0);
  });

  test("jpeg: go example.com updates url and title", async ({ page }) => {
    await openBrowser(page);
    await page.locator("#url").click();
    await page.locator("#url").fill("example.com");
    await page.locator("#go").click();
    const s = await waitState(
      page,
      (x) => typeof x.url === "string" && x.url.includes("example.com"),
      20_000,
    );
    expect(s.tabs[0].title.toLowerCase()).toContain("example");
    await expect(page.locator("#url")).toHaveValue(/example\.com/, { timeout: 10_000 });
    await expect
      .poll(async () => (await page.request.get("/shot")).status(), { timeout: 15_000 })
      .toBe(200);
    await page.getByTestId("nav-back").click();
    await waitState(
      page,
      (x) => typeof x.url === "string" && (x.url.includes("about:blank") || x.url === ""),
      15_000,
    );
  });

  test("jpeg: enter in url bar navigates", async ({ page }) => {
    await openBrowser(page);
    await page.locator("#url").fill("example.com");
    await page.locator("#url").press("Enter");
    await waitState(page, (x) => String(x.url).includes("example.com"), 20_000);
  });

  test("jpeg: + adds a second tab then close", async ({ page }) => {
    await openBrowser(page);
    await page.locator("#url").fill("example.com");
    await page.locator("#go").click();
    await waitState(page, (x) => String(x.url).includes("example.com"), 20_000);
    await page.getByTestId("tab-new").click();
    await expect(page.getByTestId("tab")).toHaveCount(2, { timeout: 15_000 });
    const s = await waitState(
      page,
      (x) =>
        Array.isArray(x.tabs) &&
        x.tabs.length === 2 &&
        x.tabs.some((t: { active?: boolean; url?: string }) => t.active && String(t.url).includes("about:blank")),
      15_000,
    );
    expect(String(s.url)).toMatch(/about:blank|^$/);
    await expect(page.locator("#url")).toHaveValue(/about:blank|^$/, { timeout: 10_000 });
    await expect(page.getByTestId("tab").last()).toHaveClass(/on|tab-active/);
    await page.locator('[data-testid="tab"] .x').last().click();
    await expect(page.getByTestId("tab")).toHaveCount(1, { timeout: 15_000 });
  });

  test("jpeg: forward after back", async ({ page }) => {
    await openBrowser(page);
    await page.locator("#url").fill("example.com");
    await page.locator("#go").click();
    await waitState(page, (x) => String(x.url).includes("example.com"), 20_000);
    await page.getByTestId("nav-back").click();
    await waitState(
      page,
      (x) => String(x.url).includes("about:blank") || x.url === "",
      15_000,
    );
    await page.getByTestId("nav-fwd").click();
    await waitState(page, (x) => String(x.url).includes("example.com"), 20_000);
  });

  test("jpeg: click and wheel on view", async ({ page }) => {
    await openBrowser(page);
    const view = page.locator("#view");
    await view.click({ position: { x: 40, y: 40 }, force: true });
    await page.mouse.wheel(0, 120);
  });

  test("one browser tab: jpeg vs turn is a setting", async ({ page }) => {
    await home(page);
    await page.getByTestId("nav-more").click();
    await expect(page.getByTestId("nav-settings")).toBeVisible();
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("settings")).toBeVisible();
    await expect(page.getByTestId("video-mode")).toBeVisible();
    await page.getByTestId("video-mode").selectOption("turn720");
    await page.getByTestId("video-save").click();
    await page.getByTestId("box-card").first().click();
    await page.getByTestId("nav-browser").click();
    await waitState(page, (x) => x.video === "webrtc" && x.height === 720, 20_000);
    await expect(page.locator("#rtc")).toBeVisible();
    await expect(page.locator("#stage")).toHaveClass(/off/);
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("video-mode").selectOption("jpeg");
    await expect(page.getByTestId("video-mode")).toHaveValue("jpeg");
    await page.getByTestId("video-save").click();
    await expect(page.getByTestId("settings")).toHaveCount(0);
    await expect
      .poll(async () => page.evaluate(() => localStorage.getItem("connect2-video")), {
        timeout: 5_000,
      })
      .toBe("jpeg");
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("video-mode")).toHaveValue("jpeg");
    await page.getByTestId("settings-back").click();
  });

  test("turn720: answer and ice from box", async ({ page }) => {
    await home(page);
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("video-mode").selectOption("turn720");
    await page.getByTestId("video-save").click();
    await openBox(page);
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#rtc")).toBeVisible();
    await expect(page.locator("#stage")).toHaveClass(/off/);
    const s = await waitState(
      page,
      (x) =>
        x.video === "webrtc" &&
        x.height === 720 &&
        typeof x.answer === "string" &&
        x.answer.length > 10 &&
        Array.isArray(x.ice) &&
        x.ice.length > 0,
      25_000,
    );
    expect(s.kind).toBe("browser");
  });

  test("agent pane send message", async ({ page }) => {
    await openBox(page);
    await page.getByTestId("nav-agent").click();
    await expect(page.locator("#q")).toBeVisible();
    await expect(page.getByTestId("nav-col-room").locator("#newchat")).toHaveAttribute("aria-label", "New chat");
    await page.locator("#q").fill("ping from e2e");
    await page.locator("#ask").evaluate((el: HTMLFormElement) => el.requestSubmit());
    await expect(page.locator("#ask button")).toHaveAttribute("aria-label", "Send");
  });

  test("agent empty ask ignored", async ({ page }) => {
    await openBox(page);
    await page.getByTestId("nav-agent").click();
    await expect(page.locator("#q")).toBeVisible();
    await page.locator("#newchat").click();
    await page.locator("#q").fill("");
    await page.locator("#ask").evaluate((el: HTMLFormElement) => el.requestSubmit());
    await expect(page.locator("#q")).toBeVisible();
  });

  test("shell stdin via /cmd", async ({ page }) => {
    await openBox(page);
    await page.getByTestId("nav-shell").click();
    await expect(page.getByTestId("nav-shell")).toBeVisible();
    const r = await page.request.post("/cmd", { data: { type: "stdin", data: "true\n" } });
    expect(r.ok()).toBeTruthy();
  });

  test("ai sidebar is always on the right", async ({ page }) => {
    await home(page);
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await expect(page.locator(".chat-head")).toHaveCount(0);
    await expect(page.locator(".chat-model")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "New", exact: true })).toHaveCount(0);
  });
});
