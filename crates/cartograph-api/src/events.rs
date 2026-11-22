//! The two event streams. Both are `text/event-stream`, read with fetch (EventSource can't send
//! the bearer header). The SSE `event:` field is the tag below; `data:` is the content; `id:` is a
//! sequence number. A heartbeat comment every 15 s.

use cartograph_core::{LinkId, NodeId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::*;

/// `GET /api/v1/events`: one stream for every workspace (ruling R15). The current state is sent on
/// connect.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", content = "data")]
pub enum ServerEvent {
    #[serde(rename = "freshness")]
    Freshness {
        ws: WorkspaceId,
        freshness: Freshness,
    },
    #[serde(rename = "job.started")]
    JobStarted {
        job_id: String,
        ws: WorkspaceId,
        kind: JobKind,
        by: Actor,
    },
    /// At most 4 a second.
    #[serde(rename = "job.progress")]
    JobProgress {
        job_id: String,
        stage: String,
        done: u64,
        total: u64,
        repo: Option<String>,
        eta_s: Option<u32>,
    },
    #[serde(rename = "job.finished")]
    JobFinished {
        job_id: String,
        ok: bool,
        generation: Option<u64>,
        took_ms: u64,
        error: Option<Problem>,
    },
    #[serde(rename = "index.changed")]
    IndexChanged {
        ws: WorkspaceId,
        generation: u64,
        fact_hash: String,
    },
    #[serde(rename = "config.changed")]
    ConfigChanged {
        ws: WorkspaceId,
        etag: String,
        file: String,
    },
    #[serde(rename = "activity.new")]
    ActivityNew(ActivityItem),
    #[serde(rename = "workspaces.changed")]
    WorkspacesChanged {},
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Done,
    Stopped,
    Failed,
    /// The answer came from the graph engine alone (G5).
    Fallback,
}

/// `POST …/ask/runs`: the run's own stream.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", content = "data")]
pub enum RunEvent {
    #[serde(rename = "run.started")]
    RunStarted {
        run_id: String,
        thread_id: String,
        turn_id: String,
        model: Option<String>,
    },
    #[serde(rename = "plan")]
    Plan { steps: Vec<String> },
    #[serde(rename = "tool.call")]
    ToolCall {
        call_id: String,
        tool: McpToolName,
        args: Value,
        by: RunActor,
    },
    #[serde(rename = "tool.result")]
    ToolResult {
        call_id: String,
        rows: u32,
        handles: Vec<String>,
        ms: u32,
    },
    /// The engine's answer, before any model text.
    #[serde(rename = "answer")]
    Answer {
        sentences: Vec<Sentence>,
        nodes: Vec<NodeId>,
        links: Vec<LinkId>,
    },
    #[serde(rename = "budget")]
    Budget {
        used_tokens: u32,
        budget_tokens: u32,
        left_out: u32,
    },
    /// Raw model text not yet split into a checked sentence.
    #[serde(rename = "tail")]
    Tail { text: String },
    #[serde(rename = "sentence")]
    Sentence(Sentence),
    /// After a repair, the whole answer is replaced.
    #[serde(rename = "answer.replace")]
    AnswerReplace { sentences: Vec<Sentence> },
    #[serde(rename = "usage")]
    Usage(Usage),
    #[serde(rename = "notice")]
    Notice { text: String },
    #[serde(rename = "error")]
    Error(Problem),
    #[serde(rename = "done")]
    Done { status: RunStatus, stats: Value },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunActor {
    Engine,
    Model,
}
