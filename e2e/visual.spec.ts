// HD-05 visual regression: every route in the dark and light themes, compared pixel for pixel with the baselines in
// e2e/__screenshots__. Fonts and rasterising differ between hosts, so this only runs against the browser in the
// pinned container (`pnpm test:e2e --visual`); anywhere else it is skipped, never failed.
import { expect, test } from "./fixtures";
import { ROUTES } from "./routes";

test.skip(!process.env.PW_REMOTE, "screenshots run only in the pinned container: pnpm test:e2e --visual");

// What changes from run to run: the index's age and build time, the fixture repos' folders on this machine, the
// per-worker home the agents page reads, and each index run's time and duration.
// .sh-bell: the server records activity on its own tick, so the unread count depends on timing.
const VOLATILE = [".sh-is", ".sh-bell", ".fr-list span", ".set-ed-facts dd", ".set-note > .set-mono", ".set-idx-state .set-note", ".set-conn-name small", ".set-idx-runs li:not([aria-hidden]) > span:nth-child(-n+2)"];
/** The timeline's window ends at today: pinned, so the strip is the same every day. After the fixtures' last commit. */
const TODAY = new Date("2026-11-20T12:00:00Z");
const THEMES = ["dark", "light"] as const;

for (const theme of THEMES) {
  test.describe(theme, () => {
    test.beforeEach(async ({ context, page }) => {
      await context.addInitScript((t) => localStorage.setItem("cartograph:appearance", JSON.stringify({ theme: t, motion: "reduce" })), theme);
      await page.clock.setFixedTime(TODAY);
    });
    for (const r of [...ROUTES, { path: "/", name: "map-table", ready: ROUTES[0].ready }]) {
      test(r.name, async ({ page, go }) => {
        await go(r.path);
        await expect(r.ready(page)).toBeVisible();
        if (r.name === "map-table") await page.getByRole("button", { name: "Table", exact: true }).click();
        await page.evaluate(() => document.fonts.ready);
        // Exact, the map included: it is SVG, drawn the same way every time in the pinned image. HD-05's 0.5% canvas
        // tolerance is for the WebGL renderer (FE-08); at 0.5% this sparse map could lose a whole folder unnoticed.
        await expect(page).toHaveScreenshot(`${r.name}-${theme}.png`, { mask: VOLATILE.map((s) => page.locator(s)) });
      });
    }
    // FE-08: the WebGL map at service level, on SwiftShader in the pinned image. The canvas gets HD-05's 0.5%.
    test("map-webgl", async ({ page, go }) => {
      await go("/#renderer=webgl");
      await expect(page.getByText("Service level")).toBeVisible();
      await expect(page.locator("#lm-at-note")).toContainText("5 services");
      await expect(page.locator(".lm-gl-stage canvas").first()).toBeAttached(); // toHaveScreenshot then waits for two equal frames
      await page.evaluate(() => document.fonts.ready);
      await expect(page).toHaveScreenshot(`map-webgl-${theme}.png`, { mask: VOLATILE.map((s) => page.locator(s)), maxDiffPixelRatio: 0.005 });
    });
  });
}
