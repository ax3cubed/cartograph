//! Facts and extractors (IDX-05, H1; Go, Python and PHP, IDX-23). A fact is what one file says, from its bytes alone, so it is
//! extracted once per blob id and cached by (blob, extractor, version).
//!
//! Parser input is hostile: blobs over 1 MB, binary blobs, LFS pointers and generated files are
//! skipped with a reason; each parse has a time limit; the syntax walk is depth-limited. Facts hold
//! specifier strings and spans only, never source text beyond the specifier.

use cartograph_core as _;
use serde::{Deserialize, Serialize};

pub mod deploy;
mod lang;
pub mod manifest;
mod manifest_lang;
pub mod openapi;
pub mod syntax;
mod ts;

pub use deploy::ManifestDeploy;
pub use lang::{GoImports, PhpImports, PyImports};
pub use manifest::ManifestJs;
pub use manifest_lang::{ManifestGo, ManifestPhp, ManifestPy};
pub use openapi::ManifestOpenapi;
pub use ts::TsImports;

/// Blobs over this size are skipped without being read.
pub const MAX_BLOB_BYTES: u64 = 1_000_000;

/// 1-based lines, 0-based columns (H4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Span {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ImportStyle {
    Esm,
    Reexport,
    Cjs,
    Dynamic,
    TypeOnly,
    /// IDX-23. New styles go at the end: the encoding of cached facts stays the same.
    Go,
    Python,
    PhpUse,
    PhpInclude,
}

/// One fact. Strings in a fact come from the repository and are untrusted text (F8).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Fact {
    /// `import`, `export … from`, `require("…")` or `import("…")` with a literal specifier.
    Import {
        spec: String,
        style: ImportStyle,
        span: Span,
    },
    /// `require(x)` or `import(x)` with a computed argument, kept for coverage.
    DynamicUnresolved { span: Span },
    /// One top-level field of a package.json or tsconfig, its value as canonical JSON.
    Manifest { field: String, value: String },
    /// A host name a compose service or k8s Service answers to (XS-02). `port` is the one it
    /// listens on, `None` for any; `context` is a compose `build` context as written (relative to
    /// the file), `None` for k8s.
    Host {
        host: String,
        port: Option<u16>,
        context: Option<String>,
    },
    /// An OpenAPI or Swagger document (XS-08): its version, title, base paths and the relative
    /// files its refs read. New facts go at the end: the encoding of cached facts stays the same.
    Spec {
        version: String,
        title: String,
        bases: Vec<String>,
        files: Vec<String>,
        truncated: bool,
    },
    /// One operation of a spec; the path is as the spec writes it, without a base.
    Operation {
        method: String,
        path: String,
        operation_id: Option<String>,
        summary: Option<String>,
        line: u32,
    },
    /// A schema of a spec and its fields.
    Schema {
        name: String,
        fields: Vec<openapi::SchemaField>,
    },
    /// A `$ref` of a spec that was not followed: `remote`, `missing`, `outside`, `cycle` or `deep`.
    SpecRef { target: String, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkipReason {
    TooLarge,
    Binary,
    LfsPointer,
    Generated,
    Timeout,
}

/// What an extractor says about one blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlobFacts {
    /// Sorted, so the encoding is deterministic (IDX-17).
    Facts(Vec<Fact>),
    Skipped(SkipReason),
}

impl BlobFacts {
    pub fn facts(&self) -> &[Fact] {
        match self {
            Self::Facts(f) => f,
            Self::Skipped(_) => &[],
        }
    }

    /// The postcard encoding stored in the fact cache and the index.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("facts always encode")
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        postcard::from_bytes(bytes).ok()
    }
}

pub trait Extractor: Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    /// The cache key's extractor part for this path (the extractor and its grammar or file kind),
    /// or `None` when this extractor doesn't handle the path.
    fn handles(&self, path: &str) -> Option<&'static str>;
    /// Every kind [`Extractor::handles`] can return.
    fn kinds(&self) -> &'static [&'static str];
    /// Facts for one blob. `kind` is what [`Extractor::handles`] returned.
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts;
    /// The most bytes of a blob of this kind that are read.
    fn max_bytes(&self, _kind: &'static str) -> u64 {
        MAX_BLOB_BYTES
    }
}

/// The built-in extractors.
pub static EXTRACTORS: [&dyn Extractor; 10] = [
    &TsImports,
    &ManifestJs,
    &GoImports,
    &PyImports,
    &PhpImports,
    &ManifestGo,
    &ManifestPy,
    &ManifestPhp,
    &ManifestDeploy,
    // Last: it handles every YAML and JSON path the ones before it did not.
    &ManifestOpenapi,
];

/// The extractor for a path, with its kind. Paths under node_modules, dist, build, vendor or .next
/// are not tracked (as in the research).
pub fn extractor_for(path: &str) -> Option<(&'static dyn Extractor, &'static str)> {
    if path
        .split('/')
        .rev()
        .skip(1)
        .any(|d| matches!(d, "node_modules" | "dist" | "build" | "vendor" | ".next"))
    {
        return None;
    }
    EXTRACTORS
        .iter()
        .find_map(|e| e.handles(path).map(|k| (*e, k)))
}

/// True when some extractor tracks this path.
pub fn tracked(path: &str) -> bool {
    extractor_for(path).is_some_and(|(_, k)| k != openapi::KIND || openapi::named(path))
}

/// The skip rules every extractor applies before parsing (size is checked before the read).
pub fn skip_reason(bytes: &[u8]) -> Option<SkipReason> {
    if bytes.len() as u64 > MAX_BLOB_BYTES {
        return Some(SkipReason::TooLarge);
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Some(SkipReason::Binary);
    }
    if bytes.starts_with(b"version https://git-lfs.github.com/spec/") {
        return Some(SkipReason::LfsPointer);
    }
    let head = &bytes[..bytes.len().min(512)];
    let has = |n: &[u8]| head.windows(n.len()).any(|w| w == n);
    if has(b"@generated") || has(b"DO NOT EDIT") {
        return Some(SkipReason::Generated);
    }
    None
}

#[cfg(test)]
mod tests;
