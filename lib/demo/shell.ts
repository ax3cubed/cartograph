// Shell demo data: what the add-workspace wizard discovers in northwind, plus the stale and failed index states
// that the G02 boards show. Move into lib/source.ts when the local API exists.
import type { FolderRow, Freshness } from "../model.ts";
import { SERVICES } from "../workspace.ts";

export const NORTHWIND_FILE = "~/code/northwind/northwind.code-workspace";
export const NORTHWIND_ID = "a41c7e90";

const repoOf = (id: string) => SERVICES.find((s) => s.id === id)!;
export const NORTHWIND_FOLDERS: FolderRow[] = [
  ...(["gateway", "auth", "billing", "catalog", "notify"] as const).map((id): FolderRow => ({ folder: repoOf(id).name, result: "repo", repo: id, detail: "Repository, branch main" })),
  { folder: "shared-libs/packages/logger", result: "repo", repo: "shared", detail: "Repository shared-libs, scope packages/logger" },
  { folder: "shared-libs/packages/events", result: "scope", repo: "shared", detail: "Second scope in shared-libs; its history is walked once" },
  { folder: "shared-libs/packages/money", result: "scope", repo: "shared", detail: "Third scope in shared-libs" },
  { folder: "vscode-remote://ssh-remote+box/srv/legacy-reports", result: "skipped", detail: "Skipped: not on this machine" },
  { folder: "notes", result: "skipped", detail: "Skipped: not a git repository" },
];

/** How a service was recognised, for the review step. Shared libraries have no deployable. */
export const FOUND_VIA: Record<string, string> = {
  gateway: "Dockerfile", auth: "Dockerfile, Helm chart", billing: "Dockerfile, OpenAPI", catalog: "Dockerfile, OpenAPI",
  notify: "Dockerfile, AsyncAPI", shared: "No deployable: kept as a library",
};

/** Commits behind HEAD per repo in the stale demo state. The totals everywhere come from these. */
const STALE_BEHIND: Record<string, number> = { billing: 9, catalog: 5 };
const repoRows = (behind: Record<string, number>) => SERVICES.map((s) => ({ name: s.name, behind: behind[s.id] ?? 0 }));
const sum = (rows: { behind: number }[]) => rows.reduce((n, r) => n + r.behind, 0);

/** One Freshness per demo state. The chip, popover, stale band and Activity all read the same object. */
export const freshnessFor = (state: "stale" | "failed"): Freshness => {
  if (state === "stale") {
    const repos = repoRows(STALE_BEHIND);
    return { state: "behind", indexVersion: 3, builtAt: "2026-11-05T10:12:00Z", behind: sum(repos), repos };
  }
  return {
    state: "failed", indexVersion: 3, builtAt: "2026-11-05T10:12:00Z", behind: 0,
    error: { what: "northwind index can't be read", action: "Rebuild the index with the command below." },
  };
};

/** Where each Activity row leads. Rows not listed here lead nowhere, so they render as plain text. */
export const ACTIVITY_HREF: Record<string, string> = {
  a1: "/settings/indexing", a2: "/#focus=billing", a3: "/settings/repositories", a4: "/contracts",
};
