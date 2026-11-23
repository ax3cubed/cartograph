#!/usr/bin/env node
// FE-01: after `next build`, hash every inline <script> in out/**/*.html and write out/csp.json, so the
// server (SRV-05) can send a CSP without 'unsafe-inline' for scripts. Each page gets its own header:
// `policy` with {hashes} replaced by that page's entry in `hashes`, keyed by the path under out/.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";

const OUT = "out";
// style-src keeps 'unsafe-inline' (checked in the phase 2 review). Hashes can't replace it cleanly:
// - React writes style="" attributes into the export (664 on index.html, CSS variables and sizes).
//   Attributes are only hashable with 'unsafe-hashes', one hash per distinct value.
// - A client-side navigation keeps the first page's header, so every page's list would be the union.
// - <style> elements inserted at run time (Radix's Select and ScrollArea, React 19 hoisted styles)
//   aren't in out/ at build time, so they would be blocked with nothing but a console error.
// What it allows is CSS, never script; CSS can't send data out because img-src and font-src are
// 'self' and connect-src is 'self'.
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
