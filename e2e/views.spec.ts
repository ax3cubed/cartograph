// FE-11 and SRV-12 end to end on the real binary: every view renders from the golden index (which has no routes, specs,
// concepts or rules, so each says what will appear), from a workspace whose repository has nothing to find, and from one
// that has routes, a spec, a client, a rule and 120 more entrypoints than a page holds. Each runs in both themes with axe.
import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, open, serve, test, type Served } from "./fixtures";
import { WCAG } from "./routes";

const THEMES = ["dark", "light"] as const;
const scan = async (page: Page) =>
  (await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
/**
 * Known, not ours (routes.spec.ts says the same): Firefox's content process sometimes crashes while axe's script is
 * evaluated. A crashed scan is run once more on a fresh page that `reopen` brings to the same view.
 */
const violations = async (page: Page, reopen: (p: Page) => Promise<void>) => {
  try { return await scan(page); } catch (e) {
    if (!/Page crashed/.test(String(e))) throw e;
    test.info().annotations.push({ type: "retried", description: "the browser crashed during the axe scan" });
    const fresh = await page.context().newPage();
    await reopen(fresh);
    return scan(fresh);
  }
};
const h1 = (page: Page, name: RegExp) => page.getByRole("heading", { level: 1, name });
const api = async (s: Served, path: string) => {
  const r = await fetch(`${s.origin}/api/v1/${path}`, { headers: { Authorization: `Bearer ${s.token}` } });
  return r.headers.get("content-type")?.startsWith("text/plain") ? r.text() : (await r.json()).data;
};

// ---------- the golden index: nothing to find, so each view says what will appear ----------

for (const theme of THEMES) {
  test.describe(`golden index, ${theme}`, () => {
    test.beforeEach(async ({ context }) => {
      await context.addInitScript((t) => localStorage.setItem("cartograph:appearance", JSON.stringify({ theme: t, motion: "reduce" })), theme);
    });
    const empties: [string, RegExp, string, string, string][] = [
      ["/entrypoints", /^Entrypoints/, "No adapters found yet", "Open repositories", "/settings/repositories"],
      ["/contracts", /^Contracts/, "No specs found yet", "Open contract settings", "/settings/contracts"],
      ["/concepts", /^Concepts/, "Concepts need the search index", "Open retrieval settings", "/settings/retrieval"],
      ["/rules", /^Rules/, "No rules yet", "Open services settings", "/settings/services"],
    ];
    for (const [path, title, state, action, href] of empties) {
      test(`${path} says what will appear, why not yet, and one thing to do`, async ({ page, go }) => {
        await go(path);
        await expect(h1(page, title)).toBeVisible();
        const box = page.getByRole("status").filter({ has: page.getByRole("heading", { level: 2, name: state }) });
        await expect(box.locator("p")).not.toBeEmpty();
        await expect(box.getByRole("link")).toHaveCount(1);
        await expect(box.getByRole("link", { name: action })).toHaveAttribute("href", href);
        expect(await violations(page, (p) => go(path, p))).toEqual([]);
      });
    }

    test("/coverage reads the summary the indexer wrote", async ({ page, go }) => {
      await go("/coverage");
      await expect(h1(page, /^Coverage/)).toBeVisible();
      const files = page.getByRole("listitem").filter({ hasText: "Files parsed" });
      await expect(files).toContainText("100%");
      await expect(files).toContainText("32 of 32");
      // Nothing to count is not 100%.
      await expect(page.getByRole("listitem").filter({ hasText: "Operations with a handler" })).toContainText("Nothing to count yet");
      await expect(page.getByText("Nothing is unresolved.").first()).toBeVisible();
      expect(await violations(page, (p) => go("/coverage", p))).toEqual([]);
    });

    test("/impact copies the server's Markdown, the text cartograph impact prints", async ({ page, go, context, browserName, served }) => {
      const list = await api(served, "w/northwind/commits?limit=1");
      const sha: string = list[0].sha;
      if (browserName === "chromium") await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: served.origin });
      await go(`/impact#sha=${sha}`);
      await expect(h1(page, /^Impact/)).toBeVisible();
      await page.getByRole("button", { name: "Copy as Markdown" }).click();
      const md: string = await api(served, `w/northwind/commits/${sha}/impact/markdown`);
      expect(md).toMatch(/affected in \d+ services/);
      if (browserName === "chromium") {
        await expect(page.getByText("Copied as Markdown.")).toBeVisible();
        expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(md);
      } else {
        // Where the clipboard is refused, the same text is selected for the reader to copy.
        await expect(page.getByRole("textbox", { name: "Impact as Markdown" }).or(page.getByText("Copied as Markdown."))).toBeVisible();
      }
      expect(await violations(page, (p) => go(`/impact#sha=${sha}`, p))).toEqual([]);
    });
  });
}

test("the first index still running says so on every view", async ({ page, go }) => {
  for (const [path, title] of [["/entrypoints", /^Entrypoints/], ["/contracts", /^Contracts/], ["/concepts", /^Concepts/], ["/coverage", /^Coverage/], ["/rules", /^Rules/]] as const) {
    await go(path);
    await expect(h1(page, title)).toBeVisible();
    // The review state of decision I9: the hash says the first index is still running.
    await page.evaluate(() => { location.hash = "indexing"; });
    await expect(page.getByRole("heading", { level: 2, name: "Building your first index" })).toBeVisible();
    await expect(page.getByRole("link", { name: "See indexing settings" })).toBeVisible();
  }
});

// ---------- two workspaces made from repositories on disk ----------

const COMPOSE = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n  bulk:\n    build: ./bulk\n";
const SERVER = 'import express from "express";\nconst app = express();\n\napp.get("/orders/:id", (req, res) => res.json({}));\napp.get("/health", (req, res) => res.send("ok"));\n';
const CLIENT = 'export function load(id: string) {\n  return fetch("http://orders:8080/orders/" + id);\n}\n';
const BULK = `import express from "express";\nconst app = express();\n${Array.from({ length: 120 }, (_, i) => `app.get("/bulk/r${i}", (req, res) => res.send("${i}"));`).join("\n")}\n`;
// `summary` is spec text: the page must show it as text.
const SPEC_V1 = "openapi: 3.0.3\ninfo: {title: Orders API}\npaths:\n  /orders/{id}:\n    get:\n      operationId: getOrder\n      summary: One order\n    delete:\n      operationId: deleteOrder\n      summary: \"<img src=x onerror=window.__pwned=1>\"\n  /orders:\n    post:\n      operationId: createOrder\n";
// v2 drops the operation the web client calls.
const SPEC_V2 = SPEC_V1.replace("    get:\n      operationId: getOrder\n      summary: One order\n", "");
const RULES = "\nrules:\n  - name: web-not-orders\n    type: forbid\n    from: {path: \"^web/\"}\n    to: {path: \"^orders/\"}\n    link: [http]\n    reason: \"web goes through the gateway, <b>never</b> direct\"\n";

/** Writes each step's files into `~/<name>` and commits it, registers the folder as a workspace and waits for its index. */
async function workspace(s: Served, name: string, steps: Record<string, string>[], afterManifest = ""): Promise<string> {
  const repo = join(s.home, name);
  const git = (...a: string[]) => execFileSync("git", ["-C", repo, ...a], { env: { ...process.env, HOME: s.home, GIT_AUTHOR_NAME: "e2e", GIT_AUTHOR_EMAIL: "e2e@localhost", GIT_COMMITTER_NAME: "e2e", GIT_COMMITTER_EMAIL: "e2e@localhost" } });
  mkdirSync(repo, { recursive: true });
  git("init", "-q", "-b", "main");
  steps.forEach((files, i) => {
    for (const [f, text] of Object.entries(files)) { mkdirSync(dirname(join(repo, f)), { recursive: true }); writeFileSync(join(repo, f), text); }
    git("add", "."); git("commit", "-q", "-m", `step ${i + 1}`);
  });
  const res = await fetch(`${s.origin}/api/v1/workspaces`, {
    method: "POST", body: JSON.stringify({ path: `~/${name}`, manifest: true }),
    headers: { Authorization: `Bearer ${s.token}`, "Content-Type": "application/json", "X-Cartograph": "1", Origin: s.origin },
  });
  const made = (await res.json()).data;
  expect(res.status, JSON.stringify(made)).toBe(200);
  if (afterManifest) appendFileSync(join(repo, "cartograph.yaml"), afterManifest);
  await expect.poll(async () => (await api(s, `jobs/${made.job_id}`)).state, { timeout: 60_000 }).toBe("done");
  return made.workspace.id;
}

test.describe("a repository with nothing to find", () => {
  test.describe.configure({ mode: "serial" });
  let s: Awaited<ReturnType<typeof serve>>;
  let ws = "";
  test.beforeAll(async () => { s = await serve(false); ws = await workspace(s, "plain", [{ "notes.txt": "nothing here\n", "package.json": '{"name":"plain"}' }]); });
  test.afterAll(async () => { await s.stop(); });

  for (const theme of THEMES) {
    test(`every view renders from an empty index, ${theme}`, async ({ page, context }) => {
      await context.addInitScript((t) => localStorage.setItem("cartograph:appearance", JSON.stringify({ theme: t, motion: "reduce" })), theme);
      const views: [string, RegExp, string][] = [
        ["/entrypoints", /^Entrypoints/, "No adapters found yet"], ["/contracts", /^Contracts/, "No specs found yet"], ["/concepts", /^Concepts/, "Concepts need the search index"],
        ["/rules", /^Rules/, "No rules yet"], ["/coverage", /^Coverage/, ""], ["/impact", /^Impact/, ""],
      ];
      for (const [path, title, state] of views) {
        await open(page, s, `${path}#ws=${ws}`);
        await expect(h1(page, title)).toBeVisible();
        if (state) await expect(page.getByRole("heading", { level: 2, name: state })).toBeVisible();
        expect(await violations(page, (p) => open(p, s, `${path}#ws=${ws}`)), path).toEqual([]);
      }
    });
  }
});

test.describe("a repository with routes, a spec, a client and a rule", () => {
  test.describe.configure({ mode: "serial" });
  let s: Awaited<ReturnType<typeof serve>>;
  let ws = "";
  test.beforeAll(async () => {
    s = await serve(false);
    ws = await workspace(s, "shop", [
      { "docker-compose.yml": COMPOSE, "orders/package.json": '{"name":"orders"}', "web/package.json": '{"name":"web"}', "bulk/package.json": '{"name":"bulk"}', "orders/src/server.ts": SERVER, "bulk/src/routes.ts": BULK },
      { "web/src/api.ts": CLIENT, "orders/openapi.yaml": SPEC_V1 },
      { "orders/openapi.yaml": SPEC_V2 },
    ], RULES);
    // The writer's own sync of the manifest is done once the job is: the rule is read from the file on each request.
    expect((await api(s, `w/${ws}/entrypoints/counts`)).total).toBe(122);
  });
  test.afterAll(async () => { await s.stop(); });

  for (const theme of THEMES) {
    test.describe(theme, () => {
      test.beforeEach(async ({ context }) => {
        await context.addInitScript((t) => localStorage.setItem("cartograph:appearance", JSON.stringify({ theme: t, motion: "reduce" })), theme);
      });

      test("entrypoints are paged, filtered and sorted by the server and drawn virtualised", async ({ page }) => {
        await open(page, s, `/entrypoints#ws=${ws}`);
        await expect(h1(page, /^Entrypoints/)).toBeVisible();
        const grid = page.getByRole("grid", { name: "Entrypoints" });
        await expect(grid.getByRole("row", { name: /bulk\/r0/ })).toBeVisible();
        // 50 of 122 are loaded (the header and a row for what is left make 52), and far fewer than that are in the page.
        await expect(grid).toHaveAttribute("aria-rowcount", "52");
        expect(await grid.getByRole("row").count()).toBeLessThan(40);
        // Scrolling to the end asks for the next page, with the cursor, until the last row is in.
        await expect(async () => {
          await grid.evaluate((el) => { el.scrollTop = el.scrollHeight; });
          await expect(grid).toHaveAttribute("aria-rowcount", "123", { timeout: 1_000 });
        }).toPass({ timeout: 20_000 });
        await expect(async () => {
          await grid.evaluate((el) => { el.scrollTop = el.scrollHeight; });
          await expect(grid.getByRole("row", { name: /health/ })).toBeVisible({ timeout: 1_000 });
        }).toPass({ timeout: 10_000 });
        expect(await grid.getByRole("row").count()).toBeLessThan(40);

        // The server filters: HTTP is all of them here; text narrows to one.
        await page.getByRole("searchbox", { name: "Filter" }).fill("health");
        await expect(grid).toHaveAttribute("aria-rowcount", "2");
        await expect(grid.getByRole("row", { name: /GET\s+\/health/ })).toBeVisible();
        // The detail comes from …/entrypoints/{id}: no consumer for /health.
        await expect(page.getByRole("complementary", { name: "Selected entrypoint" })).toContainText("No consumers found");
        await page.getByRole("searchbox", { name: "Filter" }).fill("orders");
        await expect(grid.getByRole("row", { name: /orders/ })).toHaveCount(1);
        await grid.getByRole("row", { name: /orders/ }).click();
        const aside = page.getByRole("complementary", { name: "Selected entrypoint" });
        await expect(aside).toContainText("Consumers · 1 known");
        await expect(aside.getByRole("link", { name: "api.ts" })).toBeVisible();
        // The scan is last: a browser that crashes in it leaves nothing after it to run on a dead page.
        expect(await violations(page, (p) => open(p, s, `/entrypoints#ws=${ws}`))).toEqual([]);
      });

      test("sorting by consumers is the server's order", async ({ page }) => {
        await open(page, s, `/entrypoints#ws=${ws}`);
        await page.getByRole("searchbox", { name: "Filter" }).fill("/");
        const grid = page.getByRole("grid", { name: "Entrypoints" });
        await expect(grid.getByRole("row")).not.toHaveCount(0);
        await page.getByRole("button", { name: /^Consumers/ }).click();
        await expect(page.getByRole("columnheader", { name: /^Consumers/ })).toHaveAttribute("aria-sort", "ascending");
        await page.getByRole("button", { name: /^Consumers/ }).click();
        await expect(page.getByRole("columnheader", { name: /^Consumers/ })).toHaveAttribute("aria-sort", "descending");
        await expect(grid.getByRole("row").nth(1)).toContainText("orders");
      });

      test("contracts show spec text as text, the drift and the version that broke callers", async ({ page }) => {
        await open(page, s, `/contracts#ws=${ws}`);
        await expect(h1(page, /^Contracts/)).toBeVisible();
        const grid = page.getByRole("grid", { name: "Operations" });
        // The worst come first: every route no spec lists (the 120 bulk ones included) is undocumented, and the
        // spec-only DELETE and POST are at the end, which is a scroll away in a virtualised table.
        await expect(grid.getByRole("row", { name: /bulk\/r0/ })).toContainText("Undocumented");
        await expect(grid).toHaveAttribute("aria-rowcount", "125");
        await grid.evaluate((el) => { el.scrollTop = el.scrollHeight; });
        await expect(grid.getByRole("row", { name: /DELETE/ })).toContainText("Spec-only");
        await grid.getByRole("row", { name: /DELETE/ }).click();
        const aside = page.getByRole("complementary", { name: "Selected operation" });
        // The summary is markup-shaped spec text: it is on the page as characters, and nothing ran.
        await expect(aside.getByText("<img src=x onerror=window.__pwned=1>")).toBeVisible();
        expect(await page.locator("img[src='x']").count()).toBe(0);
        expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined();
        await expect(aside.getByRole("heading", { name: /^Versions of/ })).toBeVisible();
        await expect(aside).toContainText("1 breaking change");
        await expect(aside).toContainText("1 caller still reads it");
        expect(await violations(page, (p) => open(p, s, `/contracts#ws=${ws}`))).toEqual([]);
      });

      test("coverage drills into the references behind a gap, from the summary table", async ({ page }) => {
        await open(page, s, `/coverage#ws=${ws}`);
        await expect(h1(page, /^Coverage/)).toBeVisible();
        const gaps = page.getByRole("grid", { name: "Gaps" });
        await expect(gaps.getByRole("row", { name: /operation/i }).first()).toBeVisible();
        await gaps.getByRole("row", { name: /operation/i }).first().click();
        const refs = page.getByRole("grid", { name: "Unresolved references" });
        await expect(refs.getByRole("row").nth(1)).toBeVisible();
        await expect(refs).toContainText("/orders");
        expect(await violations(page, (p) => open(p, s, `/coverage#ws=${ws}`))).toEqual([]);
      });

      test("rules say pass or fail per commit, and a rule's reason is text", async ({ page }) => {
        await open(page, s, `/rules#ws=${ws}`);
        await expect(h1(page, /^Rules/)).toBeVisible();
        const rules = page.getByRole("grid", { name: "Rules" });
        await expect(rules.getByRole("row", { name: /web-not-orders/ })).toContainText("Fail");
        const aside = page.getByRole("complementary", { name: "Selected rule" });
        await expect(aside.getByText("<b>never</b>", { exact: false })).toBeVisible();
        expect(await aside.locator("b").count()).toBe(0);
        const violationsGrid = page.getByRole("grid", { name: "Violations" });
        await expect(violationsGrid.getByRole("row").nth(1)).toContainText("web/src/api.ts");
        // Newest first: the last commit fails, the first one (before the client existed) passes.
        const commits = page.getByRole("grid", { name: "Commits" });
        await expect(commits.getByRole("row").nth(1)).toContainText("Fail");
        await expect(commits.getByRole("row").nth(3)).toContainText("Pass");
        expect(await violations(page, (p) => open(p, s, `/rules#ws=${ws}`))).toEqual([]);
      });
    });
  }
});
