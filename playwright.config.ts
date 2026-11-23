// HD-05: end-to-end and visual regression against the real binary. `pnpm test:e2e` builds what this needs and runs
// it; e2e/README.md says how the visual baselines are made and updated.
import { defineConfig, devices } from "@playwright/test";

const viewport = { width: 1440, height: 900 };
/** The browser server in the pinned container (scripts/e2e.mjs --visual). Screenshots only run against it. */
const remote = process.env.PW_REMOTE;

export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/global-setup.ts",
  fullyParallel: true,
  timeout: 60_000,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  // One environment makes the baselines, so the path carries no platform.
  snapshotPathTemplate: "e2e/__screenshots__/{arg}{ext}",
  expect: { timeout: 10_000, toHaveScreenshot: { maxDiffPixels: 0, animations: "disabled", caret: "hide" } },
  use: { viewport, reducedMotion: "reduce", trace: "retain-on-failure" },
  projects: [
    { name: "chromium", testIgnore: /visual/, use: { ...devices["Desktop Chrome"], viewport } },
    { name: "firefox", testIgnore: /visual/, use: { ...devices["Desktop Firefox"], viewport } },
    { name: "webkit", testIgnore: /visual/, use: { ...devices["Desktop Safari"], viewport } },
    {
      name: "visual",
      testMatch: /visual/,
      // an amd64 image on an arm laptop runs emulated, and slowly
      timeout: 120_000,
      expect: { timeout: 45_000 },
      use: { ...devices["Desktop Chrome"], viewport, connectOptions: remote ? { wsEndpoint: remote, exposeNetwork: "<loopback>" } : undefined },
    },
  ],
});
