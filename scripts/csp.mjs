#!/usr/bin/env node
// FE-01: after `next build`, hash every inline <script> in out/**/*.html and write out/csp.json, so the
// server (SRV-05) can send a CSP without 'unsafe-inline' for scripts. Each page gets its own header:
// `policy` with {hashes} replaced by that page's entry in `hashes`, keyed by the path under out/.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";

const OUT = "out";
// style-src needs 'unsafe-inline': React writes style="" attributes (CSS variables, sizes) into the HTML.
// ponytail: a style attribute can't run script; hashing each one ('unsafe-hashes') isn't worth it yet.
const POLICY =
  "default-src 'self'; script-src 'self' {hashes}; style-src 'self' 'unsafe-inline'; connect-src 'self'; " +
  "img-src 'self' data: blob:; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

const hashes = {};
for (const file of readdirSync(OUT, { recursive: true }).filter((f) => f.endsWith(".html")).sort()) {
  const html = readFileSync(join(OUT, file), "utf8");
  const inline = [...html.matchAll(/<script(?![^>]*\ssrc=)[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
  const set = new Set(inline.map((s) => `'sha256-${createHash("sha256").update(s).digest("base64")}'`));
  hashes[relative(OUT, join(OUT, file)).split(sep).join("/")] = [...set];
}
writeFileSync(join(OUT, "csp.json"), JSON.stringify({ policy: POLICY, hashes }, null, 1) + "\n");
console.log(`csp.json: ${Object.keys(hashes).length} pages, ${Object.values(hashes).flat().length} inline script hashes`);
