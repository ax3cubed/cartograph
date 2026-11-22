// Extra northwind demo data for the view boards: drift, spec versions, concept detail and unresolved references.
// Request: move into lib/source.ts getters when the index API exists.

export interface SpecVersion { v: number; sha: string; summary: string; breaking?: { what: string; callers: string[] } }
export const VERSIONS: Record<string, SpecVersion[]> = {
  gateway: [
    { v: 2, sha: "d03f62b", summary: "Added POST /checkout. No breaking changes." },
    { v: 1, sha: "a41c09e", summary: "First version found in history." },
  ],
  auth: [{ v: 1, sha: "3be71d2", summary: "First version found in history." }],
  billing: [
    { v: 3, sha: "3e0d7a2", summary: "", breaking: { what: "Removed receiptUrl from ChargeResponse.", callers: ["gateway/clients/billingClient.ts"] } },
    { v: 2, sha: "f5c2b18", summary: "Added POST /refunds. No breaking changes." },
    { v: 1, sha: "e17a4c3", summary: "First version found in history." },
  ],
  catalog: [
    { v: 2, sha: "e8c47d1", summary: "Added GET /search. No breaking changes." },
    { v: 1, sha: "c90f5a1", summary: "First version found in history." },
  ],
  events: [
    { v: 2, sha: "ab83e1d", summary: "Added user.created. No breaking changes." },
    { v: 1, sha: "0f6b93d", summary: "First version found in history." },
  ],
};

/** Operations whose handler disagrees with the spec. */
export const MISMATCH: Record<string, { spec: string; handler: string }> = {
  "billing.charge": { spec: "Spec v3: ChargeResponse has no receiptUrl.", handler: "Handler billing/payments/charge.ts still returns receiptUrl." },
};

/** Routes the handlers serve that no spec lists. */
export const UNDOCUMENTED: Record<string, { method: string; path: string; handler: string }[]> = {
  billing: [{ method: "POST", path: "/internal/invoices/rebuild", handler: "billing/invoices/createInvoice.ts" }],
};

export const ALIASES: Record<string, string[]> = {
  invoice: ["inv", "bill", "statement"], payment: ["charge", "pay"], session: ["token", "login"],
  pricing: ["price", "tax"], receipt: ["confirmation"], "domain-event": ["event", "message"],
};

/** A near match the indexer is unsure about; the user decides. */
export const CANDIDATES: Record<string, { name: string; node: string; score: number; why: string }> = {
  invoice: { name: "orderTotal", node: "gateway/routes/orders.ts", score: 0.64, why: "looks similar but shares no fields with InvoiceDTO" },
};

/** Coverage gaps that are not derivable from the contracts. Each list matches its row's gap count. */
export const UNRESOLVED: Record<string, { at: string; raw: string }[]> = {
  imports: [{ at: "api-gateway/src/clients/billingClient.ts:3", raw: 'import { Money } from "@shared/money"' }],
  calls: [{ at: "billing-service/src/invoices/createInvoice.ts:41", raw: "`${env.CATALOG_URL}/products`" }],
  routes: [{ at: "api-gateway/src/routes/orders.ts:52", raw: "router.get(\"/orders/:id/receipt\")" }],
};
