// Shapes the local index API will return (decision A1). lib/demo/* fills them with northwind data;
// lib/source.ts is the only place components read them from.

import type { Commit, Link } from "./workspace.ts";

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
