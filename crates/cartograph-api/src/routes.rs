//! The route table: one row per route in the Interfaces section of docs/architecture.html.
//! The write list is closed: a route not marked [`Kind::Write`] never accepts a body that changes
//! state, and HD-11 generates its rejection tests from this table.

use cartograph_core::NodeId;
use schemars::{JsonSchema, Schema, SchemaGenerator};

use crate::events::{RunEvent, ServerEvent};
use crate::types::*;

pub type SchemaFn = fn(&mut SchemaGenerator) -> Schema;

fn s<T: JsonSchema>(g: &mut SchemaGenerator) -> Schema {
    g.subschema_for::<T>()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Write,
    Stream,
}

#[derive(Clone, Copy)]
pub enum Response {
    /// JSON data inside the [`Envelope`].
    Json(SchemaFn),
    /// `text/plain` (blob text).
    Text,
    /// `text/event-stream` of the given event enum.
    Events(SchemaFn),
    /// 204.
    Empty,
}

#[derive(Clone, Copy)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    pub kind: Kind,
    /// Query parameters beyond the path's `{params}`.
    pub query: &'static [&'static str],
    pub body: Option<SchemaFn>,
    pub response: Response,
    pub owner: &'static str,
    pub summary: &'static str,
}

macro_rules! w {
    ($p:literal) => {
        concat!("/api/v1/w/{ws}", $p)
    };
}

const READ: &[&str] = &["at", "limit", "cursor", "include"];
const GRAPH: &[&str] = &[
    "lod",
    "scope",
    "at",
    "range",
    "min_confidence",
    "kinds",
    "owner",
    "repo",
    "branch",
];

#[allow(clippy::too_many_arguments)] // one call per table row reads better than a builder
const fn r(
    method: &'static str,
    path: &'static str,
    kind: Kind,
    query: &'static [&'static str],
    body: Option<SchemaFn>,
    response: Response,
    owner: &'static str,
    summary: &'static str,
) -> Route {
    Route {
        method,
        path,
        kind,
        query,
        body,
        response,
        owner,
        summary,
    }
}

use Kind::{Read as R, Stream as S, Write as W};
use Response::{Empty, Events, Json, Text};

#[rustfmt::skip]
pub fn routes() -> Vec<Route> {
    vec![
        // meta and workspaces
        r("GET", "/api/version", R, &[], None, Json(s::<Version>), "SRV-05", "Binary version"),
        r("GET", "/api/v1/meta", R, &[], None, Json(s::<Meta>), "SRV-05", "Version, format version, now, schema (A4)"),
        r("GET", "/api/v1/workspaces", R, &[], None, Json(s::<Vec<WorkspaceSummary>>), "SRV-05", "Workspaces in the registry"),
        r("GET", "/api/v1/workspaces/{ws}", R, &[], None, Json(s::<WorkspaceSummary>), "SRV-05", "One workspace"),
        r("GET", "/api/v1/events", S, &[], None, Events(s::<ServerEvent>), "SRV-13", "One event stream for every workspace"),
        r("GET", w!("/freshness"), R, &[], None, Json(s::<Freshness>), "SRV-03", "How current the index is (D1)"),
        r("GET", w!("/repos"), R, &[], None, Json(s::<Vec<Repo>>), "SRV-03", "Repos with HEAD and indexed commit"),
        // graph and panels
        r("GET", w!("/graph"), R, GRAPH, None, Json(s::<GraphSlice>), "SRV-04", "Columnar graph slice at a level of detail"),
        r("GET", w!("/nodes/{id}"), R, READ, None, Json(s::<NodeDetail>), "SRV-08", "Node with metrics"),
        r("GET", w!("/nodes/{id}/children"), R, READ, None, Json(s::<Vec<Node>>), "SRV-08", "Children one level down"),
        r("GET", w!("/nodes/{id}/metrics"), R, READ, None, Json(s::<Metrics>), "SRV-08", "Fan-in, fan-out, cross, churn"),
        r("GET", w!("/nodes/{id}/series"), R, READ, None, Json(s::<Vec<DensityBin>>), "SRV-08", "Metric series over time"),
        r("GET", w!("/nodes/{id}/neighbors"), R, READ, None, Json(s::<Vec<Link>>), "SRV-08", "Links in and out"),
        r("GET", w!("/nodes/{id}/changes"), R, READ, None, Json(s::<Vec<Commit>>), "SRV-08", "Commits that changed this node's links"),
        r("GET", w!("/nodes/{id}/ranking"), R, READ, None, Json(s::<Vec<Node>>), "SRV-08", "Children ranked by fan-in"),
        r("GET", w!("/nodes/{id}/history"), R, READ, None, Json(s::<Vec<Interval>>), "SRV-08", "Lineage (board G10)"),
        r("GET", w!("/nodes/{id}/symbols"), R, READ, None, Json(s::<Vec<Node>>), "SRV-08", "Symbols in a file"),
        r("GET", w!("/nodes/{id}/reading-order"), R, READ, None, Json(s::<Vec<NodeId>>), "SRV-08", "Suggested reading order"),
        r("GET", w!("/links/{id}"), R, &["at"], None, Json(s::<LinkDetail>), "SRV-08", "Evidence, level, score, layer, interval, replaced-by"),
        r("GET", w!("/symbols/{id}"), R, &["at"], None, Json(s::<SymbolCalls>), "XS-25", "Callers and callees grouped by file"),
        r("GET", w!("/source"), R, &["node", "commit", "lines"], None, Text, "SRV-08", "Blob text, 400 lines and 64 KB at most"),
        r("GET", w!("/search"), R, &["q", "kinds", "at", "limit"], None, Json(s::<Vec<SearchResult>>), "SRV-09", "Search, grouped by kind; the Ask @ picker"),
        r("GET", w!("/commits"), R, READ, None, Json(s::<Vec<Commit>>), "SRV-10", "Commits, paged"),
        r("GET", w!("/commits/{sha}"), R, &[], None, Json(s::<Commit>), "SRV-10", "One commit"),
        r("GET", w!("/commits/{sha}/impact"), R, &[], None, Json(s::<CommitImpact>), "XS-21", "Commit impact with prev and next"),
        r("GET", w!("/timeline/density"), R, &["bin", "from", "to"], None, Json(s::<TimelineDensity>), "SRV-10", "Commit and link density bins"),
        r("GET", w!("/timeline/markers"), R, &["from", "to"], None, Json(s::<Vec<TimelineMarker>>), "SRV-10", "Renames and rewrites"),
        r("GET", w!("/impact"), R, &["node", "at"], None, Json(s::<ImpactResult>), "SRV-11", "Affected nodes as exact, likely, guessed"),
        r("POST", w!("/impact"), R, &[], Some(s::<ImpactRequest>), Json(s::<ImpactResult>), "SRV-11", "Impact of paths, a ref, the working tree or a range. Changes nothing"),
        r("GET", w!("/path"), R, &["from", "to", "max_hops", "at"], None, Json(s::<PathResult>), "SRV-11", "Shortest path between two nodes"),
        r("GET", w!("/reach"), R, &["node", "direction", "depth", "at"], None, Json(s::<Reach>), "SRV-11", "Reachable set for Trace on the map"),
        r("GET", w!("/stories"), R, &["node", "at"], None, Json(s::<Vec<PathResult>>), "SRV-11", "Entrypoint-to-store traversals"),
        // views
        r("GET", w!("/contracts"), R, READ, None, Json(s::<Vec<Contract>>), "SRV-12", "Specs and their operations"),
        r("GET", w!("/contracts/{id}"), R, READ, None, Json(s::<Contract>), "SRV-12", "One spec"),
        r("GET", w!("/entrypoints"), R, READ, None, Json(s::<Vec<Entrypoint>>), "SRV-12", "Entrypoints"),
        r("GET", w!("/entrypoints/{id}"), R, READ, None, Json(s::<Entrypoint>), "SRV-12", "One entrypoint"),
        r("GET", w!("/concepts"), R, READ, None, Json(s::<Vec<Concept>>), "SRV-12", "Concepts"),
        r("GET", w!("/concepts/{id}"), R, READ, None, Json(s::<Concept>), "SRV-12", "One concept"),
        r("GET", w!("/coverage"), R, READ, None, Json(s::<Vec<CoverageRow>>), "SRV-12", "Coverage rows"),
        r("GET", w!("/coverage/{row}/unresolved"), R, READ, None, Json(s::<Vec<Unresolved>>), "SRV-12", "Unresolved references behind a row"),
        r("GET", w!("/owners"), R, READ, None, Json(s::<Vec<Owner>>), "SRV-12", "Owners from CODEOWNERS"),
        r("GET", w!("/rules"), R, READ, None, Json(s::<Vec<Rule>>), "SRV-12", "Rules"),
        r("GET", w!("/rules/violations"), R, READ, None, Json(s::<Vec<RuleViolation>>), "SRV-12", "Current violations"),
        r("GET", w!("/services"), R, READ, None, Json(s::<Vec<Service>>), "SRV-12", "Services"),
        r("GET", w!("/environments"), R, READ, None, Json(s::<Vec<Environment>>), "SRV-12", "Environments"),
        r("GET", w!("/adapters"), R, READ, None, Json(s::<Vec<Adapter>>), "SRV-12", "Adapters and whether each is allowed"),
        // writes: the closed list
        r("GET", w!("/corrections"), R, READ, None, Json(s::<Vec<Correction>>), "XS-13", "Corrections in cartograph.overrides.yaml"),
        r("POST", w!("/corrections"), W, &[], Some(s::<CorrectionRequest>), Json(s::<FileDiff>), "XS-13", "Append a correction (dry_run previews); returns the diff"),
        r("DELETE", w!("/corrections/{id}"), W, &[], None, Json(s::<FileDiff>), "XS-13", "Remove a correction; returns the diff"),
        r("POST", w!("/concepts/{id}/decision"), W, &[], Some(s::<ConceptDecision>), Json(s::<FileDiff>), "XS-28", "Concept decision (pending ruling N7)"),
        r("GET", w!("/config"), R, &[], None, Json(s::<ConfigFile>), "SRV-14", "cartograph.yaml with its etag"),
        r("POST", w!("/config/preview"), R, &[], Some(s::<ConfigWrite>), Json(s::<FileDiff>), "SRV-14", "Diff a proposed cartograph.yaml. Changes nothing"),
        r("PUT", w!("/config"), W, &[], Some(s::<ConfigWrite>), Json(s::<FileDiff>), "SRV-14", "Write cartograph.yaml (If-Match; 412 when it changed on disk)"),
        r("GET", w!("/settings/indexing"), R, &[], None, Json(s::<IndexingSettings>), "SRV-14", "Extractors and machine settings"),
        r("PUT", w!("/settings/indexing"), W, &[], Some(s::<IndexingSettings>), Json(s::<IndexingSettings>), "SRV-14", "Change extractors, auto_sync, hooks, hourly fetch"),
        r("POST", w!("/jobs"), W, &[], Some(s::<JobRequest>), Json(s::<Job>), "SRV-13", "Start a sync or rebuild (a fixed enum)"),
        r("GET", "/api/v1/jobs/{id}", R, &[], None, Json(s::<Job>), "SRV-13", "One job"),
        r("DELETE", "/api/v1/jobs/{id}", W, &[], None, Json(s::<Job>), "SRV-13", "Cancel a job (E5)"),
        r("GET", "/api/v1/activity", R, READ, None, Json(s::<Vec<ActivityItem>>), "SRV-13", "Activity list"),
        r("POST", "/api/v1/activity/read", W, &[], Some(s::<ActivityRead>), Empty, "SRV-13", "Mark activity read"),
        r("GET", "/api/v1/fs/list", R, &["path"], None, Json(s::<Vec<FsEntry>>), "SRV-14", "Folder names; the flags are unknown, children are never read (R25)"),
        r("POST", "/api/v1/discover", R, &[], Some(s::<DiscoverRequest>), Json(s::<Discovery>), "SRV-14", "The C1 and C2 resolution table. Changes nothing"),
        r("POST", "/api/v1/workspaces", W, &[], Some(s::<CreateWorkspace>), Json(s::<CreatedWorkspace>), "SRV-14", "Register a workspace and start its first index"),
        r("GET", "/api/v1/prefs", R, &[], None, Json(s::<Prefs>), "SRV-14", "Preferences (R1)"),
        r("PUT", "/api/v1/prefs", W, &[], Some(s::<Prefs>), Json(s::<Prefs>), "SRV-14", "Save preferences"),
        r("GET", "/api/v1/mcp/tools", R, &[], None, Json(s::<Vec<McpTool>>), "SRV-21", "The tool registry"),
        r("GET", "/api/v1/agents", R, &[], None, Json(s::<Vec<AgentStatus>>), "SRV-21", "Per-agent status"),
        r("POST", "/api/v1/agents/preview", R, &[], Some(s::<AgentInstall>), Json(s::<Vec<FileDiff>>), "SRV-21", "Diff of an agent install. Changes nothing"),
        r("POST", "/api/v1/agents/install", W, &[], Some(s::<AgentInstall>), Json(s::<Vec<FileDiff>>), "SRV-21", "Write the agent's user config"),
        // Ask
        r("POST", w!("/ask/runs"), W, &[], Some(s::<RunRequest>), Events(s::<RunEvent>), "AI-04", "Start a run; the response is its event stream"),
        r("GET", w!("/ask/runs/{id}/events"), S, &["after"], None, Events(s::<RunEvent>), "AI-04", "Reattach to a run (pending ruling N8)"),
        r("DELETE", w!("/ask/runs/{id}"), W, &[], None, Empty, "AI-04", "Stop a run"),
        r("GET", w!("/ask/commands"), R, &[], None, Json(s::<Vec<AskCommand>>), "AI-04", "The / command list"),
        r("GET", w!("/ask/threads"), R, READ, None, Json(s::<Vec<Thread>>), "AI-06", "Thread history, 30 a page"),
        r("GET", w!("/ask/threads/{id}"), R, &[], None, Json(s::<Thread>), "AI-06", "One thread"),
        r("DELETE", w!("/ask/threads/{id}"), W, &[], None, Empty, "AI-06", "Forget one thread"),
        r("DELETE", w!("/ask/threads"), W, &[], None, Empty, "AI-06", "Forget Ask history"),
        r("GET", w!("/ask/turns/{id}/context"), R, &[], None, Json(s::<TurnContext>), "AI-17", "The retrieval inspector"),
        r("GET", "/api/v1/ai/providers", R, &[], None, Json(s::<Vec<Provider>>), "AI-10", "Providers, with key_saved only"),
        r("PUT", "/api/v1/ai/providers/{id}", W, &[], Some(s::<ProviderWrite>), Json(s::<Provider>), "AI-10", "Add or change a provider"),
        r("DELETE", "/api/v1/ai/providers/{id}", W, &[], None, Empty, "AI-10", "Remove a provider"),
        r("PUT", "/api/v1/ai/providers/{id}/key", W, &[], Some(s::<KeyWrite>), Empty, "AI-10", "Save a key to the keychain (write-only)"),
        r("DELETE", "/api/v1/ai/providers/{id}/key", W, &[], None, Empty, "AI-10", "Delete a key"),
        r("POST", "/api/v1/ai/providers/{id}/models", R, &[], None, Json(s::<Vec<String>>), "AI-10", "List the provider's models"),
        r("POST", "/api/v1/ai/providers/{id}/test", R, &[], None, Json(s::<ProviderTest>), "AI-10", "Test a provider"),
        r("PUT", "/api/v1/ai/selection", W, &[], Some(s::<Selection>), Json(s::<Selection>), "AI-10", "Choose provider and model"),
        r("GET", "/api/v1/ai/privacy", R, &[], None, Json(s::<Privacy>), "AI-20", "Privacy settings"),
        r("PUT", "/api/v1/ai/privacy", W, &[], Some(s::<Privacy>), Json(s::<Privacy>), "AI-20", "Change privacy settings"),
    ]
}

/// Routes an analyst plan named that the table above replaces (INT-01's done-when).
pub const REPLACED: &[(&str, &str, &str)] = &[
    (
        "POST /api/session",
        "removed",
        "R4: the token goes fragment, sessionStorage, Authorization: Bearer. No cookie, no session exchange",
    ),
    (
        "/api/<x> and /api/w/:ws/<x>",
        "/api/v1/<x> and /api/v1/w/{ws}/<x>",
        "R15: one prefix",
    ),
    (
        "per-workspace event stream",
        "GET /api/v1/events",
        "R15: one global stream",
    ),
    (
        "POST /api/w/:ws/sync",
        "POST /api/v1/w/{ws}/jobs {kind: sync}",
        "jobs are a fixed enum",
    ),
    (
        "GET /api/fs/dirs",
        "GET /api/v1/fs/list",
        "R25: names and three flags only",
    ),
    (
        "GET …/nodes/:id/summary?t=",
        "GET /api/v1/w/{ws}/nodes/{id}?include=summary&at=",
        "one node route; at replaces t",
    ),
    (
        "GET /api/files/{id}/symbols",
        "GET /api/v1/w/{ws}/nodes/{id}/symbols",
        "files are nodes",
    ),
    (
        "POST /api/v1/ai/migrate",
        "PUT /api/v1/ai/providers/{id}/key",
        "AI-10 owns the localStorage key migration",
    ),
    (
        "PUT /api/v1/ai/settings",
        "PUT /api/v1/ai/selection and PUT /api/v1/ai/privacy",
        "split by AI-10 and AI-20",
    ),
    (
        "GET /api/ask/runs/:id/events?after=",
        "GET /api/v1/w/{ws}/ask/runs/{id}/events?after=",
        "kept, pending ruling N8",
    ),
    (
        "cartograph update --check",
        "removed",
        "R16: no network call of our own",
    ),
];
