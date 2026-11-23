// HD-05 fixtures: the real `cartograph serve` on a copy of the golden data folder, and a browser that can reach
// nothing but it. Any request off loopback is aborted and fails the test, so every run doubles as a "nothing leaves
// the machine" check (HD-11).
import { test as base, expect, type Page } from "@playwright/test";
import { spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export type Served = { origin: string; token: string; home: string };

/** Starts the binary on a fresh data folder: a copy of the golden one, or an empty one (first run). */
export async function serve(golden = true): Promise<Served & { stop: () => Promise<void> }> {
  const home = mkdtempSync(join(tmpdir(), "cartograph-e2e-w-"));
  if (golden) cpSync(process.env.CARTOGRAPH_E2E_GOLDEN!, home, { recursive: true });
  else mkdirSync(join(home, "workspaces"));
  const child = spawn(process.env.CARTOGRAPH_E2E_BIN!, ["serve", "--port", "0"], {
    // HOME too: the folder picker and the registry never see the real one.
    env: { ...process.env, CARTOGRAPH_HOME: home, HOME: home, USERPROFILE: home, NO_COLOR: "1" },
    stdio: "ignore",
  });
  const stop = async () => {
    child.kill();
    await new Promise((r) => (child.exitCode !== null ? r(null) : child.once("exit", r)));
    rmSync(home, { recursive: true, force: true });
  };
  const file = join(home, "server.json");
  for (let t = Date.now(); Date.now() - t < 20_000; await new Promise((r) => setTimeout(r, 50))) {
    if (child.exitCode !== null) break;
    if (!existsSync(file)) continue;
    try {
      const s = JSON.parse(readFileSync(file, "utf8"));
      const origin = `http://${s.host}:${s.port}`;
      await fetch(`${origin}/api/version`);
      return { origin, token: s.token, home, stop };
    } catch { /* written but not listening yet */ }
  }
  await stop();
  throw new Error("cartograph serve didn't start");
}

const local = (u: URL) => u.hostname === "127.0.0.1" || u.protocol === "data:" || u.protocol === "blob:" || u.protocol === "about:";

export const test = base.extend<{ go: (path: string, on?: Page) => Promise<void>; workspace: string }, { served: Served & { stop: () => Promise<void> } }>({
  served: [async ({}, use) => { const s = await serve(); await use(s); await s.stop(); }, { scope: "worker" }],
  workspace: ["northwind", { option: true }],
  context: async ({ context, workspace }, use) => {
    const off: string[] = [];
    await context.route((u) => !local(u), (r) => { off.push(r.request().url()); return r.abort("blockedbyclient"); });
    await context.addInitScript((ws) => { try { localStorage.setItem("cartograph.workspace", ws); } catch { /* opaque origin */ } }, workspace);
    await use(context);
    expect(off, "requests that tried to leave the machine").toEqual([]);
  },
  /** Opens `path` as `cartograph open` would: the session token rides in the fragment once. */
  go: async ({ page, served }, use) => {
    await use(async (path, on = page) => { await open(on, served, path); });
  },
});

export async function open(page: Page, s: Served, path: string) {
  await page.goto(`${s.origin}${path}${path.includes("#") ? "&" : "#"}k=${s.token}`);
}

export { expect };
