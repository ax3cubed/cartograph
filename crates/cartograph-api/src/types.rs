//! Every type that crosses the HTTP wire. snake_case everywhere (ruling R5).
//!
//! These replace `lib/model.ts` and the shapes in `lib/workspace.ts`; the TypeScript is generated
//! from them into `lib/api/types.gen.ts`. Renames from the TypeScript are listed in
//! docs/research/phase0.md under INT-01.

use std::collections::BTreeMap;

use cartograph_core::{LinkId, NodeId, RepoId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A commit sha, a date (`YYYY-MM-DD` or RFC 3339), or `now`.
pub type At = String;
/// RFC 3339 timestamp.
pub type Timestamp = String;
/// A workspace id: 8 hex characters from the registry.
pub type WorkspaceId = String;

// ---------------------------------------------------------------- envelope and errors

/// Every JSON read returns this envelope.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Envelope<T> {
    /// Envelope version. Adding fields is free; removing one bumps it.
    pub schema: u32,
    /// The index generation the answer was read from (also in `Cartograph-Generation`).
    pub generation: u64,
    pub data: T,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

/// Every error response, whatever the status code.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub schema: u32,
    pub error: ApiError,
}

/// E1 on the wire: what, why and what to try.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApiError {
    pub code: ErrorCode,
    pub what: String,
    pub why: String,
    #[serde(rename = "try")]
    pub action: String,
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 400: a malformed parameter or body.
    BadRequest,
    /// 401: the bearer token is missing or wrong.
    Unauthorized,
    /// 403: the Origin header doesn't match, or `Sec-Fetch-Site` is `cross-site`.
    ForbiddenOrigin,
    /// 403: the Host header isn't on the allow list (DNS rebinding).
    ForbiddenHost,
    /// 404.
    NotFound,
    /// 405: OPTIONS, or a method the route doesn't take. There is no CORS.
    MethodNotAllowed,
    /// 400: a node reference matches more than one node; `details.candidates` lists them.
    Ambiguous,
    /// 409: a job of that kind is already running.
    Conflict,
    /// 412: `If-Match` failed; the file changed on disk.
    PreconditionFailed,
    /// 413: a body over the limit.
    PayloadTooLarge,
    /// 415: a write without `Content-Type: application/json`.
    UnsupportedMediaType,
    /// 422: valid JSON, invalid request (a correction key that matches nothing).
    Unprocessable,
    /// 422: the answer is over the response cap; `try` names a narrower scope.
    TooLarge,
    /// 429: too many concurrent requests or Ask runs.
    TooManyRequests,
    /// 503: the index needs a migration or a rebuild.
    IndexUnavailable,
    /// 503: the query ran past its deadline.
    Timeout,
    /// 500. The body never carries SQL, a path outside the workspace or a stack trace.
    Internal,
}

impl ErrorCode {
    /// The HTTP status for this code. `--json` and MCP carry the code alone.
    ///
    /// Decided in the phase 2 review, and kept: `ambiguous` is 400 (the caller must name the node
    /// more precisely; `details.candidates` says how), `too_large` is 422 (the request is valid but
    /// its answer is over the cap; 413 is for the request body), and `timeout` is 503 with
    /// `Retry-After` (the server ran out of time, the request isn't wrong).
    pub fn status(self) -> u16 {
        match self {
            Self::BadRequest | Self::Ambiguous => 400,
            Self::Unauthorized => 401,
            Self::ForbiddenOrigin | Self::ForbiddenHost => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::Conflict => 409,
            Self::PreconditionFailed => 412,
            Self::PayloadTooLarge => 413,
            Self::UnsupportedMediaType => 415,
            Self::Unprocessable | Self::TooLarge => 422,
            Self::TooManyRequests => 429,
            Self::Internal => 500,
            Self::IndexUnavailable | Self::Timeout => 503,
        }
    }
}

impl<T> Envelope<T> {
    /// A first (or only) page.
    pub fn new(generation: u64, data: T) -> Self {
        Self {
            schema: crate::SCHEMA_VERSION,
            generation,
            data,
            next_cursor: None,
            truncated: false,
        }
    }
}

impl ApiError {
    pub fn new(code: ErrorCode, what: &str, why: &str, action: &str) -> Self {
        Self {
            code,
            what: what.into(),
            why: why.into(),
            action: action.into(),
            details: None,
        }
    }
}

impl From<ApiError> for ErrorBody {
    fn from(error: ApiError) -> Self {
        Self {
            schema: crate::SCHEMA_VERSION,
            error,
        }
    }
}

/// Page size when a read names none (F2, SRV-07).
pub const DEFAULT_LIMIT: u32 = 50;
/// The largest page any read returns.
pub const MAX_LIMIT: u32 = 1000;

/// Checks a `limit` parameter: absent means [`DEFAULT_LIMIT`]; 0 or over [`MAX_LIMIT`] is refused
/// rather than quietly changed.
pub fn page_limit(limit: Option<u32>) -> Result<u32, Box<ApiError>> {
    match limit {
        None => Ok(DEFAULT_LIMIT),
        Some(n @ 1..=MAX_LIMIT) => Ok(n),
        Some(_) => Err(Box::new(ApiError::new(
            ErrorCode::BadRequest,
            "limit is out of range",
            "a page holds 1 to 1,000 rows",
            "pass a limit from 1 to 1000, and follow next_cursor for more",
        ))),
    }
}

/// Common read parameters. Every read takes them; each route documents which it honours.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ReadParams {
    pub at: Option<At>,
    /// Default 50, maximum 1,000.
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    /// Comma-separated optional parts, e.g. `summary,metrics`.
    pub include: Option<String>,
    /// An environment name (XS-09), for the reads that depend on one. Absent: the workspace's
    /// default, `default_environment` in the manifest, else `prod`, else the first.
    pub env: Option<String>,
}

// ---------------------------------------------------------------- meta and workspaces

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Version {
    pub version: String,
}

/// `GET /api/v1/meta` (A4).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Meta {
    pub version: String,
    pub format_version: u32,
    pub now: Timestamp,
    pub schema: u32,
}

/// `lib/model.ts` IndexState, with `needs-migrate` spelled `needs_migrate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    None,
    Current,
    Behind,
    Syncing,
    Rewritten,
    NeedsMigrate,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub name: String,
    pub path: String,
    pub demo: bool,
    pub repos: u32,
    pub services: u32,
    pub modules: u32,
    pub files: u32,
    pub commits: u32,
    pub state: IndexState,
    pub built_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Repo {
    pub id: RepoId,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub indexed_commit: Option<String>,
    pub head: String,
    /// Commits behind, capped at 1,000.
    pub behind: u32,
    /// True when `behind` hit the cap; show it as "1000+".
    pub behind_capped: bool,
    /// Computed after the answer and cached; null on a first read (D1).
    pub dirty: Option<bool>,
    pub commits: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RepoBehind {
    pub repo: RepoId,
    pub name: String,
    pub behind: u32,
    pub behind_capped: bool,
}

/// What every read says about how current the index is (D1).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Freshness {
    pub state: IndexState,
    pub format_version: u32,
    pub built_at: Option<Timestamp>,
    /// Commits behind, summed over repos.
    pub behind: u32,
    pub repos: Vec<RepoBehind>,
    /// 0 to 1 while syncing.
    pub progress: Option<f32>,
    pub error: Option<Problem>,
}

/// A what, why and try triple inside a payload (not an HTTP error).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    pub what: String,
    pub why: String,
    #[serde(rename = "try")]
    pub action: String,
}

// ---------------------------------------------------------------- graph

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Workspace,
    Service,
    Module,
    File,
    Operation,
    Channel,
    Table,
    Entrypoint,
    Symbol,
    Owner,
    Concept,
}

/// Link kinds on every wire (ruling R28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Imports,
    Calls,
    Http,
    Event,
    Db,
}

/// Level of detail of a graph slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lod {
    Workspace,
    Service,
    Module,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceLevel {
    /// 0.90 to 1.00: both ends anchored.
    Exact,
    /// 0.60 to 0.89.
    Likely,
    /// 0.30 to 0.59. Under 0.30 is stored but hidden by default.
    Guessed,
}

/// `lib/model.ts` Evidence kinds, `name-match` spelled `name_match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Parsed,
    Contract,
    NameMatch,
    Env,
    History,
    Model,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub note: String,
    pub file: Option<String>,
    /// 1-based.
    pub line: Option<u32>,
}

/// The server sends `level`; clients never derive it from `score` (ruling R20).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Confidence {
    pub level: ConfidenceLevel,
    pub score: f32,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    Ts,
    Js,
    Go,
    Py,
    Php,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    /// Owning service; null for the workspace root.
    pub service: Option<NodeId>,
    pub path: String,
    pub summary: Option<String>,
    pub lang: Option<Lang>,
}

/// `lib/workspace.ts` Link: `type` becomes `kind`, and commits are given with their timeline step.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Link {
    pub id: LinkId,
    /// `from` depends on `to`.
    pub from: NodeId,
    pub to: NodeId,
    pub kind: LinkKind,
    pub added: CommitRef,
    pub removed: Option<CommitRef>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitRef {
    pub sha: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Commit {
    pub sha: String,
    pub seq: u64,
    /// Corrected time on the merged timeline.
    pub date: Timestamp,
    pub author: String,
    pub repo: RepoId,
    pub message: String,
}

/// `GET …/graph`: one level's nodes and links, columnar so 60k nodes and 200k links stay small.
/// Column `i` of every node array describes node `i`; links index into the node columns.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphSlice {
    pub lod: Lod,
    pub scope: Option<NodeId>,
    pub at: Option<CommitRef>,
    /// Set for `range=` requests: links live at any step in `[from, to)`.
    pub range: Option<[CommitRef; 2]>,
    pub nodes: NodeColumns,
    pub links: LinkColumns,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeColumns {
    pub id: Vec<NodeId>,
    pub kind: Vec<NodeKind>,
    pub name: Vec<String>,
    /// Index of the parent node in these columns, or -1.
    pub parent: Vec<i32>,
    /// Stored layout position (I2).
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub size: Vec<f32>,
    /// Number of children one level down, for the expand affordance.
    pub children: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkColumns {
    pub id: Vec<LinkId>,
    pub from: Vec<u32>,
    pub to: Vec<u32>,
    pub kind: Vec<LinkKind>,
    pub level: Vec<ConfidenceLevel>,
    pub score: Vec<f32>,
    /// A correction in `cartograph.overrides.yaml` decides this link (XS-12).
    pub corrected: Vec<bool>,
    /// How many file-level links this aggregated link stands for.
    pub weight: Vec<u32>,
    pub added_seq: Vec<u64>,
    /// -1 while the link is live.
    pub removed_seq: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct GraphParams {
    pub lod: Option<Lod>,
    pub scope: Option<NodeId>,
    pub at: Option<At>,
    /// `from..to`, two `at` values.
    pub range: Option<String>,
    pub min_confidence: Option<ConfidenceLevel>,
    /// Comma-separated link kinds.
    pub kinds: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<RepoId>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Metrics {
    pub links: u32,
    pub fan_in: u32,
    pub fan_out: u32,
    pub cross: u32,
    /// Links added or removed in the 90 days before `at`.
    pub churn: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeDetail {
    pub node: Node,
    pub metrics: Metrics,
    pub owners: Vec<String>,
    pub added: Option<CommitRef>,
    pub removed: Option<CommitRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Base,
    Config,
    Contract,
    Correction,
    Overlay,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkDetail {
    pub link: Link,
    /// The layer whose answer the map shows. Null until precedence layers exist (XS-08 onwards).
    pub layer: Option<Layer>,
    /// What the passes found, when a correction changed this link (`layer` is `correction`).
    pub base: Option<LinkBase>,
    pub interval: Interval,
    /// The commit that opened the link, and the one that closed it (null while live).
    pub opened: Commit,
    pub closed: Option<Commit>,
    /// HTTP links: the call's method and route template.
    pub operation: Option<LinkOperation>,
    /// The code behind the link, by its role in it.
    pub evidence: EvidenceGroups,
    /// A link between the same two services that closed in the commit this one opened.
    pub replaced: Option<LinkId>,
    /// The entry of `cartograph.overrides.yaml` that decided this link, `c<position>-<fingerprint>`.
    pub correction_id: Option<String>,
    /// The environment `config` is for (`?env=`, else the default); null when the link doesn't
    /// depend on one (XS-09).
    pub env: Option<String>,
    /// Every environment the link holds in.
    pub envs: Vec<String>,
    /// How `env` resolves the call, in order: the key the call site reads, the value it has there
    /// and the host that value names.
    pub config: Vec<LinkConfig>,
}

/// A link's values before a correction (XS-12), for the inspector and for undo.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkBase {
    pub level: ConfidenceLevel,
    pub score: f32,
    /// The far end the passes found; null when it is unchanged.
    pub to: Option<NodeId>,
}

/// One step of the chain from a call site to a service: key, value, host (XS-09).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkConfig {
    /// `CATALOG_URL`.
    pub key: String,
    /// What the key is set to: a URL without userinfo, query or fragment, or `host[:port]`.
    pub value: String,
    /// The host in `value`: the column the inspector shows per environment.
    pub host: String,
    /// The file that sets it, repo-relative as the index sees paths; `cartograph.yaml` for a
    /// value set by hand.
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkOperation {
    /// Lower case: `get`.
    pub method: String,
    /// `/orders/{}`.
    pub template: String,
}

/// Evidence by role. Contract rows wait for contracts (XS-08).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceGroups {
    pub contract: Vec<EvidenceSite>,
    pub caller: Vec<EvidenceSite>,
    pub handler: Vec<EvidenceSite>,
}

/// One place in a file that backs a link. `snippet` is repo text: show it as text, never markup.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceSite {
    pub file: String,
    /// 1-based.
    pub line: u32,
    pub end_line: u32,
    /// `literal`, `template` or `inferred`.
    pub kind: String,
    pub extractor: String,
    /// At most 5 lines and 400 characters, secrets masked; null when the repo can't be read.
    pub snippet: Option<String>,
}

/// One row of `…/nodes/{id}/ranking`: a file in the scope and how much depends on it (blast
/// radius), as `lib/workspace.ts` `ranking` computes it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RankedNode {
    pub node: Node,
    /// Live links into the file at `at`.
    pub used_by: u32,
    /// Live links out of it at `at`.
    pub uses: u32,
    /// `used_by` now minus `used_by` 90 days before `at`.
    pub delta: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
}

/// One row of `…/nodes/{id}/changes`: a commit that added or removed a link touching the scope.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkChange {
    pub commit: Commit,
    pub link: Link,
    pub change: Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineageVia {
    /// The path first appeared, or a structure node's span began.
    Add,
    /// Moved with the same blob.
    RenameExact,
    /// Moved with similar content; `similarity` says how similar.
    RenameSimilar,
}

/// One row of `…/nodes/{id}/history` (board G10): a path the node had, and how it got it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Lineage {
    pub path: String,
    pub from: CommitRef,
    /// Null while the node still has this path.
    pub to: Option<CommitRef>,
    pub via: LineageVia,
    /// Percent of content kept, for `rename_similar`.
    pub similarity: Option<u8>,
}

/// `[from_seq, to_seq)` on the merged first-parent timeline.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Interval {
    pub from_seq: u64,
    pub to_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SymbolCalls {
    pub symbol: Node,
    pub callers: Vec<FileGroup>,
    pub callees: Vec<FileGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileGroup {
    pub file: NodeId,
    pub symbols: Vec<NodeId>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Service,
    Module,
    File,
    Symbol,
    Concept,
    Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchResult {
    pub id: NodeId,
    pub kind: SearchKind,
    pub name: String,
    pub path: String,
    pub service: Option<NodeId>,
}

/// `GET …/search`: the hits best first (bm25, then fan-in, then shorter paths), and when nothing
/// matched, the nearest name (board G05's "did you mean").
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub near: Option<SearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitImpact {
    pub commit: Commit,
    pub prev: Option<String>,
    pub next: Option<String>,
    pub added_links: Vec<Link>,
    pub removed_links: Vec<Link>,
    /// Node ids that depend, directly or not, on what the commit touched.
    pub affected: Vec<NodeId>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Bin {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimelineDensity {
    pub bin: Bin,
    /// The repos `by_repo` counts, in that order (board G04 stacks by repo).
    pub repos: Vec<RepoId>,
    pub bins: Vec<DensityBin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DensityBin {
    pub start: Timestamp,
    pub commits: u32,
    pub links_added: u32,
    pub links_removed: u32,
    /// Commits per repo, in the order of `TimelineDensity.repos`; empty in a node's series.
    pub by_repo: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimelineMarker {
    pub at: CommitRef,
    pub kind: MarkerKind,
    pub label: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    Rename,
    Rewrite,
}

/// `POST …/impact` carries exactly one of these; it changes nothing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactRequest {
    pub paths: Option<Vec<String>>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub working_tree: Option<bool>,
    pub since: Option<String>,
}

/// What depends on the changed nodes, directly or not. A node's level is the weakest link on its
/// strongest path from a changed node.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImpactResult {
    /// The nodes the input named or changed.
    pub changed: Vec<NodeId>,
    pub exact: Vec<NodeId>,
    pub likely: Vec<NodeId>,
    pub guessed: Vec<NodeId>,
    /// The services the affected nodes belong to.
    pub services: Vec<NodeId>,
    /// A resource cap (50k visited nodes, depth 12, 5 s) stopped the walk.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathDirection {
    /// Each hop goes from a node to what it depends on.
    Forward,
    /// Each hop goes from a node to what depends on it.
    Backward,
    /// Hops in both directions.
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PathResult {
    pub hops: Vec<Link>,
    /// A path's level is its weakest hop.
    pub level: ConfidenceLevel,
    pub direction: PathDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Reach {
    pub nodes: Vec<NodeId>,
    pub links: Vec<LinkId>,
    /// A resource cap stopped the walk.
    pub truncated: bool,
}

// ---------------------------------------------------------------- views (A1-A5, S4, S5)

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Head,
    Trace,
    Event,
    /// A gRPC method (XS-18): `/pkg.Service/Method`.
    Rpc,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Caller {
    pub node: NodeId,
    pub confidence: Confidence,
}

/// Whether the code agrees with the spec about an operation (XS-08 drift).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OperationStatus {
    /// A parsed route serves it.
    Sync,
    /// The spec lists it and no route serves it.
    SpecOnly,
}

/// An operation of a spec. `id` is `o<row>`, not a node.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Operation {
    pub id: NodeId,
    pub method: Method,
    /// With the spec's base path, as the spec writes it: `/v1/orders/{id}`.
    pub path: String,
    pub operation_id: Option<String>,
    /// From the spec: untrusted text (F8), at most 200 characters.
    pub summary: Option<String>,
    pub status: OperationStatus,
    pub handler: Option<NodeId>,
    pub callers: Vec<Caller>,
    /// For an event operation (an AsyncAPI channel, XS-16): the files that publish to the channel
    /// and the files that subscribe to it, with how sure the best event link of each is.
    #[serde(default)]
    pub publishers: Vec<Caller>,
    #[serde(default)]
    pub subscribers: Vec<Caller>,
}

/// A route of a service that has a spec, which no spec lists.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UndocumentedRoute {
    pub method: Method,
    /// The route's template: `/orders/{}`.
    pub path: String,
    /// The file that serves it.
    pub handler: NodeId,
}

/// The four counts of the Contracts view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContractCounts {
    pub operations: u32,
    pub sync: u32,
    pub spec_only: u32,
    pub undocumented: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpecKind {
    Openapi,
    Events,
    Proto,
}

/// `lib/model.ts` Spec. Served from `…/contracts`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Contract {
    pub id: String,
    pub name: String,
    pub file: String,
    pub kind: SpecKind,
    pub service: NodeId,
    pub operations: Vec<Operation>,
    /// The service's undocumented routes, on its first spec.
    pub undocumented: Vec<UndocumentedRoute>,
    pub counts: ContractCounts,
}

/// A spec's history (XS-19): one row per distinct spec along first-parent history, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContractVersion {
    /// 1 for the first spec the file had.
    pub n: u32,
    /// The commit where this spec came to be.
    pub commit: Commit,
    /// Hex SHA-256 of the canonical operations and schemas.
    pub spec_hash: String,
    /// Changes from the version before; 0 for v1.
    pub changes: u32,
    /// How many of them are breaking; `breaking` lists the first 200.
    pub breaking_count: u32,
    /// Built from the diff, never by a model. Untrusted text (names from the spec).
    pub summary: String,
    pub breaking: Vec<BreakingChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BreakingChange {
    /// `operation_removed`, `field_removed`, `type_narrowed`, ...
    pub kind: String,
    /// `get /orders/{id}` or `Order.status`.
    pub subject: String,
    pub summary: String,
    /// Who called the operation in the commit before the change; empty for a schema change.
    pub callers: Vec<Caller>,
}

/// One change of a diff, breaking or not.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpecChange {
    pub kind: String,
    pub subject: String,
    pub breaking: bool,
    pub summary: String,
}

/// `…/contracts/{id}/diff`: every change between two versions, recomputed on request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContractDiff {
    pub from: u32,
    pub to: u32,
    /// The spec reads other files; the diff is of the root file alone.
    pub approximate: bool,
    pub changes: Vec<SpecChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntrypointKind {
    Http,
    Event,
    /// A cron config, a k8s CronJob or a scheduler registration.
    Job,
    /// A package's `bin`, a Go `main` package.
    Cli,
    /// A package's entry files.
    Export,
    /// A file-system route.
    Ui,
}

/// What starts work in a service (XS-20). `id` is not a node.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Entrypoint {
    pub id: String,
    pub kind: EntrypointKind,
    pub label: String,
    /// The file it starts in.
    pub node: NodeId,
    pub service: Option<NodeId>,
    /// Links into it, and declared clients.
    pub consumers: u32,
    /// Nodes it reaches ("Symbols reached"), counted on the links at the date.
    pub reach: u32,
}

/// `GET …/entrypoints/counts`: the numbers the Entrypoints view's heading and filters show, so the
/// list can be paged without counting it on the client.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EntrypointCounts {
    pub total: u32,
    /// Services that have at least one entrypoint.
    pub services: u32,
    pub kinds: Vec<KindCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KindCount {
    pub kind: EntrypointKind,
    pub count: u32,
}

/// Who calls an entrypoint: a link into its operation or channel, or a client the manifest
/// declares. Corrections and declared clients carry `overridden`, the view's "override" chip.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Consumer {
    /// The calling file; none for a declared client.
    pub node: Option<NodeId>,
    pub link: Option<LinkId>,
    /// A declared client's name: untrusted text (F8).
    pub name: Option<String>,
    pub level: ConfidenceLevel,
    pub overridden: bool,
}

/// The nodes an entrypoint reaches in one service.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceReach {
    pub service: NodeId,
    pub nodes: u32,
}

/// What an entrypoint reaches over the links valid at the date, by service, biggest first.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReachSummary {
    pub nodes: u32,
    pub services: Vec<ServiceReach>,
    /// A bound (64 hops, 50,000 nodes) stopped the walk.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EntrypointDetail {
    pub entrypoint: Entrypoint,
    pub consumers: Vec<Consumer>,
    pub reach: ReachSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Concept {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub nodes: Vec<NodeId>,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CoverageRow {
    pub id: String,
    pub label: String,
    pub done: u32,
    pub total: u32,
    /// In percent: the manifest's `coverage.targets`, else the default; none for `handlers`.
    pub target: Option<f32>,
    /// The reason that affects the most rows.
    pub gap: Option<Problem>,
    /// Every reason, most rows first (XS-14).
    pub gaps: Vec<GapGroup>,
}

/// The unresolved rows of one reason (XS-14). `reason` can carry repo text: untrusted text (F8).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GapGroup {
    pub reason: String,
    pub rows: u32,
}

/// A reference that didn't resolve (IDX-22). `text` is the raw specifier: untrusted text (F8),
/// shown as text only.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Unresolved {
    pub file: NodeId,
    /// The file's path now, repo-prefixed as node paths are.
    pub path: String,
    pub line: u32,
    pub text: String,
    pub reason: String,
}

/// One index run (IDX-22): when it ran, the binary that wrote it, and its coverage totals. The
/// totals are absent on a run that stopped before its end.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IndexRun {
    pub id: u32,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub writer_version: String,
    pub files_total: Option<u32>,
    pub files_parsed: Option<u32>,
    pub imports_total: Option<u32>,
    pub imports_resolved: Option<u32>,
}

/// An extractor the index holds facts from (IDX-21), with the live files it read.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractorInfo {
    pub name: String,
    /// The newest version that wrote facts.
    pub version: String,
    pub files: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    Team,
    Person,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Owner {
    pub id: String,
    pub name: String,
    pub kind: OwnerKind,
    pub patterns: Vec<String>,
    pub nodes: u32,
}

/// An architecture rule of cartograph.yaml with what it finds now (XS-22).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    /// The rule's name, which the baseline and violations use.
    pub id: String,
    /// `forbid` or `no_cycles`.
    pub kind: String,
    /// `error` fails `cartograph check`; `warn` and `info` only print.
    pub severity: String,
    /// The rule's `reason`: untrusted text from the repo (F8).
    pub description: String,
    /// Violations that hold now, baselined ones included.
    pub violations: u32,
    pub baselined: u32,
}

/// A link a rule forbids, in the natural keys of cartograph.baseline.json. It carries the link's
/// intervals, so whether a commit passes is a lookup.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuleViolation {
    pub rule: String,
    pub severity: String,
    /// The link kind: `import`, `package`, `http`, `event` or `db`.
    pub kind: String,
    /// `service:path`; untrusted text.
    pub from: String,
    pub to: String,
    pub op: Option<String>,
    pub level: Option<ConfidenceLevel>,
    pub baselined: bool,
    /// It holds at the newest commit.
    pub live: bool,
    /// The steps it held over: from, and to (none while it still holds).
    pub intervals: Vec<(u32, Option<u32>)>,
}

/// One commit of the merged timeline and what the rules say of it (`GET …/rules/commits`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuleCommit {
    pub commit: Commit,
    /// Error-severity violations, not baselined, that held after the commit: what fails
    /// `cartograph check` there.
    pub failing: u32,
    /// Every violation that held after the commit, baselined ones and warnings included.
    pub violations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Service {
    pub id: NodeId,
    pub name: String,
    pub repo: RepoId,
    pub paths: Vec<String>,
    pub domain: Option<String>,
    pub tags: Vec<String>,
    /// A workspace package with no deployable marker (C6).
    pub library: bool,
    /// The C6 rule that found it: `manifest`, `scope`, `backstage`, `pnpm`, `npm-workspaces`,
    /// `go.work`, `nx`, `cargo`, `composer` or `repo` (board S04).
    pub detected_by: Option<String>,
    /// The file that rule read, repository-relative: `cartograph.yaml`, a catalog or workspace file,
    /// or a package's own manifest; none for `repo` (board S04's "Detected from").
    pub detect_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Environment {
    pub name: String,
    /// The workspace's default: `default_environment`, else `prod`, else the first.
    pub is_default: bool,
    /// Key to value, as of the newest commit: only values that classify as locators (XS-10).
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Adapter {
    pub name: String,
    pub version: String,
    pub kind: AdapterKind,
    pub allowed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    BuiltIn,
    Query,
    Subprocess,
}

// ---------------------------------------------------------------- writes

/// H3's four verbs (ruling R6).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionAction {
    Remove,
    Retarget,
    Set,
    Add,
}

/// A link named by people-readable parts (ruling R28: the nested key object).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkKey {
    pub kind: LinkKind,
    pub from: String,
    pub to: Option<String>,
    pub op: Option<String>,
    pub topic: Option<String>,
    pub table: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Correction {
    pub id: String,
    pub action: CorrectionAction,
    pub key: LinkKey,
    pub to: Option<String>,
    pub confidence: Option<ConfidenceLevel>,
    pub reason: String,
    pub by: String,
    pub at: Timestamp,
    /// The correction no longer matches any link.
    pub stale: bool,
}

/// `POST …/corrections`, and MCP `correct`. `reason` is 1 to 500 characters.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrectionRequest {
    pub action: CorrectionAction,
    pub key: LinkKey,
    pub to: Option<String>,
    pub confidence: Option<ConfidenceLevel>,
    pub reason: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// What a write to a committed file did, or would do.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileDiff {
    pub file: String,
    pub diff: String,
    pub applied: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConceptDecisionKind {
    KeepSeparate,
    SameConcept,
}

/// Pending ruling N7.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConceptDecision {
    pub decision: ConceptDecisionKind,
    pub reason: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigFile {
    /// Compare with `If-Match` on `PUT`.
    pub etag: String,
    pub yaml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigWrite {
    pub yaml: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoSync {
    Background,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractorSetting {
    pub name: String,
    pub version: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexingSettings {
    pub extractors: Vec<ExtractorSetting>,
    pub auto_sync: AutoSync,
    pub hooks: bool,
    pub hourly_fetch: bool,
}

/// A fixed enum: nothing else can be started over HTTP.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Sync,
    Rebuild,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub kind: JobKind,
    pub repo: Option<RepoId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    App,
    Cli,
    Mcp,
    Hook,
    Background,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Job {
    pub id: String,
    pub ws: WorkspaceId,
    pub kind: JobKind,
    pub by: Actor,
    pub state: JobState,
    pub started_at: Timestamp,
    pub took_ms: Option<u64>,
    pub error: Option<Problem>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Index,
    Sync,
    Correction,
    Warning,
}

/// One row of the activity list (FE-16), kept in the workspace's ask.db. `text` and `detail` hold
/// repo and author names, which are untrusted: the app renders them as text. Never file contents.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActivityItem {
    /// `<workspace>:<row>`.
    pub id: String,
    pub ws: WorkspaceId,
    pub at: Timestamp,
    pub kind: ActivityKind,
    /// "billing-service synced to 3e0d7a2 · 3 commits by Tom Ellery and Jon Reyes", at most 120
    /// characters.
    pub text: String,
    /// Who ran it and which version: "Indexed by the background sync · cartograph 0.4.0".
    pub detail: String,
    /// The app view the row opens, as a path and hash.
    pub href: String,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityRead {
    /// Marks these read; empty marks everything read.
    pub ids: Vec<String>,
}

/// `GET /api/v1/fs/list`: names and three flags only (ruling R25). The list reads only the folder
/// asked for, never inside a child, so listing home can't raise macOS privacy prompts for Desktop,
/// Documents or Downloads; the flags are null (unknown) and a folder is read when it is opened.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FsEntry {
    pub name: String,
    pub is_repo: Option<bool>,
    pub has_workspace_file: Option<bool>,
    pub has_children: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverRequest {
    pub path: String,
    pub workspace_file: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FolderResult {
    Repo,
    Scope,
    Skipped,
}

/// One row of the C1 and C2 resolution table.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoveredFolder {
    pub folder: String,
    pub name: String,
    pub result: FolderResult,
    pub repo: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Discovery {
    pub folders: Vec<DiscoveredFolder>,
    pub services: Vec<Service>,
    /// From a measured rate, not a guess.
    pub estimate_s: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspace {
    pub path: String,
    pub workspace_file: Option<String>,
    pub name: Option<String>,
    /// Also write `cartograph.yaml` in the workspace folder (C7); never overwrites one.
    #[serde(default)]
    pub manifest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatedWorkspace {
    pub workspace: WorkspaceSummary,
    pub job_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Contrast {
    Standard,
    More,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Follow,
    Reduce,
    Full,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Hues {
    Default,
    Safe,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Comfortable,
    Compact,
}

/// `lib/model.ts` AppearanceSettings. `size` is a percentage: 90, 100, 115 or 130.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub theme: Theme,
    pub contrast: Contrast,
    pub motion: Motion,
    pub hues: Hues,
    pub size: u16,
    pub density: Density,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AskPlacement {
    Panel,
    Page,
}

/// Preferences moved to the server by ruling R1.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Prefs {
    pub appearance: Appearance,
    pub keymap: BTreeMap<String, String>,
    pub pins: Vec<NodeId>,
    pub sidebar_open: bool,
    pub ask_placement: AskPlacement,
}

/// The app's defaults (`lib/theme.ts` DEFAULT_APPEARANCE), served before anything is saved.
impl Default for Prefs {
    fn default() -> Self {
        Self {
            appearance: Appearance {
                theme: Theme::Dark,
                contrast: Contrast::Standard,
                motion: Motion::Follow,
                hues: Hues::Default,
                size: 100,
                density: Density::Comfortable,
            },
            keymap: BTreeMap::new(),
            pins: Vec::new(),
            sidebar_open: true,
            ask_placement: AskPlacement::Panel,
        }
    }
}

// ---------------------------------------------------------------- MCP and agents

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpToolName {
    Status,
    Find,
    Describe,
    Neighbors,
    Path,
    Impact,
    Sync,
    Correct,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Read,
    Write,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpTool {
    pub name: McpToolName,
    pub summary: String,
    pub access: Access,
    /// One of the five Ask registers.
    pub ask: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    pub detected: bool,
    pub installed: bool,
    pub config_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentInstall {
    pub agent: String,
    #[serde(default)]
    pub shared: bool,
}

// ---------------------------------------------------------------- Ask and providers

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub question: String,
    pub at: Option<At>,
    pub thread_id: Option<String>,
    /// Node ids picked with @.
    pub mentions: Vec<NodeId>,
    pub ultra: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskCommand {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub call_id: String,
    pub tool: McpToolName,
    pub args: Value,
    pub rows: u32,
    pub handles: Vec<String>,
    pub ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Citation {
    pub handle: String,
    pub node: Option<NodeId>,
    pub link: Option<LinkId>,
    pub commit: Option<String>,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Sentence {
    pub p: u32,
    pub text: String,
    pub cites: Vec<String>,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskTurn {
    pub id: String,
    pub question: String,
    pub at: Option<At>,
    pub answer: Vec<Sentence>,
    pub tool_calls: Vec<ToolCall>,
    pub citations: Vec<Citation>,
    pub model: Option<String>,
    pub fact_hash: String,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub created_at: Timestamp,
    pub turns: Vec<AskTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextBlock {
    pub label: String,
    pub tokens: u32,
    pub cited: bool,
    pub dropped: bool,
}

/// The retrieval inspector (board A6).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TurnContext {
    pub budget_tokens: u32,
    pub blocks: Vec<ContextBlock>,
    pub system_prompt: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Gemini,
    Ollama,
    OpenaiCompatible,
}

/// A key is write-only: the browser learns `key_saved` and nothing else (G4).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: Option<String>,
    pub key_saved: bool,
    pub models: Vec<String>,
    pub local: bool,
}

/// The only request bodies with a `base_url`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderWrite {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: Option<String>,
}

/// The only request body with a key.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyWrite {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProviderTest {
    pub ok: bool,
    pub models: Vec<String>,
    pub ms: u32,
    pub error: Option<Problem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    /// Pending ruling N2: refuse every non-loopback provider.
    pub local_only: bool,
    pub summaries: bool,
}
