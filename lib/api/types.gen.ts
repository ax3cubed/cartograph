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
export type SpecKind = "openapi" | "events" | "proto";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Method".
 */
export type Method = ("GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "HEAD" | "TRACE" | "EVENT") | "RPC";
/**
 * Whether the code agrees with the spec about an operation (XS-08 drift).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "OperationStatus".
 */
export type OperationStatus = "sync" | "spec-only";
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
  | "forbidden_host"
  | "not_found"
  | "method_not_allowed"
  | "ambiguous"
  | "conflict"
  | "precondition_failed"
  | "payload_too_large"
  | "unsupported_media_type"
  | "unprocessable"
  | "too_large"
  | "too_many_requests"
  | "index_unavailable"
  | "timeout"
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
 * via the `definition` "LineageVia".
 */
export type LineageVia = "add" | "rename_exact" | "rename_similar";
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Change".
 */
export type Change = "added" | "removed";
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
 * via the `definition` "PathDirection".
 */
export type PathDirection = "forward" | "backward" | "mixed";
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
      data: {
        ids: string[];
      };
      event: "activity.read";
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
  ExtractorInfo: ExtractorInfo;
  FileDiff: FileDiff;
  Freshness: Freshness;
  FsEntry: FsEntry;
  GraphParams: GraphParams;
  GraphSlice: GraphSlice;
  ImpactRequest: ImpactRequest;
  ImpactResult: ImpactResult;
  IndexRun: IndexRun;
  IndexingSettings: IndexingSettings;
  Job: Job;
  JobRequest: JobRequest;
  KeyWrite: KeyWrite;
  Lineage: Lineage;
  Link: Link;
  LinkChange: LinkChange;
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
  RankedNode: RankedNode;
  Reach: Reach;
  ReadParams: ReadParams;
  Repo: Repo;
  Rule: Rule;
  RuleViolation: RuleViolation;
  RunEvent: RunEvent;
  RunRequest: RunRequest;
  SearchResult: SearchResult;
  SearchResults: SearchResults;
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
 * One row of the activity list (FE-16), kept in the workspace's ask.db. `text` and `detail` hold
 * repo and author names, which are untrusted: the app renders them as text. Never file contents.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ActivityItem".
 */
export interface ActivityItem {
  at: string;
  /**
   * Who ran it and which version: "Indexed by the background sync · cartograph 0.4.0".
   */
  detail: string;
  /**
   * The app view the row opens, as a path and hash.
   */
  href: string;
  /**
   * `<workspace>:<row>`.
   */
  id: string;
  kind: ActivityKind;
  /**
   * "billing-service synced to 3e0d7a2 · 3 commits by Tom Ellery and Jon Reyes", at most 120
   * characters.
   */
  text: string;
  unread: boolean;
  ws: string;
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
  counts: ContractCounts;
  file: string;
  id: string;
  kind: SpecKind;
  name: string;
  operations: Operation[];
  service: NodeId;
  /**
   * The service's undocumented routes, on its first spec.
   */
  undocumented: UndocumentedRoute[];
}
/**
 * The four counts of the Contracts view.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ContractCounts".
 */
export interface ContractCounts {
  operations: number;
  spec_only: number;
  sync: number;
  undocumented: number;
}
/**
 * An operation of a spec. `id` is `o<row>`, not a node.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Operation".
 */
export interface Operation {
  callers: Caller[];
  handler?: NodeId | null;
  id: NodeId;
  method: Method;
  operation_id?: string | null;
  /**
   * With the spec's base path, as the spec writes it: `/v1/orders/{id}`.
   */
  path: string;
  /**
   * For an event operation (an AsyncAPI channel, XS-16): the files that publish to the channel
   * and the files that subscribe to it, with how sure the best event link of each is.
   */
  publishers?: Caller[];
  status: OperationStatus;
  subscribers?: Caller[];
  /**
   * From the spec: untrusted text (F8), at most 200 characters.
   */
  summary?: string | null;
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
 * A route of a service that has a spec, which no spec lists.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "UndocumentedRoute".
 */
export interface UndocumentedRoute {
  /**
   * The file that serves it.
   */
  handler: NodeId;
  method: Method;
  /**
   * The route's template: `/orders/{}`.
   */
  path: string;
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
  /**
   * Also write `cartograph.yaml` in the workspace folder (C7); never overwrites one.
   */
  manifest?: boolean;
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
  /**
   * The file that rule read, repository-relative: `cartograph.yaml`, a catalog or workspace file,
   * or a package's own manifest; none for `repo` (board S04's "Detected from").
   */
  detect_source?: string | null;
  /**
   * The C6 rule that found it: `manifest`, `scope`, `backstage`, `pnpm`, `npm-workspaces`,
   * `go.work`, `nx`, `cargo`, `composer` or `repo` (board S04).
   */
  detected_by?: string | null;
  domain?: string | null;
  id: NodeId;
  /**
   * A workspace package with no deployable marker (C6).
   */
  library: boolean;
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
  /**
   * The workspace's default: `default_environment`, else `prod`, else the first.
   */
  is_default: boolean;
  name: string;
  /**
   * Key to value, as of the newest commit: only values that classify as locators (XS-10).
   */
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
 * An extractor the index holds facts from (IDX-21), with the live files it read.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ExtractorInfo".
 */
export interface ExtractorInfo {
  files: number;
  name: string;
  /**
   * The newest version that wrote facts.
   */
  version: string;
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
 * `GET /api/v1/fs/list`: names and three flags only (ruling R25). The list reads only the folder
 * asked for, never inside a child, so listing home can't raise macOS privacy prompts for Desktop,
 * Documents or Downloads; the flags are null (unknown) and a folder is read when it is opened.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "FsEntry".
 */
export interface FsEntry {
  has_children?: boolean | null;
  has_workspace_file?: boolean | null;
  is_repo?: boolean | null;
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
  /**
   * A correction in `cartograph.overrides.yaml` decides this link (XS-12).
   */
  corrected: boolean[];
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
 * What depends on the changed nodes, directly or not. A node's level is the weakest link on its
 * strongest path from a changed node.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "ImpactResult".
 */
export interface ImpactResult {
  /**
   * The nodes the input named or changed.
   */
  changed: NodeId[];
  exact: NodeId[];
  guessed: NodeId[];
  likely: NodeId[];
  /**
   * The services the affected nodes belong to.
   */
  services: NodeId[];
  /**
   * A resource cap (50k visited nodes, depth 12, 5 s) stopped the walk.
   */
  truncated: boolean;
}
/**
 * One index run (IDX-22): when it ran, the binary that wrote it, and its coverage totals. The
 * totals are absent on a run that stopped before its end.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "IndexRun".
 */
export interface IndexRun {
  files_parsed?: number | null;
  files_total?: number | null;
  finished_at?: string | null;
  id: number;
  imports_resolved?: number | null;
  imports_total?: number | null;
  started_at: string;
  writer_version: string;
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
 * One row of `…/nodes/{id}/history` (board G10): a path the node had, and how it got it.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Lineage".
 */
export interface Lineage {
  from: CommitRef;
  path: string;
  /**
   * Percent of content kept, for `rename_similar`.
   */
  similarity?: number | null;
  /**
   * Null while the node still has this path.
   */
  to?: CommitRef | null;
  via: LineageVia;
}
/**
 * One row of `…/nodes/{id}/changes`: a commit that added or removed a link touching the scope.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkChange".
 */
export interface LinkChange {
  change: Change;
  commit: Commit;
  link: Link;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkDetail".
 */
export interface LinkDetail {
  /**
   * What the passes found, when a correction changed this link (`layer` is `correction`).
   */
  base?: LinkBase | null;
  closed?: Commit | null;
  /**
   * How `env` resolves the call, in order: the key the call site reads, the value it has there
   * and the host that value names.
   */
  config: LinkConfig[];
  /**
   * The entry of `cartograph.overrides.yaml` that decided this link, `c<position>-<fingerprint>`.
   */
  correction_id?: string | null;
  /**
   * The environment `config` is for (`?env=`, else the default); null when the link doesn't
   * depend on one (XS-09).
   */
  env?: string | null;
  /**
   * Every environment the link holds in.
   */
  envs: string[];
  /**
   * The code behind the link, by its role in it.
   */
  evidence: EvidenceGroups;
  interval: Interval;
  /**
   * The layer whose answer the map shows. Null until precedence layers exist (XS-08 onwards).
   */
  layer?: Layer | null;
  link: Link;
  /**
   * The commit that opened the link, and the one that closed it (null while live).
   */
  opened: Commit;
  /**
   * HTTP links: the call's method and route template.
   */
  operation?: LinkOperation | null;
  /**
   * A link between the same two services that closed in the commit this one opened.
   */
  replaced?: LinkId | null;
}
/**
 * A link's values before a correction (XS-12), for the inspector and for undo.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkBase".
 */
export interface LinkBase {
  level: ConfidenceLevel;
  score: number;
  /**
   * The far end the passes found; null when it is unchanged.
   */
  to?: NodeId | null;
}
/**
 * One step of the chain from a call site to a service: key, value, host (XS-09).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "LinkConfig".
 */
export interface LinkConfig {
  /**
   * The host in `value`: the column the inspector shows per environment.
   */
  host: string;
  /**
   * `CATALOG_URL`.
   */
  key: string;
  /**
   * The file that sets it, repo-relative as the index sees paths; `cartograph.yaml` for a
   * value set by hand.
   */
  source: string;
  /**
   * What the key is set to: a URL without userinfo, query or fragment, or `host[:port]`.
   */
  value: string;
}
/**
 * Evidence by role. Contract rows wait for contracts (XS-08).
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "EvidenceGroups".
 */
export interface EvidenceGroups {
  caller: EvidenceSite[];
  contract: EvidenceSite[];
  handler: EvidenceSite[];
}
/**
 * One place in a file that backs a link. `snippet` is repo text: show it as text, never markup.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "EvidenceSite".
 */
export interface EvidenceSite {
  end_line: number;
  extractor: string;
  file: string;
  /**
   * `literal`, `template` or `inferred`.
   */
  kind: string;
  /**
   * 1-based.
   */
  line: number;
  /**
   * At most 5 lines and 400 characters, secrets masked; null when the repo can't be read.
   */
  snippet?: string | null;
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
 * via the `definition` "LinkOperation".
 */
export interface LinkOperation {
  /**
   * Lower case: `get`.
   */
  method: string;
  /**
   * `/orders/{}`.
   */
  template: string;
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
  direction: PathDirection;
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
 * One row of `…/nodes/{id}/ranking`: a file in the scope and how much depends on it (blast
 * radius), as `lib/workspace.ts` `ranking` computes it.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "RankedNode".
 */
export interface RankedNode {
  /**
   * `used_by` now minus `used_by` 90 days before `at`.
   */
  delta: number;
  node: Node;
  /**
   * Live links into the file at `at`.
   */
  used_by: number;
  /**
   * Live links out of it at `at`.
   */
  uses: number;
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Reach".
 */
export interface Reach {
  links: LinkId[];
  nodes: NodeId[];
  /**
   * A resource cap stopped the walk.
   */
  truncated: boolean;
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
   * An environment name (XS-09), for the reads that depend on one. Absent: the workspace's
   * default, `default_environment` in the manifest, else `prod`, else the first.
   */
  env?: string | null;
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
 * `GET …/search`: the hits best first (bm25, then fan-in, then shorter paths), and when nothing
 * matched, the nearest name (board G05's "did you mean").
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "SearchResults".
 */
export interface SearchResults {
  near?: SearchResult | null;
  results: SearchResult[];
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
  /**
   * The repos `by_repo` counts, in that order (board G04 stacks by repo).
   */
  repos: RepoId[];
}
/**
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "DensityBin".
 */
export interface DensityBin {
  /**
   * Commits per repo, in the order of `TimelineDensity.repos`; empty in a node's series.
   */
  by_repo: number[];
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
 * A reference that didn't resolve (IDX-22). `text` is the raw specifier: untrusted text (F8),
 * shown as text only.
 *
 * This interface was referenced by `CartographApi`'s JSON-Schema
 * via the `definition` "Unresolved".
 */
export interface Unresolved {
  file: NodeId;
  line: number;
  /**
   * The file's path now, repo-prefixed as node paths are.
   */
  path: string;
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
