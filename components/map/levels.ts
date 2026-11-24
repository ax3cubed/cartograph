// FE-08 level of detail as pure functions over slices, so they run under node in `pnpm check` (levels.check.ts). The
// index serves each level (SRV-04: lod=service, module or file) with its stored positions (IDX-25, I2), and the map
// draws those. An index with no layout rows yet (migrated, not synced again) serves x = y = 0: then, and only then,
// the service and module levels are rolled up here from the file slice and the workspace's structure (services and
// modules from …/nodes/{id}/children), and laid out here.
import type { GraphSlice, Lod, Node } from "../../lib/api/types.gen.ts";

export type Level = Exclude<Lod, "workspace">;
export const LEVELS: Level[] = ["service", "module", "file"];

/**
 * Board 00: service below 60%, module 60–140%, file above 140%. Zoom is relative to the map's fit, which is drawn at
 * 50%: the workspace opens at service level, and two zoom steps in reach modules.
 */
export const zoomOf = (ratio: number) => 50 / ratio;
export const levelFor = (ratio: number): Level => {
  const z = zoomOf(ratio);
  return z < 60 ? "service" : z <= 140 ? "module" : "file";
};

/** Width of a bundle of `count` links: proportional to its square root (DESIGN.md link encoding). */
export const bundleWidth = (count: number, unit = 1) => unit * Math.sqrt(Math.max(1, count));

/** The structure a file slice rolls up into: services (top) and modules (inside a service). */
export type Holder = Pick<Node, "id" | "kind" | "name" | "path"> & { service: string };

/** Whether a slice carries stored positions (I2): an index with no layout rows serves every node at 0, 0. */
export const hasLayout = (s: GraphSlice | undefined) => !!s && s.nodes.x.some((x, i) => x !== 0 || s.nodes.y[i] !== 0);
/** Whether node `i` of `s` has a stored position. */
export const placed = (s: GraphSlice, i: number) => s.nodes.x[i] !== 0 || s.nodes.y[i] !== 0;

export interface Structure {
  services: Holder[];
  modules: Holder[];
}

const dirsOf = (p: string) => {
  const out: string[] = [];
  for (let i = p.lastIndexOf("/"); i > 0; i = p.lastIndexOf("/", i - 1)) out.push(p.slice(0, i));
  out.push("");
  return out;
};

/** For each file of `s`, the index of its module (or -1) and of its service (or -1), by the longest path prefix. */
export function owners(s: GraphSlice, st: Structure) {
  const mod = new Map(st.modules.map((m, i) => [m.path, i]));
  const svc = new Map(st.services.map((m, i) => [m.path, i]));
  const n = s.nodes.id.length;
  const module = new Int32Array(n).fill(-1), service = new Int32Array(n).fill(-1);
  for (let i = 0; i < n; i++) {
    for (const d of dirsOf(s.nodes.name[i])) {
      if (module[i] < 0 && mod.has(d)) module[i] = mod.get(d)!;
      if (svc.has(d)) { service[i] = svc.get(d)!; break; }
    }
    // a module's own service wins over a path guess (a module path can sit outside its service's root)
    if (module[i] >= 0) {
      const k = st.services.findIndex((x) => x.id === st.modules[module[i]].service);
      if (k >= 0) service[i] = k;
    }
  }
  return { module, service };
}

const empty = (lod: Lod, at: GraphSlice["at"]): GraphSlice => ({
  lod, at, nodes: { id: [], kind: [], name: [], parent: [], x: [], y: [], size: [], children: [] },
  links: { id: [], from: [], to: [], kind: [], level: [], score: [], weight: [], added_seq: [], removed_seq: [] },
});

const RANK = { guessed: 0, likely: 1, exact: 2 } as const;

/**
 * One level of `s`. At service level each file stands for its service; at module level for its module, or its
 * service when it sits in no module (loose files). A link between two files becomes one link between the nodes that
 * hold them, `weight` counting the file links; links inside one node are dropped. Node `size` is its file count.
 */
export function rollUp(s: GraphSlice, st: Structure, level: Level, own = owners(s, st)): GraphSlice {
  if (level === "file") return s;
  const out = empty(level, s.at);
  const col = new Map<string, number>();
  const add = (h: Holder) => {
    let i = col.get(h.id);
    if (i !== undefined) return i;
    i = out.nodes.id.length;
    col.set(h.id, i);
    out.nodes.id.push(h.id); out.nodes.kind.push(h.kind); out.nodes.name.push(h.name);
    out.nodes.parent.push(-1); out.nodes.x.push(0); out.nodes.y.push(0); out.nodes.size.push(0); out.nodes.children.push(0);
    return i;
  };
  const of = new Int32Array(s.nodes.id.length).fill(-1);
  const counted = new Set<string>(); // service:module pairs already counted as a child
  for (let f = 0; f < of.length; f++) {
    const sv = own.service[f], md = own.module[f];
    if (sv < 0 && md < 0) continue;
    const holder = level === "module" && md >= 0 ? st.modules[md] : sv >= 0 ? st.services[sv] : st.modules[md];
    const i = add(holder);
    of[f] = i;
    out.nodes.size[i]++;
    if (level === "module" && md >= 0 && sv >= 0) {
      const p = add(st.services[sv]);
      if (out.nodes.parent[i] < 0) { out.nodes.parent[i] = p; out.nodes.children[p]++; }
    } else if (level === "service" && md >= 0) {
      // children: the modules that have a file at this date
      const key = `${i}:${md}`;
      if (!counted.has(key)) { counted.add(key); out.nodes.children[i]++; }
    }
  }
  const pair = new Map<string, number>();
  for (let j = 0; j < s.links.id.length; j++) {
    const a = of[s.links.from[j]], b = of[s.links.to[j]];
    if (a < 0 || b < 0 || a === b) continue;
    const k = `${a}>${b}`;
    const at = pair.get(k);
    if (at !== undefined) {
      out.links.weight[at] += s.links.weight[j];
      out.links.added_seq[at] = Math.min(out.links.added_seq[at], s.links.added_seq[j]);
      if (RANK[s.links.level[j]] > RANK[out.links.level[at]]) { out.links.level[at] = s.links.level[j]; out.links.score[at] = s.links.score[j]; }
      continue;
    }
    pair.set(k, out.links.id.length);
    out.links.id.push(`${out.nodes.id[a]}>${out.nodes.id[b]}`);
    out.links.from.push(a); out.links.to.push(b);
    out.links.kind.push(s.links.kind[j]); out.links.level.push(s.links.level[j]); out.links.score.push(s.links.score[j]);
    out.links.weight.push(s.links.weight[j]); out.links.added_seq.push(s.links.added_seq[j]); out.links.removed_seq.push(-1);
  }
  return out;
}

// ---------- layout (I2) ----------

export type Pt = { x: number; y: number };
export type Disc = Pt & { r: number };

const GOLDEN = Math.PI * (3 - Math.sqrt(5));
const STEP = 1; // spacing between files in a module, in layout units (the indexer's file radius is 1 too)
/** A file's drawn radius. */
export const FILE_R = STEP * 0.32;
export const hash = (s: string) => { let h = 2166136261; for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619); return h >>> 0; };

/** Circles on shelves, largest first then by key, centred on the origin. Returns centres in input order. */
export function shelves(items: { key: string; r: number }[], gap: number): Pt[] {
  const order = items.map((_, i) => i).sort((a, b) => items[b].r - items[a].r || (items[a].key < items[b].key ? -1 : 1));
  const area = items.reduce((a, c) => a + (2 * c.r + gap) ** 2, 0);
  const width = items.length ? Math.max(2 * items[order[0]].r + gap, Math.sqrt(area) * 1.15) : 0;
  const pos: Pt[] = new Array(items.length);
  let x = 0, y = 0, row = 0;
  for (const i of order) {
    const d = 2 * items[i].r + gap;
    if (x > 0 && x + d > width) { x = 0; y += row; row = 0; }
    pos[i] = { x: x + d / 2, y: y + d / 2 };
    x += d; row = Math.max(row, d);
  }
  const w = Math.max(0, ...pos.map((p, i) => p.x + items[i].r)), h = Math.max(0, ...pos.map((p, i) => p.y + items[i].r));
  return pos.map((p) => ({ x: p.x - w / 2, y: p.y - h / 2 }));
}

/** A file's place inside a disc: a sunflower by rank, or by its id's hash when it has no rank. */
const seat = (c: Disc, rank: number, id: string) => {
  const k = rank >= 0 ? rank : hash(id) % 997;
  const rr = STEP * Math.sqrt(k + 0.5), a = k * GOLDEN;
  const f = Math.min(1, (c.r * 0.92) / Math.max(rr, 1e-9));
  return { x: c.x + rr * Math.min(1, f) * Math.cos(a), y: c.y + rr * Math.min(1, f) * Math.sin(a) };
};

export interface Layout {
  services: Map<string, Disc>;
  modules: Map<string, Disc>;
  /** File id -> position, for the files in the slice the layout was made from. */
  files: Map<string, Pt>;
  /** Every disc's bounding box, the same at all three levels so the camera never jumps when a level swaps. */
  box: { x: [number, number]; y: [number, number] };
}

/** The service and module slices the index serves, now: their stored discs replace the ones computed here. */
export interface Served {
  service?: GraphSlice;
  module?: GraphSlice;
}

/**
 * Positions for all three levels from one slice (the workspace now) and its structure. Stored positions (I2) win:
 * a file the slice places sits there, and when `served` carries the service and module levels with positions, their
 * discs are the index's. Otherwise, bottom-up: a module's radius fits its files, a service's fits its modules on
 * shelves, then services on shelves; files take sunflower seats ranked by a hash of their id, so a file keeps its
 * seat as others come and go. Deterministic: the same index gives the same positions every time it is opened.
 */
export function layout(s: GraphSlice, st: Structure, own = owners(s, st), served?: Served): Layout {
  const perMod = new Map<number, number[]>(), loose = new Map<number, number[]>();
  for (let f = 0; f < s.nodes.id.length; f++) {
    const m = own.module[f], v = own.service[f];
    if (m >= 0) (perMod.get(m) ?? perMod.set(m, []).get(m)!).push(f);
    else if (v >= 0) (loose.get(v) ?? loose.set(v, []).get(v)!).push(f);
  }
  const fileR = (n: number) => STEP * Math.sqrt(n + 1) + STEP;
  const modR = st.modules.map((_, m) => fileR(perMod.get(m)?.length ?? 0));
  const modsOf = st.services.map((sv) => st.modules.map((m, i) => (m.service === sv.id ? i : -1)).filter((i) => i >= 0));
  // inside a service: its modules plus one disc for its loose files
  const inner = st.services.map((sv, k) => {
    const items = [...modsOf[k].map((m) => ({ key: st.modules[m].id, r: modR[m] })), { key: `${sv.id}:loose`, r: fileR(loose.get(k)?.length ?? 0) }];
    const at = shelves(items, STEP);
    const r = Math.max(...items.map((it, i) => Math.hypot(at[i].x, at[i].y) + it.r)) + STEP;
    return { items, at, r };
  });
  const top = shelves(st.services.map((sv, k) => ({ key: sv.id, r: inner[k].r })), STEP * 4);
  const services = new Map<string, Disc>(), modules = new Map<string, Disc>(), files = new Map<string, Pt>();
  const looseDisc = new Map<number, Disc>();
  st.services.forEach((sv, k) => {
    const c = { ...top[k], r: inner[k].r };
    services.set(sv.id, c);
    inner[k].items.forEach((it, i) => {
      const d = { x: c.x + inner[k].at[i].x, y: c.y + inner[k].at[i].y, r: it.r };
      if (i < modsOf[k].length) modules.set(it.key, d); else looseDisc.set(k, d);
    });
  });
  const place = (list: number[] | undefined, d: Disc | undefined) => {
    if (!list || !d) return;
    [...list].sort((a, b) => hash(s.nodes.id[a]) - hash(s.nodes.id[b]) || (s.nodes.id[a] < s.nodes.id[b] ? -1 : 1))
      .forEach((f, rank) => files.set(s.nodes.id[f], placed(s, f) ? { x: s.nodes.x[f], y: s.nodes.y[f] } : seat(d, rank, s.nodes.id[f])));
  };
  // the index's discs replace these when it serves them: one plane, never two mixed
  if (hasLayout(served?.service)) {
    services.clear(); modules.clear();
    for (const lv of [served!.service!, served!.module]) {
      lv?.nodes.id.forEach((id, i) => {
        if (placed(lv, i)) (lv.nodes.kind[i] === "module" ? modules : services).set(id, { x: lv.nodes.x[i], y: lv.nodes.y[i], r: lv.nodes.size[i] });
      });
    }
    // loose files sit in their service's disc
    st.services.forEach((sv, k) => { const d = services.get(sv.id); if (d) looseDisc.set(k, d); else looseDisc.delete(k); });
  }
  for (const [m, list] of perMod) place(list, modules.get(st.modules[m].id));
  for (const [k, list] of loose) place(list, looseDisc.get(k));
  const all = [...services.values()];
  const box = all.length
    ? { x: [Math.min(...all.map((d) => d.x - d.r)), Math.max(...all.map((d) => d.x + d.r))] as [number, number], y: [Math.min(...all.map((d) => d.y - d.r)), Math.max(...all.map((d) => d.y + d.r))] as [number, number] }
    : { x: [-1, 1] as [number, number], y: [-1, 1] as [number, number] };
  return { services, modules, files, box };
}

/** Where node `id` of kind `kind` sits; a file missing from the layout (gone by now) gets a seat in its holder's disc. */
export function placeOf(L: Layout, id: string, kind: string, holder?: string): Pt & { r: number } {
  if (kind === "service") return L.services.get(id) ?? { x: 0, y: 0, r: 1 };
  if (kind === "module") return L.modules.get(id) ?? { x: 0, y: 0, r: 1 };
  const p = L.files.get(id);
  if (p) return { ...p, r: FILE_R };
  const d = (holder && (L.modules.get(holder) ?? L.services.get(holder))) || { x: 0, y: 0, r: 4 };
  return { ...seat(d, -1, id), r: FILE_R };
}

// ---------- compare (I4) ----------

export type DiffState = "added" | "removed" | "changed" | "unchanged";

export interface Compared {
  /** Union of both slices; columns as in GraphSlice. Positions come from the layout, not from here. */
  slice: GraphSlice;
  node: DiffState[];
  link: DiffState[];
}

/**
 * Two slices of one level as one: what B has that A doesn't is added, what A had that B lost is removed, a link
 * whose weight differs is changed. Node ids and link ids are stable across dates (rolled-up link ids are made of
 * their ends), so the union is by id.
 */
export function compare(a: GraphSlice, b: GraphSlice): Compared {
  const out = empty(b.lod, b.at);
  const node: DiffState[] = [], link: DiffState[] = [];
  const col = new Map<string, number>();
  const take = (s: GraphSlice, i: number) => {
    const id = s.nodes.id[i];
    let c = col.get(id);
    if (c !== undefined) return c;
    c = out.nodes.id.length;
    col.set(id, c);
    for (const k of Object.keys(out.nodes) as (keyof GraphSlice["nodes"])[]) (out.nodes[k] as unknown[]).push((s.nodes[k] as unknown[])[i]);
    out.nodes.parent[c] = -1;
    return c;
  };
  const inA = new Set(a.nodes.id), inB = new Set(b.nodes.id);
  b.nodes.id.forEach((id, i) => { const c = take(b, i); node[c] = inA.has(id) ? "unchanged" : "added"; });
  a.nodes.id.forEach((id, i) => { if (!inB.has(id)) { const c = take(a, i); node[c] = "removed"; } });
  const wA = new Map(a.links.id.map((id, j) => [id, a.links.weight[j]]));
  const seen = new Set<string>();
  const put = (s: GraphSlice, j: number, st: DiffState) => {
    seen.add(s.links.id[j]);
    out.links.id.push(s.links.id[j]);
    out.links.from.push(col.get(s.nodes.id[s.links.from[j]])!);
    out.links.to.push(col.get(s.nodes.id[s.links.to[j]])!);
    out.links.kind.push(s.links.kind[j]); out.links.level.push(s.links.level[j]); out.links.score.push(s.links.score[j]);
    out.links.weight.push(s.links.weight[j]); out.links.added_seq.push(s.links.added_seq[j]); out.links.removed_seq.push(s.links.removed_seq[j]);
    link.push(st);
  };
  b.links.id.forEach((id, j) => put(b, j, !wA.has(id) ? "added" : wA.get(id) !== b.links.weight[j] ? "changed" : "unchanged"));
  a.links.id.forEach((id, j) => { if (!seen.has(id)) put(a, j, "removed"); });
  // a node whose links changed reads as changed, unless it came or went itself
  out.links.from.forEach((f, j) => {
    if (link[j] === "unchanged") return;
    for (const c of [f, out.links.to[j]]) if (node[c] === "unchanged") node[c] = "changed";
  });
  return { slice: out, node, link };
}
