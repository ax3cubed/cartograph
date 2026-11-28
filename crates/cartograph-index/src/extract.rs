//! Extraction: facts for each distinct (blob, extractor kind), from the cache or parsed on a rayon
//! pool (IDX-05). Cache writes are batched, one transaction per 1,000 blobs.

use std::collections::HashMap;
use std::sync::Arc;

use cartograph_facts::FactCache;
use cartograph_git::{GitError, ObjectId, Repo, read_blob};
use cartograph_parse::{BlobFacts, EXTRACTORS, Extractor, SkipReason};
use rayon::prelude::*;

pub type Facts = HashMap<(ObjectId, &'static str), Arc<BlobFacts>>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExtractStats {
    pub blobs: usize,
    pub from_cache: usize,
    pub parsed: usize,
    pub skipped: usize,
}

/// The extractor that produced a kind ("ts-imports/tsx" belongs to "ts-imports").
pub fn extractor_of(kind: &str) -> &'static dyn Extractor {
    *EXTRACTORS
        .iter()
        .find(|e| kind.split('/').next() == Some(e.name()))
        .expect("a kind comes from an extractor")
}

/// Facts for every item not already in `into`.
pub fn extract(
    repo: &Repo,
    items: impl IntoIterator<Item = (ObjectId, &'static str)>,
    cache: &mut FactCache,
    pool: &rayon::ThreadPool,
    into: &mut Facts,
) -> Result<ExtractStats, GitError> {
    let mut todo: Vec<(ObjectId, &'static str)> = items
        .into_iter()
        .filter(|k| !into.contains_key(k))
        .collect();
    todo.sort_unstable();
    todo.dedup();
    let mut st = ExtractStats {
        blobs: todo.len(),
        ..Default::default()
    };
    for chunk in todo.chunks(1000) {
        let mut miss = Vec::new();
        for &(oid, kind) in chunk {
            match cache.get(oid.as_bytes(), kind, extractor_of(kind).version()) {
                Some(f) => {
                    st.from_cache += 1;
                    st.skipped += usize::from(matches!(f, BlobFacts::Skipped(_)));
                    into.insert((oid, kind), Arc::new(f));
                }
                None => miss.push((oid, kind)),
            }
        }
        let parsed: Vec<_> = pool.install(|| {
            miss.par_iter()
                .map_init(
                    || repo.local(),
                    |local, &(oid, kind)| {
                        let facts = match read_blob(local, oid, extractor_of(kind).max_bytes(kind))
                        {
                            Ok(bytes) => extractor_of(kind).extract(kind, &bytes),
                            Err(GitError::TooLarge { .. }) => {
                                BlobFacts::Skipped(SkipReason::TooLarge)
                            }
                            Err(e) => return Err(e),
                        };
                        Ok((oid, kind, facts))
                    },
                )
                .collect::<Result<_, _>>()
        })?;
        st.parsed += parsed.len();
        st.skipped += parsed
            .iter()
            .filter(|(_, _, f)| matches!(f, BlobFacts::Skipped(_)))
            .count();
        // ponytail: a cache write failure (disk full, a locked file) only costs a re-parse next run.
        let _ = cache.put_many(
            parsed
                .iter()
                .map(|(o, k, f)| (o.as_bytes(), *k, extractor_of(k).version(), f)),
        );
        into.extend(parsed.into_iter().map(|(o, k, f)| ((o, k), Arc::new(f))));
    }
    Ok(st)
}
