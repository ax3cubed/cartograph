// Run: pnpm check
import assert from "node:assert/strict";
import type { Contract, ContractVersion, Operation } from "../../lib/api/types.gen.ts";
import { breakingOf, riskText, specRisk, specRows } from "./risk.ts";

const op = (id: string, status: Operation["status"], handler?: string): Operation => ({
  id, method: "GET", path: `/${id}`, operation_id: `get_${id}`, summary: "<b>untrusted</b>", status, handler: handler ?? null, callers: [],
});
const contract = (operations: Operation[], undocumented: Contract["undocumented"] = []): Contract => ({
  id: "c1", name: "orders", file: "orders/openapi.yaml", kind: "openapi", service: "s1", operations, undocumented,
  counts: { operations: operations.length, sync: operations.filter((o) => o.status === "sync").length, spec_only: operations.filter((o) => o.status === "spec-only").length, undocumented: undocumented.length },
});
const version = (n: number, breaking_count: number): ContractVersion => ({
  n, breaking_count, changes: breaking_count, spec_hash: "00", summary: "", breaking: [],
  commit: { sha: "abc", seq: n, date: "2026-01-01T00:00:00Z", author: "a", repo: "r", message: "m" },
});

const c = contract([op("a", "sync", "n1"), op("b", "spec-only"), op("c", "sync", "n2")], [{ method: "POST", path: "/x", handler: "n3" }]);
const rows = specRows(c);
assert.deepEqual(rows.map((r) => r.status), ["undocumented", "spec-only", "sync", "sync"], "worst status first, the spec's order inside one");
assert.equal(rows[0].handler, "n3");
assert.equal(rows[1].summary, "<b>untrusted</b>", "spec text is carried as text, never changed");
assert.equal(rows[2].name, "get_a");

assert.equal(breakingOf(undefined), undefined);
assert.equal(breakingOf([version(1, 0)]), undefined);
assert.equal(breakingOf([version(1, 2), version(2, 0)]), undefined, "only the newest version counts");
assert.equal(breakingOf([version(1, 0), version(2, 3)])?.n, 2);

assert.deepEqual(specRisk(c), { rank: 1, breaking: 0, undocumented: 1, specOnly: 1 });
assert.equal(specRisk(c, [version(1, 1)]).rank, 0, "a breaking change outranks the rest");
assert.equal(specRisk(contract([op("a", "spec-only")])).rank, 2);
assert.equal(specRisk(contract([op("a", "sync", "n")])).rank, 3);
assert.equal(riskText(specRisk(c, [version(1, 1)])), "1 breaking change, 1 undocumented, 1 spec-only");
assert.equal(riskText(specRisk(contract([]))), "in sync");
console.log("risk ok");
