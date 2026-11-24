//! Fact cache, resolvers and the replay engine.

pub mod adapters;
pub mod cache;
pub mod fs;
pub mod lang;
pub mod replay;
pub mod schema;

pub use cache::FactCache;
pub use replay::{Interval, Link, LinkKind, Replay, SNAPSHOT_VERSION, from_scratch};

#[cfg(test)]
mod tests;
