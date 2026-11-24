// FE-08 to FE-10 on the real binary: scrubbing the timeline, compare, search, the inspectors and the WebGL levels.
// The northwind fixture has 22 commits on one repo; the history fixture moves src/util.ts to src/lib/util.ts.
import { expect, test } from "./fixtures";

const note = (p: import("@playwright/test").Page) => p.locator("#lm-at-note");

test("scrub: the knob steps one commit at a time and the map follows it", async ({ page, go }) => {
  await go("/");
  await expect(page.locator("svg.lm-svg")).toBeVisible();
  await expect(note(page)).toContainText("at commit");
  const latest = (await note(page).textContent())!.match(/at commit (\w+)/)![1];
  const knob = page.getByRole("slider", { name: "Date" });
  await knob.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(knob).toHaveAttribute("aria-valuetext", /, commit \w{7}: /);
  const one = (await knob.getAttribute("aria-valuetext"))!.match(/commit (\w{7})/)![1];
  await expect(note(page)).toContainText(`at commit ${one}`);
  expect(one).not.toBe(latest);
  await page.keyboard.press("ArrowLeft");
  await expect(knob).not.toHaveAttribute("aria-valuetext", new RegExp(one));
  const two = (await knob.getAttribute("aria-valuetext"))!.match(/commit (\w{7})/)![1];
  await expect(note(page)).toContainText(`at commit ${two}`);
  // the date field follows the knob, and the layout holds still: a file keeps its place across commits
  await expect(page.getByRole("textbox", { name: "At" })).not.toHaveValue("");
  const at = async () => page.locator("circle.lm-node", { has: page.locator("title", { hasText: /^shared-libs\/src\/money\/currency\.ts$/ }) }).evaluate((c) => `${c.getAttribute("cx")},${c.getAttribute("cy")}`);
  const before = await at();
  await page.keyboard.press("ArrowRight");
  await expect(note(page)).toContainText(`at commit ${one}`);
  expect(await at()).toBe(before);
  // End is now
  await page.keyboard.press("End");
  await expect(knob).toHaveAttribute("aria-valuetext", "Now");
  await expect(note(page)).toContainText(`at commit ${latest}`);
});

test("compare: two dates as an overlay, side by side, and the table's difference filter", async ({ page, go }) => {
  await go("/");
  await expect(page.locator("svg.lm-svg")).toBeVisible();
  await page.getByRole("button", { name: "Compare" }).click();
  const b = page.getByRole("slider", { name: "Date B, to compare with" });
  await b.focus();
  for (let i = 0; i < 8; i++) await page.keyboard.press("ArrowLeft");
  await expect(note(page)).toContainText("comparing");
  await expect(page.getByRole("note", { name: "Compare legend" })).toContainText("Added");
  await expect(page.locator("svg.lm-svg .lm-link.is-added").first()).toBeVisible();
  const added = await page.locator("svg.lm-svg .lm-link.is-added").count();

  await page.getByRole("button", { name: "Side by side" }).click();
  await expect(page.locator("svg.lm-svg")).toHaveCount(2);
  await page.getByRole("button", { name: "Overlay" }).click();

  await page.getByRole("button", { name: "Table", exact: true }).click();
  await page.getByRole("button", { name: /^Links/ }).click();
  await page.getByRole("combobox", { name: "Show" }).selectOption("added");
  const table = page.getByRole("table", { name: "Import links" });
  await expect(table).toHaveAttribute("aria-rowcount", String(added + 1));
  const changes = await table.locator("tbody tr[data-k] td:last-child").allTextContents();
  expect(changes.length).toBeGreaterThan(0);
  expect(changes.every((c) => c === "Added")).toBe(true);
});

test("search: results grouped by kind open the file on the map; no match offers Ask", async ({ page, go }) => {
  await go("/");
  await expect(page.locator("svg.lm-svg")).toBeVisible();
  await page.getByRole("button", { name: /^Search services/ }).click();
  const box = page.getByRole("combobox", { name: "Search" });
  await box.fill("charge");
  const hit = page.getByRole("option", { name: /charge\.ts/ });
  await expect(hit).toBeVisible();
  await expect(page.locator(".sh-sp-grp").first()).toHaveText(/Files/);
  await box.press("Enter");
  const panel = page.getByRole("complementary", { name: "Selected file" });
  await expect(panel.getByRole("heading", { level: 2 })).toHaveText("charge.ts");

  await page.getByRole("button", { name: /^Search services/ }).click();
  await expect(page.getByRole("option", { name: /charge\.ts/ })).toBeVisible(); // recent, read back by id
  await box.fill("zzqqzz");
  await expect(page.getByText("No match for “zzqqzz”")).toBeVisible();
  await expect(page.getByRole("button", { name: /^Ask “Where is zzqqzz\?”/ })).toBeVisible();
});

test("inspector: a link's evidence and the layer whose answer the map shows", async ({ page, go }) => {
  await go("/");
  await page.locator("circle.lm-node", { has: page.locator("title", { hasText: /charge\.ts$/ }) }).click();
  const panel = page.getByRole("complementary", { name: "Selected file" });
  await expect(panel.locator("dl")).toContainText("Uses");
  await panel.getByRole("button", { name: "Why this link: billing-service/src/payments/stripeAdapter.ts" }).click();
  const link = panel.getByRole("region", { name: "Link details" });
  await expect(link).toContainText("Exact · 1.00");
  await expect(link).toContainText("Parsed from the source");
  await expect(link.locator("code").first()).toBeVisible();
  await link.getByRole("button", { name: "Back" }).click();
  // trace: what charge.ts reaches, lit on the map, with a chip that clears it
  await panel.getByRole("button", { name: "Trace on the map" }).click();
  const chip = page.getByRole("button", { name: /^Tracing charge\.ts/ });
  await expect(chip).toBeVisible();
  await expect(page.locator("svg.lm-svg circle.lm-node.is-faint").first()).toBeVisible();
  await chip.click();
  await expect(page.locator("svg.lm-svg circle.lm-node.is-faint")).toHaveCount(0);
});

test.describe("history fixture", () => {
  test.use({ workspace: "history" });
  test("a moved file shows one lineage: created, then moved, with the commit to jump to", async ({ page, go }) => {
    await go("/");
    await page.locator("circle.lm-node", { has: page.locator("title", { hasText: /^src\/lib\/util\.ts$/ }) }).click();
    const panel = page.getByRole("complementary", { name: "Selected file" });
    await panel.locator("summary", { hasText: "Renames and rewrites" }).click();
    const hist = panel.getByRole("region", { name: "Renames and rewrites" });
    await expect(hist).toContainText("Moved to src/lib/util.ts from src/util.ts");
    await expect(hist).toContainText("Created as src/util.ts");
    await hist.getByRole("button", { name: /^Show the map at / }).first().click();
    await expect(note(page)).toContainText("at commit");
  });
});

test("WebGL map: opens at service level, and the table lists that level", async ({ page, go }) => {
  await go("/#renderer=webgl");
  await expect(page.getByText("Service level")).toBeVisible();
  // sigma loads lazily; a cold Firefox on CI can take longer than the default 10 s to start it.
  await expect(page.locator(".lm-gl-stage canvas").first()).toBeAttached({ timeout: 30_000 });
  await expect(note(page)).toContainText("5 services");
  await page.getByRole("button", { name: "Table", exact: true }).click();
  await expect(page.getByRole("table", { name: "Services" })).toHaveAttribute("aria-rowcount", "6");
});

test("impact: Previous and Next step the first-parent timeline", async ({ page, go }) => {
  await go("/impact");
  const pick = page.getByRole("combobox", { name: "Commit" });
  await expect(pick).toBeVisible();
  const latest = await pick.inputValue();
  await expect(page.getByRole("button", { name: "Next commit" })).toBeDisabled();
  await page.getByRole("button", { name: "Previous commit" }).click();
  await expect(pick).not.toHaveValue(latest);
  await page.getByRole("button", { name: "Next commit" }).click();
  await expect(pick).toHaveValue(latest);
});

test("opening the same index twice places every file in the same place", async ({ page, go }) => {
  const places = () => page.locator("svg.lm-svg circle.lm-node").evaluateAll((cs) => cs.map((c) => `${c.querySelector("title")?.textContent}@${c.getAttribute("cx")},${c.getAttribute("cy")}`).sort());
  await go("/");
  await expect(page.locator("svg.lm-svg circle.lm-node").first()).toBeVisible();
  const first = await places();
  await page.reload();
  await expect(page.locator("svg.lm-svg circle.lm-node").first()).toBeVisible();
  expect(await places()).toEqual(first);
});
