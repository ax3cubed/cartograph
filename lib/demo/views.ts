// Entrypoints, concepts and coverage are derived from the contracts and the graph, so the totals reconcile.
import type { Concept, CoverageRow, Entrypoint } from "../model.ts";
import { LINKS, NODES, byId } from "../workspace.ts";
import { SPECS } from "./contracts.ts";

const ops = SPECS.flatMap((s) => s.operations.map((o) => ({ ...o, spec: s })));

// every handled REST operation is an HTTP entrypoint; the events are entered through their notifications consumers
export const ENTRYPOINTS: Entrypoint[] = [
  ...ops.filter((o) => o.method !== "EVENT" && o.handler).map((o): Entrypoint => ({
    id: o.id, kind: "http", label: `${o.method} ${o.path}`, node: o.handler!, service: byId.get(o.handler!)!.service, consumers: o.callers.length,
  })),
  ...ops.filter((o) => o.method === "EVENT").flatMap((o) => o.callers.map((c): Entrypoint => ({
    id: `${o.id}.consumer`, kind: "event", label: `on ${o.path}`, node: c.node, service: byId.get(c.node)!.service, consumers: 1,
  }))),
];

export const CONCEPTS: Concept[] = [
  { id: "invoice", name: "Invoice", domain: "money", summary: "Raised by billing, priced through catalog tax rules, rendered to PDF.", nodes: ["billing/invoices/createInvoice.ts", "billing/invoices/pdf.ts", "catalog/pricing/tax.py", "shared/money/currency.ts"] },
  { id: "payment", name: "Payment", domain: "money", summary: "A charge or refund through Stripe, announced as an event.", nodes: ["billing/payments/charge.ts", "billing/payments/refund.ts", "billing/payments/stripeAdapter.ts", "billing/events/publisher.ts"] },
  { id: "session", name: "Session", domain: "identity", summary: "Login, token issuance and the Redis session store.", nodes: ["auth/handlers/login.go", "auth/handlers/token.go", "auth/session/store.go", "auth/session/jwt.go"] },
  { id: "pricing", name: "Pricing", domain: "catalog", summary: "Discount rules and regional tax, in the shared currency type.", nodes: ["catalog/pricing/rules.py", "catalog/pricing/tax.py", "shared/money/currency.ts"] },
  { id: "receipt", name: "Receipt", domain: "messaging", summary: "Email and SMS sent when a payment or a user event arrives.", nodes: ["notify/consumers/paymentEvents.ts", "notify/consumers/userEvents.ts", "notify/channels/email.ts", "notify/channels/sms.ts"] },
  { id: "domain-event", name: "Domain event", domain: "platform", summary: "The shared bus and the schemas every service publishes against.", nodes: ["shared/events/bus.ts", "shared/events/schema.ts"] },
];

const files = NODES.filter((n) => n.kind === "file").length;
const importLinks = LINKS.filter((l) => l.type === "imports" && !l.removed).length;
const crossCalls = LINKS.filter((l) => (l.type === "http") && !l.removed).length;
const restOps = ops.filter((o) => o.method !== "EVENT");
const row = (id: string, label: string, done: number, total: number, target?: number, gap?: CoverageRow["gap"]): CoverageRow => ({ id, label, done, total, target, gap });

export const COVERAGE: CoverageRow[] = [
  row("files", "Files parsed", files, files, 98),
  row("imports", "Imports resolved", importLinks, importLinks + 1, 99, { what: "1 unresolved import", why: "Path alias @shared/* is missing from tsconfig paths", action: "Show imports" }),
  row("calls", "Call sites to an operation", crossCalls, crossCalls + 1, 85, { what: "1 call site stops at CATALOG_URL", why: "Config key has no value in the prod environment", action: "Set value" }),
  row("handlers", "Operations with a handler", ops.filter((o) => o.handler).length, ops.length, undefined, { what: "1 operation with no handler", why: "Spec-only: GET /orders/{id} in api-gateway/openapi.yaml", action: "Open contract" }),
  row("routes", "Routes in a contract", restOps.filter((o) => o.handler).length, restOps.filter((o) => o.handler).length + 1, 80),
  row("consumers", "Entrypoints with a consumer", ENTRYPOINTS.filter((e) => e.consumers > 0).length, ENTRYPOINTS.length, 75, { what: `${ENTRYPOINTS.filter((e) => e.consumers === 0).length} entrypoints with no consumer`, why: "Callers outside the workspace aren't visible", action: "Declare clients" }),
];
