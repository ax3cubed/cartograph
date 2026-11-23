// HD-05: the end-to-end suite in one command (`pnpm test:e2e`).
//
//   pnpm test:e2e                   build the app, the binary that embeds it and the fixture tool, then run the
//                                   smoke, axe and journey tests in Chromium, Firefox and WebKit
//   pnpm test:e2e --visual          the screenshot tests instead, against Chromium in the pinned container (Docker)
//   pnpm test:e2e --visual --update  the same, rewriting the baselines in e2e/__screenshots__
//   --no-build                      reuse out/ and target/debug as they are
//   anything else goes to `playwright test` (a file, -g "name", --project=webkit, --headed)
//
// Screenshots differ between hosts (fonts, rasterising), so they are taken in one image only: the Playwright image
// pinned below, on linux/amd64, in CI and on a laptop alike. Its browser connects back to the binary on this machine
// through Playwright's exposeNetwork, so the page sees the same 127.0.0.1 origin as in every other test.
import { execFileSync, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { dirname } from "node:path";

// The Playwright version must match @playwright/test in package.json.
const IMAGE = "mcr.microsoft.com/playwright:v1.65.1-noble@sha256:766137e2d9caad4744187e53cdc89320ba6f2eb57cff097374020944b44dd0a9"; // linux/amd64

const args = process.argv.slice(2);
const flag = (f) => { const i = args.indexOf(f); if (i >= 0) args.splice(i, 1); return i >= 0; };
const visual = flag("--visual"), update = flag("--update"), noBuild = flag("--no-build");
const shell = process.platform === "win32";
const run = (cmd, a, env) => { const r = spawnSync(cmd, a, { stdio: "inherit", shell, env: { ...process.env, ...env } }); if (r.status !== 0) process.exit(r.status ?? 1); };

if (!noBuild) {
  run("pnpm", ["build"]);
  // build.rs embeds out/, so the binary is built after the app.
  run("cargo", ["build", "--locked", "-p", "cartograph-cli", "-p", "cartograph-fixtures", "--bin", "cartograph", "--example", "fixture-home"]);
}

if (!visual) run("pnpm", ["exec", "playwright", "test", "--project=chromium", "--project=firefox", "--project=webkit", ...args]);
else {
  // The image's browsers, driven by the same playwright-core the tests use, mounted read-only: no download.
  const core = dirname(createRequire(createRequire(import.meta.url).resolve("@playwright/test")).resolve("playwright-core/package.json"));
  const id = execFileSync("docker", ["run", "-d", "--rm", "--init", "--platform", "linux/amd64", "--user", "pwuser", "-p", "127.0.0.1::3000",
    "-v", `${core}:/pw:ro`, IMAGE, "node", "/pw/cli.js", "run-server", "--port", "3000", "--host", "0.0.0.0"], { encoding: "utf8" }).trim();
  let code = 1;
  try {
    const port = execFileSync("docker", ["port", id, "3000"], { encoding: "utf8" }).trim().split("\n")[0].split(":").pop();
    // Docker's port forward accepts before the server listens, so wait for the server's own line.
    for (let t = Date.now(); !execFileSync("docker", ["logs", id], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).includes("Listening on"); ) {
      if (Date.now() - t > 60_000) throw new Error("the browser container didn't start");
      await new Promise((r) => setTimeout(r, 500));
    }
    const r = spawnSync("pnpm", ["exec", "playwright", "test", "--project=visual", "--workers=2", ...(update ? ["--update-snapshots=all"] : []), ...args],
      { stdio: "inherit", shell, env: { ...process.env, PW_REMOTE: `ws://127.0.0.1:${port}/` } });
    code = r.status ?? 1;
  } finally {
    spawnSync("docker", ["rm", "-f", id], { stdio: "ignore" });
  }
  process.exit(code);
}
