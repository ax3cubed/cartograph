// Run: pnpm check
import assert from "node:assert/strict";
import { LINKS, dayMs, isActive, metrics, recentChange } from "./workspace.ts";

const shortcut = LINKS.find((l) => l.from.endsWith("routes/products.ts") && l.to.endsWith("postgres.py"))!;
assert.equal(isActive(shortcut, dayMs("2026-05-01")), true, "active before removal");
assert.equal(isActive(shortcut, dayMs("2026-06-02")), false, "gone on removal day");
assert.equal(recentChange(shortcut, dayMs("2026-06-20")), "removed");
assert.equal(recentChange(shortcut, dayMs("2026-08-20")), null);

assert.equal(metrics("notify", dayMs("2025-01-01")).links, 0, "nothing before first commit");
const shared = metrics("shared", dayMs("2026-11-07"));
assert.ok(shared.fanIn > 10 && shared.fanOut === 0, "shared libs are depended on, depend on nothing outside");
console.log("workspace ok", shared);

// ask engine
import { ask, mentions } from "./ask.ts";
const now = dayMs("2026-11-07");
const text = (a: ReturnType<typeof ask>) => a.lines.map((l) => l.map((p) => (typeof p === "string" ? p : "node" in p ? `[${p.node.split("/").pop()}]` : `#${p.commit}`)).join("")).join("\n  ");
assert.deepEqual(mentions("how does checkout reach stripeAdapter?"), ["gateway/routes/checkout.ts", "billing/payments/stripeAdapter.ts"]);
for (const q of ["What breaks if I change schema.ts?", "How does checkout reach stripeAdapter?", "What changed in billing since June?", "What does auth-service do?", "hello"]) {
  const a = ask(q, now);
  console.log(`\n> ${q}\n${a.title}\n  ${text(a)}`);
}
assert.ok(ask("What breaks if I change schema.ts?", now).nodes.length > 3);

