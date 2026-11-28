// XS-13 and FE-14, end to end on the real binary: a correction made in the dialog lands in cartograph.overrides.yaml,
// shows on the link once the index has applied it, survives a rename of the caller, and undo restores the file and the
// link. Each run builds its own repository, so the golden data folder is untouched.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { expect, open, serve, test } from "./fixtures";

const FILES: Record<string, string> = {
  "docker-compose.yml": "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n",
  "orders/package.json": '{"name":"orders"}',
  "web/package.json": '{"name":"web"}',
  "orders/src/server.ts": 'import express from "express";\nconst app = express();\n\napp.get("/orders/:id", (req, res) => res.json({}));\n',
  "web/src/api.ts": 'export function load(id: string) {\n  return fetch("http://orders:8080/orders/" + id);\n}\n',
};

/** The override file as the first save writes it: golden, so a change to the writer's format shows in review. */
const GOLDEN = `version: 1
corrections:
  - action: remove
    key: {kind: http, from: "shop:web/src/api.ts", to: "shop:orders/src/server.ts", op: "GET /orders/{}"}
    reason: "a log line, not a call: <b>text</b> — stays text"
`;

test("a correction made in the app reaches the file, survives a rename and undoes", async ({ page }) => {
  const s = await serve(false);
  try {
    const repo = join(s.home, "shop");
    for (const [f, text] of Object.entries(FILES)) {
      mkdirSync(dirname(join(repo, f)), { recursive: true });
      writeFileSync(join(repo, f), text);
    }
    const api = async (tail: string) => {
      const get = async (path: string) => (await (await fetch(`${s.origin}/api/v1/${path}`, { headers: { Authorization: `Bearer ${s.token}` } })).json()).data;
      const ws = (await get("workspaces")).find((w: { name: string }) => w.name === "shop").id;
      return get(`w/${ws}/${tail}`);
    };
    const git = (...a: string[]) => execFileSync("git", ["-C", repo, ...a], { env: { ...process.env, HOME: s.home, GIT_AUTHOR_NAME: "e2e", GIT_AUTHOR_EMAIL: "e2e@localhost", GIT_COMMITTER_NAME: "e2e", GIT_COMMITTER_EMAIL: "e2e@localhost" } });
    git("init", "-q", "-b", "main"); git("add", "."); git("commit", "-q", "-m", "both services");

    await open(page, s, "/welcome/add");
    await page.getByRole("textbox", { name: "Folder or workspace file" }).fill("~/shop");
    await page.getByRole("button", { name: "Find repositories" }).click();
    await page.getByRole("button", { name: "Continue to services" }).click();
    await page.getByRole("button", { name: "Build the first index" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "shop is indexed" })).toBeVisible({ timeout: 30_000 });
    await page.getByRole("button", { name: "Open the map" }).click();
    await expect(page.locator("#lm-at-note")).toContainText("link");

    // The link: select web/src/api.ts in the table, then Why on its one import of the handler's file.
    await page.getByRole("button", { name: "Dismiss hint" }).click();
    await page.getByRole("button", { name: "Table", exact: true }).click();
    await page.getByRole("searchbox").fill("api.ts");
    await page.getByRole("row", { name: /web\/src\/api\.ts/ }).click();
    const panel = page.getByRole("complementary", { name: "Selected file" });
    await panel.getByRole("button", { name: /^Why this link: orders\/src\/server\.ts/ }).click();
    await expect(panel.getByText("HTTP", { exact: true })).toBeVisible();
    await expect(panel.getByText("Overridden")).toHaveCount(0);

    // The dialog: a reason is required, and the preview is the server's diff of the file.
    await panel.getByRole("button", { name: "Correct this link" }).click();
    const dialog = page.getByRole("dialog", { name: "Correct this link" });
    await expect(dialog.getByRole("button", { name: "Save to the file" })).toBeDisabled();
    await dialog.getByRole("textbox", { name: "Reason" }).fill("a log line, not a call: <b>text</b> — stays text");
    await expect(dialog.locator(".cd-preview pre")).toContainText('+  - action: remove');
    await expect(dialog.locator(".cd-preview pre")).toContainText('reason: "a log line, not a call: <b>text</b>');
    expect(existsSync(join(repo, "cartograph.overrides.yaml")), "a preview writes nothing").toBe(false);
    await dialog.getByRole("button", { name: "Save to the file" }).click();

    // The toast is true: the file is there, as the golden says.
    await expect(page.getByText("Saved to cartograph.overrides.yaml.")).toBeVisible();
    expect(readFileSync(join(repo, "cartograph.overrides.yaml"), "utf8")).toBe(GOLDEN);
    // The list says so, and the link is gone from the map once the index has applied the entry.
    const chip = page.getByRole("button", { name: /^Corrections · 1/ });
    await expect(chip).toBeVisible();
    await expect(chip).not.toContainText("stale", { timeout: 30_000 });
    await expect.poll(async () => (await api("graph?lod=file")).links.kind, { timeout: 30_000 }).not.toContain("http");

    // Committed, the entry follows a rename of its caller.
    git("add", "."); git("commit", "-q", "-m", "a correction");
    git("mv", "web/src/api.ts", "web/src/client.ts"); git("commit", "-q", "-m", "rename the caller");
    await expect.poll(async () => JSON.stringify((await api("graph?lod=file")).nodes.name), { timeout: 30_000 }).toContain("web/src/client.ts");
    expect((await api("corrections"))[0].stale, "the entry still matches its link after the rename").toBe(false);
    await page.reload();
    await expect(chip).toBeVisible();
    await expect(chip).not.toContainText("stale");

    // Undo from the list: the entry leaves the file, and the list with it.
    await chip.click();
    const list = page.getByRole("dialog", { name: "Corrections" });
    await expect(list.getByText("a log line, not a call: <b>text</b>")).toBeVisible();
    await list.getByRole("button", { name: /^Undo:/ }).click();
    await expect(page.getByText("Removed from cartograph.overrides.yaml.")).toBeVisible();
    await expect(list.getByText("The file holds no corrections.")).toBeVisible();
    await expect.poll(async () => (await api("graph?lod=file")).links.kind, { timeout: 30_000 }).toContain("http");
    expect(readFileSync(join(repo, "cartograph.overrides.yaml"), "utf8")).toBe("version: 1\ncorrections:\n");
  } finally {
    await s.stop();
  }
});
