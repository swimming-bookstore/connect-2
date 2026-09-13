import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  timeout: 90_000,
  expect: { timeout: 30_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  testMatch: [
    "http.spec.ts",
    "ui.spec.ts",
    "daisyui.spec.ts",
    "layout.spec.ts",
    "chrome.spec.ts",
    "client.spec.ts",
    "chat.spec.ts",
    "jpeg-turn.spec.ts",
    "shell.spec.ts",
    "open.spec.ts",
    "grok.spec.ts",
    "teleport.spec.ts",
  ],
  use: {
    baseURL: process.env.DEMO_URL ?? "http://127.0.0.1:3056",
    headless: true,
    viewport: { width: 1280, height: 800 },
    launchOptions: {
      executablePath: process.env.PLAYWRIGHT_CHROMIUM || "/usr/bin/chromium",
      args: [
        "--autoplay-policy=no-user-gesture-required",
        "--use-fake-ui-for-media-stream",
        "--no-sandbox",
      ],
    },
  },
});
