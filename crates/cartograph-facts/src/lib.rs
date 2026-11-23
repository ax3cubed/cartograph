//! Fact cache, resolvers and the replay engine.

pub mod cache;
pub mod fs;
pub mod lang;
pub mod replay;

pub use cache::FactCache;
use cartograph_core as _;
pub use replay::{Interval, Link, LinkKind, Replay, SNAPSHOT_VERSION, from_scratch};

#[cfg(test)]
mod tests;
