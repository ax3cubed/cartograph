//! Architecture rules against the index (XS-22, H6).
//!
//! A violation is a link, live or past, that a rule's selectors match at or above the rule's
//! `min_confidence` (`likely` or `exact`; a link kind that carries no confidence, imports, counts as
//! exact; a guessed or weak link is never a violation). It carries the link's intervals, so whether
//! a commit passes is a lookup. `check` and the API read the live ones.
//!
//! Each rule's service selectors are resolved once to a set of service ids; one pass over the
//! `link` table then tests every rule per link by set lookups, and only a link that passes those
//! reads a path. With no rules nothing here touches the index. A violation is keyed, as the baseline
//! keys it, by names: rule, link kind, `service:path` of each end, and the operation.
//!
//! Tags: a service's `tags` in the manifest, plus `owner:<owner>` for each owner of one of its live
//! files. `domain:` and `lang:` aren't derived (the spec leaves them open), so a rule naming one
//! matches nothing.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use cartograph_discovery::manifest;
use cartograph_discovery::rules::{self as spec, BASELINE_FILE, Baseline, Entry, Rule, RuleType};
use cartograph_store::rusqlite::{Connection, Result};
use regex::Regex;

/// What failed, as a `what`, a `why` and an `action`.
pub type Failure = (String, String, String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub rule: String,
    pub kind: String,
    pub from: String,
    pub to: String,
    pub op: String,
    pub level: Option<String>,
    /// The link's intervals as (from_seq, to_seq); `to_seq` None while it holds.
    pub intervals: Vec<(i64, Option<i64>)>,
    pub live: bool,
}

impl Violation {
    pub fn key(&self) -> Entry {
        Entry {
            rule: self.rule.clone(),
            kind: self.kind.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            op: self.op.clone(),
        }
    }
}

struct Row {
    id: i64,
    kind: String,
    level: Option<String>,
    src: i64,
    dst: i64,
    src_service: Option<i64>,
    dst_service: Option<i64>,
    live: bool,
}

/// A rule with its selectors resolved.
struct Resolved<'a> {
    rule: &'a Rule,
    from: Option<HashSet<i64>>,
    to: Option<HashSet<i64>>,
    from_path: Option<Regex>,
    to_path: Option<String>,
    /// The `to` regex when it has no `$N`.
    to_fixed: Option<Regex>,
    kinds: Vec<&'static str>,
    exact_only: bool,
    broken: bool,
}

impl Resolved<'_> {
    fn admits(&self, r: &Row) -> bool {
        (self.kinds.is_empty() || self.kinds.contains(&r.kind.as_str()))
            && match r.level.as_deref() {
                None | Some("exact") => true,
                Some("likely") => !self.exact_only,
                _ => false,
            }
    }
}

fn service_names(conn: &Connection) -> Result<HashMap<i64, String>> {
    let mut st =
        conn.prepare("SELECT id, name, key FROM node WHERE kind IN ('service', 'library')")?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, name, key) = row?;
        // `service:<repo>:<root>`, as IDX-20 writes it.
        let mut parts = key.as_deref().unwrap_or_default().splitn(3, ':').skip(1);
        let repo = parts.next().unwrap_or_default();
        let root = parts.next().unwrap_or_default();
        out.insert(
            id,
            name.unwrap_or_else(|| if root.is_empty() { repo } else { root }.to_owned()),
        );
    }
    Ok(out)
}

fn owner_tags(conn: &Connection) -> Result<HashMap<i64, BTreeSet<String>>> {
    let mut st = conn.prepare(
        "SELECT DISTINCT n.service, o.owner FROM ownership o JOIN node n ON n.file = o.file
         WHERE o.to_seq IS NULL AND n.service IS NOT NULL",
    )?;
    let mut out: HashMap<i64, BTreeSet<String>> = HashMap::new();
    for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (s, o) = row?;
        out.entry(s).or_default().insert(format!("owner:{o}"));
    }
    Ok(out)
}

/// The path a node shows: a file's newest repo path, else its name (a table's).
struct Paths<'c> {
    conn: &'c Connection,
    names: HashMap<i64, String>,
    seen: HashMap<i64, String>,
}

impl Paths<'_> {
    fn of(&mut self, node: i64) -> Result<&str> {
        if !self.seen.contains_key(&node) {
            let p = self
                .conn
                .prepare_cached(
                    "SELECT CASE WHEN n.kind = 'file'
                        THEN (SELECT path FROM file_path WHERE file = n.file
                              ORDER BY from_seq DESC LIMIT 1)
                        ELSE n.name END, n.service FROM node n WHERE n.id = ?1",
                )?
                .query_row([node], |r| {
                    Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?))
                })?;
            let svc =
                p.1.and_then(|s| self.names.get(&s))
                    .map_or("", String::as_str);
            let path = p.0.unwrap_or_default();
            let end = if svc.is_empty() {
                path
            } else {
                format!("{svc}:{path}")
            };
            self.seen.insert(node, end);
        }
        Ok(&self.seen[&node])
    }

    /// The path alone, for a regex.
    fn bare(&mut self, node: i64) -> Result<String> {
        let end = self.of(node)?;
        Ok(end.split_once(':').map_or(end, |x| x.1).to_owned())
    }
}

fn resolve<'a>(
    rule: &'a Rule,
    names: &HashMap<i64, String>,
    tags: &HashMap<i64, BTreeSet<String>>,
) -> Resolved<'a> {
    let set = |s: &spec::Selector| {
        (!s.tags.is_empty() || s.service.is_some()).then(|| {
            names
                .iter()
                .filter(|(id, name)| {
                    s.service.as_ref().is_none_or(|w| w == *name)
                        && s.tags
                            .iter()
                            .all(|t| tags.get(id).is_some_and(|have| have.contains(t)))
                })
                .map(|(id, _)| *id)
                .collect()
        })
    };
    // Validated when the manifest was read; a pattern that no longer compiles matches nothing.
    let from_path = rule.from.path.as_deref().map(|p| spec::compile(p).ok());
    let to_fixed = rule
        .to
        .path
        .as_deref()
        .filter(|p| spec::refs(p) == 0)
        .map(|p| spec::compile(p).ok());
    Resolved {
        rule,
        from: set(&rule.from),
        to: set(&rule.to),
        broken: matches!(from_path, Some(None)) || matches!(to_fixed, Some(None)),
        from_path: from_path.flatten(),
        to_path: rule.to.path.clone().filter(|p| spec::refs(p) > 0),
        to_fixed: to_fixed.flatten(),
        kinds: rule.link.iter().map(|k| k.as_str()).collect(),
        exact_only: rule.min_confidence == spec::MinConfidence::Exact,
    }
}

/// Every violation of `rules`, past and live. `tags` is each manifest service's tags by name.
pub fn evaluate(
    conn: &Connection,
    rules: &[Rule],
    tags: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<Violation>> {
    if rules.is_empty() {
        return Ok(Vec::new());
    }
    let names = service_names(conn)?;
    let mut have: HashMap<i64, BTreeSet<String>> = if rules.iter().any(|r| {
        [&r.from, &r.to]
            .iter()
            .any(|s| s.tags.iter().any(|t| t.starts_with("owner:")))
    }) {
        owner_tags(conn)?
    } else {
        HashMap::new()
    };
    for (id, name) in &names {
        if let Some(t) = tags.get(name) {
            have.entry(*id).or_default().extend(t.iter().cloned());
        }
    }
    let resolved: Vec<Resolved<'_>> = rules.iter().map(|r| resolve(r, &names, &have)).collect();

    let mut st = conn.prepare(
        "SELECT l.id, l.kind, l.level, l.src, l.dst, s.service, d.service,
                EXISTS (SELECT 1 FROM interval i WHERE i.link = l.id AND i.to_seq IS NULL)
         FROM link l JOIN node s ON s.id = l.src JOIN node d ON d.id = l.dst",
    )?;
    let links: Vec<Row> = st
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                kind: r.get(1)?,
                level: r.get(2)?,
                src: r.get(3)?,
                dst: r.get(4)?,
                src_service: r.get(5)?,
                dst_service: r.get(6)?,
                live: r.get(7)?,
            })
        })?
        .collect::<Result<_>>()?;

    let mut paths = Paths {
        conn,
        names,
        seen: HashMap::new(),
    };
    let mut out = Vec::new();
    for rr in &resolved {
        match rr.rule.kind {
            RuleType::Forbid => forbid(conn, rr, &links, &mut paths, &mut out)?,
            RuleType::NoCycles => no_cycles(rr, &links, &mut paths, &mut out)?,
        }
    }
    Ok(out)
}

fn holds(set: &Option<HashSet<i64>>, service: Option<i64>) -> bool {
    set.as_ref()
        .is_none_or(|s| service.is_some_and(|x| s.contains(&x)))
}

fn violation(
    conn: &Connection,
    rr: &Resolved<'_>,
    r: &Row,
    paths: &mut Paths<'_>,
) -> Result<Violation> {
    let op: Option<String> = conn
        .prepare_cached("SELECT MIN(operation) FROM evidence WHERE link = ?1")?
        .query_row([r.id], |x| x.get(0))?;
    let intervals = conn
        .prepare_cached("SELECT from_seq, to_seq FROM interval WHERE link = ?1 ORDER BY from_seq")?
        .query_map([r.id], |x| Ok((x.get(0)?, x.get(1)?)))?
        .collect::<Result<_>>()?;
    Ok(Violation {
        rule: rr.rule.name.clone(),
        kind: r.kind.clone(),
        from: paths.of(r.src)?.to_owned(),
        to: paths.of(r.dst)?.to_owned(),
        op: op.unwrap_or_default(),
        level: r.level.clone(),
        intervals,
        live: r.live,
    })
}

fn forbid(
    conn: &Connection,
    rr: &Resolved<'_>,
    links: &[Row],
    paths: &mut Paths<'_>,
    out: &mut Vec<Violation>,
) -> Result<()> {
    let mut filled: HashMap<String, Option<Regex>> = HashMap::new();
    for r in links {
        if rr.broken
            || !rr.admits(r)
            || !holds(&rr.from, r.src_service)
            || !holds(&rr.to, r.dst_service)
        {
            continue;
        }
        let mut caps = None;
        if let Some(re) = &rr.from_path {
            let from = paths.bare(r.src)?;
            match re.captures(&from) {
                Some(c) => caps = Some(spec::groups(&c)),
                None => continue,
            }
        } else if rr.rule.from.path.is_some() {
            continue;
        }
        if let Some(re) = &rr.to_fixed {
            if !re.is_match(&paths.bare(r.dst)?) {
                continue;
            }
        } else if let Some(t) = &rr.to_path {
            let pattern = spec::fill(t, caps.as_deref().unwrap_or(&[]));
            let re = filled
                .entry(pattern.clone())
                .or_insert_with(|| spec::compile(&pattern).ok());
            if !re
                .as_ref()
                .is_some_and(|re| re.is_match(&paths.bare(r.dst).unwrap_or_default()))
            {
                continue;
            }
        } else if rr.rule.to.path.is_some() {
            continue;
        }
        out.push(violation(conn, rr, r, paths)?);
    }
    Ok(())
}

/// The links inside a strongly connected component of the services `from` selects.
fn no_cycles(
    rr: &Resolved<'_>,
    links: &[Row],
    paths: &mut Paths<'_>,
    out: &mut Vec<Violation>,
) -> Result<()> {
    let edges: Vec<(i64, i64, &Row)> = links
        .iter()
        .filter(|r| r.live && rr.admits(r))
        .filter_map(|r| Some((r.src_service?, r.dst_service?, r)))
        .filter(|(a, b, _)| a != b && holds(&rr.from, Some(*a)) && holds(&rr.from, Some(*b)))
        .collect();
    let mut adj: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for (a, b, _) in &edges {
        adj.entry(*a).or_default().push(*b);
        adj.entry(*b).or_default();
    }
    let comp = scc(&adj);
    let mut seen = BTreeSet::new();
    for (a, b, r) in edges {
        if comp[&a] != comp[&b] {
            continue;
        }
        let name = |s: i64| paths.names.get(&s).cloned().unwrap_or_default();
        let v = Violation {
            rule: rr.rule.name.clone(),
            kind: r.kind.clone(),
            from: name(a),
            to: name(b),
            op: String::new(),
            level: r.level.clone(),
            intervals: vec![],
            live: true,
        };
        if seen.insert(v.key()) {
            out.push(v);
        }
    }
    Ok(())
}

/// Tarjan's components: node to component number.
fn scc(adj: &BTreeMap<i64, Vec<i64>>) -> HashMap<i64, usize> {
    struct S<'a> {
        adj: &'a BTreeMap<i64, Vec<i64>>,
        index: HashMap<i64, usize>,
        low: HashMap<i64, usize>,
        stack: Vec<i64>,
        on: HashSet<i64>,
        comp: HashMap<i64, usize>,
        next: usize,
        n: usize,
    }
    fn go(s: &mut S<'_>, v: i64) {
        s.index.insert(v, s.next);
        s.low.insert(v, s.next);
        s.next += 1;
        s.stack.push(v);
        s.on.insert(v);
        for &w in &s.adj[&v] {
            if !s.index.contains_key(&w) {
                go(s, w);
                let l = s.low[&v].min(s.low[&w]);
                s.low.insert(v, l);
            } else if s.on.contains(&w) {
                let l = s.low[&v].min(s.index[&w]);
                s.low.insert(v, l);
            }
        }
        if s.low[&v] == s.index[&v] {
            while let Some(w) = s.stack.pop() {
                s.on.remove(&w);
                s.comp.insert(w, s.n);
                if w == v {
                    break;
                }
            }
            s.n += 1;
        }
    }
    let mut s = S {
        adj,
        index: HashMap::new(),
        low: HashMap::new(),
        stack: Vec::new(),
        on: HashSet::new(),
        comp: HashMap::new(),
        next: 0,
        n: 0,
    };
    for &v in adj.keys() {
        if !s.index.contains_key(&v) {
            go(&mut s, v);
        }
    }
    s.comp
}

/// A rule with its counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleReport {
    pub rule: Rule,
    /// Live violations, baselined ones included.
    pub violations: usize,
    pub baselined: usize,
}

#[derive(Debug, Clone)]
pub struct Found {
    pub violation: Violation,
    pub severity: spec::Severity,
    pub baselined: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub rules: Vec<RuleReport>,
    /// Every violation, past ones included; `live` says which hold now.
    pub found: Vec<Found>,
    /// Baseline entries that match no live violation any more.
    pub fixed: Vec<Entry>,
    /// Where the baseline is (or goes).
    pub baseline_path: std::path::PathBuf,
}

impl Report {
    /// The live violations the baseline doesn't cover: what `check` fails or warns on.
    pub fn new_ones(&self) -> impl Iterator<Item = &Found> {
        self.found
            .iter()
            .filter(|f| f.violation.live && !f.baselined)
    }
}

fn failure(what: impl Into<String>, why: impl Into<String>, action: &str) -> Failure {
    (what.into(), why.into(), action.into())
}

/// Reads the manifest's rules and the baseline beside it and evaluates them against the index at
/// `index`, which a manifest with no rules never opens.
pub fn report(manifest_file: &Path, index: &Path) -> std::result::Result<Report, Failure> {
    let parsed = manifest::read(manifest_file).map_err(|e| {
        failure(
            "the manifest doesn't read",
            e.to_string(),
            "fix cartograph.yaml",
        )
    })?;
    let m = parsed.manifest;
    let base = manifest_file.parent().unwrap_or(Path::new("."));
    let baseline_path = base.join(m.baseline.as_deref().unwrap_or(BASELINE_FILE));
    if m.rules.is_empty() {
        return Ok(Report {
            baseline_path,
            ..Report::default()
        });
    }
    let baseline = spec::read_baseline(&baseline_path)
        .map_err(|e| {
            failure(
                format!("{} doesn't read", baseline_path.display()),
                e,
                "fix the file, or delete it and run `cartograph check --update-baseline`",
            )
        })?
        .unwrap_or_else(|| Baseline::new(vec![]));
    let tags: BTreeMap<String, Vec<String>> = m
        .services
        .iter()
        .map(|s| (s.name.clone(), s.tags.clone()))
        .collect();
    let ix = cartograph_store::Index::open_read(index).map_err(|e| {
        failure(
            "can't open the index",
            e.to_string(),
            "run `cartograph sync`",
        )
    })?;
    let violations = evaluate(&ix.conn, &m.rules, &tags).map_err(|e| {
        failure(
            "can't read the index",
            e.to_string(),
            "run `cartograph sync`",
        )
    })?;
    let known: HashSet<&Entry> = baseline.violations.iter().collect();
    let severity: HashMap<&str, spec::Severity> = m
        .rules
        .iter()
        .map(|r| (r.name.as_str(), r.severity))
        .collect();
    let mut live: HashSet<Entry> = HashSet::new();
    let found: Vec<Found> = violations
        .into_iter()
        .map(|v| {
            let key = v.key();
            let baselined = known.contains(&key);
            if v.live {
                live.insert(key);
            }
            Found {
                severity: severity[v.rule.as_str()],
                violation: v,
                baselined,
            }
        })
        .collect();
    let fixed = baseline
        .violations
        .iter()
        .filter(|e| !live.contains(*e))
        .cloned()
        .collect();
    let rules = m
        .rules
        .into_iter()
        .map(|rule| {
            let mine = || {
                found
                    .iter()
                    .filter(|f| f.violation.live && f.violation.rule == rule.name)
            };
            RuleReport {
                violations: mine().count(),
                baselined: mine().filter(|f| f.baselined).count(),
                rule,
            }
        })
        .collect();
    Ok(Report {
        rules,
        found,
        fixed,
        baseline_path,
    })
}

/// The baseline that covers every live violation now.
pub fn current_baseline(r: &Report) -> Baseline {
    Baseline::new(
        r.found
            .iter()
            .filter(|f| f.violation.live)
            .map(|f| f.violation.key())
            .collect(),
    )
}
