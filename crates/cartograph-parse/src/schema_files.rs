//! manifest-schema (XS-17): SQL and Prisma files carry no imports and no facts. They are tracked so
//! each has a node and a history, which is what the table facts of `cartograph-index` (the tables a
//! migration or a model defines) are attached to.

use crate::{BlobFacts, Extractor};

pub struct ManifestSchema;

/// The kind [`ManifestSchema`] reads.
pub const KIND: &str = "manifest-schema";

impl Extractor for ManifestSchema {
    fn name(&self) -> &'static str {
        KIND
    }
    fn version(&self) -> &'static str {
        "1"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[KIND]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        matches!(path.rsplit_once('.'), Some((_, "sql" | "prisma"))).then_some(KIND)
    }
    fn extract(&self, _kind: &'static str, bytes: &[u8]) -> BlobFacts {
        crate::skip_reason(bytes).map_or(BlobFacts::Facts(vec![]), BlobFacts::Skipped)
    }
}
