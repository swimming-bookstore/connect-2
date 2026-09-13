import { expect, test, type Page } from "@playwright/test";

const TOKENS = { access: "e2e-access", refresh: "e2e-refresh", expires: 9_999_999_999_999 };

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
  await waitState(page, (s) => !s.dst, 25_000);
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
}

test.describe.serial("grok tokens live in the browser", () => {
  test("fresh process is not configured and has no tokens in /state", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "logout" } });
    await home(page);
    const s = await state(page);
    expect(s.grok.configured).toBeFalsy();
    expect(s.grok.tokens == null || s.grok.tokens === undefined).toBeTruthy();
    await expect(page.getByRole("button", { name: "Login" }).first()).toBeVisible();
  });

  test("POST grok_tokens adopts into memory; /state returns them", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "logout" } });
    const r = await page.request.post("/cmd", { data: { type: "grok_tokens", tokens: TOKENS } });
    expect(r.ok()).toBeTruthy();
    const s = await waitState(page, (x) => x.grok?.configured === true);
    expect(s.grok.configured).toBeTruthy();
    expect(s.grok.tokens.access).toBe(TOKENS.access);
    expect(s.grok.tokens.refresh).toBe(TOKENS.refresh);
  });

  test("WASM writes tokens to localStorage connect2-grok", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "grok_tokens", tokens: TOKENS } });
    await home(page);
    await waitState(page, (x) => x.grok?.configured === true);
    const raw = await page.evaluate(() => localStorage.getItem("connect2-grok"));
    expect(raw).toBeTruthy();
    const t = JSON.parse(raw!);
    expect(t.access).toBe(TOKENS.access);
    expect(t.refresh).toBe(TOKENS.refresh);
  });

  test("reload restores from localStorage via /cmd grok_tokens", async ({ page }) => {
    await page.addInitScript((t) => {
      localStorage.setItem("connect2-grok", JSON.stringify(t));
    }, TOKENS);
    await page.request.post("/cmd", { data: { type: "logout" } });
    expect((await state(page)).grok.configured).toBeFalsy();
    const restore = page.waitForRequest((r) => {
      if (!(r.url().includes("/cmd") && r.method() === "POST")) return false;
      try {
        const b = r.postDataJSON();
        return b?.type === "grok_tokens" && b?.tokens?.access === TOKENS.access;
      } catch {
        return false;
      }
    });
    await page.goto("/");
    await restore;
    await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
    await waitState(page, (x) => x.grok?.configured === true);
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("ai-logout")).toHaveText("Sign out");
    await expect(page.getByRole("button", { name: "Login" })).toHaveCount(0);
  });

  test("Grok Sign out clears localStorage and /state", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "grok_tokens", tokens: TOKENS } });
    await home(page);
    await waitState(page, (x) => x.grok?.configured === true);
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("ai-logout")).toHaveText("Sign out");
    await page.getByTestId("ai-logout").click();
    await waitState(page, (x) => x.grok?.configured === false);
    const raw = await page.evaluate(() => localStorage.getItem("connect2-grok"));
    expect(raw).toBeNull();
    await expect(page.getByRole("button", { name: "Login" }).first()).toBeVisible();
  });

  test("session has no grok login in the top bar", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "grok_tokens", tokens: TOKENS } });
    await home(page);
    await waitState(page, (x) => x.grok?.configured === true);
    await page.getByTestId("box-card").first().click({ clickCount: 1 });
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 15_000 });
    await expect(page.locator("#login")).toHaveCount(0);
    await expect(page.locator("#model")).toHaveCount(0);
    await page.getByTestId("nav-more").click();
    await expect(page.getByTestId("ai-logout")).toHaveCount(0);
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("ai-logout")).toHaveText("Sign out");
  });
});
