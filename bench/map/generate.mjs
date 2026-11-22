// FE-07 fixture: a seeded 60,000-node, 200,000-link graph in 62 clusters with stored positions,
// plus the 62-node service-level graph of the same data (for the level-swap measurement).
//
//   node bench/map/generate.mjs <out-dir> [--nodes 60000] [--links 200000] [--clusters 62] [--seed 7]
//
// Output is columnar JSON, the payload format the plan defaults to (open question "Graph payload format").
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const args = process.argv.slice(2);
const out = args[0];
if (!out) throw new Error("usage: node generate.mjs <out-dir> [--nodes N] [--links N] [--clusters N] [--seed N]");
const opt = (name, def) => { const i = args.indexOf("--" + name); return i < 0 ? def : Number(args[i + 1]); };
const N = opt("nodes", 60000), E = opt("links", 200000), C = opt("clusters", 62), SEED = opt("seed", 7);

// mulberry32: small, seeded, good enough for a fixture
let s = SEED >>> 0;
const rand = () => { s = (s + 0x6d2b79f5) >>> 0; let t = s; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
const gauss = () => { let u = 0; while (!u) u = rand(); return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * rand()); };

// Cluster sizes: Zipf-like, so a few services are big and most are small, summing to N.
const w = Array.from({ length: C }, (_, i) => 1 / Math.pow(i + 1, 0.8));
const wsum = w.reduce((a, b) => a + b, 0);
const size = w.map((x) => Math.max(20, Math.floor((x / wsum) * N)));
size[0] += N - size.reduce((a, b) => a + b, 0);

// Cluster centres on a golden-angle spiral; cluster radius grows with sqrt(size).
const cx = [], cy = [], start = [];
let acc = 0;
for (let c = 0; c < C; c++) {
  const r = 60 * Math.sqrt(c + 0.5), a = c * 2.399963;
  cx.push(r * Math.cos(a)); cy.push(r * Math.sin(a));
  start.push(acc); acc += size[c];
}

const x = new Array(N), y = new Array(N), cluster = new Array(N), path = new Array(N);
for (let c = 0; c < C; c++) {
  const spread = 1.2 * Math.sqrt(size[c]);
  const modules = Math.max(1, Math.round(size[c] / 40));
  for (let k = 0; k < size[c]; k++) {
    const i = start[c] + k, m = k % modules;
    x[i] = +(cx[c] + gauss() * spread * 0.35).toFixed(2);
    y[i] = +(cy[c] + gauss() * spread * 0.35).toFixed(2);
    cluster[i] = c;
    path[i] = `svc-${c}/src/mod-${m}/file-${k}.ts`;
  }
}

// Links: 85% inside a cluster (mostly to nearby indices, as files in one module), 15% across clusters,
// biased to the first ten clusters (shared libraries). No self-links, no duplicates.
const from = [], to = [], seen = new Set();
const pickIn = (c) => start[c] + Math.floor(rand() * size[c]);
while (from.length < E) {
  const a = Math.floor(rand() * N), ca = cluster[a];
  let b;
  if (rand() < 0.85) {
    const near = rand() < 0.7;
    b = near ? Math.min(start[ca] + size[ca] - 1, Math.max(start[ca], a + Math.round(gauss() * 30))) : pickIn(ca);
  } else {
    const cb = rand() < 0.5 ? Math.floor(rand() * 10) : Math.floor(rand() * C);
    b = pickIn(cb);
  }
  if (a === b) continue;
  const key = a * N + b;
  if (seen.has(key)) continue;
  seen.add(key); from.push(a); to.push(b);
}

// Service level: one node per cluster, one weighted link per cluster pair.
const svcEdges = new Map();
for (let i = 0; i < E; i++) {
  const a = cluster[from[i]], b = cluster[to[i]];
  if (a === b) continue;
  const k = a * C + b; svcEdges.set(k, (svcEdges.get(k) ?? 0) + 1);
}
const svc = {
  schema: 1, level: "service",
  nodes: { id: size.map((_, c) => `svc-${c}`), x: cx.map((v) => +v.toFixed(2)), y: cy.map((v) => +v.toFixed(2)), cluster: size.map((_, c) => c), size },
  links: { from: [], to: [], weight: [] },
};
for (const [k, v] of svcEdges) { svc.links.from.push(Math.floor(k / C)); svc.links.to.push(k % C); svc.links.weight.push(v); }

const files = {
  schema: 1, level: "file", seed: SEED, clusters: C,
  nodes: { id: path, x, y, cluster },
  links: { from, to },
};

mkdirSync(out, { recursive: true });
const fFile = join(out, `files-${N}-${E}.json`), fSvc = join(out, `services-${C}.json`);
writeFileSync(fFile, JSON.stringify(files));
writeFileSync(fSvc, JSON.stringify(svc));
console.log(JSON.stringify({ files: fFile, services: fSvc, nodes: N, links: from.length, service_links: svc.links.from.length }));
