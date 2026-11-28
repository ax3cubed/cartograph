// How far a spec and its code disagree (board A3), from what the index answers: each operation of a spec is served by
// a parsed route or it isn't, each route of a spec's service that no spec lists is undocumented, and the newest spec
// version may have removed something callers still read. Pure, so `pnpm check` tests it under node.
import type { Caller, Contract, ContractVersion } from "../../lib/api/types.gen.ts";

export type Status = "sync" | "spec-only" | "undocumented";
export type Tone = "ok" | "warn" | "bad" | "acc" | "";
export const STATUS: Record<Status, { label: string; tone: Tone; rank: number }> = {
  undocumented: { label: "Undocumented", tone: "warn", rank: 1 },
  "spec-only": { label: "Spec-only", tone: "warn", rank: 2 },
  sync: { label: "In sync", tone: "ok", rank: 3 },
};

export interface Row {
  key: string; spec: string; method: string; path: string;
  /** The spec's own id for the operation, and its summary: spec text, so untrusted. */
  name: string; summary?: string;
  handler?: string; callers: Caller[]; status: Status;
}

/** Rows of one spec, worst status first; the order inside a status is the spec's. */
export function specRows(spec: Contract): Row[] {
  const rows: Row[] = [
    ...spec.operations.map((o): Row => ({
      key: o.id, spec: spec.id, method: o.method, path: o.path, name: o.operation_id ?? o.id, summary: o.summary ?? undefined,
      handler: o.handler ?? undefined, callers: o.callers, status: o.status === "sync" ? "sync" : "spec-only",
    })),
    ...spec.undocumented.map((u): Row => ({ key: `${spec.id}.${u.method} ${u.path}`, spec: spec.id, method: u.method, path: u.path, name: "not in spec", handler: u.handler, callers: [], status: "undocumented" })),
  ];
  return rows.sort((a, b) => STATUS[a.status].rank - STATUS[b.status].rank);
}

/** The newest version of a spec removed something callers still read (versions come oldest first). */
export const breakingOf = (versions: ContractVersion[] | undefined): ContractVersion | undefined => {
  const last = versions?.at(-1);
  return last && last.breaking_count > 0 ? last : undefined;
};

/** Severity order: breaking change, undocumented, spec-only, in sync. */
export function specRisk(spec: Contract, versions?: ContractVersion[]) {
  const breaking = breakingOf(versions)?.breaking_count ?? 0;
  const { undocumented, spec_only: specOnly } = spec.counts;
  const rank = breaking ? 0 : undocumented ? 1 : specOnly ? 2 : 3;
  return { rank, breaking, undocumented, specOnly };
}

export const riskText = (r: ReturnType<typeof specRisk>) =>
  [r.breaking && `${r.breaking} breaking ${r.breaking === 1 ? "change" : "changes"}`, r.undocumented && `${r.undocumented} undocumented`, r.specOnly && `${r.specOnly} spec-only`].filter(Boolean).join(", ") || "in sync";
