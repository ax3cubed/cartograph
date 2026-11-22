import { confidence, type Spec } from "../model.ts";

const call = (node: string, score: number, note: string) => ({ node, confidence: confidence(score, { kind: score >= 0.9 ? "contract" : "name-match", note }) });

export const SPECS: Spec[] = [
  { id: "gateway", name: "api-gateway", file: "api-gateway/openapi.yaml", kind: "openapi", service: "gateway", operations: [
    { id: "gateway.orders.list", method: "GET", path: "/orders", handler: "gateway/routes/orders.ts", callers: [] },
    { id: "gateway.orders.create", method: "POST", path: "/orders", handler: "gateway/routes/orders.ts", callers: [] },
    { id: "gateway.orders.get", method: "GET", path: "/orders/{id}", callers: [] }, // spec-only: no handler yet
    { id: "gateway.checkout", method: "POST", path: "/checkout", handler: "gateway/routes/checkout.ts", callers: [] },
    { id: "gateway.products", method: "GET", path: "/products", handler: "gateway/routes/products.ts", callers: [] },
  ] },
  { id: "auth", name: "auth-service", file: "auth-service/openapi.yaml", kind: "openapi", service: "auth", operations: [
    { id: "auth.login", method: "POST", path: "/login", handler: "auth/handlers/login.go", callers: [] },
    { id: "auth.token", method: "POST", path: "/token", handler: "auth/handlers/token.go", callers: [call("gateway/clients/authClient.ts", 0.97, "Matches POST /token in the spec")] },
  ] },
  { id: "billing", name: "billing-service", file: "billing-service/openapi.yaml", kind: "openapi", service: "billing", operations: [
    { id: "billing.invoices.create", method: "POST", path: "/invoices", handler: "billing/invoices/createInvoice.ts", callers: [call("gateway/clients/billingClient.ts", 0.78, "POST /invoices matches the route template")] },
    { id: "billing.charge", method: "POST", path: "/payments/charge", handler: "billing/payments/charge.ts", callers: [call("gateway/clients/billingClient.ts", 0.71, "Charge call built from a template string")] },
    { id: "billing.refund", method: "POST", path: "/refunds", handler: "billing/payments/refund.ts", callers: [] },
  ] },
  { id: "catalog", name: "catalog-service", file: "catalog-service/openapi.yaml", kind: "openapi", service: "catalog", operations: [
    { id: "catalog.products", method: "GET", path: "/products", handler: "catalog/api/products.py", callers: [call("gateway/clients/catalogClient.ts", 0.96, "Matches GET /products in the spec"), call("billing/invoices/createInvoice.ts", 0.66, "URL built from CATALOG_URL")] },
    { id: "catalog.search", method: "GET", path: "/search", handler: "catalog/api/search.py", callers: [call("gateway/clients/catalogClient.ts", 0.95, "Matches GET /search in the spec")] },
  ] },
  { id: "events", name: "event contracts", file: "shared-libs/events/schema.ts", kind: "events", service: "shared", operations: [
    { id: "events.payment.succeeded", method: "EVENT", path: "payment.succeeded", handler: "billing/events/publisher.ts", callers: [call("notify/consumers/paymentEvents.ts", 0.82, "Subscribes to payment.succeeded")] },
    { id: "events.user.created", method: "EVENT", path: "user.created", handler: "auth/users/repo.go", callers: [call("notify/consumers/userEvents.ts", 0.71, "Subscribes to user.created by string name")] },
  ] },
];
