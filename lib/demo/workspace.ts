// northwind: the built-in demo workspace. Counts come from lib/workspace.ts so they always reconcile.
import type { ActivityItem, Correction, Freshness, Owner, Repo, WorkspaceSummary } from "../model.ts";
import { COMMITS, LINKS, NODES, SERVICES, WORKSPACE, dayMs } from "../workspace.ts";

export const NOW = dayMs("2026-11-07");
const BUILT_AT = "2026-11-07T10:12:00Z";

const lastCommit = (repo: string) => [...COMMITS].reverse().find((c) => c.repo === repo)!;

export const REPOS: Repo[] = SERVICES.map((s) => ({
  id: s.id, name: s.name, path: `~/code/northwind/${s.name}`, branch: "main",
  indexedCommit: lastCommit(s.id).sha, head: lastCommit(s.id).sha, behind: 0,
  dirty: s.id === "gateway", // one repo with uncommitted changes, so the dirty chip has a case
  commits: COMMITS.filter((c) => c.repo === s.id).length,
}));

export const FRESHNESS: Freshness = { state: "current", indexVersion: 3, builtAt: BUILT_AT, behind: REPOS.reduce((n, r) => n + r.behind, 0) };

export const WORKSPACES: WorkspaceSummary[] = [{
  id: WORKSPACE, name: WORKSPACE, path: "built into the app", demo: true,
  repos: REPOS.length, services: SERVICES.length,
  modules: NODES.filter((n) => n.kind === "module").length,
  files: NODES.filter((n) => n.kind === "file").length,
  commits: COMMITS.length, state: FRESHNESS.state, builtAt: BUILT_AT,
}];

export const OWNERS: Owner[] = ([
  { id: "edge", name: "@northwind/edge", kind: "team", patterns: ["api-gateway/**"], nodes: 0 },
  { id: "identity", name: "@northwind/identity", kind: "team", patterns: ["auth-service/**"], nodes: 0 },
  { id: "payments", name: "@northwind/payments", kind: "team", patterns: ["billing-service/**"], nodes: 0 },
  { id: "catalog", name: "@northwind/catalog", kind: "team", patterns: ["catalog-service/**"], nodes: 0 },
  { id: "messaging", name: "@northwind/messaging", kind: "team", patterns: ["notifications/**"], nodes: 0 },
  { id: "platform", name: "@northwind/platform", kind: "team", patterns: ["shared-libs/**"], nodes: 0 },
] as const).map((o, i): Owner => ({ ...o, patterns: [...o.patterns], nodes: NODES.filter((n) => n.service === SERVICES[i].id && n.kind === "file").length }));

export const CORRECTIONS: Correction[] = [
  { id: "c1", action: "set", target: "billing/invoices/createInvoice.ts>catalog/api/products.py", value: "exact", reason: "CATALOG_URL points at catalog-service in every environment.", by: "Tom Ellery", at: "2026-10-30" },
  { id: "c2", action: "retarget", target: "auth/users/repo.go>shared/events/bus.ts", to: "shared/events/schema.ts", reason: "user.created is declared in schema.ts; the bus only carries it.", by: "Jon Reyes", at: "2026-10-02" },
  { id: "c3", action: "add", target: "notify/consumers/userEvents.ts", to: "auth/users/repo.go", reason: "Welcome SMS is triggered by the user.created event, which the string match missed.", by: "Sade Bello", at: "2026-09-26" },
];

export const ACTIVITY: ActivityItem[] = [
  { id: "a1", at: "2026-11-07T10:12:00Z", kind: "index", text: "Index v3 built in 41 seconds. 6 repos, 22 commits.", unread: true },
  { id: "a2", at: "2026-11-05T16:40:00Z", kind: "correction", text: "Tom Ellery set billing to catalog as exact.", unread: true },
  { id: "a3", at: "2026-10-28T09:05:00Z", kind: "sync", text: "billing-service synced to 3e0d7a2.", unread: false },
  { id: "a4", at: "2026-10-02T13:22:00Z", kind: "warning", text: `${LINKS.filter((l) => !l.removed && (l.type === "http" || l.type === "event")).length} links cross a service boundary without a contract.`, unread: false },
];
