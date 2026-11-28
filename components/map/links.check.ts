// FE-14: what the correction dialog sends for each verb, and what it refuses before sending. Run with `pnpm check`.
import assert from "node:assert/strict";
import { formProblem, keyOf, MAX_REASON, request, type Form } from "./links.ts";
import type { LinkDetail } from "../../lib/api/types.gen.ts";

const d = { link: { kind: "http" }, operation: { method: "get", template: "/orders/{}" } } as unknown as LinkDetail;
const key = keyOf(d, "web:web/src/api.ts", "orders:orders/src/server.ts");
const form = (p: Partial<Form>): Form => ({ verb: "remove", from: "", to: "", level: "exact", reason: " a log line ", ...p });

assert.deepEqual(key, { kind: "http", from: "web:web/src/api.ts", to: "orders:orders/src/server.ts", op: "GET /orders/{}", topic: null, table: null });
assert.deepEqual(request(form({}), key), { action: "remove", key, reason: "a log line" }, "the reason is trimmed");
assert.deepEqual(request(form({ verb: "retarget", to: " orders:orders/src/v2.ts " }), key), { action: "retarget", key, to: "orders:orders/src/v2.ts", reason: "a log line" });
assert.deepEqual(request(form({ verb: "set", level: "guessed" }), key), { action: "set", key, confidence: "guessed", reason: "a log line" });
assert.deepEqual(
  request(form({ verb: "add", from: "web:web/src/other.ts", to: "orders:orders/src/legacy.ts", level: "likely" }), key),
  { action: "add", key: { ...key, op: null, from: "web:web/src/other.ts", to: "orders:orders/src/legacy.ts" }, confidence: "likely", reason: "a log line" },
  "an added link joins the two files the form names, with no operation",
);

assert.equal(formProblem(form({})), null);
assert.match(formProblem(form({ reason: "  " }))!, /why/);
assert.match(formProblem(form({ reason: "x".repeat(MAX_REASON + 1) }))!, /501/);
assert.equal(formProblem(form({ reason: "é".repeat(MAX_REASON) })), null, "the limit counts characters, not bytes");
for (const bad of ["two\nlines", "tab\there", "nul\0", "line sep", "﻿bom"]) assert.match(formProblem(form({ reason: bad }))!, /one line/, JSON.stringify(bad));
assert.match(formProblem(form({ verb: "retarget" }))!, /file/);
assert.match(formProblem(form({ verb: "add", from: "a:b" }))!, /both/);
console.log("links.check: ok");
