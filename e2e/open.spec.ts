import { expect, test, type Page } from "@playwright/test";

async function state(page: Page) {
  const r = await page.request.get("/state");
  expect(r.ok()).toBeTruthy();
  return r.json();
}

async function waitState(page: Page, pred: (s: any) => boolean, ms = 15_000) {
  const t0 = Date.now();
  let last: any;
  while (Date.now() - t0 < ms) {
    last = await state(page);
    if (pred(last)) return last;
    await page.waitForTimeout(150);
  }
  throw new Error(`state wait failed: ${JSON.stringify(last)}`);
}

async function home(page: Page) {
  await page.request.post("/cmd", { data: { type: "home" } });
  await waitState(page, (s) => !s.dst);
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
}

test.describe.serial("one click open", () => {
  test("Open button is not remounted by /state polls", async ({ page }) => {
    await home(page);
    const btn = page.getByTestId("box-card").first();
    await expect(btn).toBeVisible();
    const handle = await btn.elementHandle();
    expect(handle).toBeTruthy();
    // /state ticks every 120ms. If the table rebuilds, this node dies.
    await page.waitForTimeout(900);
    const alive = await handle!.evaluate((el) => {
      const first = document.querySelector('[data-testid="box-card"]');
      return el.isConnected && el === first;
    });
    expect(alive, "Open button was replaced while idle — first click would miss").toBeTruthy();
  });

  test("one click after /state polls opens shell; never needs a second click", async ({ page }) => {
    for (let round = 0; round < 3; round++) {
      await home(page);
      await expect(page.getByTestId("box-card").first()).toBeVisible();
      await page.waitForTimeout(500);

      let opens = 0;
      const onReq = (r: { url: () => string; method: () => string; postDataJSON: () => any }) => {
        if (!(r.url().includes("/cmd") && r.method() === "POST")) return;
        try {
          if (r.postDataJSON()?.type === "open") opens += 1;
        } catch {
          /* ignore */
        }
      };
      page.on("request", onReq);

      await page.getByTestId("box-card").first().click({ clickCount: 1, force: false });
      await expect(page.getByTestId("nav-shell"), `round ${round}: first click must open`).toBeVisible({
        timeout: 8_000,
      });
      await expect(page.getByTestId("dir-title")).toHaveCount(0);
      await waitState(page, (s) => !!s.dst && s.kind === "shell");
      expect(opens, `round ${round}: first click must POST open`).toBeGreaterThanOrEqual(1);

      page.off("request", onReq);
      await page.getByTestId("nav-boxes").click({ clickCount: 1 });
      await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 8_000 });
    }
  });

  test("one Open click lands on shell and stays there", async ({ page }) => {
    await home(page);
    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("dir-title")).toHaveCount(0);
    await expect(page.getByTestId("nav-shell")).toHaveClass(/on/);
    await expect(page.locator("#term")).not.toHaveClass(/off/);
    const opened = await waitState(page, (s) => !!s.dst && s.kind === "shell");
    expect(opened.kind).toBe("shell");
    await page.waitForTimeout(700);
    await expect(page.getByTestId("nav-shell")).toBeVisible();
    await expect(page.getByTestId("dir-title")).toHaveCount(0);
    expect((await state(page)).kind).toBe("shell");
  });

  test("leftover #turn720 does not steal the first Open into browser", async ({ page }) => {
    await home(page);
    await page.goto("/#turn720");
    await expect(page.getByTestId("dir-title")).toBeVisible();
    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("nav-shell")).toHaveClass(/on/);
    await expect(page.locator("#term")).not.toHaveClass(/off/);
    await expect(page.locator("#live")).toHaveClass(/off/);
    await waitState(page, (s) => s.kind === "shell");
    expect(page.url()).toMatch(/\/shell/);
    await page.waitForTimeout(600);
    expect((await state(page)).kind).toBe("shell");
    await expect(page.getByTestId("nav-browser")).not.toHaveClass(/on/);
  });

  test("leftover #jpeg does not steal the first Open", async ({ page }) => {
    await home(page);
    await page.goto("/#jpeg");
    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-shell")).toHaveClass(/on/, { timeout: 15_000 });
    await waitState(page, (s) => s.kind === "shell");
    await expect(page.locator("#stage")).toHaveClass(/off/);
  });

  test("one Boxes click returns home and stays", async ({ page }) => {
    await home(page);
    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-boxes")).toBeVisible({ timeout: 15_000 });
    await page.getByTestId("nav-boxes").click({ clickCount: 1 });
    await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("nav-shell")).toHaveCount(0);
    await waitState(page, (s) => !s.dst);
    await page.waitForTimeout(600);
    await expect(page.getByTestId("dir-title")).toBeVisible();
    await expect(page.getByTestId("box-card").first()).toBeVisible();
  });

  test("Boxes clears path so the next Open is still shell", async ({ page }) => {
    await home(page);
    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 15_000 });
    await page.getByTestId("nav-browser").click();
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("video-mode").selectOption("turn720");
    await page.getByTestId("video-save").click();
    await expect(page.getByTestId("nav-browser")).toBeVisible();
    expect(page.url()).toMatch(/\/turn720/);
    await page.getByTestId("nav-boxes").click({ clickCount: 1 });
    await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 15_000 });
    const path = await page.evaluate(() => location.pathname);
    expect(path === "/" || path === "").toBeTruthy();
    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-shell")).toHaveClass(/on/, { timeout: 15_000 });
    await waitState(page, (s) => s.kind === "shell");
  });
});
