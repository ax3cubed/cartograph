// HD-05 journeys that work on today's binary. Each runs in Chromium, Firefox and WebKit.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, open, serve, test } from "./fixtures";

// FE-12's done-when: "find what billing depends on, as of <a past date>" with no pointer. In the northwind fixture,
// charge.ts imported only the Stripe adapter until 17 June 2025, when it began publishing payment events.
test("table-only keyboard journey: what billing's charge depends on, as of 1 June 2025", async ({ page, go, browserName }) => {
  // WebKit on macOS moves Tab between text fields only, as Safari does by default; Option+Tab reaches everything.
  const TAB = browserName === "webkit" && process.platform === "darwin" ? "Alt+Tab" : "Tab";
  await go("/");
  await expect(page.locator("svg.lm-svg")).toBeVisible();
  const tabTo = async (what: string, is: () => Promise<boolean>) => {
    for (let i = 0; i < 40 && !(await is()); i++) await page.keyboard.press(TAB);
    expect(await is(), `Tab reaches ${what}`).toBe(true);
  };
  const date = page.getByRole("textbox", { name: "At" });
  await tabTo("the date", () => date.evaluate((e) => e === document.activeElement));
  await date.fill("2025-06-01");
  await expect(page.locator("#lm-at-note")).toContainText("at commit");
  const table = page.getByRole("button", { name: "Table", exact: true });
  await tabTo("Map | Table", () => table.evaluate((e) => e === document.activeElement));
  await page.keyboard.press("Enter");
  await expect(table).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press(`Shift+${TAB}`); await page.keyboard.press(`Shift+${TAB}`);
  await page.keyboard.type("billing");
  await expect(page.getByRole("table", { name: "Files" })).toHaveAttribute("aria-rowcount", "5"); // header + 4 files
  await tabTo("the first row", () => page.evaluate(() => document.activeElement?.tagName === "TR"));
  for (let i = 0; i < 10 && !(await page.evaluate(() => document.activeElement?.textContent?.includes("charge.ts"))); i++) await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  const panel = page.getByRole("complementary", { name: "Selected file" });
  await expect(panel).toBeFocused();
  await expect(panel.locator("section").first()).toContainText("Imports 1");
  await expect(panel.locator("section").first()).toContainText("billing-service/src/payments/stripeAdapter.ts");
  // Escape closes the details and gives focus back to the row.
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(page.locator("tr:focus")).toContainText("charge.ts");
});

test("keyboard walk on the map: siblings, down, up and linked files, each said aloud", async ({ page, go }) => {
  await go("/");
  await page.locator("svg.lm-svg").focus();
  const said = page.locator(".lm-canvas [aria-live=polite]");
  await expect(said).toHaveText(/^folder \S+, \d+ files?$/); // the first folder, by name
  // Arrows move between sibling folders, in name order, until billing-service.
  const until = async (key: string, want: RegExp) => {
    for (let i = 0; i < 30 && !want.test((await said.textContent()) ?? ""); i++) {
      const before = await said.textContent();
      await page.keyboard.press(key);
      await expect(said).not.toHaveText(before ?? "");
    }
    await expect(said).toHaveText(want);
  };
  await until("ArrowRight", /^folder billing-service, \d+ files$/);
  await page.keyboard.press("Enter"); // down a level
  await expect(said).toHaveText(/^file billing-service\/package\.json, 0 incoming, 0 outgoing$/);
  await until("ArrowDown", /^file billing-service\/src\/events\/publisher\.ts, \d+ incoming, \d+ outgoing$/);
  await page.keyboard.press("l"); // the files it imports come first
  await expect(said).toHaveText(/^linked 1 of \d+: file shared-libs\/src\/events\/bus\.ts, /);
  await page.keyboard.press("l");
  await expect(said).toHaveText(/^linked 2 of \d+: file shared-libs\/src\/events\/schema\.ts, /);
  await page.keyboard.press("Escape"); // up a level, to the folder the walk is in now
  await expect(said).toHaveText(/^folder shared-libs, \d+ files$/);
});

test("the table lists what the map draws, under the same filter and date", async ({ page, go }) => {
  await go("/");
  await expect(page.locator("svg.lm-svg")).toBeVisible();
  const note = page.locator("#lm-at-note");
  const all = await note.textContent();
  await page.getByRole("searchbox", { name: "Filter" }).fill("shared");
  await expect(note).not.toHaveText(all ?? "");
  const drawn = await page.locator("svg.lm-svg circle.lm-node title").allTextContents();
  const lines = await page.locator("svg.lm-svg line.lm-link").count();
  expect(drawn.length).toBeGreaterThan(0);
  expect(drawn.every((d) => d.includes("shared"))).toBe(true);
  await page.getByRole("button", { name: "Table", exact: true }).click();
  const rows = await page.getByRole("table", { name: "Files" }).locator("tbody tr[data-k] td:first-child").allTextContents();
  expect(rows.sort()).toEqual(drawn.sort());
  await page.getByRole("button", { name: /^Links/ }).click();
  await expect(page.getByRole("table", { name: "Import links" })).toHaveAttribute("aria-rowcount", String(lines + 1)); // and a header row
});

// First run to map: an empty data folder shows the welcome screen; adding a folder with one git repository builds the
// first index, and the map then shows its files. Then a commit made while the map is open reaches it.
test("first run: add a workspace, build the first index, see it on the map, sync while open", async ({ page }) => {
  const s = await serve(false);
  try {
    const repo = join(s.home, "shop");
    mkdirSync(join(repo, "src"), { recursive: true });
    writeFileSync(join(repo, "src/a.ts"), 'import { b } from "./b";\nexport const a = b;\n');
    writeFileSync(join(repo, "src/b.ts"), "export const b = 1;\n");
    const git = (...a: string[]) => execFileSync("git", ["-C", repo, ...a], { env: { ...process.env, HOME: s.home, GIT_AUTHOR_NAME: "e2e", GIT_AUTHOR_EMAIL: "e2e@localhost", GIT_COMMITTER_NAME: "e2e", GIT_COMMITTER_EMAIL: "e2e@localhost" } });
    git("init", "-q", "-b", "main"); git("add", "."); git("commit", "-q", "-m", "first");

    await open(page, s, "/");
    await expect(page).toHaveURL(/\/welcome$/);
    await page.getByRole("link", { name: "Add a workspace" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Pick a folder or workspace file" })).toBeVisible();
    await page.getByRole("textbox", { name: "Folder or workspace file" }).fill("~/shop");
    await page.getByRole("button", { name: "Find repositories" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "1 repository found in 1 folder" })).toBeVisible();
    await page.getByRole("button", { name: "Continue to services" }).click();
    await page.getByRole("button", { name: "Build the first index" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "shop is indexed" })).toBeVisible({ timeout: 30_000 });
    await page.getByRole("button", { name: "Open the map" }).click();
    await expect(page.locator("#lm-at-note")).toContainText("2 files · 1 import link");

    // Sync while open: a new commit is noticed, synced in the background, and the open map updates by itself.
    writeFileSync(join(repo, "src/c.ts"), 'import { b } from "./b";\nexport const c = b;\n');
    git("add", "."); git("commit", "-q", "-m", "second");
    await expect(page.locator("#lm-at-note")).toContainText("3 files · 2 import links", { timeout: 30_000 });
  } finally {
    await s.stop();
  }
});
