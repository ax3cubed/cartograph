//! The read-only query layer. Never writes, never calls the network (A1).
//!
//! Phase 0 skeleton: the crate exists so the dependency graph is fixed before product code.

use cartograph_api as _;
use cartograph_core as _;
use cartograph_store as _;
