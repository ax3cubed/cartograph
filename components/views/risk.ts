import type { Spec } from "@/lib/model";
import { chipText } from "@/lib/model";
import { MISMATCH, UNDOCUMENTED, VERSIONS } from "@/lib/demo/views-extra";
import { toneOf, type Tone } from "./shared";

export type Status = "sync" | "spec-only" | "undocumented" | "mismatch";
export const STATUS: Record<Status, { label: string; tone: Tone; rank: number }> = {
  mismatch: { label: "Schema mismatch", tone: "bad", rank: 0 },
  undocumented: { label: "Undocumented", tone: "warn", rank: 2 },
  "spec-only": { label: "Spec-only", tone: "warn", rank: 3 },
  sync: { label: "In sync", tone: "ok", rank: 4 },
};
export interface Row { key: string; spec: string; method: string; path: string; name: string; handler?: string; callers: { node: string; chip: string; tone: Tone; note: string }[]; status: Status; diff?: { spec: string; handler: string } }

/** Rows of one spec, worst status first. */
export function specRows(spec: Spec): Row[] {
  const rows: Row[] = [
    ...spec.operations.map((o): Row => ({
      key: o.id, spec: spec.id, method: o.method, path: o.path, name: o.id, handler: o.handler,
      callers: o.callers.map((c) => ({ node: c.node, chip: chipText(c.confidence), tone: toneOf(c.confidence.score), note: c.confidence.evidence[0]?.note ?? "" })),
      status: MISMATCH[o.id] && o.handler ? "mismatch" : o.handler ? "sync" : "spec-only", diff: MISMATCH[o.id],
    })),
    ...(UNDOCUMENTED[spec.id] ?? []).map((u): Row => ({ key: `${spec.id}.${u.method}${u.path}`, spec: spec.id, method: u.method, path: u.path, name: "not in spec", handler: u.handler, callers: [], status: "undocumented" })),
  ];
  return rows.sort((a, b) => STATUS[a.status].rank - STATUS[b.status].rank);
}

/** The latest spec version removed something callers still read. */
export const breakingOf = (specId: string) => VERSIONS[specId]?.[0]?.breaking;

/** Severity order: schema mismatch, breaking, undocumented, spec-only, in sync. */
export function specRisk(spec: Spec) {
  const rows = specRows(spec);
  const n = (s: Status) => rows.filter((r) => r.status === s).length;
  const breaking = breakingOf(spec.id) ? 1 : 0;
  const rank = n("mismatch") ? 0 : breaking ? 1 : n("undocumented") ? 2 : n("spec-only") ? 3 : 4;
  return { rank, mismatch: n("mismatch"), breaking, undocumented: n("undocumented"), specOnly: n("spec-only") };
}
