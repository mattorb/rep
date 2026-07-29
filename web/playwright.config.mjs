import { defineConfig } from "@playwright/test";

const browserName = process.env.REP_BROWSER || "chromium";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30_000,
  expect: { timeout: 8_000 },
  fullyParallel: true,
  workers: process.env.CI || browserName !== "chromium" ? 1 : undefined,
  reporter: process.env.CI ? [["line"], ["html", { open: "never" }]] : "line",
  use: {
    browserName,
    headless: true,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
});
