// The one place components read data from. Today these return demo data, synchronously.
// Decision A1: these become fetches to the local Rust API, so callers should treat them as async-shaped
// (read once, keep in state) and never call them in render loops.
import type {
  ActivityItem, AskTurn, CommitImpact, Concept, Correction, CoverageRow, Entrypoint,
  Freshness, McpTool, Owner, Provider, Repo, SearchResult, Spec, WorkspaceSummary,
} from "./model.ts";
import { COMMITS, NODES, WORKSPACE, type Commit } from "./workspace.ts";
import { ACTIVITY, CORRECTIONS, FRESHNESS, NOW, OWNERS, REPOS, WORKSPACES } from "./demo/workspace.ts";
import { SPECS } from "./demo/contracts.ts";
import { CONCEPTS, COVERAGE, ENTRYPOINTS } from "./demo/views.ts";
import { ASK_HISTORY, MCP_TOOLS, PROVIDERS } from "./demo/ai.ts";
import { commitImpact } from "./demo/impact.ts";

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
export const getCommitImpact = (sha: string): CommitImpact | undefined => commitImpact(sha);
export const getMcpTools = (): McpTool[] => MCP_TOOLS;
export const getProviders = (): Provider[] => PROVIDERS;
export const getAskHistory = (): AskTurn[] => ASK_HISTORY;

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
