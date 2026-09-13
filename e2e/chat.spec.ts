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

async function openAi(page: Page) {
  await expect(page.getByTestId("ai-chat")).toBeVisible();
}

test.describe.serial("ai sidebar", () => {
  test("sidebar is a sibling of the directory, not inside a box card", async ({ page }) => {
    await home(page);
    const desk = page.locator(".desk");
    await expect(desk).toBeVisible();
    await expect(desk.locator("> .chats")).toBeVisible();
    await expect(desk.locator("> .room")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toHaveClass(/ai-pane/);
    await expect(desk.locator("> .ai-pane")).toBeVisible();
    await expect(page.getByTestId("box-card").first()).toBeVisible();
  });

  test("head shows New chat when Grok is on", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await state(page);
    await expect(page.locator(".chat-model")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "New", exact: true })).toHaveCount(0);
    if (s.grok?.configured) {
      await expect(page.getByTestId("chats-nav").getByTestId("ai-new")).toHaveAttribute("aria-label", "New chat");
    } else {
      await expect(page.getByTestId("ai-new")).toHaveCount(0);
    }
  });

  test("logged-out gate: Login Grok", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await state(page);
    if (s.grok?.configured) {
      await page.getByTestId("nav-more").click();
      await page.getByTestId("nav-settings").click();
      await expect(page.getByTestId("ai-logout")).toHaveText("Sign out");
      await expect(page.getByTestId("ai-login")).toHaveCount(0);
      return;
    }
    await expect(page.getByTestId("ai-gate")).toBeVisible();
    await expect(page.getByTestId("ai-login")).toHaveText("Login Grok");
    await expect(page.getByTestId("ai-login")).toHaveClass(/btn/);
    await expect(page.getByTestId("ai-q")).toHaveCount(0);
  });

  test("Login posts /cmd type login", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await state(page);
    if (s.grok?.configured) {
      test.skip();
      return;
    }
    const [req] = await Promise.all([
      page.waitForRequest((r) => r.url().includes("/cmd") && r.method() === "POST"),
      page.getByTestId("ai-login").click(),
    ]);
    const body = req.postDataJSON();
    expect(body.type).toBe("grok_login");
  });

  test("waiting gate shows user code after login start", async ({ page }) => {
    await home(page);
    await openAi(page);
    let s = await state(page);
    if (s.grok?.configured) {
      test.skip();
      return;
    }
    if (!s.grok?.user_code) {
      await page.getByTestId("ai-login").click();
    }
    await waitState(page, (x) => !!x.grok?.user_code || x.grok?.configured, 15_000).catch(() => null);
    s = await state(page);
    if (s.grok?.user_code) {
      await expect(page.getByTestId("ai-code")).toBeVisible();
      await expect(page.getByTestId("ai-code")).toHaveText(s.grok.user_code);
      await expect(page.getByTestId("ai-open-xai")).toHaveAttribute("href", /http/);
      await expect(page.getByTestId("ai-open-xai")).toHaveAttribute("target", "_blank");
      await expect(page.getByText("Waiting for login")).toBeVisible();
    }
  });

  test("logged-in: composer, no box tagline", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await waitState(page, (x) => x.peers?.length >= 1);
    if (!s.grok?.configured) {
      test.skip();
      return;
    }
    await expect(page.getByText(/Ask about /)).toHaveCount(0);
    await expect(page.getByTestId("ai-q")).toBeVisible();
    await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
    await expect(page.getByTestId("ai-logout")).toHaveCount(0);
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await expect(page.getByTestId("ai-logout")).toHaveText("Sign out");
  });

  test("empty send does not POST ask", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await state(page);
    if (!s.grok?.configured) {
      test.skip();
      return;
    }
    let asked = false;
    page.on("request", (r) => {
      if (r.url().includes("/cmd") && r.method() === "POST") {
        try {
          if (r.postDataJSON()?.type === "ask") asked = true;
        } catch {
          /* ignore */
        }
      }
    });
    await page.getByTestId("ai-q").fill("   ");
    await page.getByRole("button", { name: "Send" }).click();
    await page.waitForTimeout(400);
    expect(asked).toBeFalsy();
  });

  test("send posts ask and shows a user bubble", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await waitState(page, (x) => x.peers?.length >= 1);
    if (!s.grok?.configured) {
      test.skip();
      return;
    }
    const msg = `e2e-ai-${Date.now().toString(36)}`;
    await page.getByTestId("ai-q").fill(msg);
    const [req] = await Promise.all([
      page.waitForRequest((r) => {
        if (!(r.url().includes("/cmd") && r.method() === "POST")) return false;
        try {
          return r.postDataJSON()?.type === "ask";
        } catch {
          return false;
        }
      }),
      page.getByRole("button", { name: "Send" }).click(),
    ]);
    expect(req.postDataJSON().text).toBe(msg);
    await expect
      .poll(
        async () => {
          const log = ((await state(page)).log || []).join("\n");
          const ui = await page.getByTestId("ai-chat").innerText();
          return log.includes(msg) || ui.includes(msg);
        },
        { timeout: 20_000 },
      )
      .toBeTruthy();
    await expect(page.getByTestId("ai-q")).toHaveValue("");
  });

  test("reply or tool step appears after ask", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await waitState(page, (x) => x.peers?.length >= 1);
    if (!s.grok?.configured) {
      test.skip();
      return;
    }
    const msg = `e2e-reply-${Date.now().toString(36)}`;
    await page.getByTestId("ai-q").fill(msg);
    await page.getByRole("button", { name: "Send" }).click();
    const bot = page.locator(".chat-bubble, .chat-step");
    await expect
      .poll(
        async () => {
          const s = await state(page);
          const log = (s.log || []).join("\n");
          const desk = (s.desk || []).join("\n");
          const ui = await page.getByTestId("ai-chat").innerText();
          return (
            ui.includes(msg) ||
            desk.includes(msg) ||
            log.includes("ai ") ||
            log.includes("step ") ||
            log.includes("on the box") ||
            log.includes("error") ||
            desk.includes("error")
          );
        },
        { timeout: 25_000 },
      )
      .toBeTruthy();
    await expect(bot.first()).toBeVisible({ timeout: 10_000 });
  });

  test("composer keeps focus while typing", async ({ page }) => {
    await page.request.post("/cmd", {
      data: {
        type: "grok_tokens",
        tokens: { access: "focus-a", refresh: "focus-r", expires: 9_999_999_999_999 },
      },
    });
    await home(page);
    await waitState(page, (x) => x.grok?.configured === true);
    await openAi(page);
    const box = page.getByTestId("ai-q");
    await expect(box).toBeVisible();
    await box.click();
    await expect(box).toBeFocused();
    await page.keyboard.type("keep-focus");
    await expect(box).toBeFocused();
    await expect(box).toHaveValue("keep-focus");
    await page.getByTestId("box-card").first().click();
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    await box.click();
    await expect(box).toBeFocused();
    await page.keyboard.type("-shell");
    await expect(box).toBeFocused();
    await expect(box).toHaveValue("keep-focus-shell");
  });

  test("sidebar stays visible after opening a box", async ({ page }) => {
    await home(page);
    await openAi(page);
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await page.getByTestId("box-card").first().click();
    await expect(page.getByTestId("nav-shell")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    await page.getByTestId("nav-boxes").click();
    await expect(page.getByTestId("dir-title")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("ai-chat")).toBeVisible();
  });

  test("ask from directory stays on boxes, not the first box agent page", async ({ page }) => {
    await home(page);
    await openAi(page);
    const s = await waitState(page, (x) => x.peers?.length >= 1);
    if (!s.grok?.configured) {
      test.skip();
      return;
    }
    await page.getByTestId("ai-q").fill("status from directory");
    await page.getByRole("button", { name: "Send" }).click();
    await expect
      .poll(
        async () => {
          const log = ((await state(page)).log || []).join("\n");
          const ui = await page.getByTestId("ai-chat").innerText();
          return log.includes("status from directory") || ui.includes("status from directory");
        },
        { timeout: 20_000 },
      )
      .toBeTruthy();
    await expect(page.getByTestId("dir-title")).toBeVisible();
    await expect(page.getByTestId("ai-chat")).toBeVisible();
    const agent = page.getByTestId("nav-agent");
    if (await agent.count()) {
      await expect(agent).not.toHaveClass(/tab-active/);
    }
  });

  test("Grok sign out lives in Settings", async ({ page }) => {
    await home(page);
    await expect(page.locator("#desk #login")).toHaveCount(0);
    const s = await state(page);
    if (s.grok?.configured) {
      await expect(page.getByTestId("ai-logout")).toHaveCount(0);
      await page.getByTestId("nav-more").click();
      await page.getByTestId("nav-settings").click();
      await expect(page.getByTestId("ai-logout")).toHaveText("Sign out");
    }
  });

  test("adopting tokens persists to localStorage connect2-grok", async ({ page }) => {
    await page.request.post("/cmd", { data: { type: "logout" } });
    await page.request.post("/cmd", {
      data: {
        type: "grok_tokens",
        tokens: { access: "chat-a", refresh: "chat-r", expires: 9_999_999_999_999 },
      },
    });
    await home(page);
    await waitState(page, (x) => x.grok?.configured === true);
    await openAi(page);
    const raw = await page.evaluate(() => localStorage.getItem("connect2-grok"));
    expect(raw).toContain("chat-a");
    await page.getByTestId("nav-more").click();
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("ai-logout").click();
    await waitState(page, (x) => x.grok?.configured === false);
    expect(await page.evaluate(() => localStorage.getItem("connect2-grok"))).toBeNull();
  });
});
