// Tests for the Freshness the app shows: run with `pnpm check`.
import assert from "node:assert/strict";
import { toFreshness } from "./freshness.ts";
import { endOfLocalDay, localDay } from "./at.ts";
import type { Freshness as ApiFreshness } from "./types.gen.ts";

const api = (cappedWeb: boolean): ApiFreshness => ({
  state: "behind", format_version: 2, built_at: "2026-11-22T10:00:00Z", behind: 1003, progress: null, error: null,
  repos: [
    { repo: "api", name: "api", behind: 3, behind_capped: false },
    { repo: "web", name: "web", behind: 1000, behind_capped: cappedWeb },
  ],
});

// A count that stopped at its cap reads "N+" in the app, as `cartograph status` prints it (SRV-03).
const capped = toFreshness(api(true));
assert.equal(capped.behindCapped, true);
assert.deepEqual(capped.repos, [{ name: "api", behind: 3, capped: false }, { name: "web", behind: 1000, capped: true }]);
assert.equal(toFreshness(api(false)).behindCapped, false);

// The date picker: local days, and a picked day reaches the API as its last local second, in any zone.
assert.equal(localDay(new Date(2026, 10, 22, 0, 30)), "2026-11-22");
assert.equal(localDay(new Date(2026, 10, 22, 23, 30)), "2026-11-22");
for (const day of ["2026-11-22", "2026-03-29", "2026-10-25"]) {
  const [y, m, d] = day.split("-").map(Number);
  assert.equal(Date.parse(endOfLocalDay(day)), new Date(y, m - 1, d, 23, 59, 59).getTime(), day);
}

console.log("live: ok");
