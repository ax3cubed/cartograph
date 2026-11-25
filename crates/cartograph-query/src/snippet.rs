//! Code snippets for link evidence (XS-06): read on demand from the repo's object store by blob
//! id and line span, never copied into the index.
//!
//! ponytail: no LRU keyed by blob id. A link has a handful of evidence rows and the object read is
//! a loose-object or pack lookup behind gix's own object cache; add an LRU when a profile asks.

use std::collections::HashMap;
use std::path::Path;

use cartograph_git::{ObjectId, Repo};

const MAX_LINES: usize = 5;
const MAX_CHARS: usize = 400;
const MAX_BLOB: u64 = 1 << 20;

/// Repos opened so far in one request, so two rows in one repo open it once.
#[derive(Default)]
pub(crate) struct Repos(HashMap<String, Option<Repo>>);

impl Repos {
    /// Lines `start..=end` (1-based) of `blob`: at most 5 lines and 400 characters, secrets masked.
    /// None when the repo or blob can't be read, or the file is binary or the span is outside it.
    pub fn snippet(&mut self, repo: &str, blob: &[u8], start: u32, end: u32) -> Option<String> {
        let repo = self
            .0
            .entry(repo.to_owned())
            .or_insert_with(|| Repo::open(Path::new(repo)).ok())
            .as_ref()?;
        let oid = ObjectId::try_from(blob).ok()?;
        let bytes = cartograph_git::read_blob(&repo.local(), oid, MAX_BLOB).ok()?;
        if bytes.iter().take(8000).any(|&b| b == 0) {
            return None;
        }
        Some(cut(&String::from_utf8_lossy(&bytes), start, end))
    }
}

fn cut(text: &str, start: u32, end: u32) -> String {
    let skip = (start as usize).saturating_sub(1);
    let take = ((end.max(start) - start) as usize + 1).min(MAX_LINES);
    let lines: Vec<&str> = text.lines().skip(skip).take(take).collect();
    let joined = lines.join("\n");
    let masked = cartograph_core::secrets::mask(&joined);
    masked.chars().take(MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::cut;

    #[test]
    fn spans_are_one_based_and_capped() {
        let text = "a\nb\nc\nd\ne\nf\ng\n";
        assert_eq!(cut(text, 2, 3), "b\nc");
        assert_eq!(cut(text, 1, 99), "a\nb\nc\nd\ne");
        assert_eq!(cut(text, 40, 41), "");
        assert_eq!(cut(&"x".repeat(1000), 1, 1).len(), 400);
    }

    #[test]
    fn secrets_are_masked() {
        let out = cut("const k = \"AKIAIOSFODNN7EXAMPLE\";\n", 1, 1);
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "{out}");
    }
}
