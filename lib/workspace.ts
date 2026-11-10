// Mock multi-service workspace. Real indexing (file scan + `git log`) should emit this same shape.
// ponytail: hand-authored data, swap for a generated JSON when the indexer exists.

export type Kind = "workspace" | "service" | "module" | "file";
export type LinkType = "imports" | "calls" | "http" | "event" | "db";

export interface Node {
  id: string;
  name: string;
  kind: Kind;
  parent: string | null;
  service: string; // owning service id ("" for the workspace root)
  path: string;
  summary: string;
  lang?: "TS" | "GO" | "PY";
}

export interface Link {
  from: string; // `from` depends on `to`
  to: string;
  type: LinkType;
  added: string; // commit sha
  removed?: string;
}

export interface Commit {
  sha: string;
  date: string; // ISO day
  author: string;
  repo: string; // service id
  message: string;
}

export const WORKSPACE = "northwind";

export const SERVICES = [
  { id: "gateway", name: "api-gateway", color: "#d9ad3c", lang: "TS" },
  { id: "auth", name: "auth-service", color: "#46b99c", lang: "GO" },
  { id: "billing", name: "billing-service", color: "#cf5b80", lang: "TS" },
  { id: "catalog", name: "catalog-service", color: "#9d83d6", lang: "PY" },
  { id: "notify", name: "notifications", color: "#79b3cc", lang: "TS" },
  { id: "shared", name: "shared-libs", color: "#e3894f", lang: "TS" },
] as const;

export const serviceColor = (id: string) => SERVICES.find((s) => s.id === id)?.color ?? "#9aa3b8";

// [service, module, [file, summary][], module summary]
const TREE: [string, string, string, [string, string][]][] = [
  ["gateway", "routes", "Public HTTP surface. Maps URLs to downstream services.", [
    ["orders.ts", "Lists and creates orders for the signed-in customer; forwards creation to billing."],
    ["checkout.ts", "Checkout endpoint. Authenticates, then asks billing to raise and charge an invoice."],
    ["products.ts", "Product listing and search pass-through to the catalog service."],
  ]],
  ["gateway", "middleware", "Cross-cutting request handling applied before any route.", [
    ["authGuard.ts", "Rejects requests without a valid session token by verifying it with auth-service."],
    ["rateLimit.ts", "Per-token request budget. Reads the token identity resolved by authGuard."],
  ]],
  ["gateway", "clients", "Typed HTTP clients for each downstream service.", [
    ["authClient.ts", "Client for auth-service token verification."],
    ["billingClient.ts", "Client for billing-service invoice endpoints."],
    ["catalogClient.ts", "Client for catalog-service product and search endpoints."],
  ]],
  ["auth", "handlers", "HTTP handlers exposed by auth-service.", [
    ["login.go", "Checks credentials against the user repo and opens a session."],
    ["token.go", "Issues and verifies signed session tokens. The gateway calls this on every request."],
  ]],
  ["auth", "session", "Session lifecycle and token signing.", [
    ["store.go", "Session storage. Moved from Postgres to Redis in Jan 2026."],
    ["jwt.go", "Signs and validates JWTs with the rotating key set."],
  ]],
  ["auth", "users", "User records.", [
    ["repo.go", "Reads and writes users; emits user.created on sign-up."],
  ]],
  ["billing", "invoices", "Invoice creation and rendering.", [
    ["createInvoice.ts", "Builds an invoice from cart lines, prices them via catalog, then charges."],
    ["pdf.ts", "Renders invoice PDFs for email attachments."],
  ]],
  ["billing", "payments", "Money movement through the payment provider.", [
    ["stripeAdapter.ts", "Thin wrapper over the Stripe SDK. The only file that talks to Stripe."],
    ["charge.ts", "Charges a payment method and publishes payment.succeeded / failed."],
    ["refund.ts", "Full and partial refunds; publishes payment.refunded."],
  ]],
  ["billing", "events", "Outbound domain events.", [
    ["publisher.ts", "Validates payment events against the shared schema and puts them on the bus."],
  ]],
  ["catalog", "api", "HTTP endpoints of catalog-service.", [
    ["products.py", "Product detail and listing with computed prices."],
    ["search.py", "Full-text product search over the products table."],
  ]],
  ["catalog", "pricing", "Price computation.", [
    ["rules.py", "Applies discounts and promotions to a base price."],
    ["tax.py", "Regional tax rates; reads currency rules from shared money helpers."],
  ]],
  ["catalog", "store", "Persistence.", [
    ["postgres.py", "Connection pool and queries for the products database."],
  ]],
  ["notify", "consumers", "Event subscribers.", [
    ["paymentEvents.ts", "Listens for payment events and sends receipts."],
    ["userEvents.ts", "Listens for user.created and sends welcome email + SMS."],
  ]],
  ["notify", "channels", "Delivery channels.", [
    ["email.ts", "Sends transactional email through the mail provider."],
    ["sms.ts", "Sends SMS through the telecom provider."],
  ]],
  ["shared", "logger", "Structured logging used by every service.", [
    ["index.ts", "JSON logger with request-id propagation."],
  ]],
  ["shared", "events", "The event bus contract shared by producers and consumers.", [
    ["schema.ts", "Typed schemas for every domain event. Breaking changes here ripple to all consumers."],
    ["bus.ts", "Publish/subscribe client over Kafka."],
  ]],
  ["shared", "money", "Currency-safe arithmetic.", [
    ["currency.ts", "Minor-unit money type, rounding and currency conversion."],
  ]],
];

const SERVICE_SUMMARY: Record<string, string> = {
  gateway: "Single entry point for web and mobile clients. Authenticates, rate-limits and routes to services.",
  auth: "Owns users, sessions and tokens. Every authenticated request passes through it.",
  billing: "Raises invoices, charges and refunds payments, publishes payment events.",
  catalog: "Products, search and pricing (discounts and tax).",
  notify: "Turns domain events into email and SMS.",
  shared: "Libraries imported by every service: logging, event contracts, money.",
};

export const NODES: Node[] = [
  { id: WORKSPACE, name: WORKSPACE, kind: "workspace", parent: null, service: "", path: WORKSPACE, summary: "Six services and their shared libraries, mapped as one workspace." },
];
for (const s of SERVICES) {
  NODES.push({ id: s.id, name: s.name, kind: "service", parent: WORKSPACE, service: s.id, path: s.name, summary: SERVICE_SUMMARY[s.id], lang: s.lang });
}
for (const [svc, mod, modSummary, files] of TREE) {
  const svcName = SERVICES.find((s) => s.id === svc)!.name;
  const modId = `${svc}/${mod}`;
  NODES.push({ id: modId, name: mod, kind: "module", parent: svc, service: svc, path: `${svcName}/src/${mod}`, summary: modSummary });
  for (const [file, summary] of files) {
    const lang = file.endsWith(".go") ? "GO" : file.endsWith(".py") ? "PY" : "TS";
    NODES.push({ id: `${modId}/${file}`, name: file, kind: "file", parent: modId, service: svc, path: `${svcName}/src/${mod}/${file}`, summary, lang });
  }
}

export const byId = new Map(NODES.map((n) => [n.id, n]));

export const COMMITS: Commit[] = [
  { sha: "a41c09e", date: "2025-02-04", author: "Maya Okafor", repo: "gateway", message: "gateway: initial routes and auth guard" },
  { sha: "3be71d2", date: "2025-02-18", author: "Jon Reyes", repo: "auth", message: "auth: token issuance and JWT signer" },
  { sha: "c90f5a1", date: "2025-03-11", author: "Priya Natarajan", repo: "catalog", message: "catalog: products API over Postgres" },
  { sha: "5d2e8b7", date: "2025-04-02", author: "Maya Okafor", repo: "gateway", message: "gateway: proxy product routes to catalog" },
  { sha: "e17a4c3", date: "2025-04-29", author: "Tom Ellery", repo: "billing", message: "billing: invoice creation and Stripe adapter" },
  { sha: "0f6b93d", date: "2025-05-20", author: "Sade Bello", repo: "shared", message: "shared: event bus and schemas" },
  { sha: "8a3d1f0", date: "2025-06-17", author: "Tom Ellery", repo: "billing", message: "billing: publish payment events" },
  { sha: "b25e7c4", date: "2025-07-08", author: "Sade Bello", repo: "notify", message: "notifications: receipts on payment events" },
  { sha: "4c81a9e", date: "2025-08-12", author: "Priya Natarajan", repo: "catalog", message: "catalog: pricing rules with regional tax" },
  { sha: "d03f62b", date: "2025-09-03", author: "Maya Okafor", repo: "gateway", message: "gateway: checkout route calls billing" },
  { sha: "71ea0c5", date: "2025-10-14", author: "Sade Bello", repo: "shared", message: "shared: money/currency helpers" },
  { sha: "f5c2b18", date: "2025-11-25", author: "Tom Ellery", repo: "billing", message: "billing: refunds" },
  { sha: "2a9d4e6", date: "2026-01-13", author: "Jon Reyes", repo: "auth", message: "auth: move session store to Redis" },
  { sha: "96b0f3a", date: "2026-02-10", author: "Maya Okafor", repo: "gateway", message: "gateway: per-token rate limiting" },
  { sha: "e8c47d1", date: "2026-03-03", author: "Priya Natarajan", repo: "catalog", message: "catalog: search endpoint" },
  { sha: "1d7f2b9", date: "2026-04-07", author: "Tom Ellery", repo: "billing", message: "billing: render invoice PDFs" },
  { sha: "c6a3e05", date: "2026-05-12", author: "Sade Bello", repo: "notify", message: "notifications: SMS channel and user events" },
  { sha: "7b19d8c", date: "2026-06-02", author: "Maya Okafor", repo: "gateway", message: "gateway: drop direct read of catalog DB" },
  { sha: "05e6a4f", date: "2026-07-17", author: "Tom Ellery", repo: "billing", message: "billing: charge uses shared money type" },
  { sha: "9f4b2c7", date: "2026-08-19", author: "Priya Natarajan", repo: "catalog", message: "catalog: tax reads currency rules from shared" },
  { sha: "ab83e1d", date: "2026-09-23", author: "Jon Reyes", repo: "auth", message: "auth: emit user.created event" },
  { sha: "3e0d7a2", date: "2026-10-28", author: "Tom Ellery", repo: "billing", message: "billing: invoices price via catalog API, not its tables" },
];
export const commitBySha = new Map(COMMITS.map((c) => [c.sha, c]));

const g = "gateway", a = "auth", b = "billing", c = "catalog", n = "notify", s = "shared";
const L = (from: string, to: string, type: LinkType, added: string, removed?: string): Link => ({ from, to, type, added, removed });

export const LINKS: Link[] = [
  L(`${g}/routes/orders.ts`, `${g}/middleware/authGuard.ts`, "imports", "a41c09e"),
  L(`${g}/middleware/authGuard.ts`, `${g}/clients/authClient.ts`, "calls", "a41c09e"),
  L(`${g}/middleware/authGuard.ts`, `${s}/logger/index.ts`, "imports", "a41c09e"),
  L(`${g}/clients/authClient.ts`, `${a}/handlers/token.go`, "http", "3be71d2"),
  L(`${a}/handlers/login.go`, `${a}/users/repo.go`, "calls", "3be71d2"),
  L(`${a}/handlers/login.go`, `${a}/session/store.go`, "calls", "3be71d2"),
  L(`${a}/handlers/login.go`, `${s}/logger/index.ts`, "imports", "3be71d2"),
  L(`${a}/handlers/token.go`, `${a}/session/jwt.go`, "calls", "3be71d2"),
  L(`${a}/session/store.go`, `${a}/users/repo.go`, "calls", "3be71d2", "2a9d4e6"),
  L(`${a}/users/repo.go`, `${s}/logger/index.ts`, "imports", "3be71d2"),
  L(`${c}/api/products.py`, `${c}/store/postgres.py`, "db", "c90f5a1"),
  L(`${c}/api/products.py`, `${s}/logger/index.ts`, "imports", "c90f5a1"),
  L(`${g}/routes/products.ts`, `${g}/clients/catalogClient.ts`, "calls", "5d2e8b7"),
  L(`${g}/routes/products.ts`, `${c}/store/postgres.py`, "db", "5d2e8b7", "7b19d8c"),
  L(`${g}/clients/catalogClient.ts`, `${c}/api/products.py`, "http", "5d2e8b7"),
  L(`${g}/clients/catalogClient.ts`, `${s}/logger/index.ts`, "imports", "5d2e8b7"),
  L(`${b}/invoices/createInvoice.ts`, `${b}/payments/charge.ts`, "calls", "e17a4c3"),
  L(`${b}/payments/charge.ts`, `${b}/payments/stripeAdapter.ts`, "calls", "e17a4c3"),
  L(`${b}/invoices/createInvoice.ts`, `${c}/pricing/tax.py`, "imports", "e17a4c3", "3e0d7a2"),
  L(`${s}/events/bus.ts`, `${s}/events/schema.ts`, "imports", "0f6b93d"),
  L(`${s}/events/bus.ts`, `${s}/logger/index.ts`, "imports", "0f6b93d"),
  L(`${b}/payments/charge.ts`, `${b}/events/publisher.ts`, "calls", "8a3d1f0"),
  L(`${b}/events/publisher.ts`, `${s}/events/bus.ts`, "event", "8a3d1f0"),
  L(`${b}/events/publisher.ts`, `${s}/events/schema.ts`, "imports", "8a3d1f0"),
  L(`${n}/consumers/paymentEvents.ts`, `${s}/events/bus.ts`, "event", "b25e7c4"),
  L(`${n}/consumers/paymentEvents.ts`, `${s}/events/schema.ts`, "imports", "b25e7c4"),
  L(`${n}/consumers/paymentEvents.ts`, `${n}/channels/email.ts`, "calls", "b25e7c4"),
  L(`${n}/channels/email.ts`, `${s}/logger/index.ts`, "imports", "b25e7c4"),
  L(`${c}/api/products.py`, `${c}/pricing/rules.py`, "calls", "4c81a9e"),
  L(`${c}/pricing/rules.py`, `${c}/pricing/tax.py`, "calls", "4c81a9e"),
  L(`${g}/routes/checkout.ts`, `${g}/middleware/authGuard.ts`, "imports", "d03f62b"),
  L(`${g}/routes/checkout.ts`, `${g}/clients/billingClient.ts`, "calls", "d03f62b"),
  L(`${g}/routes/orders.ts`, `${g}/clients/billingClient.ts`, "calls", "d03f62b"),
  L(`${g}/clients/billingClient.ts`, `${b}/invoices/createInvoice.ts`, "http", "d03f62b"),
  L(`${b}/invoices/createInvoice.ts`, `${s}/money/currency.ts`, "imports", "71ea0c5"),
  L(`${b}/payments/refund.ts`, `${b}/payments/stripeAdapter.ts`, "calls", "f5c2b18"),
  L(`${b}/payments/refund.ts`, `${b}/events/publisher.ts`, "calls", "f5c2b18"),
  L(`${g}/middleware/rateLimit.ts`, `${g}/middleware/authGuard.ts`, "imports", "96b0f3a"),
  L(`${g}/clients/catalogClient.ts`, `${c}/api/search.py`, "http", "e8c47d1"),
  L(`${c}/api/search.py`, `${c}/store/postgres.py`, "db", "e8c47d1"),
  L(`${b}/invoices/createInvoice.ts`, `${b}/invoices/pdf.ts`, "calls", "1d7f2b9"),
  L(`${n}/consumers/userEvents.ts`, `${s}/events/bus.ts`, "event", "c6a3e05"),
  L(`${n}/consumers/userEvents.ts`, `${n}/channels/sms.ts`, "calls", "c6a3e05"),
  L(`${n}/consumers/userEvents.ts`, `${n}/channels/email.ts`, "calls", "c6a3e05"),
  L(`${b}/payments/charge.ts`, `${s}/money/currency.ts`, "imports", "05e6a4f"),
  L(`${c}/pricing/tax.py`, `${s}/money/currency.ts`, "imports", "9f4b2c7"),
  L(`${a}/users/repo.go`, `${s}/events/bus.ts`, "event", "ab83e1d"),
  L(`${b}/invoices/createInvoice.ts`, `${c}/api/products.py`, "http", "3e0d7a2"),
];

// ---------- time ----------

const DAY = 86_400_000;
export const dayMs = (iso: string) => Date.parse(iso + "T12:00:00Z");
const at = (sha: string) => dayMs(commitBySha.get(sha)!.date);

export const isActive = (l: Link, t: number) => at(l.added) <= t && (!l.removed || at(l.removed) > t);

/** "added" | "removed" when the link changed in the `window` days up to t. */
export function recentChange(l: Link, t: number, windowDays = 30): "added" | "removed" | null {
  if (l.removed && at(l.removed) <= t && t - at(l.removed) < windowDays * DAY) return "removed";
  const ad = at(l.added);
  if (ad <= t && t - ad < windowDays * DAY && isActive(l, t)) return "added";
  return null;
}

// ---------- ontology ----------

export const children = (id: string) => NODES.filter((x) => x.parent === id);

export function descendants(id: string): Set<string> {
  const out = new Set<string>([id]);
  for (const x of NODES) {
    for (let p: string | null = x.id; p; p = byId.get(p)!.parent) if (p === id) { out.add(x.id); break; }
  }
  return out;
}

export const ancestry = (id: string) => {
  const chain: Node[] = [];
  for (let p: string | null = id; p; p = byId.get(p)!.parent) chain.unshift(byId.get(p)!);
  return chain;
};

// ---------- metrics ----------

export interface Metrics { links: number; fanIn: number; fanOut: number; cross: number; churn: number }

export function metrics(scopeId: string, t: number): Metrics {
  const scope = descendants(scopeId);
  const m: Metrics = { links: 0, fanIn: 0, fanOut: 0, cross: 0, churn: 0 };
  for (const l of LINKS) {
    const fi = scope.has(l.from), ti = scope.has(l.to);
    if (!fi && !ti) continue;
    const ch = [at(l.added), l.removed ? at(l.removed) : Infinity].filter((d) => d <= t && t - d < 90 * DAY).length;
    m.churn += ch;
    if (!isActive(l, t)) continue;
    m.links++;
    if (ti && !fi) m.fanIn++;
    if (fi && !ti) m.fanOut++;
    if (byId.get(l.from)!.service !== byId.get(l.to)!.service) m.cross++;
  }
  return m;
}

/** Monthly samples of one metric, oldest first, ending at t. */
export function series(scopeId: string, t: number, key: keyof Metrics, months: number) {
  const out: { t: number; v: number }[] = [];
  for (let i = months; i >= 0; i--) {
    const d = new Date(t);
    d.setUTCMonth(d.getUTCMonth() - i);
    out.push({ t: d.getTime(), v: metrics(scopeId, d.getTime())[key] });
  }
  return out;
}

/** Active links crossing the scope boundary, grouped by the node on the other end. */
export function neighbours(scopeId: string, t: number) {
  const scope = descendants(scopeId);
  const dependsOn = new Map<string, Link[]>(), usedBy = new Map<string, Link[]>();
  const push = (m: Map<string, Link[]>, k: string, l: Link) => m.set(k, [...(m.get(k) ?? []), l]);
  for (const l of LINKS) {
    if (!isActive(l, t)) continue;
    const fi = scope.has(l.from), ti = scope.has(l.to);
    if (fi && !ti) push(dependsOn, l.to, l);
    if (ti && !fi) push(usedBy, l.from, l);
  }
  return { dependsOn, usedBy };
}

/** Commits that added or removed a link touching the scope, newest first. */
export function changesFor(scopeId: string, t: number) {
  const scope = descendants(scopeId);
  const rows: { commit: Commit; link: Link; kind: "added" | "removed" }[] = [];
  for (const l of LINKS) {
    if (!scope.has(l.from) && !scope.has(l.to)) continue;
    if (at(l.added) <= t) rows.push({ commit: commitBySha.get(l.added)!, link: l, kind: "added" });
    if (l.removed && at(l.removed) <= t) rows.push({ commit: commitBySha.get(l.removed)!, link: l, kind: "removed" });
  }
  return rows.sort((x, y) => y.commit.date.localeCompare(x.commit.date));
}

/** Files in scope ranked by how many active links point at them (blast radius). */
export function ranking(scopeId: string, t: number) {
  const scope = descendants(scopeId);
  const ago = t - 90 * DAY;
  return NODES.filter((x) => x.kind === "file" && scope.has(x.id))
    .map((x) => {
      const now = LINKS.filter((l) => l.to === x.id && isActive(l, t)).length;
      const then = LINKS.filter((l) => l.to === x.id && isActive(l, ago)).length;
      return { node: x, usedBy: now, delta: now - then, uses: LINKS.filter((l) => l.from === x.id && isActive(l, t)).length };
    })
    .filter((r) => r.usedBy + r.uses > 0)
    .sort((p, q) => q.usedBy - p.usedBy || q.uses - p.uses);
}
