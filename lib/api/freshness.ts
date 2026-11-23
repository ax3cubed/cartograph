// The wire's freshness and workspace summary mapped onto the interim lib/model.ts shapes (see live.ts).
// Kept apart from the hooks, with type-only imports, so `pnpm check` can run them under node.
import type { Freshness as ApiFreshness, WorkspaceSummary as ApiWorkspace } from "./types.gen.ts";
import type { Freshness, IndexState, WorkspaceSummary } from "../model.ts";

const state = (s: ApiFreshness["state"]): IndexState => (s === "needs_migrate" ? "needs-migrate" : s);
export const toFreshness = (f: ApiFreshness): Freshness => ({
  state: state(f.state), indexVersion: f.format_version, builtAt: f.built_at ?? "", behind: f.behind,
  behindCapped: f.repos.some((r) => r.behind_capped),
  repos: f.repos.map((r) => ({ name: r.name, behind: r.behind, capped: r.behind_capped })), progress: f.progress ?? undefined,
  error: f.error ? { what: f.error.what, action: f.error.try } : undefined,
});
export const toSummary = (w: ApiWorkspace): WorkspaceSummary => ({ ...w, state: state(w.state), builtAt: w.built_at ?? "" });
