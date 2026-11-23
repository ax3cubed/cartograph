// FE-12: the table's rows equal the map's elements for random slices and filter states. Run with `pnpm check`.
import assert from "node:assert/strict";
import { degrees, shown, sortRows, type Dir } from "./table.ts";
import type { GraphSlice } from "../../lib/api/types.gen.ts";

let seed = 7;
const rnd = (n: number) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };
const parts = ["api", "billing", "src", "lib", "events", "money", "Pay", "util"];

function slice(): GraphSlice {
  const n = 1 + rnd(60), m = rnd(120);
  const name = Array.from({ length: n }, (_, i) => `${parts[rnd(parts.length)]}/${parts[rnd(parts.length)]}/f${i}.ts`);
  const from = Array.from({ length: m }, () => rnd(n)), to = Array.from({ length: m }, () => rnd(n));
  return {
    lod: "file",
    nodes: { id: name.map((_, i) => `n${i}`), kind: name.map(() => "file"), name, parent: name.map(() => -1), children: name.map(() => 0), size: name.map(() => 1), x: name.map(() => 0), y: name.map(() => 0) },
    links: { id: from.map((_, j) => `l${j}`), from, to, kind: from.map(() => "imports"), level: from.map(() => "exact"), score: from.map(() => rnd(100) / 100), weight: from.map(() => 1), added_seq: from.map(() => 0), removed_seq: from.map(() => -1) },
  };
}

const same = (a: number[], b: number[]) => assert.deepEqual([...a].sort((x, y) => x - y), [...b].sort((x, y) => x - y));

for (let t = 0; t < 500; t++) {
  const s = slice();
  const f = ["", "billing", "BILL", "src/", "pay", "zzz", " lib "][rnd(7)];
  const map = shown(s, f); // what LiveMap draws
  const deg = degrees(s);
  const dir: Dir = rnd(2) ? "ascending" : "descending";
  // what TableView lists, under every sort it offers
  const fileKeys = [(i: number) => s.nodes.name[i], (i: number) => deg.out[i], (i: number) => deg.inn[i]];
  const linkKeys = [(j: number) => s.nodes.name[s.links.from[j]], (j: number) => s.links.score[j]];
  for (const k of fileKeys) same(sortRows(map.nodes, k, dir), map.nodes);
  for (const k of linkKeys) same(sortRows(map.links, k, dir), map.links);
  // and both match the filter's meaning, worked out the slow way
  const want = s.nodes.name.map((n, i) => [n, i] as const).filter(([n]) => n.toLowerCase().includes(f.trim().toLowerCase())).map(([, i]) => i);
  same(map.nodes, want);
  same(map.links, s.links.from.map((a, j) => [a, j]).filter(([a, j]) => want.includes(a) && want.includes(s.links.to[j])).map(([, j]) => j));
}

// sorting is stable and direction-aware
assert.deepEqual(sortRows([0, 1, 2, 3], (i) => [2, 1, 2, 0][i], "ascending"), [3, 1, 0, 2]);
assert.deepEqual(sortRows([0, 1, 2, 3], (i) => [2, 1, 2, 0][i], "descending"), [0, 2, 1, 3]);

console.log("table: ok");
