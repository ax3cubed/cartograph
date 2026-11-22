// Demo data for the settings pages (northwind). Moves into lib/source.ts when the local API exists.
import { NODES, SERVICES } from "../workspace.ts";
import { SPECS } from "./contracts.ts";

export type Tone = "ok" | "warn" | "bad" | "off";
export type ProviderKind = "folder" | "github" | "gitlab" | "bitbucket" | "k8s" | "spec" | "traces" | "azure";
export interface Connection { id: string; kind: ProviderKind; ic: string; name: string; auth: string; provides: string[]; status: string; tone: Tone; when: string; action?: string }

export const CONNECTIONS: Connection[] = [
  { id: "local", kind: "folder", ic: "FS", name: "Local folder · ~/code/northwind", auth: "git on disk · no credentials", provides: ["Code", "History"], status: "Connected", tone: "ok", when: "synced 2h ago" },
  { id: "github", kind: "github", ic: "GH", name: "GitHub · northwind-inc", auth: "gh CLI login · read-only · 6 of 24 repos", provides: ["Code", "History"], status: "Connected", tone: "ok", when: "synced 2h ago" },
  { id: "gitlab", kind: "gitlab", ic: "GL", name: "GitLab · gitlab.northwind.dev/platform", auth: "Access token · read_repository", provides: ["Code", "History"], status: "Expires in 6 days", tone: "warn", when: "synced 2h ago", action: "Renew token" },
  { id: "k8s", kind: "k8s", ic: "K8", name: "Kubernetes · prod-eu", auth: "kubeconfig context · read services, configmaps", provides: ["Deploy", "Hosts"], status: "Connected", tone: "ok", when: "read 2h ago" },
  { id: "helm", kind: "github", ic: "HV", name: "Helm values · infra-values", auth: "Through the GitHub connection", provides: ["Config", "3 environments"], status: "Connected", tone: "ok", when: "synced 4h ago" },
  { id: "spec", kind: "spec", ic: "{}", name: "Spec URL · pricing openapi.json", auth: "https://pricing.northwind.example/openapi.json", provides: ["Contract"], status: "Not found (404)", tone: "bad", when: "failed 2h ago", action: "Fix URL" },
  { id: "traces", kind: "traces", ic: "OT", name: "Traces · Tempo", auth: "Optional · confirms inferred links", provides: ["Runtime"], status: "Off", tone: "off", when: "never", action: "Set up" },
];

export const PROVIDER_KINDS: { kind: ProviderKind; ic: string; label: string; needs: [string, string][] }[] = [
  { kind: "folder", ic: "FS", label: "Local folder", needs: [] },
  { kind: "github", ic: "GH", label: "GitHub", needs: [["Contents", "read"], ["Metadata", "read"]] },
  { kind: "gitlab", ic: "GL", label: "GitLab", needs: [["read_repository", "scope"]] },
  { kind: "bitbucket", ic: "BB", label: "Bitbucket", needs: [["Repositories", "read"]] },
  { kind: "k8s", ic: "K8", label: "Kubernetes", needs: [["services, configmaps", "read"]] },
  { kind: "spec", ic: "{}", label: "Spec URL", needs: [] },
  { kind: "traces", ic: "OT", label: "Traces", needs: [] },
  { kind: "azure", ic: "AZ", label: "Azure DevOps", needs: [["Code", "read"]] },
];

const filesOf = (lang: string) => NODES.filter((n) => n.kind === "file" && SERVICES.find((s) => s.id === n.service)?.lang === lang).length;
export interface Extractor { id: string; name: string; note: string; v: string; files: string; on: boolean }
export const EXTRACTORS: Extractor[] = [
  { id: "ts", name: "TypeScript and JavaScript", note: "imports, exports, symbols, calls, fetch sites", v: "0.4.0", files: String(filesOf("TS")), on: true },
  { id: "go", name: "Go", note: "imports, symbols, net/http and gRPC clients", v: "0.3.0", files: String(filesOf("GO")), on: true },
  { id: "py", name: "Python", note: "imports, FastAPI and Django routes", v: "0.3.1", files: String(filesOf("PY")), on: true },
  { id: "openapi", name: "OpenAPI and Swagger", note: "3.0, 3.1 and Swagger 2.0", v: "0.2.0", files: String(SPECS.filter((s) => s.kind === "openapi").length), on: true },
  { id: "asyncapi", name: "AsyncAPI", note: "channels and payloads", v: "0.1.0", files: String(SPECS.filter((s) => s.kind === "events").length), on: true },
  { id: "proto", name: "Protocol Buffers", note: "No files in the selected repositories", v: "0.2.0", files: "0", on: false },
  { id: "graphql", name: "GraphQL SDL", note: "No files in the selected repositories", v: "0.1.0", files: "0", on: false },
  { id: "sql", name: "SQL migrations", note: "tables and columns over time", v: "0.2.0", files: "0", on: false },
  { id: "deploy", name: "Helm, k8s, compose, Dockerfile", note: "deployables, hosts, config keys", v: "0.1.0", files: String(SERVICES.length), on: true },
  { id: "diff", name: "Code changes", note: "for history questions and search", v: "0.1.0", files: "all", on: true },
];

export const RUNS = [
  { id: "r1", what: "Full rebuild · CLI 0.4.0", took: "41 s", parsed: "98.6%" },
  { id: "r2", what: "Update · 3 commits", took: "1.4 s", parsed: "98.7%" },
  { id: "r3", what: "Update · 12 commits", took: "2.9 s", parsed: "98.6%" },
];

export const DEFAULT_EXCLUDES = "**/node_modules/**\n**/vendor/**\n**/dist/**\n**/*.generated.*\n**/__snapshots__/**";

export const DOMAINS = ["edge", "identity", "payments", "catalog", "messaging", "platform", "other"];
export const SERVICE_DETECT: Record<string, { how: string; domain: string; owner: string }> = {
  gateway: { how: "Dockerfile, Helm release gateway", domain: "edge", owner: "@northwind/edge" },
  auth: { how: "go.work module, Helm", domain: "identity", owner: "@northwind/identity" },
  billing: { how: "Dockerfile, Helm release billing", domain: "payments", owner: "@northwind/payments" },
  catalog: { how: "Dockerfile, OpenAPI 3.1", domain: "catalog", owner: "@northwind/catalog" },
  notify: { how: "compose service", domain: "messaging", owner: "@northwind/messaging" },
  shared: { how: "pnpm workspace package, no deployable", domain: "platform", owner: "@northwind/platform" },
};

export type EnvName = "prod" | "staging" | "local";
export interface EnvKey { key: string; value: string; from: string; to: string; tone: "ok" | "off" }
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
  { id: "hunks", label: "Code changes from the last 2 years", n: fileCount * 9, on: true },
  { id: "contracts", label: "API operations and event channels", n: SPECS.reduce((n, s) => n + s.operations.length, 0), on: true },
  { id: "docs", label: "READMEs, ADRs and docs", n: SERVICES.length * 3, on: true },
  { id: "msgs", label: "Commit messages only, without the changes", n: 22, on: false },
];
export const SECRET_HITS = [
  { id: "h1", mask: "AKIA••••••••••7Q", what: "AWS key in", file: "billing-service/.env.old", year: "2019" },
  { id: "h2", mask: "sk_live_••••••••", what: "Stripe key in", file: "billing-service/payments/test.ts", year: "2021" },
  { id: "h3", mask: "-----BEGIN RSA••••", what: "in", file: "auth-service/dev-certs/", year: "" },
];
