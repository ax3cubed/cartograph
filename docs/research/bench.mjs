// Experiments for docs/research: content-addressed fact replay on real git history.
// Read-only: git plumbing only. Writes nothing unless --json <file> is given.
//
//   node docs/research/bench.mjs <repo | folder of repos> [--runs 5] [--samples 24] [--par 4] [--reps 3] [--quick] [--json out.json]
//
// What it does, for JS/TS import links and package links (bare specifier -> package.json with that name):
//   walk     one `git log --first-parent --raw` per repository
//   extract  facts per distinct blob id, once
//   replay   oldest first, with a reverse index (key -> references waiting on it), writing [added, removed) intervals
// and then checks the result against an independent from-scratch index of sampled commits, times the
// baselines (index every commit; re-resolve everything on every structural commit), and measures
// incremental updates, time-travel queries and edge lifetimes.
//
// ponytail: extraction is a regular expression, not a parser. It finds import specifiers and nothing else.
// Swap in a real parser per language when the indexer is built; everything after extraction is unchanged.
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { Worker, isMainThread, parentPort, workerData } from "node:worker_threads";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, writeFileSync } from "node:fs";
import { posix, join, basename, resolve as absolute } from "node:path";
import assert from "node:assert/strict";

const SRC = /\.(ts|tsx|js|jsx|mjs|cjs)$/;
const SKIP = /(^|\/)(node_modules|dist|build|vendor|\.next)\//;
const EXTS = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];
const isPkg = (p) => p === "package.json" || p.endsWith("/package.json");
const tracked = (p) => (SRC.test(p) || isPkg(p)) && !SKIP.test(p);
const now = () => Number(process.hrtime.bigint()) / 1e6;
const median = (a) => { const s = [...a].sort((x, y) => x - y); return s[s.length >> 1]; };
const hashOf = (arr) => createHash("sha1").update(arr.sort().join("\n")).digest("hex");

// ---------- facts: what one file says, from its bytes alone ----------
const IMPORT = /(?:\bimport\s+(?:[^'"()]*?\sfrom\s+)?|\bexport\s+[^'"()]*?\sfrom\s+|\brequire\s*\(\s*|\bimport\s*\(\s*)['"]([^'"\n]+)['"]/g;
const extractSrc = (src) => { const out = new Set(); for (const m of src.matchAll(IMPORT)) out.add(m[1]); return [...out]; };
const pkgName = (src) => { try { const n = JSON.parse(src).name; return typeof n === "string" ? n : null; } catch { return null; } };

// The keys a reference could resolve to, in priority order. A path is a key; "#name" is a package key.
function candidates(from, spec) {
  if (spec[0] !== ".") {
    if (spec.startsWith("node:")) return [];
    const parts = spec.split("/");
    return ["#" + (spec[0] === "@" ? parts.slice(0, 2).join("/") : parts[0])];
  }
  const base = posix.join(posix.dirname(from), spec).replace(/\/$/, "");
  const out = SRC.test(base) ? [base] : [];
  const m = /\.(js|jsx|mjs|cjs)$/.exec(base);
  if (m) { const stem = base.slice(0, -m[0].length); out.push(stem + ".ts", stem + ".tsx"); } // TS ESM: "./x.js" names x.ts
  for (const e of EXTS) out.push(base + e);
  for (const e of EXTS) out.push(base + "/index" + e);
  return out;
}

// ---------- git plumbing ----------
async function catBlobs(repo, oids, onBlob) {
  if (!oids.length) return;
  const p = spawn("git", ["-C", repo, "cat-file", "--batch"], { stdio: ["pipe", "pipe", "inherit"] });
  let i = 0, n = 0, buf = Buffer.alloc(0), need = -1;
  const feed = () => { let ok = true; while (ok && i < oids.length) ok = p.stdin.write(oids[i++] + "\n"); if (i >= oids.length) p.stdin.end(); };
  p.stdin.on("drain", feed);
  feed();
  for await (const chunk of p.stdout) {
    buf = buf.length ? Buffer.concat([buf, chunk]) : chunk;
    for (;;) {
      if (need < 0) {
        const nl = buf.indexOf(10);
        if (nl < 0) break;
        const h = buf.toString("latin1", 0, nl).split(" ");
        buf = buf.subarray(nl + 1);
        if (h[1] === "missing") { onBlob(n++, null, 0); continue; }
        need = +h[2];
      }
      if (buf.length < need + 1) break;
      onBlob(n++, buf, need);
      buf = buf.subarray(need + 1); need = -1;
    }
  }
}

if (!isMainThread) { // extraction worker: one cat-file process, facts for its share of the blobs
  const { repo, src, pkg, parse } = workerData;
  const facts = [], names = [], sizes = [];
  await catBlobs(repo, src.concat(pkg), (i, buf, size) => {
    if (i < src.length) { sizes.push(size); facts.push(!parse || !buf || size > 1_000_000 ? [] : extractSrc(buf.toString("utf8", 0, size))); } // size cap: skip generated bundles
    else names.push(parse && buf ? pkgName(buf.toString("utf8", 0, size)) : null);
  });
  parentPort.postMessage({ facts, names, sizes });
}

async function extractAll(repo, src, pkg, P, into, parse = true) {
  const shards = Array.from({ length: P }, () => ({ src: [], pkg: [] }));
  src.forEach((o, i) => shards[i % P].src.push(o));
  pkg.forEach((o, i) => shards[i % P].pkg.push(o));
  const res = await Promise.all(shards.map((s) => new Promise((ok, bad) => {
    const w = new Worker(new URL(import.meta.url), { workerData: { repo, ...s, parse } });
    w.once("message", ok); w.once("error", bad);
  })));
  res.forEach((r, k) => {
    shards[k].src.forEach((o, i) => { into.facts.set(o, r.facts[i]); into.size.set(o, r.sizes[i]); });
    shards[k].pkg.forEach((o, i) => into.names.set(o, r.names[i]));
  });
}

// One streaming walk. Each commit: the tracked paths it changed against its first parent, with the new blob id (null = gone).
// --diff-merges=first-parent is spelled out (git 2.31+): without a diff for merge commits the replayed tree would silently drift.
async function walk(repo, prefix, range = "HEAD", flags = ["--first-parent", "--diff-merges=first-parent"]) {
  const commits = [];
  const p = spawn("git", ["-C", repo, "-c", "core.quotePath=false", "log", ...flags, "--reverse", "--raw", "--no-abbrev", "--no-renames", "--format=\x02%H %ct", range], { stdio: ["ignore", "pipe", "inherit"] });
  let cur = null, cd = 0;
  for await (const line of createInterface({ input: p.stdout })) {
    if (line[0] === "\x02") {
      const [sha, ct] = line.slice(1).split(" ");
      cd = Math.max(+ct, cd + 1); // corrected date: committer dates are not monotonic
      commits.push((cur = { sha, date: cd, changes: [] }));
      continue;
    }
    if (line[0] !== ":") continue;
    const tab = line.indexOf("\t");
    const [, newMode, , newOid] = line.slice(1, tab).split(" ");
    const path = line.slice(tab + 1);
    if (!tracked(path)) continue;
    cur.changes.push([prefix + path, newMode.startsWith("100") ? newOid : null, isPkg(path)]); // symlinks and gitlinks count as absent
  }
  return commits;
}

function lsTree(repo, rev, prefix = "") {
  const out = execFileSync("git", ["-C", repo, "-c", "core.quotePath=false", "ls-tree", "-r", rev], { maxBuffer: 1 << 30 }).toString();
  const tree = new Map();
  for (const line of out.split("\n")) {
    if (!line) continue;
    const tab = line.indexOf("\t"), [mode, type, oid] = line.slice(0, tab).split(" "), path = line.slice(tab + 1);
    if (type === "blob" && mode.startsWith("100") && tracked(path)) tree.set(prefix + path, oid);
  }
  return tree;
}

// ---------- the index: replay with a reverse index ----------
// reverse=false is the ablation: no reverse index, every live reference is re-resolved on any commit that adds or removes a key.
const EMPTY = [];
function makeIndex(facts, names, reverse = true) {
  const tree = new Map(), refs = new Map(), waiting = new Map(), pkgs = new Map(), edges = new Map(), openAt = new Map(), intervals = [];
  const touched = new Map();
  const st = { resolves: 0, structural: false };
  const resolve = (ref) => {
    st.resolves++;
    for (const k of ref.cands) {
      if (k.charCodeAt(0) === 35) { const s = pkgs.get(k); if (s) { let m; for (const p of s) if (m === undefined || p < m) m = p; return m; } }
      else if (tree.has(k)) return k;
    }
    return null;
  };
  const bump = (from, to, d) => { const k = from + "\0" + to, n = edges.get(k) ?? 0; if (!touched.has(k)) touched.set(k, n > 0); n + d ? edges.set(k, n + d) : edges.delete(k); };
  const setTarget = (ref, t) => { if (ref.target === t) return; if (ref.target !== null) bump(ref.from, ref.target, -1); if (t !== null) bump(ref.from, t, 1); ref.target = t; };
  const notify = (key) => { if (!reverse) { st.structural = true; return; } const w = waiting.get(key); if (w) for (const ref of w) setTarget(ref, resolve(ref)); };
  const addRef = (from, spec) => {
    const ref = { from, cands: candidates(from, spec), target: null };
    refs.set(from + "\0" + spec, ref);
    if (reverse) for (const k of ref.cands) { let w = waiting.get(k); if (!w) waiting.set(k, (w = new Set())); w.add(ref); }
    setTarget(ref, resolve(ref));
  };
  const delRef = (from, spec) => {
    const key = from + "\0" + spec, ref = refs.get(key);
    refs.delete(key); setTarget(ref, null);
    if (reverse) for (const k of ref.cands) { const w = waiting.get(k); w.delete(ref); if (!w.size) waiting.delete(k); }
  };
  function apply(c, ord) {
    for (const [path, newOid, pkg] of c.changes) {
      const old = tree.get(path);
      if (old === newOid || (old === undefined && newOid === null)) continue;
      newOid === null ? tree.delete(path) : tree.set(path, newOid);
      if (pkg) {
        const a = old === undefined ? null : names.get(old), b = newOid === null ? null : names.get(newOid);
        if (a === b) continue;
        if (a != null) { const s = pkgs.get("#" + a); s.delete(path); if (!s.size) pkgs.delete("#" + a); notify("#" + a); }
        if (b != null) { let s = pkgs.get("#" + b); if (!s) pkgs.set("#" + b, (s = new Set())); s.add(path); notify("#" + b); }
        continue;
      }
      if (old === undefined || newOid === null) notify(path);
      const before = old === undefined ? EMPTY : facts.get(old), after = newOid === null ? EMPTY : facts.get(newOid);
      if (before === after) continue;
      const b = new Set(before), a = new Set(after);
      for (const s of before) if (!a.has(s)) delRef(path, s);
      for (const s of after) if (!b.has(s)) addRef(path, s);
    }
    if (st.structural) { st.structural = false; for (const ref of refs.values()) setTarget(ref, resolve(ref)); }
    for (const [k, was] of touched) { // net effect of the commit only: an edge that closes and reopens inside one commit never left
      const is = edges.has(k);
      if (was && !is) { intervals.push([k, openAt.get(k), ord]); openAt.delete(k); }
      else if (!was && is) openAt.set(k, ord);
    }
    touched.clear();
  }
  return { apply, tree, edges, refs, openAt, intervals, st };
}

// Independent oracle: the edges of one tree, computed with no state carried over.
function fromScratch(tree, facts, names) {
  const pk = new Map();
  for (const [p, oid] of tree) if (isPkg(p)) { const n = names.get(oid); if (n != null && !(pk.get(n) < p)) pk.set(n, p); }
  const out = new Set();
  for (const [p, oid] of tree) {
    if (isPkg(p)) continue;
    for (const s of facts.get(oid)) for (const k of candidates(p, s)) {
      const t = k[0] === "#" ? pk.get(k.slice(1)) : tree.has(k) ? k : undefined;
      if (t !== undefined) { out.add(p + "\0" + t); break; }
    }
  }
  return out;
}

// The baseline everything is compared with: index one commit from nothing, straight from git.
async function naiveIndex(repo, sha) {
  const t0 = now();
  const tree = lsTree(repo, sha), facts = new Map(), names = new Map();
  const src = [...new Set([...tree].filter(([p]) => !isPkg(p)).map(([, o]) => o))], pkg = [...new Set([...tree].filter(([p]) => isPkg(p)).map(([, o]) => o))];
  await catBlobs(repo, src.concat(pkg), (i, buf, size) => {
    if (i < src.length) facts.set(src[i], !buf || size > 1_000_000 ? [] : extractSrc(buf.toString("utf8", 0, size)));
    else names.set(pkg[i - src.length], buf ? pkgName(buf.toString("utf8", 0, size)) : null);
  });
  const edges = fromScratch(tree, facts, names);
  return { ms: now() - t0, files: tree.size, edges: edges.size, treeHash: hashOf([...tree].map(([p, o]) => p + " " + o)), edgeHash: hashOf([...edges]) };
}

// ---------- self-check on a tiny hand-made history (no git needed) ----------
{
  assert.deepEqual(extractSrc(`import a from "./a"; import "./side"; export * from '../b'; const c = require("c"); await import('./d')`), ["./a", "./side", "../b", "c", "./d"]);
  assert.deepEqual(candidates("src/a.ts", "./db.js").slice(0, 3), ["src/db.js", "src/db.ts", "src/db.tsx"]);
  assert.deepEqual(candidates("src/a.ts", "@scope/pkg/sub"), ["#@scope/pkg"]);
  const facts = new Map([["A1", ["./b", "lib"]], ["B1", []], ["A2", ["./b"]]]), names = new Map([["P1", "lib"], ["P2", "other"]]);
  const ix = makeIndex(facts, names);
  ix.apply({ changes: [["src/a.ts", "A1", false]] }, 0); // b and lib do not exist yet: no edges
  assert.equal(ix.edges.size, 0);
  ix.apply({ changes: [["src/b/index.ts", "B1", false], ["lib/package.json", "P1", true]] }, 1); // both appear: two edges open
  ix.apply({ changes: [["src/b.ts", "B1", false]] }, 2); // a better candidate appears: the edge moves
  ix.apply({ changes: [["lib/package.json", "P2", true], ["src/a.ts", "A2", false]] }, 3); // package renamed, import dropped
  assert.deepEqual(ix.intervals.map(([k, a, r]) => k.replace("\0", ">") + ` ${a}-${r}`).sort(), ["src/a.ts>lib/package.json 1-3", "src/a.ts>src/b/index.ts 1-2"]);
  assert.deepEqual([...ix.openAt].map(([k, a]) => k.replace("\0", ">") + ` ${a}`), ["src/a.ts>src/b.ts 2"]);
  assert.deepEqual([...ix.edges.keys()].sort(), [...fromScratch(ix.tree, facts, names)].sort());
}

// ---------- main ----------
if (isMainThread && process.argv[2]) {
  const arg = (name, dflt) => { const i = process.argv.indexOf("--" + name); return i < 0 ? dflt : process.argv[i + 1] ?? true; };
  const target = absolute(process.argv[2]), RUNS = +arg("runs", 5), SAMPLES = +arg("samples", 24), PAR = +arg("par", 4), REPS = +arg("reps", 3), QUICK = arg("quick", false);
  const discover = (dir, depth = 0) => existsSync(join(dir, ".git")) ? [dir] : depth > 2 ? [] :
    readdirSync(dir, { withFileTypes: true }).filter((d) => d.isDirectory() && d.name[0] !== "." && d.name !== "node_modules").flatMap((d) => discover(join(dir, d.name), depth + 1));
  const repos = discover(target), single = repos.length === 1;
  assert(repos.length, "no git repository found at " + target);
  const prefixOf = (r) => single ? "" : basename(r) + "/";
  const log = (...a) => console.error(...a);
  const R = { repos: repos.length, node: process.version, git: execFileSync("git", ["--version"]).toString().trim() };

  // One full run: walk every repository, extract each distinct blob once, merge the timelines, replay.
  async function pipeline(P) {
    const t0 = now();
    const per = await Promise.all(repos.map((r) => walk(r, prefixOf(r))));
    const t1 = now();
    const store = { facts: new Map(), names: new Map(), size: new Map() };
    let shared = 0;
    for (let i = 0; i < repos.length; i++) {
      const src = new Set(), pkg = new Set();
      for (const c of per[i]) for (const [, oid, p] of c.changes) if (oid) (p ? pkg : src).add(oid);
      const fresh = (s, m) => [...s].filter((o) => (m.has(o) ? (shared++, false) : true));
      await extractAll(repos[i], fresh(src, store.facts), fresh(pkg, store.names), P, store);
    }
    const t2 = now();
    const timeline = per.flatMap((cs, ri) => cs.map((c, ord) => ({ ...c, ri, ord }))); // ponytail: sort, not a k-way merge; fine below millions of commits
    if (!single) timeline.sort((a, b) => a.date - b.date || a.ri - b.ri || a.ord - b.ord);
    const ix = makeIndex(store.facts, store.names);
    timeline.forEach((c, t) => ix.apply(c, t));
    const t3 = now();
    return { walk: t1 - t0, extract: t2 - t1, replay: t3 - t2, total: t3 - t0, per, store, timeline, ix, shared };
  }

  log(`${repos.length} repositor${single ? "y" : "ies"}; ${RUNS} timed runs`);
  const runs = [];
  for (let i = 0; i < RUNS; i++) { runs.push(await pipeline(PAR)); const r = runs.at(-1); log(` run ${i + 1}: walk ${r.walk | 0} extract ${r.extract | 0} replay ${r.replay | 0} total ${r.total | 0} ms`); if (i < RUNS - 1) runs[i] = { walk: r.walk, extract: r.extract, replay: r.replay, total: r.total }; }
  const last = runs.at(-1), { store, timeline } = last, { facts, names, size } = store, N = timeline.length;
  const stat = (k) => ({ median: Math.round(median(runs.map((r) => r[k]))), min: Math.round(Math.min(...runs.map((r) => r[k]))), max: Math.round(Math.max(...runs.map((r) => r[k]))) });
  R.time = { runs: RUNS, workers: PAR, walk: stat("walk"), extract: stat("extract"), replay: stat("replay"), total: stat("total") };
  R.commits = N;
  R.perRepo = single ? undefined : last.per.map((cs, i) => ({ repo: i + 1, commits: cs.length })).sort((a, b) => b.commits - a.commits);
  R.sharedBlobsAcrossRepos = last.shared;

  const lastOid = new Map(); // path -> last blob id it ever had, to name a package that was later deleted
  const headTreeOrLast = (p) => lastOid.get(p);
  // Instrumented replay: counts, snapshots at sampled commits, exact renames, scaling curve.
  const sampleAt = new Set(Array.from({ length: Math.min(SAMPLES, N) }, (_, i) => Math.round(((i + 1) * N) / Math.min(SAMPLES, N)) - 1));
  const ix = makeIndex(facts, names), snaps = new Map(), renames = new Map(), curve = [], seen = new Set();
  let events = 0, parsesDiff = 0, bytesDiff = 0, naiveParses = 0, bytesUnique = 0, replayMs = 0;
  const step = Math.max(1, Math.ceil(N / 40));
  timeline.forEach((c, t) => {
    const gone = new Map(); let ren;
    for (const [path, oid, p] of c.changes) { events++; if (oid) { lastOid.set(path, oid); parsesDiff++; bytesDiff += size.get(oid) ?? 0; if (!seen.has(oid)) { seen.add(oid); bytesUnique += size.get(oid) ?? 0; } } else if (!p && ix.tree.has(path)) gone.set(ix.tree.get(path), path); }
    if (gone.size) for (const [path, oid, p] of c.changes) if (oid && !p && !ix.tree.has(path) && gone.has(oid)) (ren ??= new Map()).set(gone.get(oid), path);
    if (ren) renames.set(t, ren);
    const t0 = now(); ix.apply(c, t); replayMs += now() - t0;
    naiveParses += ix.tree.size;
    if (sampleAt.has(t)) snaps.set(t, { treeHash: hashOf([...ix.tree].map(([p, o]) => p + " " + o)), edgeHash: hashOf([...ix.edges.keys()]), edges: ix.edges.size, files: ix.tree.size });
    if (t % step === step - 1 || t === N - 1) curve.push({ commits: t + 1, events, unique: seen.size, naive: naiveParses, replayMs: +replayMs.toFixed(1) });
  });
  R.work = { changeEvents: events, parsesDiffDriven: parsesDiff, parsesMemoised: seen.size, parsesNaive: naiveParses, mbDiffDriven: +(bytesDiff / 1e6).toFixed(1), mbMemoised: +(bytesUnique / 1e6).toFixed(1), sourceBlobs: facts.size, packageJsonBlobs: names.size, filesAtHead: ix.tree.size, refsAtHead: ix.refs.size };
  R.curve = curve;
  R.resolvesReverse = ix.st.resolves;
  const all = ix.intervals.map(([k, a, r]) => [k, a, r]).concat([...ix.openAt].map(([k, a]) => [k, a, Infinity]));
  R.edges = { intervalsClosed: ix.intervals.length, openAtHead: ix.openAt.size, packageLinksAtHead: [...ix.openAt.keys()].filter((k) => isPkg(k.split("\0")[1])).length };
  if (!single) { // links whose two ends are in different repositories
    const cross = all.filter(([k]) => { const [f, t] = k.split("\0"); return f.split("/")[0] !== t.split("/")[0]; });
    const repoNo = new Map(repos.map((r, i) => [basename(r), i + 1])), pairs = new Set(), byPkg = new Map();
    for (const [k] of cross) { const [f, t] = k.split("\0"); pairs.add(repoNo.get(f.split("/")[0]) + ">" + repoNo.get(t.split("/")[0])); const n = names.get(headTreeOrLast(t)) ?? "(file)"; byPkg.set(n, (byPkg.get(n) ?? 0) + 1); }
    R.crossRepo = { ever: cross.length, openAtHead: cross.filter(([, , r]) => r === Infinity).length, repoPairs: pairs.size, topTargets: [...byPkg].sort((a, b) => b[1] - a[1]).slice(0, 5), firstAddedStep: Math.min(...cross.map(([, a]) => a)) + 1 };
  }

  // Check 1: intervals reproduce every snapshot. Check 2: the final state equals a from-scratch index of each HEAD.
  for (const [t, s] of snaps) assert.equal(hashOf(all.filter(([, a, r]) => a <= t && t < r).map(([k]) => k)), s.edgeHash, `intervals disagree with replay at step ${t}`);
  const headTree = new Map(repos.flatMap((r) => [...lsTree(r, "HEAD", prefixOf(r))]));
  assert.equal(hashOf([...headTree].map(([p, o]) => p + " " + o)), hashOf([...ix.tree].map(([p, o]) => p + " " + o)), "replayed tree differs from git ls-tree HEAD");
  assert.equal(hashOf([...fromScratch(headTree, facts, names)]), hashOf([...ix.edges.keys()]), "replayed edges differ from a from-scratch index of HEAD");
  R.checks = { intervalSnapshots: snaps.size, headTree: "equal", headEdges: "equal" };
  log(`checks: ${snaps.size} interval snapshots ok, HEAD tree and edges equal from-scratch`);

  // Edge lifetimes (Kaplan-Meier, open edges censored). Exact-content renames are stitched so a move is not a death.
  {
    const openedAt = new Map(); all.forEach(([k, a], i) => openedAt.set(a + "\0" + k, i));
    const next = new Map(), hasPrev = new Set();
    all.forEach(([k, , r], i) => { const ren = renames.get(r); if (!ren) return; const [f, to] = k.split("\0"), f2 = ren.get(f) ?? f, t2 = ren.get(to) ?? to; if (f2 === f && t2 === to) return; const j = openedAt.get(r + "\0" + f2 + "\0" + t2); if (j !== undefined) { next.set(i, j); hasPrev.add(j); } });
    const day = (t) => timeline[t].date / 86400, end = day(N - 1), obs = [];
    all.forEach(([, a], i) => { if (hasPrev.has(i)) return; let j = i; while (next.has(j)) j = next.get(j); const r = all[j][2]; obs.push(r === Infinity ? [end - day(a), 0] : [day(r) - day(a), 1]); });
    obs.sort((x, y) => x[0] - y[0]);
    let S = 1, atRisk = obs.length, medianDays = null; const km = [];
    for (let i = 0; i < obs.length;) { let d = 0, n = 0; const t = obs[i][0]; while (i < obs.length && obs[i][0] === t) { d += obs[i][1]; n++; i++; } if (d) { S *= 1 - d / atRisk; km.push([t, S]); if (medianDays === null && S <= 0.5) medianDays = +t.toFixed(1); } atRisk -= n; }
    const at = (d) => { let s = 1; for (const [t, v] of km) { if (t > d) break; s = v; } return +s.toFixed(4); };
    R.lifetimes = { edges: obs.length, removed: obs.filter((o) => o[1]).length, renameStitched: next.size, spanDays: Math.round(end - day(0)), medianDays, survival: [7, 30, 90, 180, 365, 730, 1095, 1460].filter((d) => d <= end - day(0)).map((d) => ({ days: d, alive: at(d) })), curve: Array.from({ length: 49 }, (_, i) => { const d = ((end - day(0)) * i) / 48; return [Math.round(d), at(d)]; }) };
  }

  // Time travel: the edge set at a random step, by scanning every interval and by anchor + deltas.
  {
    const adds = Array.from({ length: N }, () => []), rems = Array.from({ length: N + 1 }, () => []);
    all.forEach(([, a, r], i) => { adds[a].push(i); if (r !== Infinity) rems[r].push(i); });
    const qs = Array.from({ length: 300 }, (_, i) => (i * 7919 + 13) % N);
    const scan = (t) => { let n = 0; for (const [, a, r] of all) if (a <= t && t < r) n++; return n; };
    const best = (fn) => { let b = Infinity, out; for (let i = 0; i < 3; i++) { const t0 = now(); out = qs.map(fn); b = Math.min(b, (now() - t0) / qs.length); } return [out, b]; };
    const [want, scanMs] = best(scan);
    R.timeTravel = { intervals: all.length, scanMsPerQuery: +scanMs.toFixed(4), anchors: [] };
    for (const K of [64, 256, 1024, 4096]) {
      if (K > N) continue;
      const anchors = [], live = new Set(); let entries = 0;
      for (let t = 0; t < N; t++) { for (const i of rems[t]) live.delete(i); for (const i of adds[t]) live.add(i); if (t % K === K - 1) { anchors.push(Int32Array.from(live)); entries += live.size; } }
      const query = (t) => { const a = Math.floor((t + 1) / K) - 1, s = new Set(a >= 0 ? anchors[a] : []); for (let u = (a + 1) * K; u <= t; u++) { for (const i of rems[u]) s.delete(i); for (const i of adds[u]) s.add(i); } return s.size; };
      const [got, ms] = best(query);
      assert.deepEqual(got, want, `anchors K=${K} disagree with scan`);
      R.timeTravel.anchors.push({ K, anchors: anchors.length, entries, msPerQuery: +ms.toFixed(4) });
    }
  }

  if (!QUICK) {
    // Ablation: no reverse index. Must write exactly the same intervals.
    {
      let b, ms = Infinity;
      for (let i = 0; i < 3; i++) { b = makeIndex(facts, names, false); const t0 = now(); timeline.forEach((c, t) => b.apply(c, t)); ms = Math.min(ms, now() - t0); }
      const key = (x) => hashOf(x.intervals.map((i) => i.join(" ")).concat([...x.openAt].map((e) => e.join(" "))));
      assert.equal(key(b), key(ix), "rescan ablation wrote different intervals");
      R.ablation = { rescanMs: Math.round(ms), rescanResolves: b.st.resolves, reverseMs: R.time.replay.min, reverseResolves: ix.st.resolves }; // both are best-of-n
      log(`ablation: rescan ${ms | 0} ms / ${b.st.resolves} resolves, reverse ${R.time.replay.min} ms / ${ix.st.resolves} resolves, same intervals`);
    }
    // Extraction: how it scales with worker count, and how much of it is git reading blobs.
    {
      const r0 = repos.reduce((a, r, i) => (last.per[i].length > last.per[a].length ? i : a), 0), src = new Set(), pkg = new Set();
      for (const c of last.per[r0]) for (const [, oid, p] of c.changes) if (oid) (p ? pkg : src).add(oid);
      const time = async (P, parse) => { let b = Infinity; for (let i = 0; i < REPS; i++) { const t0 = now(); await extractAll(repos[r0], [...src], [...pkg], P, { facts: new Map(), names: new Map(), size: new Map() }, parse); b = Math.min(b, now() - t0); } return Math.round(b); };
      R.extraction = { blobs: src.size + pkg.size, readOnlyMs: await time(1, false), byWorkers: [] };
      for (const P of [1, 2, 4, 8]) R.extraction.byWorkers.push({ workers: P, ms: await time(P, true) });
      log("extraction:", JSON.stringify(R.extraction));
    }
    if (single) {
      const repo = repos[0];
      // Baseline: index sampled commits from nothing, and compare each with the replayed state at that commit.
      const samples = [];
      for (const t of [...sampleAt].sort((a, b) => a - b)) {
        let n = await naiveIndex(repo, timeline[t].sha); const again = await naiveIndex(repo, timeline[t].sha), s = snaps.get(t); if (again.ms < n.ms) n = again;
        assert.equal(n.treeHash, s.treeHash, `tree differs from git at ${timeline[t].sha}`);
        assert.equal(n.edgeHash, s.edgeHash, `edges differ from a from-scratch index at ${timeline[t].sha}`);
        samples.push({ step: t + 1, files: n.files, edges: n.edges, ms: Math.round(n.ms) });
      }
      const meanMs = samples.reduce((a, s) => a + s.ms, 0) / samples.length;
      R.naive = { samples, sampledCommits: samples.length, meanMsPerCommit: Math.round(meanMs), extrapolatedTotalS: Math.round((meanMs * N) / 1000), allEqualReplay: true };
      log(`naive: ${samples.length} sampled commits indexed from scratch, all equal to replay; mean ${meanMs | 0} ms, extrapolated ${R.naive.extrapolatedTotalS} s for ${N} commits`);
      // Incremental update: the last K commits arrive after everything before them is indexed.
      const K = 100;
      if (N > 2 * K) {
        const base = makeIndex(facts, names), before = new Set();
        for (let t = 0; t < N - K; t++) { base.apply(timeline[t], t); for (const [, oid] of timeline[t].changes) if (oid) before.add(oid); }
        const t0 = now();
        const fresh = await walk(repo, "", `${timeline[N - K - 1].sha}..HEAD`);
        assert.equal(fresh.length, K);
        const src = new Set(), pkg = new Set();
        for (const c of fresh) for (const [, oid, p] of c.changes) if (oid && !before.has(oid)) (p ? pkg : src).add(oid);
        const into = { facts: new Map(), names: new Map(), size: new Map() };
        await extractAll(repo, [...src], [...pkg], 1, into);
        for (const [o, f] of into.facts) assert.deepEqual(f, facts.get(o));
        fresh.forEach((c, i) => base.apply(c, N - K + i));
        const ms = now() - t0;
        assert.equal(hashOf([...base.edges.keys()]), hashOf([...ix.edges.keys()]), "incremental update ended in a different state");
        const t1 = now(); execFileSync("git", ["-C", repo, "rev-parse", "HEAD"]);
        R.incremental = { commits: K, newBlobs: src.size + pkg.size, ms: Math.round(ms), noChangeMs: +(now() - t1).toFixed(1) };
      }
      // How much the blob cache saves when every commit is indexed, not only the first-parent chain.
      const every = await walk(repo, "", "HEAD", ["--diff-merges=first-parent"]);
      let ev = 0; const uniq = new Set();
      for (const c of every) for (const [, oid] of c.changes) if (oid) { ev++; uniq.add(oid); }
      R.allCommits = { commits: every.length, parsesDiffDriven: ev, parsesMemoised: uniq.size };
      R.head = timeline[N - 1].sha;
    }
  }
  R.peakRssMB = Math.round(process.resourceUsage().maxRSS / 1024);
  const out = JSON.stringify(R, null, 1);
  const file = arg("json", null);
  if (file) writeFileSync(file, out); else console.log(out);
  log(`done. peak RSS ${R.peakRssMB} MB`);
} else if (isMainThread) {
  console.log("self-check passed. usage: node bench.mjs <repo | folder of repos> [--runs 5] [--samples 24] [--par 4] [--quick] [--json out.json]");
}
