//! Fact cache, resolvers and the replay engine.

pub mod cache;
pub mod fs;
pub mod replay;

pub use cache::FactCache;
use cartograph_core as _;
pub use replay::{Interval, Link, LinkKind, Replay, from_scratch};

#[cfg(test)]
mod tests;
