// FE-12: what the map and the table both show, as pure functions over one slice, so the two can't disagree
// (table.check.ts holds them to it). The map draws `shown(...)`; the table lists the same rows, sorted.
import type { GraphSlice } from "../../lib/api/types.gen.ts";

export type Shown = { nodes: number[]; links: number[] };

/** The files matching `filter` (a case-insensitive part of the path) and the links between two of them. */
export function shown(s: GraphSlice, filter: string): Shown {
  const f = filter.trim().toLowerCase();
  const on = s.nodes.name.map((n) => !f || n.toLowerCase().includes(f));
  const nodes: number[] = [], links: number[] = [];
  on.forEach((v, i) => { if (v) nodes.push(i); });
  s.links.from.forEach((a, j) => { if (on[a] && on[s.links.to[j]]) links.push(j); });
  return { nodes, links };
}

/** Incoming and outgoing link counts per file, over the whole slice (a filter hides rows, not links). */
export function degrees(s: GraphSlice) {
  const inn = new Array<number>(s.nodes.id.length).fill(0), out = new Array<number>(s.nodes.id.length).fill(0);
  s.links.from.forEach((a, j) => { out[a]++; inn[s.links.to[j]]++; });
  return { inn, out };
}

/** The files linked to `i`: the ones it imports, then the ones importing it. */
export function linked(s: GraphSlice, i: number) {
  const out: number[] = [], inn: number[] = [];
  s.links.from.forEach((a, j) => { if (a === i) out.push(s.links.to[j]); if (s.links.to[j] === i) inn.push(a); });
  return { out, in: inn };
}

export const topFolder = (path: string) => (path.includes("/") ? path.slice(0, path.indexOf("/")) : ".");

export type Dir = "ascending" | "descending";
/** `rows` sorted by `key` (numbers or strings), ties by index so the order is stable across renders. */
export function sortRows(rows: number[], key: (i: number) => string | number, dir: Dir) {
  const k = dir === "ascending" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const x = key(a), y = key(b);
    const c = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
    return c * k || a - b;
  });
}

/** One move of the keyboard walk, as the polite live region says it. */
export const sayFile = (name: string, inn: number, out: number) => `file ${name}, ${inn} incoming, ${out} outgoing`;
export const sayFolder = (name: string, files: number) => `folder ${name}, ${files} ${files === 1 ? "file" : "files"}`;
