#!/usr/bin/env node
// FE-01: the static export the binary embeds. Run after `pnpm build`:  node scripts/check-export.mjs
// Fails when app/api exists (no server routes, A2), a route's HTML is missing, or out/ is over budget.
import { brotliCompressSync, constants } from "node:zlib";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const OUT = "out";
const RAW_MAX = 6_000_000; // bytes, raw
const BROTLI_MAX = 1_800_000; // bytes, each file brotli-compressed
const SETTINGS = ["connections", "repositories", "indexing", "services", "contracts", "models", "editor", "retrieval", "appearance", "keyboard"];
const ROUTES = [
  "index", "ask", "welcome", "welcome/add", "404",
  "concepts", "contracts", "coverage", "entrypoints", "impact",
  ...SETTINGS.map((s) => `settings/${s}`),
].map((r) => `${r}.html`);

const fail = [];
if (existsSync("app/api")) fail.push("app/api exists: the export has no server routes (A2)");
for (const r of ROUTES) if (!existsSync(join(OUT, r))) fail.push(`missing ${OUT}/${r}`);

let raw = 0, br = 0;
for (const f of readdirSync(OUT, { recursive: true })) {
  const p = join(OUT, f);
  if (!statSync(p).isFile()) continue;
  const buf = readFileSync(p);
  raw += buf.length;
  br += brotliCompressSync(buf, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
}
const mb = (n) => `${(n / 1e6).toFixed(2)} MB`;
console.log(`out/: ${mb(raw)} raw (max ${mb(RAW_MAX)}), ${mb(br)} brotli (max ${mb(BROTLI_MAX)}), ${ROUTES.length} routes checked`);
if (raw > RAW_MAX) fail.push(`out/ is ${mb(raw)} raw, over ${mb(RAW_MAX)}`);
if (br > BROTLI_MAX) fail.push(`out/ is ${mb(br)} brotli, over ${mb(BROTLI_MAX)}`);

for (const f of fail) console.error(`✗ ${f}`);
process.exit(fail.length ? 1 : 0);
