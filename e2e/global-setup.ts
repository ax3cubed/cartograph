// HD-05: once per run, the golden data folder (workspaces `northwind` and `history`, from fixtures/gen). Each
// worker serves its own copy (fixtures.ts), so a test that writes can't change what another one sees.
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const exe = process.platform === "win32" ? ".exe" : "";

export default function setup() {
  const bin = process.env.CARTOGRAPH_BIN ?? resolve(`target/debug/cartograph${exe}`);
  const gen = process.env.CARTOGRAPH_FIXTURE_HOME ?? resolve(`target/debug/examples/fixture-home${exe}`);
  for (const f of [bin, gen]) if (!existsSync(f)) throw new Error(`${f} is missing: run pnpm test:e2e, which builds it`);
  const golden = mkdtempSync(join(tmpdir(), "cartograph-e2e-"));
  execFileSync(gen, [golden], { stdio: "ignore" });
  process.env.CARTOGRAPH_E2E_BIN = bin;
  process.env.CARTOGRAPH_E2E_GOLDEN = golden;
  return () => rmSync(golden, { recursive: true, force: true });
}
