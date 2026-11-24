// The one place components read data from. Today these return demo data, synchronously.
// Decision A1: these become fetches to the local Rust API, so callers should treat them as async-shaped
// (read once, keep in state) and never call them in render loops.
import type {
  ActivityItem, AskTurn, Commit, Concept, Correction, CoverageRow, Entrypoint,
  Freshness, McpTool, Owner, Provider, Repo, SearchResult, Spec, WorkspaceSummary,
} from "./model.ts";
import type { DiscoveredFolder, Discovery, ExtractorInfo, IndexRun, Service, Unresolved } from "./api/types.gen.ts";
import { COMMITS, NODES, SERVICES, WORKSPACE } from "./workspace.ts";
import { ACTIVITY, CORRECTIONS, FRESHNESS, NOW, OWNERS, REPOS, WORKSPACES } from "./demo/workspace.ts";
import { SPECS } from "./demo/contracts.ts";
import { CONCEPTS, COVERAGE, ENTRYPOINTS } from "./demo/views.ts";
import { ASK_HISTORY, MCP_TOOLS, PROVIDERS } from "./demo/ai.ts";

/** Demo switches. `noWorkspace` makes the app behave as on first run (D2): no indexes, so /welcome is shown. */
export const demo = { noWorkspace: false };

export const now = () => NOW; // the demo's pinned "today"
export const getWorkspaces = (): WorkspaceSummary[] => (demo.noWorkspace ? [] : WORKSPACES);
export const getCurrentWorkspace = (): WorkspaceSummary | undefined => getWorkspaces().find((w) => w.id === WORKSPACE);
export const getRepos = (): Repo[] => REPOS;
export const getFreshness = (): Freshness => FRESHNESS;
export const getContracts = (): Spec[] => SPECS;
export const getEntrypoints = (): Entrypoint[] => ENTRYPOINTS;
export const getConcepts = (): Concept[] => CONCEPTS;
export const getCoverage = (): CoverageRow[] => COVERAGE;
export const getCorrections = (): Correction[] => CORRECTIONS;
export const getOwners = (): Owner[] => OWNERS;
export const getActivity = (): ActivityItem[] => ACTIVITY;
export const getCommits = (): Commit[] => COMMITS;
export const getMcpTools = (): McpTool[] => MCP_TOOLS;
export const getProviders = (): Provider[] => PROVIDERS;
export const getAskHistory = (): AskTurn[] => ASK_HISTORY;

// ---------- demo mode for the screens the binary answers from the index ----------
// With the binary these come from …/runs, …/extractors, …/services, …/coverage/imports/unresolved, /discover and
// …/freshness. Without it (pnpm dev, a static host: session.ts "demo") they are derived from lib/workspace.ts here,
// in the wire shapes, so each screen has one code path.

const filesIn = (lang: string) => NODES.filter((n) => n.kind === "file" && SERVICES.find((s) => s.id === n.service)?.lang === lang).length;
const servicesIn = (lang: string) => SERVICES.filter((s) => s.lang === lang).length;
const cov = (id: string) => COVERAGE.find((r) => r.id === id)!;

export const getExtractors = (): ExtractorInfo[] => [
  ["go-imports", filesIn("GO")], ["manifest-go", servicesIn("GO")], ["manifest-js", servicesIn("TS")], ["manifest-php", 0],
  ["manifest-py", servicesIn("PY")], ["php-imports", 0], ["py-imports", filesIn("PY")], ["ts-imports", filesIn("TS")],
].map(([name, files]) => ({ name: name as string, version: "1.0.0", files: files as number }));

export const getRuns = (): IndexRun[] => [{
  id: 1, started_at: new Date(Date.parse(FRESHNESS.builtAt) - 41_000).toISOString(), finished_at: FRESHNESS.builtAt, writer_version: "0.2.0",
  files_total: cov("files").total, files_parsed: cov("files").done, imports_total: cov("imports").total, imports_resolved: cov("imports").done,
}];

/** The shared libraries are a pnpm workspace; every other repo is one service. */
export const getServices = (): Service[] => SERVICES.map((s) => ({
  id: s.id, name: s.name, repo: s.name, paths: [], domain: null, tags: [], library: s.id === "shared", detected_by: s.id === "shared" ? "pnpm" : "repo",
}));

/** The one unresolved import behind the demo's imports row. */
export const getUnresolved = (): Unresolved[] => {
  const path = "api-gateway/src/clients/billingClient.ts";
  return [{ file: NODES.find((n) => n.path === path)?.id ?? "", path, line: 3, text: "@shared/money", reason: "not-found" }];
};

/** The demo's #state=stale and #state=failed boards (G02). The chip, popover, band and Activity read this one object. */
export function getDemoFreshness(state: "stale" | "failed"): Freshness {
  if (state === "failed") return { ...FRESHNESS, state: "failed", behind: 0, error: { what: "northwind index can't be read", action: "Rebuild the index with the command below." } };
  const behind: Record<string, number> = { billing: 9, catalog: 5 };
  const repos = SERVICES.map((s) => ({ name: s.name, behind: behind[s.id] ?? 0 }));
  return { ...FRESHNESS, state: "behind", behind: repos.reduce((n, r) => n + r.behind, 0), repos };
}

export const DEMO_WORKSPACE_FILE = "~/code/northwind/northwind.code-workspace";
/** What /discover answers for the demo walkthrough: northwind, or a folder with nothing to index. */
export function getDemoDiscovery(empty: boolean): Discovery {
  if (empty) return { folders: [{ folder: "notes", name: "notes", result: "skipped", reason: "Skipped: not a git repository" }], services: [] };
  const repo = (id: string) => SERVICES.find((s) => s.id === id)!.name;
  return {
    folders: [
      ...["gateway", "auth", "billing", "catalog", "notify"].map((id): DiscoveredFolder => ({ folder: repo(id), name: repo(id), result: "repo", repo: repo(id) })),
      ...["logger", "events", "money"].map((p): DiscoveredFolder => ({ folder: `shared-libs/packages/${p}`, name: p, result: "scope", repo: "shared-libs" })),
      { folder: "vscode-remote://ssh-remote+box/srv/legacy-reports", name: "legacy-reports", result: "skipped", reason: "Skipped: not on this machine" },
      { folder: "notes", name: "notes", result: "skipped", reason: "Skipped: not a git repository" },
    ],
    services: getServices(),
  };
}

export function search(q: string, limit = 12): SearchResult[] {
  const t = q.trim().toLowerCase();
  if (!t) return [];
  const hits: SearchResult[] = [];
  for (const n of NODES) {
    if (n.kind === "workspace") continue;
    if (n.name.toLowerCase().includes(t) || n.path.toLowerCase().includes(t)) hits.push({ id: n.id, kind: n.kind, name: n.name, path: n.path, service: n.service });
  }
  for (const c of CONCEPTS) if (c.name.toLowerCase().includes(t)) hits.push({ id: c.id, kind: "concept", name: c.name, path: c.summary, service: "" });
  for (const s of SPECS) for (const o of s.operations) {
    const label = `${o.method} ${o.path}`;
    if (label.toLowerCase().includes(t)) hits.push({ id: o.id, kind: "operation", name: label, path: s.file, service: s.service });
  }
  return hits.slice(0, limit);
}
