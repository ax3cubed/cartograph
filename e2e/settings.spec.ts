// FE-13 and FE-21 on the real binary: the agents page reads the server's tool registry and agent status and previews
// every file change before it writes, and a file-backed settings page round-trips cartograph.yaml, conflict included.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, open, serve, test } from "./fixtures";

test("agents page: tools from the server, agents on this machine, a preview before anything is written", async ({ page, go, served }) => {
  const tools = await (await fetch(`${served.origin}/api/v1/mcp/tools`, { headers: { Authorization: `Bearer ${served.token}` } })).json();
  await go("/settings/editor");
  const list = page.getByRole("heading", { name: /^What the agent can do/ }).locator("..").locator("li");
  await expect(list).toHaveCount(tools.data.length);
  for (const t of tools.data) await expect(list.filter({ has: page.locator("code", { hasText: new RegExp(`^${t.name}$`) }) })).toHaveCount(1);

  const agents = page.getByRole("region", { name: "Agents on this machine" }).or(page.locator("section", { has: page.getByRole("heading", { name: "Agents on this machine" }) }));
  await expect(agents.getByText("Claude Code", { exact: true })).toBeVisible();
  await expect(agents.getByText("Cursor", { exact: true })).toBeVisible();

  // The per-worker HOME is empty: Cursor isn't set up. The preview shows the diff; nothing is on disk until confirmed.
  const config = join(served.home, ".cursor/mcp.json");
  await page.getByRole("button", { name: "Preview set-up", exact: true }).and(page.locator("[aria-describedby='set-ed-a-cursor']")).click();
  const confirm = page.getByRole("alertdialog", { name: /^Set up Cursor\?/ });
  await expect(confirm.getByLabel(/^Changes to /)).toContainText("cartograph");
  expect(existsSync(config), "a preview writes nothing").toBe(false);
  await confirm.getByRole("button", { name: /^Write th/ }).click();
  await expect(page.getByRole("status").filter({ hasText: "Cursor is set up" })).toBeVisible();
  expect(readFileSync(config, "utf8")).toContain("cartograph");
  await expect(agents.locator("li", { hasText: "Cursor" }).getByText("Set up", { exact: true })).toBeVisible();
});

test("indexing settings: save writes only options.exclude to cartograph.yaml; a hand edit meanwhile gives the 412 flow", async ({ page }) => {
  const s = await serve(false);
  try {
    const repo = join(s.home, "shop");
    mkdirSync(join(repo, "src"), { recursive: true });
    writeFileSync(join(repo, "src/a.ts"), "export const a = 1;\n");
    const git = (...a: string[]) => execFileSync("git", ["-C", repo, ...a], { env: { ...process.env, HOME: s.home, GIT_AUTHOR_NAME: "e2e", GIT_AUTHOR_EMAIL: "e2e@localhost", GIT_COMMITTER_NAME: "e2e", GIT_COMMITTER_EMAIL: "e2e@localhost" } });
    git("init", "-q", "-b", "main"); git("add", "."); git("commit", "-q", "-m", "first");
    const res = await fetch(`${s.origin}/api/v1/workspaces`, {
      method: "POST",
      headers: { Authorization: `Bearer ${s.token}`, Origin: s.origin, "X-Cartograph": "1", "Content-Type": "application/json" },
      body: JSON.stringify({ path: "~/shop", manifest: true }),
    });
    expect(res.ok, await res.clone().text()).toBe(true);
    const ws = (await res.json()).data.workspace.id as string;
    const file = join(repo, "cartograph.yaml");
    const before = readFileSync(file, "utf8");

    await open(page, s, `/settings/indexing#ws=${ws}`);
    const area = page.getByRole("textbox", { name: "Paths to leave out, one per line" });
    await expect(area).toBeEditable();
    await area.fill("dist/**\ntmp/**");
    const bar = page.getByRole("region", { name: "Unsaved changes" });
    await expect(bar).toContainText("1 unsaved change");
    await bar.getByText("Preview the change to cartograph.yaml").click();
    await expect(bar.getByLabel("Diff of cartograph.yaml")).toContainText("+    - tmp/**");
    await bar.getByRole("button", { name: "Save", exact: true }).click();
    await expect(bar).toHaveCount(0);
    const after = readFileSync(file, "utf8");
    expect(after).toContain("  exclude:\n    - dist/**\n    - tmp/**\n");
    for (const line of before.split("\n")) expect(after.split("\n"), "every line the user had is kept").toContain(line);

    // Edited by hand while a draft is open: Save gets 412, and Overwrite writes the draft over the new text.
    await area.fill("dist/**");
    writeFileSync(file, `${readFileSync(file, "utf8")}# edited by hand\n`);
    await page.getByRole("region", { name: "Unsaved changes" }).getByRole("button", { name: "Save", exact: true }).click();
    const conflict = page.getByRole("alert").filter({ hasText: "cartograph.yaml changed on disk" });
    await expect(conflict).toBeVisible();
    await conflict.getByRole("button", { name: "Overwrite" }).click();
    await expect(conflict).toHaveCount(0);
    const last = readFileSync(file, "utf8");
    expect(last).toContain("# edited by hand");
    expect(last).toContain("  exclude:\n    - dist/**\n");
    expect(last).not.toContain("tmp/**");
  } finally {
    await s.stop();
  }
});
