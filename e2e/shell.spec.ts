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

async function waitPty(page: Page, ms = 20_000) {
  const t0 = Date.now();
  let stdout = "";
  while (Date.now() - t0 < ms) {
    const s = await page.request.get("/state").then((r) => r.json());
    stdout = s.stdout || "";
    if (stdout.includes("$") || stdout.includes("packer") || stdout.length > 8) return stdout;
    await page.waitForTimeout(200);
  }
  throw new Error(`empty PTY: ${JSON.stringify(stdout)}`);
}

async function session(page: Page) {
  const s = await page.request.get("/state").then((r) => r.json());
  if (s.dst && (s.stdout || "").length > 0) {
    await page.goto("/");
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    await page.getByTestId("nav-shell").click();
    await waitPty(page);
    return;
  }
  await page.goto("/");
  await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 20_000 });
  await page.getByTestId("box-card").first().click({ clickCount: 1 });
  await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
  await waitPty(page);
}

test.describe.serial("shell", () => {
  test("term pane is visible with no custom xterm theme", async ({ page }) => {
    await session(page);
    await expect(page.locator("#term")).not.toHaveClass(/off/);
    await expect(page.locator("#term")).toBeVisible();
  });

  test("clicking a box shows Connecting until the hop is up", async ({ page }) => {
    await home(page);
    await page.getByTestId("box-card").first().click();
    await expect(page.getByTestId("box-loading")).toBeVisible({ timeout: 5_000 });
    await expect(page.getByTestId("box-loading")).toContainText("Connecting");
    await expect(page.getByTestId("box-loading")).toHaveCount(0, { timeout: 30_000 });
    const stdout = await waitPty(page);
    expect(stdout.length).toBeGreaterThan(0);
  });

  test("clicking a box opens a live PTY", async ({ page }) => {
    await session(page);
    await page.getByTestId("nav-boxes").click();
    await expect(page.getByTestId("box-card").first()).toBeVisible();
    await page.getByTestId("box-card").first().click();
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("#term")).not.toHaveClass(/off/);
    const stdout = await waitPty(page);
    expect(stdout.length).toBeGreaterThan(0);
    await page.locator("#term").click();
    await page.keyboard.type("echo SHELL-OK\n");
    const t1 = Date.now();
    while (Date.now() - t1 < 10_000) {
      const s = await page.request.get("/state").then((r) => r.json());
      if ((s.stdout || "").includes("SHELL-OK")) return;
      await page.waitForTimeout(200);
    }
    const last = await page.request.get("/state").then((r) => r.json());
    expect(last.stdout || "").toContain("SHELL-OK");
  });
});
