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

mod adjacency;
mod at;
mod config;
mod contracts;
mod coverage;
mod detail;
mod entrypoints;
mod error;
mod find;
mod graph;
mod hosts;
mod owners;
mod page;
mod read;
mod resolve;
mod snippet;
mod store;
mod timeline;
mod traverse;
mod versions;

pub use at::{At, parse_instant, rfc3339};
pub use cartograph_store::FORMAT_VERSION;
pub use detail::{WORKSPACE, level};
pub use error::QueryError;
pub use find::{MAX_QUERY, SearchQuery};
pub use graph::{GraphQuery, MAX_LINKS, MAX_NODES};
pub use hosts::{HostMatch, host_and_port};
pub use page::{Page, Paged};
pub use resolve::{link_id, node_id};
pub use store::{DEFAULT_DEADLINE, HARD_DEADLINE, Limits, Snapshot, Store, valid_workspace_id};
pub use timeline::{MAX_BINS, bin_start};
pub use traverse::{
    DEADLINE as TRAVERSAL_DEADLINE, ImpactInput, MAX_DEPTH, MAX_VISITED, Neighbor, NeighborQuery,
};

#[cfg(test)]
mod tests;
