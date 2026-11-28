//! Table links (XS-17): which service owns a table, and how sure a use of a name is that it means
//! that table. Pure: no database. The facts come from [`scan`], the rows from `crate::tables`.
//!
//! A table is owned by the service whose files define it: a migration that creates it or an ORM
//! model for it. A use (a read or a write in SQL a file spells out) resolves to a definition in
//! three ways, from sure to unsure:
//!
//! * the same name, defined by a migration or by a model that spells the name: [`Match::Defined`];
//! * the same name, defined only by the name an ORM derives from its class: [`Match::Convention`];
//! * a name that differs only in case, underscores or a plural: [`Match::NameOnly`].

use std::collections::{BTreeMap, HashMap};

mod scan;
pub use scan::{MAX_LITERAL, VERSION, applies, clean, scan};

#[cfg(test)]
mod tests;

/// A stretch of steps, `[from, to)`; `None` runs to the last step.
pub type Span = (i64, Option<i64>);

/// How a use matched a definition, sure first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    Defined,
    Convention,
    NameOnly,
}

/// A definition: (table name, owning service, whether an ORM derived the name).
pub type DefKey = (String, i64, bool);

/// The definitions of a workspace, each with the steps it was live in.
#[derive(Default)]
pub struct Defs {
    live: BTreeMap<DefKey, Vec<Span>>,
    /// Names with case, underscores and a plural taken out, to the definitions that have them.
    loose: HashMap<String, Vec<DefKey>>,
}

/// A name as the loose match sees it.
fn loose(name: &str) -> String {
    let n: String = name
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '$'))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    n.strip_suffix('s').map_or(n.clone(), str::to_owned)
}

impl Defs {
    pub fn add(&mut self, name: &str, service: i64, convention: bool, span: Span) {
        self.live
            .entry((name.to_owned(), service, convention))
            .or_default()
            .push(span);
    }

    /// Merges each definition's spans and builds the loose index; call once after the last
    /// [`Defs::add`].
    pub fn finish(&mut self) {
        for spans in self.live.values_mut() {
            *spans = merge(std::mem::take(spans));
        }
        for k in self.live.keys() {
            self.loose.entry(loose(&k.0)).or_default().push(k.clone());
        }
    }

    pub fn spans(&self, k: &DefKey) -> &[Span] {
        self.live.get(k).map_or(&[], Vec::as_slice)
    }

    /// The definition a use of `name` in a file of `service` means, and how sure that is.
    pub fn resolve(&self, name: &str, service: i64) -> Option<(&DefKey, Match)> {
        let from = (name.to_owned(), i64::MIN, false);
        let same = self
            .live
            .range(from..)
            .map(|(k, _)| k)
            .take_while(|k| k.0 == name)
            // A model before a derived name, then the user's own service, then the lowest id.
            .min_by_key(|k| (k.2, k.1 != service, k.1));
        if let Some(k) = same {
            return Some((
                k,
                if k.2 {
                    Match::Convention
                } else {
                    Match::Defined
                },
            ));
        }
        // A name alone: only when every definition it fits is one table of one service.
        let near = self.loose.get(&loose(name))?;
        let first = near.first()?;
        near.iter()
            .all(|k| k.0 == first.0 && k.1 == first.1)
            .then_some((first, Match::NameOnly))
    }
}

fn end(s: &Span) -> i64 {
    s.1.unwrap_or(i64::MAX)
}

/// The spans as the fewest, in order: overlapping and touching ones joined.
pub fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|s| s.1.is_none_or(|t| t > s.0));
    spans.sort_by_key(|s| (s.0, end(s)));
    let mut out: Vec<Span> = Vec::new();
    for s in spans {
        match out.last_mut() {
            Some(l) if end(l) >= s.0 => {
                if end(&s) > end(l) {
                    l.1 = s.1;
                }
            }
            _ => out.push(s),
        }
    }
    out
}

/// The steps both lists have; both are merged.
pub fn intersect(a: &[Span], b: &[Span]) -> Vec<Span> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let (from, to) = (a[i].0.max(b[j].0), end(&a[i]).min(end(&b[j])));
        if from < to {
            out.push((from, (to != i64::MAX).then_some(to)));
        }
        if end(&a[i]) < end(&b[j]) {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}
