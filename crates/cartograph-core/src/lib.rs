//! Shared types for every Cartograph crate: ids, the error contract (E1, E2), the data folder (B1)
//! and logging.

pub mod error;
pub mod ids;
pub mod logging;
pub mod paths;

pub use error::{ExitCode, Explain, Report};
pub use ids::{LinkId, NodeId, Oid, RepoId, Seq};

/// The product name lives here and nowhere else, until ruling N1 settles it.
pub const PRODUCT: &str = "cartograph";
