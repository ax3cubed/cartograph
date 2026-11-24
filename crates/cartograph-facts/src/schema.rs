//! The interface fact and the registry around it (XS-01): what an adapter says about one place in a
//! file, in one shape shared by the built-in query adapters and (later) subprocess adapters. The
//! JSON form is the NDJSON line of H4, and `fact.schema.json` is generated from these types.
//!
//! Everything here is untrusted text (F8), so every string has a cap in its type: a value over the
//! cap fails to parse instead of being cut, and an unknown kind, predicate or class is an error, not
//! a stored row.

use std::collections::BTreeMap;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

/// The envelope version a fact carries (`"schema":1`).
pub const SCHEMA_VERSION: u32 = 1;

/// Caps. Path 4 KiB, attribute value 2 KiB, evidence text 200 characters (XS-01).
pub const MAX_PATH: usize = 4096;
pub const MAX_ATTR_VALUE: usize = 2048;
pub const MAX_ATTR_KEY: usize = 64;
pub const MAX_EVIDENCE: usize = 200;
pub const MAX_ATTRS: usize = 32;
pub const MAX_PARTS: usize = 32;
const MAX_NAME: usize = 64;
const MAX_VERSION: usize = 32;
const MAX_REPO: usize = 256;
const MAX_COMMIT: usize = 64;

/// A string of at most `N` characters. Parsing one longer fails; [`Capped::truncated`] is how code
/// that builds a value from source text keeps it in range.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Capped<const N: usize>(String);

/// A string over its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong {
    pub max: usize,
}

impl fmt::Display for TooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "longer than {} characters", self.max)
    }
}

impl<const N: usize> Capped<N> {
    pub fn new(s: impl Into<String>) -> Result<Self, TooLong> {
        let s = s.into();
        // Bytes bound the characters, so the count is only taken when the length could be over.
        if s.len() > N && s.chars().count() > N {
            return Err(TooLong { max: N });
        }
        Ok(Self(s))
    }

    /// `s` cut to `N` characters.
    pub fn truncated(s: &str) -> Self {
        Self(s.chars().take(N).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> Serialize for Capped<N> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de, const N: usize> Deserialize<'de> for Capped<N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(de::Error::custom)
    }
}

impl<const N: usize> JsonSchema for Capped<N> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("Capped{N}").into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "maxLength": N })
    }
}

/// What an adapter found.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum FactKind {
    #[serde(rename = "route")]
    Route,
    #[serde(rename = "http.call")]
    HttpCall,
    #[serde(rename = "event.publish")]
    EventPublish,
    #[serde(rename = "event.subscribe")]
    EventSubscribe,
    #[serde(rename = "db.read")]
    DbRead,
    #[serde(rename = "db.write")]
    DbWrite,
    #[serde(rename = "db.table.def")]
    DbTableDef,
    #[serde(rename = "env.read")]
    EnvRead,
    #[serde(rename = "client.base")]
    ClientBase,
    #[serde(rename = "entrypoint")]
    Entrypoint,
    #[serde(rename = "symbol.def")]
    SymbolDef,
    #[serde(rename = "symbol.ref")]
    SymbolRef,
    #[serde(rename = "codegen.marker")]
    CodegenMarker,
}

impl FactKind {
    pub const ALL: [FactKind; 13] = [
        Self::Route,
        Self::HttpCall,
        Self::EventPublish,
        Self::EventSubscribe,
        Self::DbRead,
        Self::DbWrite,
        Self::DbTableDef,
        Self::EnvRead,
        Self::ClientBase,
        Self::Entrypoint,
        Self::SymbolDef,
        Self::SymbolRef,
        Self::CodegenMarker,
    ];

    /// The wire name (`http.call`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::HttpCall => "http.call",
            Self::EventPublish => "event.publish",
            Self::EventSubscribe => "event.subscribe",
            Self::DbRead => "db.read",
            Self::DbWrite => "db.write",
            Self::DbTableDef => "db.table.def",
            Self::EnvRead => "env.read",
            Self::ClientBase => "client.base",
            Self::Entrypoint => "entrypoint",
            Self::SymbolDef => "symbol.def",
            Self::SymbolRef => "symbol.ref",
            Self::CodegenMarker => "codegen.marker",
        }
    }

    /// The kind with this wire name.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// The node classes this phase adds to the graph.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum NodeClass {
    Operation,
    Channel,
    Table,
    Entrypoint,
    Client,
    Contract,
    ConfigKey,
    Symbol,
    Owner,
    Concept,
}

/// What a link between two nodes means.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Predicate {
    Consumes,
    HandledBy,
    Publishes,
    Subscribes,
    Reads,
    Writes,
    ConformsTo,
    ResolvesTo,
    Calls,
    OwnedBy,
    InstanceOf,
}

/// Where a link's evidence came from, weakest last: a correction overrides the rest.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OriginLayer {
    Parsed,
    Contract,
    Config,
    Runtime,
    Correction,
}

/// `[start_line, start_col, end_line, end_col]`: 1-based lines, 0-based columns (H4).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct FactSpan(pub [u32; 4]);

impl From<cartograph_parse::Span> for FactSpan {
    fn from(s: cartograph_parse::Span) -> Self {
        Self([s.start_line, s.start_col, s.end_line, s.end_col])
    }
}

/// One piece of a value built from a template or a concatenation: text the code spells out, or an
/// expression it doesn't, so a matcher can see which segments are literal.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Part {
    Lit(Capped<MAX_ATTR_VALUE>),
    Expr(Capped<MAX_ATTR_VALUE>),
}

/// At most [`MAX_PARTS`] parts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Parts(Vec<Part>);

impl Parts {
    pub fn new(parts: Vec<Part>) -> Result<Self, TooLong> {
        if parts.len() > MAX_PARTS {
            return Err(TooLong { max: MAX_PARTS });
        }
        Ok(Self(parts))
    }

    pub fn as_slice(&self) -> &[Part] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Parts {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(Vec::deserialize(d)?).map_err(|_| de::Error::custom("too many parts"))
    }
}

impl JsonSchema for Parts {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Parts".into()
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "array", "items": g.subschema_for::<Part>(), "maxItems": MAX_PARTS })
    }
}

/// An attribute's value: a plain string, or the parts of a template.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(untagged)]
pub enum AttrValue {
    Str(Capped<MAX_ATTR_VALUE>),
    Parts { parts: Parts },
}

/// At most [`MAX_ATTRS`] attributes, in key order (so a fact serialises one way).
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Attrs(BTreeMap<Capped<MAX_ATTR_KEY>, AttrValue>);

impl Attrs {
    pub fn new(map: BTreeMap<Capped<MAX_ATTR_KEY>, AttrValue>) -> Result<Self, TooLong> {
        if map.len() > MAX_ATTRS {
            return Err(TooLong { max: MAX_ATTRS });
        }
        Ok(Self(map))
    }

    pub fn get(&self, key: &str) -> Option<&AttrValue> {
        self.0
            .iter()
            .find_map(|(k, v)| (k.as_str() == key).then_some(v))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &AttrValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for Attrs {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(BTreeMap::deserialize(d)?).map_err(|_| de::Error::custom("too many attributes"))
    }
}

impl JsonSchema for Attrs {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Attrs".into()
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "additionalProperties": g.subschema_for::<AttrValue>(),
            "propertyNames": { "maxLength": MAX_ATTR_KEY },
            "maxProperties": MAX_ATTRS
        })
    }
}

/// How the evidence was read from the source.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    /// The source spells the value out.
    Literal,
    /// A template or concatenation with parts the code doesn't spell out.
    Template,
    /// Worked out from more than one place.
    Inferred,
}

/// The source text a fact stands on, cut to [`MAX_EVIDENCE`] characters.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    #[serde(rename = "type")]
    pub kind: EvidenceType,
    pub text: Capped<MAX_EVIDENCE>,
}

/// The extractor that produced a fact; with the blob it is the cache key.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ExtractorRef {
    pub name: Capped<MAX_NAME>,
    pub version: Capped<MAX_VERSION>,
}

/// The `"schema":1` of a line: any other value is a fact this build can't read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaTag;

impl Serialize for SchemaTag {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(SCHEMA_VERSION)
    }
}

impl<'de> Deserialize<'de> for SchemaTag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u32::deserialize(d)? {
            SCHEMA_VERSION => Ok(Self),
            v => Err(de::Error::custom(format!("unsupported fact schema {v}"))),
        }
    }
}

impl JsonSchema for SchemaTag {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SchemaTag".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "const": SCHEMA_VERSION })
    }
}

/// One interface fact, as an adapter prints it (H4).
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub schema: SchemaTag,
    pub kind: FactKind,
    pub repo: Capped<MAX_REPO>,
    pub commit: Capped<MAX_COMMIT>,
    pub path: Capped<MAX_PATH>,
    pub span: FactSpan,
    #[serde(default)]
    pub attrs: Attrs,
    pub evidence: Evidence,
    pub extractor: ExtractorRef,
}

/// A fact as the cache keeps it: the part that follows from one blob's bytes. Where the blob sits
/// (repo, commit, path) is not in it, so one blob read in two places is parsed once, and the
/// extractor is the cache key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobFact {
    pub kind: FactKind,
    pub span: FactSpan,
    #[serde(default, skip_serializing_if = "Attrs::is_empty")]
    pub attrs: Attrs,
    pub evidence: Evidence,
}

impl BlobFact {
    /// The fact in a place, from an extractor.
    pub fn locate(
        self,
        repo: &str,
        commit: &str,
        path: &str,
        extractor: ExtractorRef,
    ) -> Result<Fact, TooLong> {
        Ok(Fact {
            schema: SchemaTag,
            kind: self.kind,
            repo: Capped::new(repo)?,
            commit: Capped::new(commit)?,
            path: Capped::new(path)?,
            span: self.span,
            attrs: self.attrs,
            evidence: self.evidence,
            extractor,
        })
    }
}

/// The JSON Schema of [`Fact`], as `fact.schema.json` keeps it.
pub fn json_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Fact)).expect("a schema is JSON")
}

/// One blob's interface facts as the cache stores them: one JSON array per (blob, extractor).
pub fn encode(facts: &[BlobFact]) -> Vec<u8> {
    serde_json::to_vec(facts).expect("facts always encode")
}

/// `None` when the bytes aren't facts this build reads, which a cache counts as a miss.
pub fn decode(bytes: &[u8]) -> Option<Vec<BlobFact>> {
    serde_json::from_slice(bytes).ok()
}

#[cfg(test)]
mod tests;
