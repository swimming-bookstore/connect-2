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
  await waitState(page, (s) => !s.dst);
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
}

async function openBox(page: Page) {
  const s = await state(page);
  if (s.dst) {
    await page.goto("/");
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    return;
  }
  await home(page);
  await page.getByTestId("box-card").first().click({ clickCount: 1 });
  await waitState(page, (x) => !!x.dst && x.kind === "shell");
  await expect(page.getByTestId("nav-shell")).toBeVisible();
}

async function setVideo(page: Page, value: "jpeg" | "turn720" | "turn1080") {
  await page.getByTestId("nav-more").click();
  await page.getByTestId("nav-settings").click();
  await expect(page.getByTestId("settings")).toBeVisible();
  await page.getByTestId("video-mode").selectOption(value);
  await page.getByTestId("video-save").click();
}

async function openJpeg(page: Page) {
  await home(page);
  await setVideo(page, "jpeg");
  await openBox(page);
  await page.getByTestId("nav-browser").click();
  await waitState(
    page,
    (x) => x.kind === "browser" && x.video === "jpeg" && x.tabs?.length >= 1,
  );
  await expect(page.locator("#stage")).not.toHaveClass(/off/);
  await expect(page.locator("#live")).toHaveClass(/off/);
}

test.describe.serial("jpeg / turn (connect2 loop)", () => {
  test("jpeg open: /shot is a real jpeg and the view paints", async ({ page }) => {
    await openJpeg(page);
    await expect
      .poll(async () => (await page.request.get("/shot")).status(), { timeout: 15_000 })
      .toBe(200);
    const shot = await page.request.get("/shot");
    expect(shot.headers()["content-type"]).toMatch(/jpeg/);
    const body = await shot.body();
    expect(body.length).toBeGreaterThan(100);
    expect(body[0]).toBe(0xff);
    expect(body[1]).toBe(0xd8);
    await expect
      .poll(async () => page.locator("#view").getAttribute("src"), { timeout: 15_000 })
      .toMatch(/shot/);
    const box = await page.locator("#view").boundingBox();
    expect(box?.width ?? 0).toBeGreaterThan(8);
    expect(box?.height ?? 0).toBeGreaterThan(8);
  });

  test("jpeg then TURN 720 is not a leftover jpeg stage", async ({ page }) => {
    await openJpeg(page);
    await setVideo(page, "turn720");
    await expect(page.getByTestId("nav-browser")).toBeVisible();
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#stage")).toHaveClass(/off/);
    await expect(page.locator("#live")).not.toHaveClass(/off/);
    await expect(page.locator("#rtc")).toBeVisible();
    await waitState(page, (x) => x.kind === "browser" && x.video === "webrtc" && x.height === 720);
  });

  test("TURN 720: hello size then answer after laptop offer", async ({ page }) => {
    await home(page);
    await setVideo(page, "turn720");
    await openBox(page);
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#rtc")).toBeVisible();
    const s = await waitState(
      page,
      (x) =>
        x.kind === "browser" &&
        x.video === "webrtc" &&
        x.height === 720 &&
        x.width === 1280 &&
        typeof x.answer === "string" &&
        x.answer.length > 10 &&
        Array.isArray(x.ice) &&
        x.ice.length > 0,
      25_000,
    );
    expect(s.answer).toMatch(/ice-ufrag/i);
    expect(s.answer).toMatch(/a=fingerprint/i);
    expect(page.url()).toMatch(/\/turn720/);
  });

  test("720 → 1080: new hello size", async ({ page }) => {
    await home(page);
    await setVideo(page, "turn720");
    await openBox(page);
    await page.getByTestId("nav-browser").click();
    await waitState(page, (x) => x.video === "webrtc" && x.height === 720);
    await setVideo(page, "turn1080");
    await page.getByTestId("nav-browser").click();
    const s = await waitState(
      page,
      (x) => x.kind === "browser" && x.video === "webrtc" && x.height === 1080 && x.width === 1920,
      25_000,
    );
    expect(s.height).toBe(1080);
    await expect(page.locator("#live")).not.toHaveClass(/off/);
  });

  test("leaving TURN then returning still shows live pane", async ({ page }) => {
    await home(page);
    await setVideo(page, "turn720");
    await openBox(page);
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#live")).not.toHaveClass(/off/);
    await waitState(page, (x) => x.video === "webrtc" && x.height === 720);
    await page.getByTestId("nav-shell").click();
    await expect(page.locator("#live")).toHaveClass(/off/);
    await expect(page.locator("#term")).not.toHaveClass(/off/);
    await waitState(page, (x) => x.kind === "shell");
    await page.getByTestId("nav-browser").click();
    await expect(page.locator("#live")).not.toHaveClass(/off/);
    await expect(page.locator("#rtc")).toBeVisible();
    await waitState(page, (x) => x.kind === "browser" && x.video === "webrtc" && x.height === 720);
  });
});
