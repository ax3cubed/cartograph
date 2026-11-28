//! Concepts (board A3): named ideas such as "Invoice", each with the files that carry it.
//!
//! Concepts are written by the AI step, which isn't built yet, so the index holds none. The reads
//! answer exactly that: an empty list, and no concept by any id. The Concepts view shows its
//! "concepts come after search" empty state on it.

use cartograph_api::types::Concept;

use crate::{Page, Paged, QueryError, Snapshot};

impl Snapshot<'_> {
    /// `GET …/concepts`.
    pub fn concepts(&self, page: Page) -> Result<Paged<Concept>, QueryError> {
        Ok(page.take(Vec::new()))
    }

    /// `GET …/concepts/{id}`.
    pub fn concept(&self, _id: &str) -> Result<Concept, QueryError> {
        Err(QueryError::NotFound {
            what: "no such concept".into(),
            action: "list concepts with …/concepts; the index has none until concepts are built"
                .into(),
        })
    }
}
