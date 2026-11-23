//! `QueryError`: one stable code per failure, rendered once into the shared error shape (SRV-07).
//!
//! Nothing here quotes SQL, a SQLite message or a file path: an error body is read by a browser
//! and by agents, and the data folder sits outside every workspace. Details go to the log.

use cartograph_api::types::{ApiError, ErrorCode};
use cartograph_core::Explain;
use cartograph_store::StoreError;
use cartograph_store::rusqlite;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("{what}")]
    BadRequest {
        what: String,
        why: String,
        action: String,
    },
    #[error("{what}")]
    NotFound { what: String, action: String },
    #[error("{reference} matches more than one node")]
    Ambiguous {
        reference: String,
        candidates: Vec<String>,
    },
    #[error("the answer is too large")]
    TooLarge { why: String, narrower: String },
    #[error("the query ran past its deadline")]
    Timeout,
    #[error("the index can't be read")]
    Unavailable { why: String, action: String },
    #[error("the query failed")]
    Internal,
}

impl QueryError {
    pub(crate) fn bad(
        what: impl Into<String>,
        why: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self::BadRequest {
            what: what.into(),
            why: why.into(),
            action: action.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadRequest { .. } => ErrorCode::BadRequest,
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::Ambiguous { .. } => ErrorCode::Ambiguous,
            Self::TooLarge { .. } => ErrorCode::TooLarge,
            Self::Timeout => ErrorCode::Timeout,
            Self::Unavailable { .. } => ErrorCode::IndexUnavailable,
            Self::Internal => ErrorCode::Internal,
        }
    }

    /// The error as HTTP, `--json` and MCP carry it.
    pub fn to_api(&self) -> ApiError {
        let mut e = ApiError::new(self.code(), &self.what(), &self.why(), &self.action());
        if let Self::Ambiguous { candidates, .. } = self {
            e.details = Some(json!({ "candidates": candidates }));
        }
        e
    }
}

impl Explain for QueryError {
    fn why(&self) -> String {
        match self {
            Self::BadRequest { why, .. }
            | Self::TooLarge { why, .. }
            | Self::Unavailable { why, .. } => why.clone(),
            Self::NotFound { .. } => "nothing in the index has that name".into(),
            Self::Ambiguous { candidates, .. } => {
                format!("{} nodes match it", candidates.len())
            }
            Self::Timeout => {
                "each query has a time limit, so no read can hold the index open".into()
            }
            Self::Internal => "an internal error; the details are in the log".into(),
        }
    }

    fn action(&self) -> String {
        match self {
            Self::BadRequest { action, .. }
            | Self::NotFound { action, .. }
            | Self::Unavailable { action, .. } => action.clone(),
            Self::TooLarge { narrower, .. } => narrower.clone(),
            Self::Ambiguous { .. } => "use one of the candidates' ids".into(),
            Self::Timeout => "narrow the query, or try again once a sync finishes".into(),
            Self::Internal => "cartograph report writes the details to share".into(),
        }
    }
}

impl From<rusqlite::Error> for QueryError {
    fn from(e: rusqlite::Error) -> Self {
        if e.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
            return Self::Timeout;
        }
        tracing::error!(error = %e, "query failed");
        Self::Internal
    }
}

impl From<StoreError> for QueryError {
    fn from(e: StoreError) -> Self {
        let unavailable = |why: &str, action: &str| Self::Unavailable {
            why: why.into(),
            action: action.into(),
        };
        // The store's own messages name the file; these don't.
        match &e {
            StoreError::TooNew { .. } => unavailable(
                "a newer Cartograph wrote it in a format this one can't read",
                "upgrade Cartograph to the latest version",
            ),
            StoreError::NotAnIndex { .. } | StoreError::Damaged { .. } => {
                tracing::error!(error = %e, why = %e.why(), "index can't be opened");
                unavailable("the index file is damaged", "cartograph index --rebuild")
            }
            StoreError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
                Self::NotFound {
                    what: "this workspace has no index".into(),
                    action: "cartograph index <repo>".into(),
                }
            }
            _ => {
                tracing::error!(error = %e, why = %e.why(), "index can't be opened");
                Self::Internal
            }
        }
    }
}
