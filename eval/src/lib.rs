//! XS-07: how many of the links the indexer draws are right, and how many of the right ones it
//! draws. Dev only: nothing here ships.
//!
//! Set (a) is a small workspace of checked-in files under `fixtures/set-a/workspace`, committed
//! into a throwaway git repository, indexed with the real pipeline, and compared with the links a
//! person labelled in `fixtures/set-a/expected.json`. A link is the triple (calling service,
//! called service, operation), so two calls to one route count once.
//!
//! The index stores one link per pair of files and the confidence of that pair, so every
//! operation a pair carries takes the pair's level. Each operation is read off the route-site
//! evidence of the link.
//!
//! Recall by level is that level's true positives over all labelled links, so the rows add up to
//! the overall recall; a link the indexer misses has no level to be filed under.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// The CI gate: exact links must be right at least this often, and likely ones.
pub const GATE_EXACT: f64 = 0.95;
pub const GATE_LIKELY: f64 = 0.75;

const LEVELS: [&str; 4] = ["exact", "likely", "guessed", "weak"];
const EVIDENCE: [&str; 3] = ["literal", "template", "inferred"];
const SERVICES: [&str; 7] = [
    "gateway",
    "orders",
    "users",
    "payments",
    "inventory",
    "notifier",
    "web",
];

/// One labelled link.
#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub from: String,
    pub to: String,
    pub kind: String,
    /// `GET /orders/:id`.
    pub operation: String,
    /// Why the indexer is expected to miss it, for the report.
    #[serde(default)]
    pub why: Option<String>,
}

/// What a link is called when compared: `from -> to http get /orders/{}`.
fn identity(from: &str, to: &str, kind: &str, operation: &str) -> String {
    format!("{from} -> {to} {kind} {}", normalise(operation))
}

/// Lower-case method, and every `:id` or `{id}` as `{}`.
fn normalise(operation: &str) -> String {
    let param = Regex::new(r":[A-Za-z_][A-Za-z0-9_]*|\{[A-Za-z_][A-Za-z0-9_]*\}").unwrap();
    let (method, path) = operation.split_once(' ').unwrap_or(("", operation));
    format!(
        "{} {}",
        method.to_lowercase(),
        param.replace_all(path, "{}")
    )
}

/// A link the indexer drew, for one operation.
#[derive(Debug, Clone)]
struct Predicted {
    id: String,
    level: String,
    /// The call sites' evidence kinds on the link.
    evidence: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub name: String,
    pub predicted: usize,
    pub true_positive: usize,
    /// `None` when nothing was predicted: there is no ratio to report.
    pub precision: Option<f64>,
    pub recall: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub set: String,
    pub labelled: usize,
    pub by_level: Vec<Row>,
    pub by_evidence: Vec<Row>,
    pub overall: Row,
    /// Labelled links the indexer did not draw, with the reason when the label gives one.
    pub missed: Vec<String>,
    /// Links the indexer drew that nobody labelled, with their level.
    pub false_positives: Vec<String>,
}

fn row(name: &str, predicted: &[&Predicted], truth: &BTreeSet<String>, labelled: usize) -> Row {
    let tp = predicted.iter().filter(|p| truth.contains(&p.id)).count();
    Row {
        name: name.to_owned(),
        predicted: predicted.len(),
        true_positive: tp,
        precision: (!predicted.is_empty()).then(|| tp as f64 / predicted.len() as f64),
        recall: tp as f64 / labelled as f64,
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/set-a")
}

fn copy_into(f: &Fixture, root: &Path, dir: &Path) {
    for e in std::fs::read_dir(dir).expect("read fixture dir") {
        let p = e.expect("entry").path();
        if p.is_dir() {
            copy_into(f, root, &p);
        } else {
            let rel = p.strip_prefix(root).expect("under root");
            f.write(
                &rel.to_string_lossy().replace('\\', "/"),
                &std::fs::read_to_string(&p).expect("fixture is utf-8"),
            );
        }
    }
}

/// Indexes set (a)'s workspace and returns the links it drew, one per operation.
fn predict() -> Vec<Predicted> {
    let d = tempfile::tempdir().expect("tempdir");
    let mut f = Fixture::init(&d.path().join("repo"));
    let root = fixtures().join("workspace");
    copy_into(&f, &root, &root);
    f.commit("set a");
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: SERVICES
            .iter()
            .map(|s| ServiceSpec {
                name: (*s).into(),
                repo: "repo".into(),
                path: Some((*s).into()),
            })
            .collect(),
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).expect("fact cache");
    match sync(&w, &SyncOptions::default(), &mut cache).expect("sync") {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let ix = Index::open_read(&w.index_path()).expect("index");
    // One row per piece of evidence on each live link.
    let mut st = ix
        .conn
        .prepare(
            "SELECT l.id, l.level, ss.name, ds.name, e.file = nd.file, e.kind, e.text
             FROM link l
               JOIN node ns ON ns.id = l.src JOIN node nd ON nd.id = l.dst
               JOIN node ss ON ss.id = ns.service JOIN node ds ON ds.id = nd.service
               JOIN evidence e ON e.link = l.id
             WHERE l.kind = 'http'
               AND EXISTS (SELECT 1 FROM interval i WHERE i.link = l.id AND i.to_seq IS NULL)
             ORDER BY l.id",
        )
        .expect("prepare");
    type Ev = (String, String, String, bool, String, String);
    let rows: Vec<Ev> = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?.to_string(),
                r.get(1)?,
                format!("{} -> {}", r.get::<_, String>(2)?, r.get::<_, String>(3)?),
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    let route =
        Regex::new(r#"\.\s*(get|post|put|patch|delete|head|options|all)\(\s*["'`]([^"'`]+)["'`]"#)
            .unwrap();
    // link -> (level, edge, call kinds, operations)
    type Link = (String, String, BTreeSet<String>, BTreeSet<String>);
    let mut links: BTreeMap<String, Link> = BTreeMap::new();
    for (link, level, edge, is_route, kind, text) in rows {
        let l = links
            .entry(link)
            .or_insert((level, edge, BTreeSet::new(), BTreeSet::new()));
        if !is_route {
            l.2.insert(kind);
        } else if let Some(c) = route.captures(&text) {
            l.3.insert(format!("{} {}", c[1].to_uppercase(), &c[2]));
        }
    }
    links
        .into_values()
        .flat_map(|(level, edge, evidence, ops)| {
            let (from, to) = edge.split_once(" -> ").expect("edge").to_owned_pair();
            ops.into_iter().map(move |op| Predicted {
                id: identity(&from, &to, "http", &op),
                level: level.clone(),
                evidence: evidence.clone(),
            })
        })
        .collect()
}

trait OwnedPair {
    fn to_owned_pair(self) -> (String, String);
}
impl OwnedPair for (&str, &str) {
    fn to_owned_pair(self) -> (String, String) {
        (self.0.to_owned(), self.1.to_owned())
    }
}

/// Runs set (a).
pub fn evaluate_set_a() -> Report {
    let labels: Vec<Label> = serde_json::from_str(
        &std::fs::read_to_string(fixtures().join("expected.json")).expect("expected.json"),
    )
    .expect("labels parse");
    let truth: BTreeMap<String, &Label> = labels
        .iter()
        .map(|l| (identity(&l.from, &l.to, &l.kind, &l.operation), l))
        .collect();
    let keys: BTreeSet<String> = truth.keys().cloned().collect();
    let predicted = predict();
    let n = keys.len();

    let by_level = LEVELS
        .iter()
        .map(|lv| {
            let ps: Vec<_> = predicted.iter().filter(|p| p.level == *lv).collect();
            row(lv, &ps, &keys, n)
        })
        .collect();
    let by_evidence = EVIDENCE
        .iter()
        .map(|ev| {
            let ps: Vec<_> = predicted
                .iter()
                .filter(|p| p.evidence.contains(*ev))
                .collect();
            row(ev, &ps, &keys, n)
        })
        .collect();
    let all: Vec<_> = predicted.iter().collect();
    let found: BTreeSet<&String> = predicted.iter().map(|p| &p.id).collect();
    Report {
        set: "a".into(),
        labelled: n,
        by_level,
        by_evidence,
        overall: row("all", &all, &keys, n),
        missed: truth
            .iter()
            .filter(|(k, _)| !found.contains(k))
            .map(|(k, l)| match &l.why {
                Some(w) => format!("{k}  ({w})"),
                None => k.clone(),
            })
            .collect(),
        false_positives: predicted
            .iter()
            .filter(|p| !keys.contains(&p.id))
            .map(|p| format!("{} [{}]", p.id, p.level))
            .collect(),
    }
}

/// The gate's failures: a level below its floor. A level with nothing predicted is not checked.
pub fn gate(r: &Report) -> Vec<String> {
    [("exact", GATE_EXACT), ("likely", GATE_LIKELY)]
        .into_iter()
        .filter_map(|(level, floor)| {
            let p = r.by_level.iter().find(|x| x.name == level)?.precision?;
            (p < floor).then(|| format!("{level} precision {p:.2} is below {floor:.2}"))
        })
        .collect()
}

fn pct(x: Option<f64>) -> String {
    x.map_or("n/a".into(), |v| format!("{v:.2}"))
}

fn table(title: &str, rows: &[Row]) -> String {
    let mut s = format!(
        "| {title} | Predicted | True positives | Precision | Recall |\n|---|---:|---:|---:|---:|\n"
    );
    for r in rows {
        s += &format!(
            "| {} | {} | {} | {} | {:.2} |\n",
            r.name,
            r.predicted,
            r.true_positive,
            pct(r.precision),
            r.recall
        );
    }
    s
}

/// The tables and lists, as Markdown.
pub fn render(r: &Report) -> String {
    let mut s = format!(
        "{}\n{}\n**{} labelled links.** Gate: exact precision at least {GATE_EXACT}, likely at least {GATE_LIKELY}; a level with nothing predicted is n/a and passes.\n\n",
        table("Level", &r.by_level),
        table("Evidence", &r.by_evidence),
        r.labelled
    );
    s += &format!("Overall: {}\n\n", {
        let o = &r.overall;
        format!(
            "{} predicted, {} true, precision {}, recall {:.2}.",
            o.predicted,
            o.true_positive,
            pct(o.precision),
            o.recall
        )
    });
    s += "Missed:\n\n";
    s += &list(&r.missed);
    s += "\nFalse positives:\n\n";
    s += &list(&r.false_positives);
    s
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        return "- none\n".into();
    }
    items.iter().map(|i| format!("- {i}\n")).collect()
}

/// The page at `docs/precision.md`.
pub fn page(r: &Report) -> String {
    format!(
        "# Link precision (XS-07)\n\n\
Run `cargo run -p cartograph-eval` to print this table; add `--write` to rewrite this page and `precision.json`. The gate runs as a test, so `cargo test -p cartograph-eval` fails when it is not met.\n\n\
## Set (a): the hand-labelled TS, Go and Python workspace\n\n\
Seven services under `eval/fixtures/set-a/workspace`, committed into a throwaway repository and indexed by the real pipeline, compared with `eval/fixtures/set-a/expected.json`. Only the TypeScript adapters (Express routes; fetch, axios, got and ky calls) exist today, so the Go and Python services are in the labels to keep recall honest: every link into or out of them is a miss. The fixture also holds traps: hostless calls, one route in three services, a pattern route, an external host and an unknown host. A link's level is the pair of files' level, so an operation takes its file pair's. Recall by level or evidence is that row's true positives over all labelled links.\n\n\
{}\n\
## Set (b): the 17 Code2DFD applications\n\n\
Pending. It needs Java and Spring adapters, which do not exist yet.\n",
        render(r)
    )
}

/// Writes `docs/precision.md` and `docs/precision.json` under the repository root.
pub fn write_docs(r: &Report) -> std::io::Result<()> {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs");
    std::fs::write(docs.join("precision.md"), page(r))?;
    std::fs::write(
        docs.join("precision.json"),
        serde_json::to_string_pretty(r).expect("serialises") + "\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_a_meets_the_gate() {
        let r = evaluate_set_a();
        let failures = gate(&r);
        assert!(failures.is_empty(), "{failures:?}\n{}", render(&r));
    }

    #[test]
    fn a_level_with_nothing_predicted_is_not_gated() {
        let mut r = evaluate_set_a();
        for x in &mut r.by_level {
            x.predicted = 0;
            x.precision = None;
        }
        assert!(gate(&r).is_empty());
    }

    #[test]
    fn operations_compare_by_shape() {
        assert_eq!(normalise("GET /orders/:id"), normalise("get /orders/{oid}"));
    }
}
