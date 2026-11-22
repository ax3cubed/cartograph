// HD-12: npm production dependencies checked against the same licence allow list as deny.toml.
//   node scripts/check-licenses.mjs        (needs `pnpm install` first)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const deny = readFileSync(new URL("../deny.toml", import.meta.url), "utf8");
const block = /\[licenses\][\s\S]*?allow\s*=\s*\[([\s\S]*?)\]/.exec(deny)?.[1] ?? "";
const allow = new Set([...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]));

// Build-time only: these never reach the static export embedded in the binary. Each needs a reason.
const exceptions = {
  "@img/sharp-libvips-": "LGPL-3.0: libvips for next/image's optimiser; the app is a static export and never runs it",
  "caniuse-lite": "CC-BY-4.0: browser data read by the build tooling, not shipped",
};

const shell = process.platform === "win32";
const report = JSON.parse(execFileSync("pnpm", ["licenses", "ls", "--json", "--prod"], { encoding: "utf8", shell }));
const bad = [];
for (const [license, pkgs] of Object.entries(report)) {
  // "(MIT OR Apache-2.0)" passes when any alternative is allowed; "A AND B" needs both.
  const ok = license
    .replace(/[()]/g, "")
    .split(/\s+OR\s+/)
    .some((alt) => alt.split(/\s+AND\s+/).every((l) => allow.has(l.trim())));
  if (ok) continue;
  for (const p of pkgs) {
    const why = Object.entries(exceptions).find(([prefix]) => p.name.startsWith(prefix))?.[1];
    if (why) console.log(`allowed by exception: ${p.name} (${license}): ${why}`);
    else bad.push(`${p.name}@${p.versions.join(",")}: ${license}`);
  }
}
if (bad.length) {
  console.error(`Licences outside the allow list in deny.toml:\n  ${bad.join("\n  ")}`);
  process.exit(1);
}
console.log(`npm licences ok (${Object.values(report).flat().length} production packages)`);
