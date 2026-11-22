//! Identifiers shared across crates. Each is a newtype so a repo id can't be passed as a node id.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// A repository in a workspace: its canonical path's registry key.
    RepoId
);
string_id!(
    /// A node: workspace, service, module, file, operation, channel, table, entrypoint, symbol.
    NodeId
);
string_id!(
    /// A link between two nodes.
    LinkId
);

/// A step on the merged first-parent timeline. Intervals are `[from_seq, to_seq)`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Seq(pub u64);

/// A git object id (SHA-1 today, SHA-256 later), held as lowercase hex.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Oid(pub String);

impl Oid {
    /// Accepts 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
    pub fn parse(hex: &str) -> Option<Self> {
        let ok = matches!(hex.len(), 40 | 64)
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then(|| Self(hex.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn any_40_hex_digits_parse(s in "[0-9a-f]{40}") {
            prop_assert_eq!(Oid::parse(&s).map(|o| o.0), Some(s));
        }

        #[test]
        fn other_lengths_do_not(s in "[0-9a-f]{1,39}") {
            prop_assert!(Oid::parse(&s).is_none());
        }
    }
}
