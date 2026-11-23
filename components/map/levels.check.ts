// FE-08 and FE-09: the zoom-to-level rule, bundle widths, roll-ups, layout and compare. Run with `pnpm check`.
import assert from "node:assert/strict";
import { bundleWidth, compare, layout, levelFor, owners, placeOf, rollUp, type Structure } from "./levels.ts";
import type { GraphSlice } from "../../lib/api/types.gen.ts";

// zoom to level (board 00): fit is 50%; service below 60%, module 60–140%, file above
assert.equal(levelFor(1), "service");
assert.equal(levelFor(50 / 59.9), "service");
assert.equal(levelFor(50 / 60), "module");
assert.equal(levelFor(50 / 140), "module");
assert.equal(levelFor(50 / 141), "file");
assert.equal(levelFor(0.05), "file");

// bundle width: proportional to the square root of the count, never under one link's width
assert.equal(bundleWidth(1), 1);
assert.equal(bundleWidth(9, 2), 6);
assert.equal(bundleWidth(0), 1);

let seed = 11;
const rnd = (n: number) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };
const st: Structure = {
  services: [
    { id: "s1", kind: "service", name: "root", path: "", service: "s1" },
    { id: "s2", kind: "service", name: "billing", path: "billing", service: "s2" },
    { id: "s3", kind: "service", name: "shared", path: "shared", service: "s3" },
  ],
  modules: [
    { id: "m1", kind: "module", name: "pay", path: "billing/src/pay", service: "s2" },
    { id: "m2", kind: "module", name: "inv", path: "billing/src/inv", service: "s2" },
    { id: "m3", kind: "module", name: "ev", path: "shared/ev", service: "s3" },
    { id: "m4", kind: "module", name: "auth", path: "auth/src", service: "s1" },
  ],
};
const dirs = ["billing/src/pay", "billing/src/inv", "billing", "shared/ev", "shared", "auth/src", "auth", "tools"];

function slice(n: number, m: number, tag = ""): GraphSlice {
  const name = Array.from({ length: n }, (_, i) => `${dirs[rnd(dirs.length)]}/f${i}${tag}.ts`);
  const from = Array.from({ length: m }, () => rnd(n)), to = Array.from({ length: m }, () => rnd(n));
  return {
    lod: "file",
    nodes: { id: name.map((_, i) => `n${i}`), kind: name.map(() => "file"), name, parent: name.map(() => -1), children: name.map(() => 0), size: name.map(() => 1), x: name.map(() => 0), y: name.map(() => 0) },
    links: { id: from.map((_, j) => `l${j}`), from, to, kind: from.map(() => "imports"), level: from.map(() => "exact"), score: from.map(() => 1), weight: from.map(() => 1), added_seq: from.map((_, j) => j), removed_seq: from.map(() => -1) },
  };
}

// every file has an owner; a module's files belong to that module's service
{
  const s = slice(40, 0);
  const own = owners(s, st);
  s.nodes.name.forEach((p, i) => {
    assert.ok(own.service[i] >= 0, `${p} has a service`);
    if (p.startsWith("billing/src/pay/")) { assert.equal(st.modules[own.module[i]].id, "m1"); assert.equal(st.services[own.service[i]].id, "s2"); }
    if (p.startsWith("auth/src/")) assert.equal(st.services[own.service[i]].id, "s1", "a module's service wins over the path");
    if (p.startsWith("tools/")) { assert.equal(own.module[i], -1); assert.equal(st.services[own.service[i]].id, "s1"); }
  });
}

// property: a bundle's count is the sum of its member links, and every link crossing two holders is in one bundle
for (let t = 0; t < 200; t++) {
  const s = slice(1 + rnd(50), rnd(150));
  const own = owners(s, st);
  for (const level of ["service", "module"] as const) {
    const r = rollUp(s, st, level, own);
    const holder = (f: number) => (level === "module" && own.module[f] >= 0 ? st.modules[own.module[f]].id : st.services[own.service[f]].id);
    const want = new Map<string, number>();
    s.links.from.forEach((a, j) => {
      const x = holder(a), y = holder(s.links.to[j]);
      if (x !== y) want.set(`${x}>${y}`, (want.get(`${x}>${y}`) ?? 0) + 1);
    });
    const got = new Map(r.links.id.map((id, j) => [id, r.links.weight[j]]));
    assert.deepEqual(got, want, `${level} bundles equal their member links`);
    assert.equal(r.nodes.size.reduce((a, b) => a + b, 0), s.nodes.id.length, `${level} sizes count every file once`);
    r.links.from.forEach((a, j) => assert.notEqual(a, r.links.to[j], "no link inside one node"));
  }
}

// layout: the same input gives the same positions; files sit inside their module's disc; no two services overlap
{
  const s = slice(300, 0);
  const a = layout(s, st), b = layout(structuredClone(s), structuredClone(st));
  assert.deepEqual([...a.files], [...b.files], "deterministic");
  assert.deepEqual([...a.services], [...b.services]);
  const own = owners(s, st);
  s.nodes.id.forEach((id, f) => {
    const m = own.module[f];
    if (m < 0) return;
    const d = a.modules.get(st.modules[m].id)!, p = a.files.get(id)!;
    assert.ok(Math.hypot(p.x - d.x, p.y - d.y) <= d.r + 1e-9, `${id} inside its module`);
  });
  const sv = [...a.services.values()];
  for (let i = 0; i < sv.length; i++) for (let j = i + 1; j < sv.length; j++) assert.ok(Math.hypot(sv[i].x - sv[j].x, sv[i].y - sv[j].y) >= sv[i].r + sv[j].r - 1e-9, "services don't overlap");
  // a file gone by now still gets a seat inside its holder
  const gone = placeOf(a, "n-gone", "file", "m1"), d = a.modules.get("m1")!;
  assert.ok(Math.hypot(gone.x - d.x, gone.y - d.y) <= d.r, "a gone file sits in its module");
}

// compare: added, removed, changed and unchanged by id
{
  const A = slice(20, 30), B = structuredClone(A);
  // B drops file 0 (and its links), adds a file and a link, and doubles one link's weight
  const keep = B.links.from.map((f, j) => f !== 0 && B.links.to[j] !== 0);
  const cut = <T,>(xs: T[]) => xs.filter((_, j) => keep[j]);
  B.links = { id: cut(B.links.id), from: cut(B.links.from), to: cut(B.links.to), kind: cut(B.links.kind), level: cut(B.links.level), score: cut(B.links.score), weight: cut(B.links.weight), added_seq: cut(B.links.added_seq), removed_seq: cut(B.links.removed_seq) };
  B.nodes.id[0] = "n-new";
  B.links.id.push("l-new"); B.links.from.push(1); B.links.to.push(2); B.links.kind.push("imports"); B.links.level.push("exact"); B.links.score.push(1); B.links.weight.push(1); B.links.added_seq.push(99); B.links.removed_seq.push(-1);
  if (B.links.id.length > 1) B.links.weight[0] = 2;
  const c = compare(A, B);
  const st2 = (id: string) => c.node[c.slice.nodes.id.indexOf(id)];
  assert.equal(st2("n-new"), "added");
  assert.equal(st2("n0"), "removed");
  assert.equal(c.link[c.slice.links.id.indexOf("l-new")], "added");
  assert.equal(c.slice.links.id.length, new Set([...A.links.id, ...B.links.id]).size, "the union of both");
  for (let j = 0; j < A.links.id.length; j++) if (!keep[j]) assert.equal(c.link[c.slice.links.id.indexOf(A.links.id[j])], "removed");
  const same = compare(A, A);
  assert.ok(same.node.every((x) => x === "unchanged") && same.link.every((x) => x === "unchanged"), "a slice compared with itself is unchanged");
}

console.log("levels ok");
