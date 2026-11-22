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

// ai layer
import { buildMessages, linkify, sseDeltas } from "./ai.ts";
const sse = sseDeltas('data: {"choices":[{"delta":{"content":"Hel"}}]}\n\ndata: {"choices":[{"delta":{"content":"lo"}}]}\n: keep-alive\ndata: [DONE]\ndata: {"choi');
assert.deepEqual(sse, { text: "Hello", rest: 'data: {"choi' }, "sse deltas joined, partial frame kept for the next chunk");
assert.deepEqual(linkify("checkout.ts was linked in a1f3c09."), [{ node: "gateway/routes/checkout.ts" }, " was linked in a1f3c09."].map((p) => p), "known names become evidence");
const msgs = buildMessages(ask("What breaks if I change schema.ts?", now), now, false);
assert.ok(msgs[1].content.includes("schema.ts") && msgs[1].content.includes("<map>"), "prompt carries the evidence");
assert.ok(buildMessages(ask("hello", now), now, true)[1].content.length > msgs[1].content.length, "ultra sends the whole map");
console.log("ai ok");

// demo data reconciles
import { getContracts, getCoverage, getEntrypoints, getFreshness, getOwners, getRepos, getWorkspaces, getCommitImpact, search } from "./source.ts";
import { NODES, COMMITS } from "./workspace.ts";
import { chipText, levelOf } from "./model.ts";
const ws = getWorkspaces()[0];
assert.equal(ws.files, NODES.filter((n) => n.kind === "file").length, "workspace summary counts the files in the graph");
assert.equal(ws.repos, getRepos().length);
assert.equal(getRepos().reduce((n, r) => n + r.commits, 0), COMMITS.length, "every commit belongs to one repo");
assert.equal(getOwners().reduce((n, o) => n + o.nodes, 0), ws.files, "every file has an owner");
assert.equal(getFreshness().behind, getRepos().reduce((n, r) => n + r.behind, 0), "freshness sums the repos");
const ops = getContracts().flatMap((s) => s.operations);
assert.equal(getCoverage().find((r) => r.id === "handlers")!.total, ops.length, "coverage counts every operation");
assert.equal(getCoverage().find((r) => r.id === "handlers")!.done, ops.filter((o) => o.handler).length);
assert.equal(getCoverage().find((r) => r.id === "consumers")!.total, getEntrypoints().length);
for (const o of ops) for (const c of o.callers) assert.equal(c.confidence.level, levelOf(c.confidence.score), "confidence level matches its score");
for (const r of getCoverage()) assert.ok(r.done <= r.total, `${r.id} done <= total`);
assert.equal(chipText({ level: "likely", score: 0.62, evidence: [] }), "Likely · 0.62");
assert.ok(LINKS.some((l) => l.confidence), "some links carry a confidence");
assert.equal(getCommitImpact("3e0d7a2")!.removedLinks.length, 1, "3e0d7a2 removed the createInvoice to tax link");
assert.ok(search("checkout").length > 0 && search("zzzz").length === 0);
console.log("demo ok");

// citations (decision G5) and the retrieval budget (G8)
import { checkCitations } from "./ask.ts";
import { buildContext } from "./ai.ts";
{
  const t0 = dayMs("2026-11-07");
  const hs = [
    { handle: "e1", from: dayMs("2025-01-01"), to: null },
    { handle: "e2", from: dayMs("2025-01-01"), to: dayMs("2026-06-02") }, // removed before t0
    { handle: "e3", from: dayMs("2027-01-01"), to: null }, // not yet true at t0
  ];
  const r = checkCitations("A calls B [e1]. It used to call C [e2]. Nothing backs this. Made up [e9].\nSecond paragraph. [e1] [e3]", hs, t0);
  const flat = r.paragraphs.flat();
  assert.equal(r.paragraphs.length, 2);
  assert.deepEqual(flat.map((s) => s.supported), [true, false, false, false, true], "only sentences with a surviving citation are supported");
  assert.equal(flat[0].text, "A calls B.", "citation markers are stripped from the text");
  assert.equal(flat[1].cites[0].why, "not true on the date asked about");
  assert.equal(flat[3].cites[0].why, "not in this turn's results");
  assert.equal(flat[4].text, "Second paragraph.", "a citation after the full stop joins the sentence before it");
  assert.deepEqual(r.stats, { sentences: 5, unsupported: 3, cited: 4, kept: 1, dropped: 3 });
  assert.equal(checkCitations("A calls B [e1].", hs, dayMs("2024-01-01")).stats.dropped, 1, "interval must cover the asked date");
  assert.equal(checkCitations("", hs, t0).stats.sentences, 0);

  const base = ask("What breaks if I change schema.ts?", t0);
  const small = buildContext(base, t0, false), big = buildContext(base, t0, true);
  assert.ok(small.handles.every((h, i) => h.handle === `e${i + 1}`) && small.handles.length > 3, "every row has a handle");
  assert.ok(small.messages[1].content.includes("[e1]"), "the prompt lists handles");
  assert.ok(big.tokens <= big.budget && big.leftOut <= small.leftOut, "ultra stays inside its budget and leaves out fewer nodes");
  assert.ok(small.tools.some((x) => x.tool === "impact") && small.tools[0].tool === "find", "tool calls are derived from the plan");
  console.log("citations ok", r.stats, `ultra left out ${big.leftOut}, base ${small.leftOut}`);
}

// ---- composer: "@" picks and "/" tools ----
import { parseComposer } from "./ask.ts";
{
  const schema = NODES.find((n) => n.name === "schema.ts")!.id;
  const stripe = NODES.find((n) => n.name === "stripeAdapter.ts")!.id;
  const p = parseComposer("/impact @schema.ts ", [{ token: "@schema.ts", id: schema }, { token: "@gone", id: "x" }]);
  assert.equal(p.tool, "impact");
  assert.deepEqual(p.ids, [schema]); // refs whose token was deleted are dropped
  assert.equal(parseComposer("what about a/b", []).tool, undefined); // "/" only counts at the start
  assert.equal(parseComposer("/nope x", []).tool, undefined);

  const t = dayMs("2026-11-07");
  assert.equal(ask("/impact @schema.ts", t, { ids: [schema], tool: "impact" }).plan, "impact");
  assert.equal(ask("/describe @schema.ts", t, { ids: [schema], tool: "describe" }).plan, "describe");
  assert.equal(ask("/changes", t, { tool: "changes" }).plan, "history");
  assert.equal(ask("/path @schema.ts @stripeAdapter.ts", t, { ids: [schema, stripe], tool: "path" }).plan, "path");
  assert.equal(ask("/path @schema.ts", t, { ids: [schema], tool: "path" }).title, "Pick two nodes");
  // a forced tool wins over the wording: this would otherwise read as a history question
  assert.equal(ask("what changed in schema.ts", t, { ids: [schema], tool: "impact" }).plan, "impact");
  console.log("composer ok");
}
