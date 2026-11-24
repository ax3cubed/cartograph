//! Confidence engine (XS-05, H2): evidence in, level and score out. Pure: no database, no IO.
//!
//! A link's chain has three steps: how literal the call site is, how the base or host was
//! resolved, and how the target is anchored. Each evidence type belongs to a step (or, for a
//! matched contract, to all of them) and has a fixed class. A step is as good as its best
//! evidence; a step with no evidence is `Weak`. The link's class is its weakest step. Inside that
//! class the score is the noisy-or of the evidence of that class, clamped to the class band.
//! Adding evidence can only keep or raise a link's level and score.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

/// Most evidence rows that count towards one link; the rest are ignored.
pub const MAX_EVIDENCE: usize = 32;

/// Name the confidence engine is recorded under as an extractor.
pub const EXTRACTOR: &str = "links/confidence";

const WEIGHTS_TOML: &str = include_str!("weights.toml");

/// Confidence class, weakest first. `Weak` is stored but hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Weak,
    Guessed,
    Likely,
    Exact,
}

impl Level {
    /// The inclusive score band of this level.
    pub fn band(self) -> (f64, f64) {
        match self {
            Level::Weak => (0.05, 0.29),
            Level::Guessed => (0.30, 0.59),
            Level::Likely => (0.60, 0.89),
            Level::Exact => (0.90, 1.00),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Weak => "weak",
            Level::Guessed => "guessed",
            Level::Likely => "likely",
            Level::Exact => "exact",
        }
    }
}

/// The chain step an evidence type speaks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    CallSite,
    Base,
    Target,
    /// A matched contract settles every step.
    All,
}

/// Everything that can support a link. The order is the registry order and fixes the order the
/// cap applies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EvidenceType {
    /// OpenAPI or proto operation matched on both ends.
    ContractMatched,
    /// The call site holds the whole URL as a literal.
    LiteralCallSiteUrl,
    /// A framework adapter anchored the handler.
    HandlerAnchored,
    /// The base URL or host came from a resolved env or config value.
    EnvResolvedBase,
    /// The host matched a known service alias.
    HostAliasResolved,
    /// The whole path template matched a route.
    FullPathMatch,
    /// Only part of the path matched.
    PartialPathMatch,
    /// One route in the workspace fits, and the host is unknown.
    UniqueAcrossWorkspace,
    /// Names alone look alike. Never shown.
    NameSimilarity,
}

impl EvidenceType {
    pub const ALL: [EvidenceType; 9] = [
        EvidenceType::ContractMatched,
        EvidenceType::LiteralCallSiteUrl,
        EvidenceType::HandlerAnchored,
        EvidenceType::EnvResolvedBase,
        EvidenceType::HostAliasResolved,
        EvidenceType::FullPathMatch,
        EvidenceType::PartialPathMatch,
        EvidenceType::UniqueAcrossWorkspace,
        EvidenceType::NameSimilarity,
    ];

    /// The key used in `weights.toml` and in snapshots.
    pub fn name(self) -> &'static str {
        match self {
            EvidenceType::ContractMatched => "contract_matched",
            EvidenceType::LiteralCallSiteUrl => "literal_call_site_url",
            EvidenceType::HandlerAnchored => "handler_anchored",
            EvidenceType::EnvResolvedBase => "env_resolved_base",
            EvidenceType::HostAliasResolved => "host_alias_resolved",
            EvidenceType::FullPathMatch => "full_path_match",
            EvidenceType::PartialPathMatch => "partial_path_match",
            EvidenceType::UniqueAcrossWorkspace => "unique_across_workspace",
            EvidenceType::NameSimilarity => "name_similarity",
        }
    }

    /// Fixed by the type, not configurable.
    pub fn class(self) -> Level {
        match self {
            EvidenceType::ContractMatched
            | EvidenceType::LiteralCallSiteUrl
            | EvidenceType::HandlerAnchored => Level::Exact,
            EvidenceType::EnvResolvedBase
            | EvidenceType::HostAliasResolved
            | EvidenceType::FullPathMatch => Level::Likely,
            EvidenceType::PartialPathMatch | EvidenceType::UniqueAcrossWorkspace => Level::Guessed,
            EvidenceType::NameSimilarity => Level::Weak,
        }
    }

    pub fn step(self) -> Step {
        match self {
            EvidenceType::ContractMatched => Step::All,
            EvidenceType::LiteralCallSiteUrl => Step::CallSite,
            EvidenceType::EnvResolvedBase | EvidenceType::HostAliasResolved => Step::Base,
            EvidenceType::HandlerAnchored
            | EvidenceType::FullPathMatch
            | EvidenceType::PartialPathMatch
            | EvidenceType::UniqueAcrossWorkspace
            | EvidenceType::NameSimilarity => Step::Target,
        }
    }

    /// Fixed by `weights.toml`.
    pub fn weight(self) -> f64 {
        weights()[self as usize]
    }
}

impl fmt::Display for EvidenceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A scored link.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Confidence {
    pub level: Level,
    pub score: f64,
}

fn weights() -> &'static [f64; EvidenceType::ALL.len()] {
    static W: OnceLock<[f64; EvidenceType::ALL.len()]> = OnceLock::new();
    W.get_or_init(|| {
        // The file is compiled in; `weights_file_is_complete_and_valid` keeps it honest.
        let table: BTreeMap<String, f64> = toml::from_str(WEIGHTS_TOML).expect("weights.toml");
        EvidenceType::ALL.map(|t| table[t.name()])
    })
}

/// The extractor version of the confidence engine: a format number plus a hash of
/// `weights.toml`, so a re-weight changes it and nothing else does.
pub fn extractor_version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        let digest = Sha256::digest(WEIGHTS_TOML.as_bytes());
        let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        format!("1+w{hex}")
    })
}

/// Scores one link from its evidence. At most [`MAX_EVIDENCE`] rows count, taken in registry
/// order, so the result does not depend on the order of `evidence`.
pub fn score(evidence: &[EvidenceType]) -> Confidence {
    let mut counts = [0usize; EvidenceType::ALL.len()];
    for &e in evidence {
        counts[e as usize] += 1;
    }
    let mut budget = MAX_EVIDENCE;
    for c in &mut counts {
        *c = (*c).min(budget);
        budget -= *c;
    }

    let mut steps = [Level::Weak; 3];
    for t in EvidenceType::ALL {
        if counts[t as usize] == 0 {
            continue;
        }
        let range = match t.step() {
            Step::CallSite => 0..1,
            Step::Base => 1..2,
            Step::Target => 2..3,
            Step::All => 0..3,
        };
        for s in &mut steps[range] {
            *s = (*s).max(t.class());
        }
    }
    let level = steps.into_iter().min().unwrap_or(Level::Weak);

    let mut miss = 1.0;
    for t in EvidenceType::ALL {
        if t.class() == level {
            // powi on a count of at most 32: no dependence on order.
            miss *= (1.0 - t.weight()).powi(counts[t as usize] as i32);
        }
    }
    let (lo, hi) = level.band();
    Confidence {
        level,
        score: (1.0 - miss).clamp(lo, hi),
    }
}

#[cfg(test)]
mod tests {
    use super::EvidenceType::*;
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn weights_file_is_complete_and_valid() {
        let table: BTreeMap<String, f64> = toml::from_str(WEIGHTS_TOML).unwrap();
        assert_eq!(table.len(), EvidenceType::ALL.len(), "no stray keys");
        for t in EvidenceType::ALL {
            let w = table[t.name()];
            assert!(w > 0.0 && w < 1.0, "{t}");
        }
    }

    #[test]
    fn weight_table_golden() {
        let mut out = String::new();
        for t in EvidenceType::ALL {
            out.push_str(&format!(
                "{:<24} {:<8} {:<9} {:.2}\n",
                t.name(),
                t.class().as_str(),
                format!("{:?}", t.step()),
                t.weight()
            ));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn extractor_version_tracks_the_weights() {
        assert!(extractor_version().starts_with("1+w"));
        assert_eq!(extractor_version().len(), 3 + 16);
    }

    #[test]
    fn contract_matched_on_both_ends_is_exact() {
        let c = score(&[ContractMatched]);
        assert_eq!(c.level, Level::Exact);
        assert!(c.score >= 0.90);
    }

    #[test]
    fn literal_call_with_env_resolved_host_is_likely() {
        let c = score(&[LiteralCallSiteUrl, EnvResolvedBase, FullPathMatch]);
        assert_eq!(c.level, Level::Likely);
        assert!((0.60..=0.89).contains(&c.score));
    }

    #[test]
    fn partial_path_is_guessed() {
        let c = score(&[LiteralCallSiteUrl, EnvResolvedBase, PartialPathMatch]);
        assert_eq!(c.level, Level::Guessed);
        assert!((0.30..=0.59).contains(&c.score));
    }

    #[test]
    fn a_missing_step_or_no_evidence_is_weak() {
        assert_eq!(score(&[]).level, Level::Weak);
        assert_eq!(score(&[]).score, 0.05);
        assert_eq!(score(&[LiteralCallSiteUrl, FullPathMatch]).level, Level::Weak);
    }

    #[test]
    fn extra_evidence_of_the_same_class_stays_in_band() {
        let base = [LiteralCallSiteUrl, EnvResolvedBase, FullPathMatch];
        let before = score(&base);
        for t in [HostAliasResolved, FullPathMatch, EnvResolvedBase, PartialPathMatch] {
            let mut more = base.to_vec();
            more.push(t);
            let after = score(&more);
            assert_eq!(after.level, before.level, "{t}");
            assert!(after.score >= before.score, "{t}");
        }
    }

    #[test]
    fn evidence_over_the_cap_is_ignored() {
        let few = vec![FullPathMatch; MAX_EVIDENCE];
        let many = vec![FullPathMatch; 1000];
        assert_eq!(score(&few), score(&many));
    }

    fn any_type() -> impl Strategy<Value = EvidenceType> {
        proptest::sample::select(EvidenceType::ALL.to_vec())
    }

    fn evidence() -> impl Strategy<Value = Vec<EvidenceType>> {
        proptest::collection::vec(any_type(), 0..40)
    }

    proptest! {
        #[test]
        fn score_is_inside_its_band(e in evidence()) {
            let c = score(&e);
            let (lo, hi) = c.level.band();
            prop_assert!(c.score >= lo && c.score <= hi);
        }

        #[test]
        fn more_evidence_never_lowers_level_or_score(e in evidence(), extra in any_type()) {
            let before = score(&e);
            let mut more = e;
            more.push(extra);
            let after = score(&more);
            prop_assert!(after.level >= before.level);
            prop_assert!(after.score >= before.score);
        }

        #[test]
        fn order_does_not_matter(e in evidence()) {
            let mut sorted = e.clone();
            sorted.sort();
            let mut reversed = e.clone();
            reversed.reverse();
            prop_assert_eq!(score(&e), score(&sorted));
            prop_assert_eq!(score(&e), score(&reversed));
        }
    }
}
