//! Timeline and commits (SRV-10): commits paged newest first, one commit, density bins stacked by
//! repo (board G04), rename and rewrite markers, and a commit's impact with its neighbours along
//! the repo's first-parent line.
//!
//! Density is one pass over the generation's in-memory timeline and link end points (SRV-11's
//! graph, already cached per generation), so ten years of weekly bins is a few hundred thousand
//! integer comparisons. Commit messages and authors are repo text: HTTP shows them as plain text.

use cartograph_api::types::{
    Bin, Commit, CommitImpact, DensityBin, MarkerKind, TimelineDensity, TimelineMarker,
};
use cartograph_core::{NodeId, RepoId};
use cartograph_store::rusqlite::{self, OptionalExtension};

use crate::adjacency::NONE;
use crate::at::{civil_from_days, days_from_civil};
use crate::page::{Page, Paged};
use crate::resolve::{hex, node_id};
use crate::{At, QueryError, Snapshot, rfc3339};

const DAY: i64 = 86_400;
/// More bins than this in one answer is refused: ten years of days is 3,653.
pub const MAX_BINS: usize = 5_000;
/// Markers in one answer.
const MAX_MARKERS: usize = 1_000;

/// Midnight UTC on the first of `t`'s month.
pub(crate) fn month_start(t: i64) -> i64 {
    let (y, m, _) = civil_from_days(t.div_euclid(DAY));
    days_from_civil(y, m, 1) * DAY
}

/// The first of the month `n` months after the month starting at `start`.
pub(crate) fn add_months(start: i64, n: i64) -> i64 {
    let (y, m, _) = civil_from_days(start.div_euclid(DAY));
    let k = y * 12 + (m - 1) + n;
    days_from_civil(k.div_euclid(12), k.rem_euclid(12) + 1, 1) * DAY
}

/// The start of the bin holding `t`; weeks start on Monday, midnight UTC.
pub fn bin_start(bin: Bin, t: i64) -> i64 {
    let day = t.div_euclid(DAY);
    match bin {
        Bin::Day => day * DAY,
        // 1970-01-01 was a Thursday: day 4 (1970-01-05) is the first Monday.
        Bin::Week => ((day - 4).div_euclid(7) * 7 + 4) * DAY,
        Bin::Month => month_start(t),
    }
}

fn next_bin(bin: Bin, start: i64) -> i64 {
    match bin {
        Bin::Day => start + DAY,
        Bin::Week => start + 7 * DAY,
        Bin::Month => add_months(start, 1),
    }
}

impl Snapshot<'_> {
    /// An instant for a range end: a date as given, a commit's own time, now's last commit.
    fn instant(&self, at: &At) -> Result<i64, QueryError> {
        Ok(self.step(at)?.1)
    }

    /// `GET …/commits`: commits at or before `at`, newest first.
    pub fn commits(&self, at: &At, page: Page) -> Result<Paged<Commit>, QueryError> {
        let c = self.resolve_at(at)?;
        let mut st = self.conn().prepare_cached(
            "SELECT seq FROM commits WHERE seq <= ?1 ORDER BY seq DESC LIMIT ?2 OFFSET ?3",
        )?;
        let seqs: Vec<i64> = st
            .query_map(
                rusqlite::params![c.seq as i64, page.limit as i64 + 1, page.offset as i64],
                |r| r.get(0),
            )?
            .collect::<Result<_, _>>()?;
        let more = seqs.len() > page.limit;
        Ok(Paged {
            rows: seqs
                .into_iter()
                .take(page.limit)
                .map(|s| self.commit(s as u32))
                .collect::<Result<_, _>>()?,
            next_cursor: more.then(|| format!("o{}", page.offset + page.limit)),
        })
    }

    /// `GET …/commits/{sha}`.
    pub fn commit_by_sha(&self, sha: &str) -> Result<Commit, QueryError> {
        let c = self.commit_at(sha)?;
        self.commit(c.seq as u32)
    }

    /// A sha (or a prefix of one) on the indexed timeline; a date or `now` is refused.
    fn commit_at(&self, sha: &str) -> Result<cartograph_api::types::CommitRef, QueryError> {
        match At::parse(sha)? {
            at @ At::Sha(_) => self.resolve_at(&at),
            _ => Err(QueryError::bad(
                "that isn't a commit sha",
                "this route takes a commit sha, at least 4 hex digits",
                "pass a sha from …/commits",
            )),
        }
    }

    /// `GET …/timeline/density`: commits per bin, stacked by repo, and the links added and removed
    /// in each, from `from` to `to` (the whole history by default). Every bin in the range is
    /// listed, empty ones too, so the bins' commits sum to the commits in the range.
    pub fn density(
        &self,
        from: Option<&At>,
        to: Option<&At>,
        bin: Bin,
    ) -> Result<TimelineDensity, QueryError> {
        let g = self.adjacency()?;
        let (Some(&first), Some(&last)) = (g.time.first(), g.time.last()) else {
            return Ok(TimelineDensity {
                bin,
                repos: Vec::new(),
                bins: Vec::new(),
            });
        };
        let lo = from.map(|a| self.instant(a)).transpose()?.unwrap_or(first);
        let hi = to.map(|a| self.instant(a)).transpose()?.unwrap_or(last);
        if hi < lo {
            return Err(QueryError::bad(
                "from is after to",
                "the range runs from from to to",
                "swap them",
            ));
        }
        let mut starts = vec![bin_start(bin, lo)];
        while let Some(&s) = starts.last() {
            let n = next_bin(bin, s);
            if n > hi {
                break;
            }
            if starts.len() >= MAX_BINS {
                return Err(QueryError::TooLarge {
                    why: format!("the range holds more than {MAX_BINS} bins"),
                    narrower: "use a coarser bin (week or month) or a shorter range".into(),
                });
            }
            starts.push(n);
        }
        let repos: Vec<(i64, String)> = self
            .conn()
            .prepare_cached(
                "SELECT id, name FROM repo WHERE id IN (SELECT DISTINCT repo FROM commits) ORDER BY id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut bins: Vec<DensityBin> = starts
            .iter()
            .map(|&s| DensityBin {
                start: rfc3339(s),
                commits: 0,
                links_added: 0,
                links_removed: 0,
                by_repo: vec![0; repos.len()],
            })
            .collect();
        let slot = |t: i64| -> Option<usize> {
            (lo..=hi)
                .contains(&t)
                .then(|| starts.partition_point(|&s| s <= t) - 1)
        };
        for (seq, &t) in g.time.iter().enumerate() {
            if let Some(b) = slot(t) {
                bins[b].commits += 1;
                if let Some(r) = repos.iter().position(|(id, _)| *id == g.repo[seq]) {
                    bins[b].by_repo[r] += 1;
                }
            }
        }
        for e in &g.edges {
            if let Some(b) = slot(g.time[e.from as usize]) {
                bins[b].links_added += 1;
            }
            if e.to != NONE
                && let Some(b) = slot(g.time[e.to as usize])
            {
                bins[b].links_removed += 1;
            }
        }
        Ok(TimelineDensity {
            bin,
            repos: repos.into_iter().map(|(_, n)| RepoId(n)).collect(),
            bins,
        })
    }

    /// `GET …/timeline/markers`: renames (a file taking a new path) and rewritten histories, oldest
    /// first, at most 1,000; the flag says whether more were left out.
    pub fn markers(
        &self,
        from: Option<&At>,
        to: Option<&At>,
    ) -> Result<(Vec<TimelineMarker>, bool), QueryError> {
        let g = self.adjacency()?;
        let lo = match from {
            Some(a) => self.resolve_at(a)?.seq as i64,
            None => 0,
        };
        let hi = match to {
            Some(a) => self.resolve_at(a)?.seq as i64,
            None => i64::MAX,
        };
        let mut out: Vec<(u32, TimelineMarker)> = Vec::new();
        let renames: Vec<(i64, String, Option<String>)> = self
            .conn()
            .prepare_cached(
                "SELECT fp.from_seq, fp.path,
                   (SELECT p.path FROM file_path p WHERE p.file = fp.file AND p.to_seq = fp.from_seq
                    ORDER BY p.from_seq DESC LIMIT 1)
                 FROM file_path fp WHERE fp.via != 'add' AND fp.from_seq BETWEEN ?1 AND ?2
                 ORDER BY fp.from_seq, fp.path LIMIT ?3",
            )?
            .query_map(rusqlite::params![lo, hi, MAX_MARKERS as i64 + 1], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        for (seq, path, old) in renames {
            let label = match old {
                Some(o) => format!("{o} → {path}"),
                None => format!("→ {path}"),
            };
            out.push((
                seq as u32,
                TimelineMarker {
                    at: self.commit_ref(seq as u32)?,
                    kind: MarkerKind::Rename,
                    label,
                },
            ));
        }
        let rewrites: Vec<(Option<i64>, String)> = self
            .conn()
            .prepare_cached(
                "SELECT w.at, r.name FROM rewrite w JOIN repo r ON r.id = w.repo ORDER BY w.id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (at, repo) in rewrites {
            // Marked at the merge base: the last step that both histories share.
            let seq = at.and_then(|t| g.seq_at(t)).unwrap_or(0);
            if (lo..=hi).contains(&i64::from(seq)) {
                out.push((
                    seq,
                    TimelineMarker {
                        at: self.commit_ref(seq)?,
                        kind: MarkerKind::Rewrite,
                        label: format!("{repo}: history rewritten after this commit"),
                    },
                ));
            }
        }
        out.sort_by_key(|(s, _)| *s);
        let truncated = out.len() > MAX_MARKERS;
        out.truncate(MAX_MARKERS);
        Ok((out.into_iter().map(|(_, m)| m).collect(), truncated))
    }

    /// `GET …/commits/{sha}/impact`: the links the commit added and removed, the previous and next
    /// commits on its repo's first-parent line, and what depends on the files whose links changed
    /// (over the links live just after the commit).
    pub fn commit_impact(&self, sha: &str) -> Result<CommitImpact, QueryError> {
        let c = self.commit_at(sha)?;
        let seq = c.seq as u32;
        let commit = self.commit(seq)?;
        let g = self.adjacency()?;
        let repo = g.repo[seq as usize];
        let neighbour = |sql: &str| -> Result<Option<String>, QueryError> {
            Ok(self
                .conn()
                .prepare_cached(sql)?
                .query_row(rusqlite::params![repo, c.seq as i64], |r| {
                    r.get::<_, Vec<u8>>(0)
                })
                .optional()?
                .map(|s| hex(&s)))
        };
        let prev = neighbour(
            "SELECT sha FROM commits WHERE repo = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT 1",
        )?;
        let next =
            neighbour("SELECT sha FROM commits WHERE repo = ?1 AND seq > ?2 ORDER BY seq LIMIT 1")?;
        let added: Vec<_> = g.edges.iter().filter(|e| e.from == seq).collect();
        let removed: Vec<_> = g.edges.iter().filter(|e| e.to == seq).collect();
        let mut touched: Vec<u32> = added.iter().chain(&removed).map(|e| e.src).collect();
        touched.sort_unstable();
        touched.dedup();
        let walk = crate::traverse::dependents(&g, &touched, seq, 0.0);
        Ok(CommitImpact {
            commit,
            prev,
            next,
            added_links: added
                .iter()
                .map(|e| self.wire_link(&g, e))
                .collect::<Result<_, _>>()?,
            removed_links: removed
                .iter()
                .map(|e| self.wire_link(&g, e))
                .collect::<Result<_, _>>()?,
            affected: walk
                .nodes
                .iter()
                .map(|&n| NodeId(node_id(g.row[n as usize])))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_instant;

    #[test]
    fn bins_start_on_the_day_monday_and_first_of_the_month() {
        let t = parse_instant("2024-03-14T15:00:00Z").unwrap();
        assert_eq!(rfc3339(bin_start(Bin::Day, t)), "2024-03-14T00:00:00Z");
        assert_eq!(rfc3339(bin_start(Bin::Week, t)), "2024-03-11T00:00:00Z");
        assert_eq!(rfc3339(bin_start(Bin::Month, t)), "2024-03-01T00:00:00Z");
        assert_eq!(rfc3339(bin_start(Bin::Week, 0)), "1969-12-29T00:00:00Z");
        let m = bin_start(Bin::Month, t);
        assert_eq!(rfc3339(add_months(m, 10)), "2025-01-01T00:00:00Z");
        assert_eq!(rfc3339(add_months(m, -3)), "2023-12-01T00:00:00Z");
    }
}
