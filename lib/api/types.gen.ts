// Generated from crates/cartograph-api (INT-01) by pnpm gen:types. Do not edit.

/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ActivityKind".
 */
export type ActivityKind = "index" | "sync" | "correction" | "warning";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "AdapterKind".
 */
export type AdapterKind = "built_in" | "query" | "subprocess";
/**
 * A repository in a workspace: its canonical path's registry key.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RepoId".
 */
export type RepoId = string;
/**
 * `lib/model.ts` Evidence kinds, `name-match` spelled `name_match`.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "EvidenceKind".
 */
export type EvidenceKind = "parsed" | "contract" | "name_match" | "env" | "history" | "model";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ConfidenceLevel".
 */
export type ConfidenceLevel = "exact" | "likely" | "guessed";
/**
 * A node: workspace, service, module, file, operation, channel, table, entrypoint, symbol.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "NodeId".
 */
export type NodeId = string;
/**
 * A link between two nodes.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkId".
 */
export type LinkId = string;
/**
 * Link kinds on every wire (ruling R28).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkKind".
 */
export type LinkKind = "imports" | "calls" | "http" | "event" | "db";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ConceptDecisionKind".
 */
export type ConceptDecisionKind = "keep_separate" | "same_concept";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "SpecKind".
 */
export type SpecKind = "openapi" | "events";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Method".
 */
export type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "EVENT";
/**
 * H3's four verbs (ruling R6).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "CorrectionAction".
 */
export type CorrectionAction = "remove" | "retarget" | "set" | "add";
/**
 * `lib/model.ts` IndexState, with `needs-migrate` spelled `needs_migrate`.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "IndexState".
 */
export type IndexState = "none" | "current" | "behind" | "syncing" | "rewritten" | "needs_migrate" | "failed";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "FolderResult".
 */
export type FolderResult = "repo" | "scope" | "skipped";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "EntrypointKind".
 */
export type EntrypointKind = "http" | "event" | "job" | "cli";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ErrorCode".
 */
export type ErrorCode =
  | "bad_request"
  | "unauthorized"
  | "forbidden_origin"
  | "not_found"
  | "conflict"
  | "precondition_failed"
  | "payload_too_large"
  | "unsupported_media_type"
  | "unprocessable"
  | "too_many_requests"
  | "index_unavailable"
  | "internal";
/**
 * Level of detail of a graph slice.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Lod".
 */
export type Lod = "workspace" | "service" | "module" | "file";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "NodeKind".
 */
export type NodeKind =
  | "workspace"
  | "service"
  | "module"
  | "file"
  | "operation"
  | "channel"
  | "table"
  | "entrypoint"
  | "symbol"
  | "owner"
  | "concept";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "AutoSync".
 */
export type AutoSync = "background" | "never";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Actor".
 */
export type Actor = "app" | "cli" | "mcp" | "hook" | "background";
/**
 * A fixed enum: nothing else can be started over HTTP.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "JobKind".
 */
export type JobKind = "sync" | "rebuild";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "JobState".
 */
export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Layer".
 */
export type Layer = "base" | "config" | "contract" | "correction" | "overlay";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Access".
 */
export type Access = "read" | "write";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "McpToolName".
 */
export type McpToolName = "status" | "find" | "describe" | "neighbors" | "path" | "impact" | "sync" | "correct";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Lang".
 */
export type Lang = "ts" | "js" | "go" | "py" | "php" | "other";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "OwnerKind".
 */
export type OwnerKind = "team" | "person";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Contrast".
 */
export type Contrast = "standard" | "more";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Density".
 */
export type Density = "comfortable" | "compact";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Hues".
 */
export type Hues = "default" | "safe";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Motion".
 */
export type Motion = "follow" | "reduce" | "full";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Theme".
 */
export type Theme = "system" | "light" | "dark";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "AskPlacement".
 */
export type AskPlacement = "panel" | "page";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ProviderKind".
 */
export type ProviderKind = "openai" | "anthropic" | "gemini" | "ollama" | "openai_compatible";
/**
 * `POST …/ask/runs`: the run's own stream.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RunEvent".
 */
export type RunEvent =
  | {
      data: {
        model?: string | null;
        run_id: string;
        thread_id: string;
        turn_id: string;
      };
      event: "run.started";
    }
  | {
      data: {
        steps: string[];
      };
      event: "plan";
    }
  | {
      data: {
        args: unknown;
        by: RunActor;
        call_id: string;
        tool: McpToolName;
      };
      event: "tool.call";
    }
  | {
      data: {
        call_id: string;
        handles: string[];
        ms: number;
        rows: number;
      };
      event: "tool.result";
    }
  | {
      data: {
        links: LinkId[];
        nodes: NodeId[];
        sentences: Sentence[];
      };
      event: "answer";
    }
  | {
      data: {
        budget_tokens: number;
        left_out: number;
        used_tokens: number;
      };
      event: "budget";
    }
  | {
      data: {
        text: string;
      };
      event: "tail";
    }
  | {
      data: Sentence;
      event: "sentence";
    }
  | {
      data: {
        sentences: Sentence[];
      };
      event: "answer.replace";
    }
  | {
      data: Usage;
      event: "usage";
    }
  | {
      data: {
        text: string;
      };
      event: "notice";
    }
  | {
      data: Problem;
      event: "error";
    }
  | {
      data: {
        stats: unknown;
        status: RunStatus;
      };
      event: "done";
    };
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RunActor".
 */
export type RunActor = "engine" | "model";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RunStatus".
 */
export type RunStatus = ("done" | "stopped" | "failed") | "fallback";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "SearchKind".
 */
export type SearchKind = "service" | "module" | "file" | "symbol" | "concept" | "operation";
/**
 * `GET /api/v1/events`: one stream for every workspace (ruling R15). The current state is sent on
 * connect.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ServerEvent".
 */
export type ServerEvent =
  | {
      data: {
        freshness: Freshness;
        ws: string;
      };
      event: "freshness";
    }
  | {
      data: {
        by: Actor;
        job_id: string;
        kind: JobKind;
        ws: string;
      };
      event: "job.started";
    }
  | {
      data: {
        done: number;
        eta_s?: number | null;
        job_id: string;
        repo?: string | null;
        stage: string;
        total: number;
      };
      event: "job.progress";
    }
  | {
      data: {
        error?: Problem | null;
        generation?: number | null;
        job_id: string;
        ok: boolean;
        took_ms: number;
      };
      event: "job.finished";
    }
  | {
      data: {
        fact_hash: string;
        generation: number;
        ws: string;
      };
      event: "index.changed";
    }
  | {
      data: {
        etag: string;
        file: string;
        ws: string;
      };
      event: "config.changed";
    }
  | {
      data: ActivityItem;
      event: "activity.new";
    }
  | {
      data: {};
      event: "workspaces.changed";
    };
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Bin".
 */
export type Bin = "day" | "week" | "month";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "MarkerKind".
 */
export type MarkerKind = "rename" | "rewrite";

/**
 * Every Cartograph HTTP wire type. Generated from crates/cartograph-api; do not edit.
 */
export interface CartographApi {
  ActivityItem: ActivityItem;
  ActivityRead: ActivityRead;
  Adapter: Adapter;
  AgentInstall: AgentInstall;
  AgentStatus: AgentStatus;
  AskCommand: AskCommand;
  Commit: Commit;
  CommitImpact: CommitImpact;
  Concept: Concept;
  ConceptDecision: ConceptDecision;
  ConfigFile: ConfigFile;
  ConfigWrite: ConfigWrite;
  Contract: Contract;
  Correction: Correction;
  CorrectionRequest: CorrectionRequest;
  CoverageRow: CoverageRow;
  CreateWorkspace: CreateWorkspace;
  CreatedWorkspace: CreatedWorkspace;
  DiscoverRequest: DiscoverRequest;
  Discovery: Discovery;
  Entrypoint: Entrypoint;
  Environment: Environment;
  ErrorBody: ErrorBody;
  FileDiff: FileDiff;
  Freshness: Freshness;
  FsEntry: FsEntry;
  GraphParams: GraphParams;
  GraphSlice: GraphSlice;
  ImpactRequest: ImpactRequest;
  ImpactResult: ImpactResult;
  IndexingSettings: IndexingSettings;
  Job: Job;
  JobRequest: JobRequest;
  KeyWrite: KeyWrite;
  Link: Link;
  LinkDetail: LinkDetail;
  McpTool: McpTool;
  Meta: Meta;
  Node: Node;
  NodeDetail: NodeDetail;
  Owner: Owner;
  PathResult: PathResult;
  Prefs: Prefs;
  Privacy: Privacy;
  Provider: Provider;
  ProviderTest: ProviderTest;
  ProviderWrite: ProviderWrite;
  Reach: Reach;
  ReadParams: ReadParams;
  Repo: Repo;
  Rule: Rule;
  RuleViolation: RuleViolation;
  RunEvent: RunEvent;
  RunRequest: RunRequest;
  SearchResult: SearchResult;
  Selection: Selection;
  ServerEvent: ServerEvent;
  Service: Service;
  SymbolCalls: SymbolCalls;
  Thread: Thread;
  TimelineDensity: TimelineDensity;
  TimelineMarker: TimelineMarker;
  TurnContext: TurnContext;
  Unresolved: Unresolved;
  Version: Version;
  WorkspaceSummary: WorkspaceSummary;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ActivityItem".
 */
export interface ActivityItem {
  at: string;
  id: string;
  kind: ActivityKind;
  text: string;
  unread: boolean;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ActivityRead".
 */
export interface ActivityRead {
  /**
   * Marks these read; empty marks everything read.
   */
  ids: string[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Adapter".
 */
export interface Adapter {
  allowed: boolean;
  kind: AdapterKind;
  name: string;
  version: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "AgentInstall".
 */
export interface AgentInstall {
  agent: string;
  shared?: boolean;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "AgentStatus".
 */
export interface AgentStatus {
  config_path?: string | null;
  detected: boolean;
  id: string;
  installed: boolean;
  name: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "AskCommand".
 */
export interface AskCommand {
  name: string;
  summary: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Commit".
 */
export interface Commit {
  author: string;
  /**
   * Corrected time on the merged timeline.
   */
  date: string;
  message: string;
  repo: RepoId;
  seq: number;
  sha: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "CommitImpact".
 */
export interface CommitImpact {
  added_links: Link[];
  /**
   * Node ids that depend, directly or not, on what the commit touched.
   */
  affected: NodeId[];
  commit: Commit;
  next?: string | null;
  prev?: string | null;
  removed_links: Link[];
}
/**
 * `lib/workspace.ts` Link: `type` becomes `kind`, and commits are given with their timeline step.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Link".
 */
export interface Link {
  added: CommitRef;
  confidence: Confidence;
  /**
   * `from` depends on `to`.
   */
  from: NodeId;
  id: LinkId;
  kind: LinkKind;
  removed?: CommitRef | null;
  to: NodeId;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "CommitRef".
 */
export interface CommitRef {
  seq: number;
  sha: string;
}
/**
 * The server sends `level`; clients never derive it from `score` (ruling R20).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Confidence".
 */
export interface Confidence {
  evidence: Evidence[];
  level: ConfidenceLevel;
  score: number;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Evidence".
 */
export interface Evidence {
  file?: string | null;
  kind: EvidenceKind;
  /**
   * 1-based.
   */
  line?: number | null;
  note: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Concept".
 */
export interface Concept {
  domain: string;
  id: string;
  name: string;
  nodes: NodeId[];
  summary: string;
}
/**
 * Pending ruling N7.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ConceptDecision".
 */
export interface ConceptDecision {
  decision: ConceptDecisionKind;
  dry_run?: boolean;
  reason: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ConfigFile".
 */
export interface ConfigFile {
  /**
   * Compare with `If-Match` on `PUT`.
   */
  etag: string;
  yaml: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ConfigWrite".
 */
export interface ConfigWrite {
  yaml: string;
}
/**
 * `lib/model.ts` Spec. Served from `…/contracts`.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Contract".
 */
export interface Contract {
  file: string;
  id: string;
  kind: SpecKind;
  name: string;
  operations: Operation[];
  service: NodeId;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Operation".
 */
export interface Operation {
  callers: Caller[];
  handler?: NodeId | null;
  id: NodeId;
  method: Method;
  path: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Caller".
 */
export interface Caller {
  confidence: Confidence;
  node: NodeId;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Correction".
 */
export interface Correction {
  action: CorrectionAction;
  at: string;
  by: string;
  confidence?: ConfidenceLevel | null;
  id: string;
  key: LinkKey;
  reason: string;
  /**
   * The correction no longer matches any link.
   */
  stale: boolean;
  to?: string | null;
}
/**
 * A link named by people-readable parts (ruling R28: the nested key object).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkKey".
 */
export interface LinkKey {
  from: string;
  kind: LinkKind;
  op?: string | null;
  table?: string | null;
  to?: string | null;
  topic?: string | null;
}
/**
 * `POST …/corrections`, and MCP `correct`. `reason` is 1 to 500 characters.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "CorrectionRequest".
 */
export interface CorrectionRequest {
  action: CorrectionAction;
  confidence?: ConfidenceLevel | null;
  dry_run?: boolean;
  key: LinkKey;
  reason: string;
  to?: string | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "CoverageRow".
 */
export interface CoverageRow {
  done: number;
  gap?: Problem | null;
  id: string;
  label: string;
  target?: number | null;
  total: number;
}
/**
 * A what, why and try triple inside a payload (not an HTTP error).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Problem".
 */
export interface Problem {
  try: string;
  what: string;
  why: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "CreateWorkspace".
 */
export interface CreateWorkspace {
  name?: string | null;
  path: string;
  workspace_file?: string | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "CreatedWorkspace".
 */
export interface CreatedWorkspace {
  job_id: string;
  workspace: WorkspaceSummary;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "WorkspaceSummary".
 */
export interface WorkspaceSummary {
  built_at?: string | null;
  commits: number;
  demo: boolean;
  files: number;
  id: string;
  modules: number;
  name: string;
  path: string;
  repos: number;
  services: number;
  state: IndexState;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "DiscoverRequest".
 */
export interface DiscoverRequest {
  path: string;
  workspace_file?: string | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Discovery".
 */
export interface Discovery {
  /**
   * From a measured rate, not a guess.
   */
  estimate_s?: number | null;
  folders: DiscoveredFolder[];
  services: Service[];
}
/**
 * One row of the C1 and C2 resolution table.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "DiscoveredFolder".
 */
export interface DiscoveredFolder {
  folder: string;
  name: string;
  reason?: string | null;
  repo?: string | null;
  result: FolderResult;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Service".
 */
export interface Service {
  domain?: string | null;
  id: NodeId;
  name: string;
  paths: string[];
  repo: RepoId;
  tags: string[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Entrypoint".
 */
export interface Entrypoint {
  consumers: number;
  id: string;
  kind: EntrypointKind;
  label: string;
  node: NodeId;
  service: NodeId;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Environment".
 */
export interface Environment {
  name: string;
  values: {
    [k: string]: string;
  };
}
/**
 * Every error response, whatever the status code.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ErrorBody".
 */
export interface ErrorBody {
  error: ApiError;
  schema: number;
}
/**
 * E1 on the wire: what, why and what to try.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ApiError".
 */
export interface ApiError {
  code: ErrorCode;
  details?: unknown;
  try: string;
  what: string;
  why: string;
}
/**
 * What a write to a committed file did, or would do.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "FileDiff".
 */
export interface FileDiff {
  applied: boolean;
  diff: string;
  file: string;
}
/**
 * What every read says about how current the index is (D1).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Freshness".
 */
export interface Freshness {
  /**
   * Commits behind, summed over repos.
   */
  behind: number;
  built_at?: string | null;
  error?: Problem | null;
  format_version: number;
  /**
   * 0 to 1 while syncing.
   */
  progress?: number | null;
  repos: RepoBehind[];
  state: IndexState;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RepoBehind".
 */
export interface RepoBehind {
  behind: number;
  behind_capped: boolean;
  name: string;
  repo: RepoId;
}
/**
 * `GET /api/v1/fs/list`: names and three flags only (ruling R25).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "FsEntry".
 */
export interface FsEntry {
  has_children: boolean;
  has_workspace_file: boolean;
  is_repo: boolean;
  name: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "GraphParams".
 */
export interface GraphParams {
  at?: string | null;
  branch?: string | null;
  /**
   * Comma-separated link kinds.
   */
  kinds?: string | null;
  lod?: Lod | null;
  min_confidence?: ConfidenceLevel | null;
  owner?: string | null;
  /**
   * `from..to`, two `at` values.
   */
  range?: string | null;
  repo?: RepoId | null;
  scope?: NodeId | null;
}
/**
 * `GET …/graph`: one level's nodes and links, columnar so 60k nodes and 200k links stay small.
 * Column `i` of every node array describes node `i`; links index into the node columns.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "GraphSlice".
 */
export interface GraphSlice {
  at?: CommitRef | null;
  links: LinkColumns;
  lod: Lod;
  nodes: NodeColumns;
  /**
   * Set for `range=` requests: links live at any step in `[from, to)`.
   *
   * @minItems 2
   * @maxItems 2
   */
  range?: [CommitRef, CommitRef] | null;
  scope?: NodeId | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkColumns".
 */
export interface LinkColumns {
  added_seq: number[];
  from: number[];
  id: LinkId[];
  kind: LinkKind[];
  level: ConfidenceLevel[];
  /**
   * -1 while the link is live.
   */
  removed_seq: number[];
  score: number[];
  to: number[];
  /**
   * How many file-level links this aggregated link stands for.
   */
  weight: number[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "NodeColumns".
 */
export interface NodeColumns {
  /**
   * Number of children one level down, for the expand affordance.
   */
  children: number[];
  id: NodeId[];
  kind: NodeKind[];
  name: string[];
  /**
   * Index of the parent node in these columns, or -1.
   */
  parent: number[];
  size: number[];
  /**
   * Stored layout position (I2).
   */
  x: number[];
  y: number[];
}
/**
 * `POST …/impact` carries exactly one of these; it changes nothing.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ImpactRequest".
 */
export interface ImpactRequest {
  paths?: string[] | null;
  ref?: string | null;
  since?: string | null;
  working_tree?: boolean | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ImpactResult".
 */
export interface ImpactResult {
  exact: NodeId[];
  guessed: NodeId[];
  likely: NodeId[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "IndexingSettings".
 */
export interface IndexingSettings {
  auto_sync: AutoSync;
  extractors: ExtractorSetting[];
  hooks: boolean;
  hourly_fetch: boolean;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ExtractorSetting".
 */
export interface ExtractorSetting {
  enabled: boolean;
  name: string;
  version: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Job".
 */
export interface Job {
  by: Actor;
  error?: Problem | null;
  id: string;
  kind: JobKind;
  started_at: string;
  state: JobState;
  took_ms?: number | null;
  ws: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "JobRequest".
 */
export interface JobRequest {
  kind: JobKind;
  repo?: RepoId | null;
}
/**
 * The only request body with a key.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "KeyWrite".
 */
export interface KeyWrite {
  key: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkDetail".
 */
export interface LinkDetail {
  correction_id?: string | null;
  interval: Interval;
  layer: Layer;
  link: Link;
  replaced_by?: LinkId | null;
}
/**
 * `[from_seq, to_seq)` on the merged first-parent timeline.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Interval".
 */
export interface Interval {
  from_seq: number;
  to_seq?: number | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "McpTool".
 */
export interface McpTool {
  access: Access;
  /**
   * One of the five Ask registers.
   */
  ask: boolean;
  name: McpToolName;
  summary: string;
}
/**
 * `GET /api/v1/meta` (A4).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Meta".
 */
export interface Meta {
  format_version: number;
  now: string;
  schema: number;
  version: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Node".
 */
export interface Node {
  id: NodeId;
  kind: NodeKind;
  lang?: Lang | null;
  name: string;
  parent?: NodeId | null;
  path: string;
  /**
   * Owning service; null for the workspace root.
   */
  service?: NodeId | null;
  summary?: string | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "NodeDetail".
 */
export interface NodeDetail {
  added?: CommitRef | null;
  metrics: Metrics;
  node: Node;
  owners: string[];
  removed?: CommitRef | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Metrics".
 */
export interface Metrics {
  /**
   * Links added or removed in the 90 days before `at`.
   */
  churn: number;
  cross: number;
  fan_in: number;
  fan_out: number;
  links: number;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Owner".
 */
export interface Owner {
  id: string;
  kind: OwnerKind;
  name: string;
  nodes: number;
  patterns: string[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "PathResult".
 */
export interface PathResult {
  hops: Link[];
  /**
   * A path's level is its weakest hop.
   */
  level: ConfidenceLevel;
}
/**
 * Preferences moved to the server by ruling R1.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Prefs".
 */
export interface Prefs {
  appearance: Appearance;
  ask_placement: AskPlacement;
  keymap: {
    [k: string]: string;
  };
  pins: NodeId[];
  sidebar_open: boolean;
}
/**
 * `lib/model.ts` AppearanceSettings. `size` is a percentage: 90, 100, 115 or 130.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Appearance".
 */
export interface Appearance {
  contrast: Contrast;
  density: Density;
  hues: Hues;
  motion: Motion;
  size: number;
  theme: Theme;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Privacy".
 */
export interface Privacy {
  /**
   * Pending ruling N2: refuse every non-loopback provider.
   */
  local_only: boolean;
  summaries: boolean;
}
/**
 * A key is write-only: the browser learns `key_saved` and nothing else (G4).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Provider".
 */
export interface Provider {
  base_url?: string | null;
  id: string;
  key_saved: boolean;
  kind: ProviderKind;
  local: boolean;
  models: string[];
  name: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ProviderTest".
 */
export interface ProviderTest {
  error?: Problem | null;
  models: string[];
  ms: number;
  ok: boolean;
}
/**
 * The only request bodies with a `base_url`.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ProviderWrite".
 */
export interface ProviderWrite {
  base_url?: string | null;
  kind: ProviderKind;
  name: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Reach".
 */
export interface Reach {
  links: LinkId[];
  nodes: NodeId[];
}
/**
 * Common read parameters. Every read takes them; each route documents which it honours.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ReadParams".
 */
export interface ReadParams {
  at?: string | null;
  cursor?: string | null;
  /**
   * Comma-separated optional parts, e.g. `summary,metrics`.
   */
  include?: string | null;
  /**
   * Default 50, maximum 1,000.
   */
  limit?: number | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Repo".
 */
export interface Repo {
  /**
   * Commits behind, capped at 1,000.
   */
  behind: number;
  /**
   * True when `behind` hit the cap; show it as "1000+".
   */
  behind_capped: boolean;
  branch: string;
  commits: number;
  /**
   * Computed after the answer and cached; null on a first read (D1).
   */
  dirty?: boolean | null;
  head: string;
  id: RepoId;
  indexed_commit?: string | null;
  name: string;
  path: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Rule".
 */
export interface Rule {
  description: string;
  id: string;
  kind: string;
  violations: number;
}
/**
 * Natural keys, as in cartograph.baseline.json.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RuleViolation".
 */
export interface RuleViolation {
  baselined: boolean;
  from: NodeId;
  kind: LinkKind;
  op?: string | null;
  rule: string;
  to: NodeId;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Sentence".
 */
export interface Sentence {
  cites: string[];
  p: number;
  supported: boolean;
  text: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Usage".
 */
export interface Usage {
  cached_tokens: number;
  completion_tokens: number;
  prompt_tokens: number;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RunRequest".
 */
export interface RunRequest {
  at?: string | null;
  /**
   * Node ids picked with @.
   */
  mentions: NodeId[];
  question: string;
  thread_id?: string | null;
  ultra: boolean;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "SearchResult".
 */
export interface SearchResult {
  id: NodeId;
  kind: SearchKind;
  name: string;
  path: string;
  service?: NodeId | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Selection".
 */
export interface Selection {
  model: string;
  provider: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "SymbolCalls".
 */
export interface SymbolCalls {
  callees: FileGroup[];
  callers: FileGroup[];
  symbol: Node;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "FileGroup".
 */
export interface FileGroup {
  file: NodeId;
  symbols: NodeId[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Thread".
 */
export interface Thread {
  created_at: string;
  id: string;
  title: string;
  turns: AskTurn[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "AskTurn".
 */
export interface AskTurn {
  answer: Sentence[];
  at?: string | null;
  citations: Citation[];
  fact_hash: string;
  id: string;
  model?: string | null;
  question: string;
  stale: boolean;
  tool_calls: ToolCall[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Citation".
 */
export interface Citation {
  checked: boolean;
  commit?: string | null;
  handle: string;
  link?: LinkId | null;
  node?: NodeId | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ToolCall".
 */
export interface ToolCall {
  args: unknown;
  call_id: string;
  handles: string[];
  ms: number;
  rows: number;
  tool: McpToolName;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "TimelineDensity".
 */
export interface TimelineDensity {
  bin: Bin;
  bins: DensityBin[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "DensityBin".
 */
export interface DensityBin {
  commits: number;
  links_added: number;
  links_removed: number;
  start: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "TimelineMarker".
 */
export interface TimelineMarker {
  at: CommitRef;
  kind: MarkerKind;
  label: string;
}
/**
 * The retrieval inspector (board A6).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "TurnContext".
 */
export interface TurnContext {
  blocks: ContextBlock[];
  budget_tokens: number;
  system_prompt: string;
  usage?: Usage | null;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ContextBlock".
 */
export interface ContextBlock {
  cited: boolean;
  dropped: boolean;
  label: string;
  tokens: number;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Unresolved".
 */
export interface Unresolved {
  file: NodeId;
  line: number;
  reason: string;
  text: string;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Version".
 */
export interface Version {
  version: string;
}
