import { expect, test, type Page } from "@playwright/test";

async function state(page: Page) {
  return page.request.get("/state").then((r) => r.json());
}

async function waitState(page: Page, pred: (s: any) => boolean, ms = 20_000) {
  const t0 = Date.now();
  let last: any;
  while (Date.now() - t0 < ms) {
    last = await state(page);
    if (pred(last)) return last;
    await page.waitForTimeout(200);
  }
  throw new Error(`state wait failed: ${JSON.stringify(last)}`);
}

test.describe.serial("teleport cluster", () => {
  test("hub is signed in with three tunneled boxes", async ({ page }) => {
    const s = await waitState(page, (x) => x.authed && (x.peers?.length ?? 0) >= 3);
    expect(s.authed).toBeTruthy();
    expect(s.me).toBe("demo");
    const names = (s.peers as { name: string; tunnel?: boolean }[]).map((p) => p.name).sort();
    expect(names).toEqual(["box-1", "box-2", "box-3"]);
    expect(s.peers.every((p: { tunnel?: boolean }) => p.tunnel !== false)).toBeTruthy();
  });

  test("directory lists Teleport hostnames, not UUIDs", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "home" } });
    await waitState(page, (s) => !s.dst);
    await page.goto("/");
    await expect(page.getByTestId("dir-title")).toBeVisible();
    const cards = page.getByTestId("box-card");
    await expect(cards).toHaveCount(3);
    const labels = (await cards.allTextContents()).map((t) => t.trim()).sort();
    expect(labels.join(" ")).toContain("box-1");
    expect(labels.join(" ")).toContain("box-2");
    expect(labels.join(" ")).toContain("box-3");
    expect(labels.join(" ")).not.toMatch(
      /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i,
    );
  });

  test("open box-2 then box-3 over reverse tunnel", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "home" } });
    await waitState(page, (s) => !s.dst);
    await page.request.post("/cmd", {
      data: { type: "open", dst: "box-2", kind: "shell" },
    });
    await waitState(page, (s) => s.dst === "box-2" && s.kind === "shell");
    await page.request.post("/cmd", { data: { type: "home" } });
    await waitState(page, (s) => !s.dst);
    await page.request.post("/cmd", {
      data: { type: "open", dst: "box-3", kind: "shell" },
    });
    await waitState(page, (s) => s.dst === "box-3");
    await page.request.post("/cmd", { data: { type: "home" } });
  });

  test("sign-out control is a Teleport cluster logout", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "home" } });
    await page.goto("/");
    await page.getByTestId("nav-more").click();
    await expect(page.getByTestId("plane-logout")).toBeVisible();
    await expect(page.getByTestId("plane-logout")).toHaveText("Sign out");
    await expect(page.getByTestId("cluster-gate")).toHaveCount(0);
  });
});
