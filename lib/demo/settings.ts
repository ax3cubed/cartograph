// Demo data the settings pages keep in phase 3 (northwind): call matching is cross-service (XS-*), and the retrieval
// chunk kinds and secret hits are Ask's (AI-*). Everything the index answers comes from the API, or lib/source.ts
// without the binary.
import type { EnvKey, EnvName } from "../model.ts";
import { NODES, SERVICES } from "../workspace.ts";
import { SPECS } from "./contracts.ts";

const ENV_SRC: Record<EnvName, string> = { prod: "helm · infra-values/prod", staging: "helm · infra-values/staging", local: ".env.example" };
const HOSTS: Record<EnvName, [string, string, string, string]> = {
  prod: ["auth.identity.svc", "billing.payments.svc", "catalog.catalog.svc", "user-events"],
  staging: ["auth.identity.stg.svc", "billing.payments.stg.svc", "catalog.catalog.stg.svc", "user-events-stg"],
  local: ["localhost:4102", "localhost:4103", "localhost:4101", "user-events-dev"],
};
export const envKeys = (e: EnvName): EnvKey[] => {
  const [a, b, c, q] = HOSTS[e];
  const from = ENV_SRC[e];
  return [
    { key: "AUTH_URL", value: `http://${a}`, from, to: "auth-service", tone: "ok" },
    { key: "BILLING_URL", value: `http://${b}`, from, to: "billing-service", tone: "ok" },
    { key: "CATALOG_URL", value: `http://${c}`, from, to: "catalog-service", tone: "ok" },
    { key: "NOTIFY_QUEUE", value: q, from: e === "local" ? ".env.example" : "configmap · notifications", to: "channel user.created", tone: "ok" },
    { key: "STRIPE_BASE", value: "https://api.stripe.com", from: ".env.example", to: "External · Stripe", tone: "off" },
  ];
};
/** Client call sites: how many resolve, and how many wait on SEARCH_URL. */
export const CALL_SITES = { total: 50, resolved: 36, searchUrl: 9 };
export const UNRECOGNISED_ROUTES = { count: 3, where: "api-gateway/legacy" };
export const ADAPTERS = ["Express", "Next.js routes", "FastAPI", "Gin", "gRPC", "BullMQ consumers", "k8s CronJob"];

const nodeCards = NODES.filter((n) => n.kind !== "workspace").length;
const fileCount = NODES.filter((n) => n.kind === "file").length;
export const CHUNKS = [
  { id: "cards", label: "Node cards: name, path, summary and main links", n: nodeCards, on: true },
  { id: "symbols", label: "Function and class bodies", n: fileCount * 6, on: true },
  { id: "changes", label: "Code changes from the last 2 years", n: fileCount * 9, on: true },
  { id: "contracts", label: "API operations and event channels", n: SPECS.reduce((n, s) => n + s.operations.length, 0), on: true },
  { id: "docs", label: "READMEs, ADRs and docs", n: SERVICES.length * 3, on: true },
  { id: "commit_messages", label: "Commit messages only, without the changes", n: 22, on: false },
];
export const SECRET_HITS = [
  { id: "h1", mask: "AKIA••••••••••7Q", what: "AWS key in", file: "billing-service/.env.old", year: "2019" },
  { id: "h2", mask: "sk_live_••••••••", what: "Stripe key in", file: "billing-service/payments/test.ts", year: "2021" },
  { id: "h3", mask: "-----BEGIN RSA••••", what: "in", file: "auth-service/dev-certs/", year: "" },
];
