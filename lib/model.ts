// Shapes the local index API will return (decision A1). lib/demo/* fills them with northwind data;
// lib/source.ts is the only place components read them from. Components and app/ import types from here,
// never from lib/workspace.ts or lib/demo/* (FE-02).
//
// ponytail: interim, hand-written. The merge note on FE-02 retires this file in favour of the generated
// contract, lib/api/types.gen.ts (SRV-07 owns it; never hand-edit it). Each type here gives way to its
// generated counterpart once FE-03 moves components onto the API client. Mapping, interim → generated:
//   Node → Node (lang a string; kind NodeKind)      Link → Link (type → kind, added/removed CommitRef, id)
//   Commit → Commit (+ seq)                         Kind → NodeKind / Lod      LinkType → LinkKind
//   Evidence, Confidence, ConfidenceLevel           Repo, Freshness (format_version, fact_hash)
//   Correction (target → key: LinkKey)              Spec, Operation → Contract, Operation, Caller
//   Entrypoint, Concept, CoverageRow, Owner         ActivityItem, WorkspaceSummary, SearchResult
//   CommitImpact (added_links, removed_links)       McpToolName, McpTool, Provider, AppearanceSettings
//   Bin (a histogram bin; the generated Bin is the bin size), FileEvent, Rewrite, Sym, SymLink,
//   Connection, ProviderKind, Extractor, EnvKey, EnvName, SpecVersion, FolderRow, Metrics:
//   no generated type yet; each needs a Rust type with schemars in crates/cartograph-api.

// ---------- the map: nodes, links, commits (moved from lib/workspace.ts) ----------
export type Kind = "workspace" | "service" | "module" | "file";
export type LinkType = "imports" | "calls" | "http" | "event" | "db";

export interface Node {
  id: string;
  name: string;
  kind: Kind;
  parent: string | null;
  service: string; // owning service id ("" for the workspace root)
  path: string;
  summary: string;
  lang?: string; // a display label today ("TS"); the generated Node has a Lang
}

export interface Link {
  from: string; // `from` depends on `to`
  to: string;
  type: LinkType;
  added: string; // commit sha
  removed?: string;
  confidence?: Confidence; // absent = exact
}

export interface Commit {
  sha: string;
  date: string; // ISO day
  author: string;
  repo: string; // service id
  message: string;
}

// ---------- confidence ----------
export type ConfidenceLevel = "exact" | "likely" | "guessed";
/** exact 0.90-1.00, likely 0.60-0.89, guessed 0.30-0.59. Links under 0.30 are hidden by default. */
export interface Evidence { kind: "parsed" | "contract" | "name-match" | "env" | "history" | "model"; note: string; file?: string; line?: number }
export interface Confidence { level: ConfidenceLevel; score: number; evidence: Evidence[] }
export const levelOf = (score: number): ConfidenceLevel => (score >= 0.9 ? "exact" : score >= 0.6 ? "likely" : "guessed");
export const HIDDEN_BELOW = 0.3; // links under this score are hidden by default
/** Chip text, e.g. "Likely · 0.62". */
export const chipText = (c: Confidence) => `${c.level[0].toUpperCase()}${c.level.slice(1)} · ${c.score.toFixed(2)}`;
export const confidence = (score: number, ...evidence: Evidence[]): Confidence => ({ level: levelOf(score), score, evidence });

// ---------- repos and index state ----------
export type IndexState = "none" | "current" | "behind" | "syncing" | "rewritten" | "needs-migrate" | "failed";
export interface Repo {
  id: string; // = service id in the demo
  name: string; path: string; branch: string;
  indexedCommit: string; head: string; behind: number; dirty: boolean;
  commits: number;
}
export interface Freshness {
  state: IndexState;
  indexVersion: number; builtAt: string; // ISO
  behind: number; // commits behind HEAD, summed over repos
  repos?: { name: string; behind: number }[]; // per repo; behind above is their sum
  progress?: number; // 0-1 while syncing
  error?: { what: string; action: string };
}

// ---------- corrections ----------
export type CorrectionAction = "remove" | "retarget" | "set" | "add";
export interface Correction {
  id: string; action: CorrectionAction; target: string; // link or node id
  to?: string; value?: string; reason: string; by: string; at: string;
}

// ---------- contracts ----------
export interface Operation {
  id: string; method: "GET" | "POST" | "PUT" | "DELETE" | "EVENT"; path: string;
  handler?: string; // node id, missing when the operation has no handler
  callers: { node: string; confidence: Confidence }[];
}
export interface Spec {
  id: string; name: string; file: string; kind: "openapi" | "events"; service: string;
  operations: Operation[];
}

// ---------- entrypoints, concepts, coverage, owners ----------
export interface Entrypoint {
  id: string; kind: "http" | "event" | "job" | "cli"; label: string; node: string; service: string;
  consumers: number; // callers or subscribers found
}
export interface Concept { id: string; name: string; summary: string; nodes: string[]; domain: string }
export interface CoverageRow {
  id: string; label: string; done: number; total: number; target?: number; // percentages are done/total
  gap?: { what: string; why: string; action: string };
}
export interface Owner { id: string; name: string; kind: "team" | "person"; patterns: string[]; nodes: number }

// ---------- activity, workspace, search ----------
export interface ActivityItem { id: string; at: string; kind: "index" | "sync" | "correction" | "warning"; text: string; unread: boolean }
export interface WorkspaceSummary {
  id: string; name: string; path: string; demo: boolean;
  repos: number; services: number; modules: number; files: number; commits: number;
  state: IndexState; builtAt: string;
}
export interface SearchResult { id: string; kind: "service" | "module" | "file" | "concept" | "operation"; name: string; path: string; service: string }

// ---------- commit impact (A5) ----------
export interface CommitImpact {
  commit: Commit; prev?: string; next?: string; // first-parent neighbours, by sha
  addedLinks: Link[]; removedLinks: Link[];
  affected: string[]; // node ids that depend, directly or not, on what the commit touched
}

// ---------- ask and MCP ----------
export interface ToolCall { tool: McpToolName; args: string; result: string }
export interface Citation { handle: string; node?: string; commit?: string; checked: boolean }
export interface AskTurn { id: string; question: string; answer: string; toolCalls: ToolCall[]; citations: Citation[]; model: string }
export type McpToolName = "status" | "find" | "describe" | "neighbors" | "path" | "impact" | "sync" | "correct";
export interface McpTool { name: McpToolName; summary: string; access: "read" | "write" }

// ---------- models and settings ----------
export interface Provider { id: string; name: string; keySaved: boolean; models: string[]; local: boolean }
export interface AppearanceSettings {
  theme: "system" | "light" | "dark";
  contrast: "standard" | "more";
  motion: "follow" | "reduce" | "full";
  hues: "default" | "safe";
  size: 90 | 100 | 115 | 130;
  density: "comfortable" | "compact";
}

// ---------- metrics (moved from lib/workspace.ts) ----------
export interface Metrics { links: number; fanIn: number; fanOut: number; cross: number; churn: number }

// ---------- history (moved from lib/demo/history.ts) ----------
export interface Bin { start: number; end: number; total: number; byRepo: Record<string, number> }

/** One file's lifecycle. `kind: "rename"` moves it; `change` is a commit that touched it. */
export interface FileEvent {
  id: string;
  kind: "created" | "rename" | "change";
  date: string; // ISO day
  sha: string;
  author: string;
  text: string; // what happened, short
  from?: string; // earlier path, for renames
  similarity?: number; // 0..1 content kept across a rename
  note?: string;
}

/** History was rewritten: sync replayed from the merge base (decision on rewritten history). */
export interface Rewrite {
  id: string;
  repo: string; // service id
  date: string;
  kind: "force-push" | "rebase";
  mergeBase: string;
  linksRebuilt: number; // links kept separately, so they don't vanish
}

// ---------- symbols (moved from lib/demo/symbols.ts) ----------
export interface Sym {
  id: string; name: string; kind: "function" | "type"; file: string; lines: [number, number];
  sig: string; summary: string; // summary is written by a model
}
export interface SymLink { from: string; to: string; kind: "call" | "type" }

// ---------- settings (moved from lib/demo/settings.ts) ----------
export type Tone = "ok" | "warn" | "bad" | "off";
export type ProviderKind = "folder" | "github" | "gitlab" | "bitbucket" | "k8s" | "spec" | "traces" | "azure";
export interface Connection { id: string; kind: ProviderKind; ic: string; name: string; auth: string; provides: string[]; status: string; tone: Tone; when: string; action?: string }
export interface Extractor { id: string; name: string; note: string; v: string; files: string; on: boolean }
export type EnvName = "prod" | "staging" | "local";
export interface EnvKey { key: string; value: string; from: string; to: string; tone: "ok" | "off" }

// ---------- views and shell (moved from lib/demo/views-extra.ts and lib/demo/shell.ts) ----------
export interface SpecVersion { v: number; sha: string; summary: string; breaking?: { what: string; callers: string[] } }
export interface FolderRow {
  folder: string; // as written in the workspace file
  name?: string; // display name from folders[].name
  result: "repo" | "scope" | "skipped";
  repo?: string; // service id for repo and scope rows
  detail: string;
}
