//! The read-only query layer (A1). HTTP handlers, MCP tools, CLI commands and Ask are thin
//! wrappers over it; none of them holds SQL. Never writes, never calls the network.
//!
//! Every SQL string here is static, with bound parameters: no request text is ever spliced into
//! a statement.
//!
//! ```no_run
//! # use cartograph_query::*;
//! # fn f(data: &std::path::Path) -> Result<(), QueryError> {
//! let store = Store::open(data, "acme-7f3a91c2")?;
//! let snap = store.snapshot(Limits::default())?;
//! let slice = snap.graph(&GraphQuery::from_params(&Default::default())?)?;
//! println!("generation {}: {} files", snap.generation, slice.nodes.id.len());
//! # Ok(()) }
//! ```

mod at;
mod error;
mod graph;
mod read;
mod resolve;
mod store;

pub use at::{At, parse_instant, rfc3339};
pub use cartograph_store::FORMAT_VERSION;
pub use error::QueryError;
pub use graph::{GraphQuery, MAX_LINKS, MAX_NODES};
pub use resolve::{link_id, node_id};
pub use store::{DEFAULT_DEADLINE, HARD_DEADLINE, Limits, Snapshot, Store, valid_workspace_id};

#[cfg(test)]
mod tests;
