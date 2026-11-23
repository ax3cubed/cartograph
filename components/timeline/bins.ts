// FE-09's pure rules, kept apart so `pnpm check` runs them under node (bins.check.ts).
import type { Bin } from "../../lib/api/types.gen.ts";

export const DAY = 86_400_000;

/** The bin for a range of `days` days: 3 months or less by day, 2 years or less by week, more by month. */
export const binFor = (days: number): Bin => (days <= 92 ? "day" : days <= 731 ? "week" : "month");
export const binDays = (b: Bin) => (b === "day" ? 1 : b === "week" ? 7 : 30);

/**
 * The index of the commit nearest `ms` in `days` (sorted day numbers), or -1 when there are none. On a tie, and
 * among the commits of one day, the latest: the knob shows the state at the end of the day it rests on.
 */
export function nearest(days: Int32Array, ms: number) {
  if (!days.length) return -1;
  const d = Math.floor(ms / DAY);
  let lo = 0, hi = days.length - 1;
  while (lo < hi) { const m = (lo + hi) >> 1; if (days[m] < d) lo = m + 1; else hi = m; }
  if (days[lo] < d) return days.length - 1; // past the last commit
  if (lo > 0 && d - days[lo - 1] < days[lo] - d) lo--;
  while (lo + 1 < days.length && days[lo + 1] === days[lo]) lo++;
  return lo;
}
