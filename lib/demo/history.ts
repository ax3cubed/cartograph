// Demo history for the timeline (G04) and the file history panel (G10).
// A getter in source.ts can replace these later; nothing here reads the network.
import { SERVICES } from "../workspace.ts";

export const DAY = 86_400_000;
export const dayOf = (iso: string) => Date.parse(`${iso}T12:00:00Z`);
export const HISTORY_START = dayOf("2017-01-02");

/** Commits per day for one repo: weekdays only, busier each year. Deterministic, so reloads match. */
const perDay = (repoIdx: number, day: number) => {
  const wd = new Date(day).getUTCDay();
  if (wd === 0 || wd === 6) return 0;
  const n = Math.floor(day / DAY);
  const h = Math.imul(n * 31 + repoIdx * 97 + 7, 2654435761) >>> 0;
  const years = (day - HISTORY_START) / (365 * DAY);
  return Math.floor(((h % 1000) / 1000) * (1.2 + years * 0.45) * (1 + repoIdx * 0.1));
};

export interface Bin { start: number; end: number; total: number; byRepo: Record<string, number> }

/** Commit counts per bin between two days, split by repo. */
export function density(from: number, to: number, binDays: number | "month"): Bin[] {
  const bins: Bin[] = [];
  let start = from;
  while (start <= to) {
    let end: number;
    if (binDays === "month") {
      const d = new Date(start);
      end = Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1, 12);
    } else end = start + binDays * DAY;
    const byRepo: Record<string, number> = {};
    let total = 0;
    SERVICES.forEach((s, i) => {
      let c = 0;
      for (let d = start; d < end; d += DAY) c += perDay(i, d);
      byRepo[s.id] = c;
      total += c;
    });
    bins.push({ start, end, total, byRepo });
    start = end;
  }
  return bins;
}

/** One file's lifecycle. `kind: "rename"` moves it; `change` is a commit that touched it. */
export interface FileEvent {
  id: string;
  kind: "created" | "rename" | "change";
  date: string; // ISO day
  sha: string;
  author: string;
  text: string; // what happened, short
  from?: string; // earlier path, for renames
  similarity?: number; // 0..1 content kept across a rename
  note?: string;
}

/** History was rewritten: sync replayed from the merge base (decision on rewritten history). */
export interface Rewrite {
  id: string;
  repo: string; // service id
  date: string;
  kind: "force-push" | "rebase";
  mergeBase: string;
  linksRebuilt: number; // links kept separately, so they don't vanish
}

export const REWRITES: Rewrite[] = [
  { id: "rw-billing-2023", repo: "billing", date: "2023-03-03", kind: "force-push", mergeBase: "3b9e0c4", linksRebuilt: 12 },
];

const CREATE_INVOICE: FileEvent[] = [
  { id: "ci1", kind: "rename", date: "2026-08-14", sha: "e5a7b30", author: "Priya Natarajan", text: "Moved", from: "billing-service/src/payments/createInvoice.ts", similarity: 0.98 },
  { id: "ci2", kind: "change", date: "2026-07-17", sha: "05e6a4f", author: "Tom Ellery", text: "Now imports currency.ts" },
  { id: "ci3", kind: "rename", date: "2021-02-02", sha: "77fe210", author: "Jon Reyes", text: "Moved", from: "src/pay/createInvoice.ts", similarity: 0.92, note: "monorepo split" },
  { id: "ci4", kind: "created", date: "2019-06-11", sha: "0c4d9e1", author: "Tom Ellery", text: "Created", from: "src/pay/createInvoice.ts" },
];

/** Demo lineage by node id. Other nodes have no renames. */
export const FILE_EVENTS: Record<string, FileEvent[]> = {
  "billing/invoices/createInvoice.ts": CREATE_INVOICE,
};

export const fileEvents = (nodeId: string): FileEvent[] => FILE_EVENTS[nodeId] ?? [];
export const renameDays = (): number[] =>
  Object.values(FILE_EVENTS).flat().filter((e) => e.kind === "rename").map((e) => dayOf(e.date));
