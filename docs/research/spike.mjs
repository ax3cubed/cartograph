// Spike: content-addressed temporal indexing on a real repo. Read-only (git plumbing only).
// usage: node docs/research/spike.mjs <path-to-a-git-repo>
// Measures (1) the history walk, (2) parse-once-per-unique-blob extraction, (3) interval replay,
// and compares the work done with the naive "re-parse the tree at every commit" approach.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import assert from "node:assert/strict";

const repo = process.argv[2];
const SRC = /\.(ts|tsx|js|jsx|mjs|cjs)$/;
const SKIP = /(^|\/)(node_modules|dist|build|vendor|\.next)\//;
const now = () => Number(process.hrtime.bigint()) / 1e6;
const fmt = (n) => Math.round(n).toLocaleString("en-US");

// ---- 1. walk: one streaming process, first-parent, oldest first ----
let t0 = now();
const commits = []; // { sha, date, changes: [path, oldOid, newOid][] }
const unique = new Set();
{
  const p = spawn("git", ["-C", repo, "-c", "core.quotePath=false", "log", "--first-parent", "--reverse", "--raw", "--no-abbrev", "--no-renames", "--format=\x02%H %ct"], { stdio: ["ignore", "pipe", "inherit"] });
  let cur = null;
  for await (const line of createInterface({ input: p.stdout })) {
    if (line[0] === "\x02") { const [sha, ct] = line.slice(1).split(" "); commits.push((cur = { sha, date: +ct, changes: [] })); continue; }
    if (line[0] !== ":") continue;
    const tab = line.indexOf("\t");
    const [, , oldOid, newOid] = line.slice(1, tab).split(" ");
    const path = line.slice(tab + 1);
    if (!SRC.test(path) || SKIP.test(path)) continue;
    cur.changes.push([path, oldOid, newOid]);
    if (!/^0+$/.test(newOid)) unique.add(newOid);
  }
}
const tWalk = now() - t0;

// ---- 2. extract: each unique blob read and parsed exactly once ----
t0 = now();
const IMPORT = /(?:\bimport\s+(?:[^'"()]*?\sfrom\s+)?|\bexport\s+[^'"()]*?\sfrom\s+|\brequire\s*\(\s*|\bimport\s*\(\s*)['"]([^'"\n]+)['"]/g;
const extract = (src) => { const out = new Set(); for (const m of src.matchAll(IMPORT)) out.add(m[1]); return [...out]; };
assert.deepEqual(extract(`import a from "./a"; import "./side"; export * from '../b'; const c = require("c"); await import('./d')`), ["./a", "./side", "../b", "c", "./d"]);

const facts = new Map(); // oid -> specifiers
let bytes = 0;
{
  const p = spawn("git", ["-C", repo, "cat-file", "--batch"], { stdio: ["pipe", "pipe", "inherit"] });
  const oids = [...unique];
  let i = 0, buf = Buffer.alloc(0), need = -1, oid = "";
  const feed = () => { let ok = true; while (ok && i < oids.length) ok = p.stdin.write(oids[i++] + "\n"); if (i >= oids.length) p.stdin.end(); };
  p.stdin.on("drain", feed);
  feed();
  for await (const chunk of p.stdout) {
    buf = buf.length ? Buffer.concat([buf, chunk]) : chunk;
    for (;;) {
      if (need < 0) {
        const nl = buf.indexOf(10);
        if (nl < 0) break;
        const [o, , size] = buf.toString("latin1", 0, nl).split(" ");
        oid = o; need = +size; buf = buf.subarray(nl + 1);
      }
      if (buf.length < need + 1) break;
      bytes += need;
      facts.set(oid, need > 1_000_000 ? [] : extract(buf.toString("utf8", 0, need))); // size cap: skip generated bundles
      buf = buf.subarray(need + 1); need = -1;
    }
  }
}
const tExtract = now() - t0;
assert.equal(facts.size, unique.size, "every unique blob extracted once");

// ---- 3. replay: apply fact deltas per commit, open/close intervals ----
t0 = now();
const tree = new Map(); // path -> oid
const open = new Map(); // "path\0spec" -> ordinal it was added at
const intervals = []; // [path, spec, addedOrd, removedOrd]
let naiveParses = 0, events = 0;
commits.forEach((c, ord) => {
  for (const [path, , newOid] of c.changes) {
    events++;
    const before = new Set(facts.get(tree.get(path)) ?? []);
    const gone = /^0+$/.test(newOid);
    const after = new Set(gone ? [] : facts.get(newOid));
    gone ? tree.delete(path) : tree.set(path, newOid);
    for (const s of before) if (!after.has(s)) { const k = path + "\0" + s; intervals.push([path, s, open.get(k), ord]); open.delete(k); }
    for (const s of after) if (!before.has(s)) open.set(path + "\0" + s, ord);
  }
  naiveParses += tree.size; // what a snapshot-per-commit indexer would parse here
});
const tReplay = now() - t0;

// differential check: the replayed end state must equal a from-scratch extraction of HEAD's tree
const head = new Set();
for (const [path, oid] of tree) for (const s of facts.get(oid)) head.add(path + "\0" + s);
assert.deepEqual([...open.keys()].sort(), [...head].sort(), "incremental end state equals from-scratch HEAD");

console.log(JSON.stringify({
  commitsFirstParent: commits.length,
  sourceFilesAtHead: tree.size,
  changeEvents: events,
  uniqueBlobs: unique.size,
  naiveParses,
  parseReduction: +(naiveParses / unique.size).toFixed(0),
  sourceMB: +(bytes / 1e6).toFixed(0),
  walkMs: Math.round(tWalk), extractMs: Math.round(tExtract), replayMs: Math.round(tReplay),
  totalS: +((tWalk + tExtract + tReplay) / 1000).toFixed(1),
  blobsPerSec: Math.round(unique.size / (tExtract / 1000)),
  mbPerSec: +(bytes / 1e6 / (tExtract / 1000)).toFixed(0),
  commitsPerSecWalk: Math.round(commits.length / (tWalk / 1000)),
  refIntervalsClosed: intervals.length, refIntervalsOpen: open.size,
  peakRssMB: Math.round(process.resourceUsage().maxRSS / 1024),
}, null, 1));
console.error(`walk ${fmt(tWalk)}ms · extract ${fmt(tExtract)}ms · replay ${fmt(tReplay)}ms`);
