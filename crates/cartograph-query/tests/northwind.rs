//! SRV-08, SRV-10 and SRV-11 golden: the query functions against the `lib/workspace.ts` and
//! `lib/ask.ts` helpers on the northwind data loaded as a fixture index.
//!
//! The oracle below is those helpers ported line for line (a loop over every link, `descendants`
//! by walking parents), run over the demo links the fixture turns into imports. It is compared
//! with the query layer at every commit, for every service, module and file.

use std::collections::{BTreeMap, BTreeSet};

use cartograph_api::types::{Bin, Change, GraphParams, PathDirection};
use cartograph_fixtures::northwind::{Demo, demo, noon};
use cartograph_query::{At, GraphQuery, ImpactInput, Limits, Page, Snapshot, Store};

const DAY: i64 = 86_400;

/// A demo link as the index holds it: the generator imports TypeScript-to-TypeScript `imports`,
/// `calls` and `event` links; one row per (from, to) interval.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct L {
    from: String,
    to: String,
    added: usize,
    removed: Option<usize>,
}

struct Oracle {
    d: Demo,
    links: Vec<L>,
    /// Demo node id -> parent demo id; services have none.
    parent: BTreeMap<String, String>,
    service: BTreeMap<String, String>,
    path: BTreeMap<String, String>,
}

impl Oracle {
    fn new() -> Self {
        let d = demo();
        let order: BTreeMap<&str, usize> = d
            .commits
            .iter()
            .enumerate()
            .map(|(i, c)| (c.sha.as_str(), i))
            .collect();
        let mut links: BTreeSet<L> = BTreeSet::new();
        for l in &d.links {
            if l.from.ends_with(".ts")
                && l.to.ends_with(".ts")
                && matches!(l.kind.as_str(), "imports" | "calls" | "event")
            {
                links.insert(L {
                    from: l.from.clone(),
                    to: l.to.clone(),
                    added: order[l.added.as_str()],
                    removed: l.removed.as_deref().map(|r| order[r]),
                });
            }
        }
        let name = |s: &str| d.services.iter().find(|x| x.id == s).unwrap().name.clone();
        let (mut parent, mut service, mut path) =
            (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        for (s, m, f) in &d.files {
            let module = format!("{s}/{m}");
            let file = format!("{module}/{f}");
            parent.insert(file.clone(), module.clone());
            parent.insert(module.clone(), s.clone());
            service.insert(file.clone(), s.clone());
            service.insert(module.clone(), s.clone());
            service.insert(s.clone(), s.clone());
            path.insert(file, format!("{}/src/{m}/{f}", name(s)));
            path.insert(module, format!("{}/src/{m}", name(s)));
            path.insert(s.clone(), name(s));
        }
        Self {
            links: links.into_iter().collect(),
            d,
            parent,
            service,
            path,
        }
    }

    fn time(&self, i: usize) -> i64 {
        noon(&self.d.commits[i].date)
    }

    fn active(&self, l: &L, t: i64) -> bool {
        self.time(l.added) <= t && l.removed.is_none_or(|r| self.time(r) > t)
    }

    fn descendants(&self, id: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::from([id.to_owned()]);
        for x in self.parent.keys() {
            let mut p = Some(x.as_str());
            while let Some(q) = p {
                if q == id {
                    out.insert(x.clone());
                    break;
                }
                p = self.parent.get(q).map(String::as_str);
            }
        }
        out
    }

    /// `lib/workspace.ts` `metrics`: links, fan_in, fan_out, cross, churn.
    fn metrics(&self, scope: &str, t: i64) -> [u32; 5] {
        let s = self.descendants(scope);
        let mut m = [0u32; 5];
        for l in &self.links {
            let (fi, ti) = (s.contains(&l.from), s.contains(&l.to));
            if !fi && !ti {
                continue;
            }
            let ends = [Some(self.time(l.added)), l.removed.map(|r| self.time(r))];
            m[4] += ends
                .iter()
                .flatten()
                .filter(|&&d| d <= t && t - d < 90 * DAY)
                .count() as u32;
            if !self.active(l, t) {
                continue;
            }
            m[0] += 1;
            m[1] += u32::from(ti && !fi);
            m[2] += u32::from(fi && !ti);
            m[3] += u32::from(self.service[&l.from] != self.service[&l.to]);
        }
        m
    }

    /// `neighbours`: live links crossing the boundary, as (from path, to path).
    fn neighbours(&self, scope: &str, t: i64) -> BTreeSet<(String, String)> {
        let s = self.descendants(scope);
        self.links
            .iter()
            .filter(|l| self.active(l, t) && (s.contains(&l.from) != s.contains(&l.to)))
            .map(|l| (self.path[&l.from].clone(), self.path[&l.to].clone()))
            .collect()
    }

    /// `changesFor`: (commit index, from, to, added?) for links touching the scope.
    fn changes(&self, scope: &str, t: i64) -> BTreeSet<(usize, String, String, bool)> {
        let s = self.descendants(scope);
        let mut out = BTreeSet::new();
        for l in &self.links {
            if !s.contains(&l.from) && !s.contains(&l.to) {
                continue;
            }
            let (f, to) = (self.path[&l.from].clone(), self.path[&l.to].clone());
            if self.time(l.added) <= t {
                out.insert((l.added, f.clone(), to.clone(), true));
            }
            if let Some(r) = l.removed.filter(|&r| self.time(r) <= t) {
                out.insert((r, f, to, false));
            }
        }
        out
    }

    /// `ranking`: (path, used_by, uses, delta) for files in scope with any live link.
    fn ranking(&self, scope: &str, t: i64) -> BTreeSet<(String, u32, u32, i32)> {
        let s = self.descendants(scope);
        let ago = t - 90 * DAY;
        let files: BTreeSet<&String> = self.path.keys().filter(|k| k.ends_with(".ts")).collect();
        files
            .into_iter()
            .filter(|f| s.contains(*f))
            .filter_map(|f| {
                let count = |pred: &dyn Fn(&L) -> bool, at| {
                    self.links
                        .iter()
                        .filter(|l| pred(l) && self.active(l, at))
                        .count() as u32
                };
                let used_by = count(&|l| &l.to == f, t);
                let uses = count(&|l| &l.from == f, t);
                let then = count(&|l| &l.to == f, ago);
                (used_by + uses > 0).then(|| {
                    (
                        self.path[f].clone(),
                        used_by,
                        uses,
                        used_by as i32 - then as i32,
                    )
                })
            })
            .collect()
    }

    /// `lib/ask.ts` `impact`: every node depending on the scope, as paths.
    fn impact(&self, scope: &str, t: i64) -> BTreeSet<String> {
        let start = self.descendants(scope);
        let mut seen = start.clone();
        let mut frontier = start;
        let mut out = BTreeSet::new();
        while !frontier.is_empty() {
            let mut next = BTreeSet::new();
            for l in &self.links {
                if self.active(l, t) && frontier.contains(&l.to) && !seen.contains(&l.from) {
                    next.insert(l.from.clone());
                }
            }
            for n in &next {
                seen.insert(n.clone());
                out.insert(self.path[n].clone());
            }
            frontier = next;
        }
        out
    }

    /// `lib/ask.ts` `path`: the hop count and direction of the shortest chain, if any.
    fn path(&self, a: &str, b: &str, t: i64) -> Option<(usize, PathDirection)> {
        let (from, to) = (self.descendants(a), self.descendants(b));
        let live: Vec<&L> = self.links.iter().filter(|l| self.active(l, t)).collect();
        let bfs = |dir: PathDirection| -> Option<usize> {
            let mut dist: BTreeMap<String, usize> = from.iter().map(|n| (n.clone(), 0)).collect();
            let mut queue: std::collections::VecDeque<String> = from.iter().cloned().collect();
            while let Some(n) = queue.pop_front() {
                if to.contains(&n) {
                    return Some(dist[&n]);
                }
                for l in &live {
                    let next = match dir {
                        PathDirection::Forward => (l.from == n).then(|| l.to.clone()),
                        PathDirection::Backward => (l.to == n).then(|| l.from.clone()),
                        PathDirection::Mixed => {
                            if l.from == n {
                                Some(l.to.clone())
                            } else if l.to == n {
                                Some(l.from.clone())
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(m) = next
                        && !dist.contains_key(&m)
                    {
                        dist.insert(m.clone(), dist[&n] + 1);
                        queue.push_back(m);
                    }
                }
            }
            None
        };
        [
            PathDirection::Forward,
            PathDirection::Backward,
            PathDirection::Mixed,
        ]
        .into_iter()
        .find_map(|d| bfs(d).map(|n| (n, d)))
    }
}

fn path_of(s: &Snapshot<'_>, id: &str) -> String {
    s.node_detail(id, &At::Now).unwrap().node.path
}

#[test]
fn every_detail_function_matches_the_typescript_helpers_on_northwind() {
    let o = Oracle::new();
    let dir = tempfile::tempdir().unwrap();
    let ix = cartograph_fixtures::index(&cartograph_fixtures::northwind(), dir.path());
    let store = Store::open_path(&ix).unwrap();
    // One snapshot answers hundreds of comparisons; its deadline is per snapshot, so a slow CI
    // runner needs more than the default 5 s for all of them together.
    let s = store
        .snapshot(Limits {
            deadline: std::time::Duration::from_secs(120),
            ..Limits::default()
        })
        .unwrap();
    let all = Page::parse(Some(1000), None).unwrap();

    // Scopes the index has under the demo's names: the TypeScript services (Go and Python ones
    // have no package manifest, so the index folds them into the repo's remainder service), every
    // module and every file.
    let mut scopes: Vec<String> = ["gateway", "billing", "notify", "shared"]
        .map(String::from)
        .to_vec();
    scopes.extend(o.parent.keys().cloned());

    let commits = s.commits(&At::Now, all).unwrap().rows;
    assert_eq!(commits.len(), o.d.commits.len(), "one step per demo commit");
    // A path names what is alive at `at`, so scopes are named by id, resolved once at the end.
    let last = commits[0].seq;
    let id: BTreeMap<&String, String> = scopes
        .iter()
        .map(|sc| (sc, s.resolve_node(&o.path[sc], last).unwrap()))
        .collect();
    let mut compared = 0;
    for (i, c) in commits.iter().rev().enumerate() {
        let at = At::Sha(c.sha.clone());
        let t = o.time(i);

        // The index holds exactly the oracle's links at this step.
        let slice = s
            .graph(
                &GraphQuery::from_params(&GraphParams {
                    at: Some(c.sha.clone()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        let live: BTreeSet<(String, String)> = (0..slice.links.id.len())
            .map(|k| {
                (
                    slice.nodes.name[slice.links.from[k] as usize].clone(),
                    slice.nodes.name[slice.links.to[k] as usize].clone(),
                )
            })
            .collect();
        let want: BTreeSet<(String, String)> = o
            .links
            .iter()
            .filter(|l| o.active(l, t))
            .map(|l| (o.path[&l.from].clone(), o.path[&l.to].clone()))
            .collect();
        assert_eq!(live, want, "links at commit {i}");

        for scope in &scopes {
            let r = &id[scope];
            let m = s.metrics(r, &at).unwrap();
            assert_eq!(
                [m.links, m.fan_in, m.fan_out, m.cross, m.churn],
                o.metrics(scope, t),
                "metrics of {scope} at commit {i}"
            );
            let n: BTreeSet<(String, String)> = s
                .neighbors(r, &at, all)
                .unwrap()
                .rows
                .iter()
                .map(|l| (path_of(&s, &l.from.0), path_of(&s, &l.to.0)))
                .collect();
            assert_eq!(n, o.neighbours(scope, t), "neighbours of {scope} at {i}");
            let ch = s.changes(r, &at, all).unwrap().rows;
            assert!(
                ch.windows(2).all(|w| w[0].commit.seq >= w[1].commit.seq),
                "newest first"
            );
            let ch: BTreeSet<(usize, String, String, bool)> = ch
                .iter()
                .map(|x| {
                    (
                        x.commit.seq as usize,
                        path_of(&s, &x.link.from.0),
                        path_of(&s, &x.link.to.0),
                        x.change == Change::Added,
                    )
                })
                .collect();
            assert_eq!(ch, o.changes(scope, t), "changes of {scope} at {i}");
            let rk = s.ranking(r, &at, all).unwrap().rows;
            assert!(
                rk.windows(2)
                    .all(|w| (w[0].used_by, w[0].uses) >= (w[1].used_by, w[1].uses)),
                "ranked by used_by, then uses"
            );
            let rk: BTreeSet<(String, u32, u32, i32)> = rk
                .iter()
                .map(|x| (x.node.path.clone(), x.used_by, x.uses, x.delta))
                .collect();
            assert_eq!(rk, o.ranking(scope, t), "ranking of {scope} at {i}");
            let imp = s.impact(&ImpactInput::Nodes(vec![r.clone()]), &at).unwrap();
            assert!(
                imp.likely.is_empty() && imp.guessed.is_empty(),
                "parsed links are exact"
            );
            let imp: BTreeSet<String> = imp.exact.iter().map(|n| path_of(&s, &n.0)).collect();
            assert_eq!(imp, o.impact(scope, t), "impact of {scope} at {i}");
            compared += 1;
        }
    }
    assert!(compared > 500, "{compared} comparisons");

    // Paths between every pair of files at the last commit.
    let files: Vec<&String> = o.path.keys().filter(|k| k.ends_with(".ts")).collect();
    let t = o.time(o.d.commits.len() - 1);
    let mut found = 0;
    for a in &files {
        for b in &files {
            let got = s.path(&id[*a], &id[*b], 12, &At::Now).unwrap();
            let want = o.path(a, b, t);
            match (got.first(), want) {
                (None, None) => {}
                (Some(p), Some((hops, dir))) => {
                    assert_eq!((p.hops.len(), p.direction), (hops, dir), "path {a} -> {b}");
                    assert!(got.iter().all(|q| q.hops.len() <= hops + 1));
                    found += 1;
                }
                (g, w) => panic!("path {a} -> {b}: got {g:?}, want {w:?}"),
            }
        }
    }
    assert!(found > 50, "{found} paths");
}

#[test]
fn series_density_and_commit_impact_agree_with_the_links() {
    let dir = tempfile::tempdir().unwrap();
    let ix = cartograph_fixtures::index(&cartograph_fixtures::northwind(), dir.path());
    let store = Store::open_path(&ix).unwrap();
    // One snapshot answers hundreds of comparisons; its deadline is per snapshot, so a slow CI
    // runner needs more than the default 5 s for all of them together.
    let s = store
        .snapshot(Limits {
            deadline: std::time::Duration::from_secs(120),
            ..Limits::default()
        })
        .unwrap();
    let all = Page::parse(Some(1000), None).unwrap();

    // The series' net changes add up to the live count, month by month back to the start.
    let bins = s.series("workspace", &At::Now, 36).unwrap();
    let live = s.metrics("workspace", &At::Now).unwrap().links as i64;
    let net: i64 = bins
        .iter()
        .map(|b| i64::from(b.links_added) - i64::from(b.links_removed))
        .sum();
    assert_eq!(net, live);

    // Density: every commit lands in exactly one bin, for each bin size.
    let commits = s.commits(&At::Now, all).unwrap().rows;
    for bin in [Bin::Day, Bin::Week, Bin::Month] {
        let d = s.density(None, None, bin).unwrap();
        let total: u32 = d.bins.iter().map(|b| b.commits).sum();
        assert_eq!(total as usize, commits.len(), "{bin:?}");
        for b in &d.bins {
            assert_eq!(b.by_repo.iter().sum::<u32>(), b.commits);
        }
    }

    // Commit impact: the links added and removed are the diff of the slices either side.
    let pairs = |at: &str| -> BTreeSet<String> {
        let g = s
            .graph(
                &GraphQuery::from_params(&GraphParams {
                    at: Some(at.into()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        g.links.id.iter().map(|l| l.0.clone()).collect()
    };
    for w in commits.windows(2) {
        let (after, before) = (&w[0], &w[1]);
        let imp = s.commit_impact(&after.sha).unwrap();
        assert_eq!(imp.prev.as_deref(), Some(before.sha.as_str()));
        let (a, b) = (pairs(&after.sha), pairs(&before.sha));
        let added: BTreeSet<String> = imp.added_links.iter().map(|l| l.id.0.clone()).collect();
        let removed: BTreeSet<String> = imp.removed_links.iter().map(|l| l.id.0.clone()).collect();
        assert_eq!(added, a.difference(&b).cloned().collect(), "{}", after.sha);
        assert_eq!(
            removed,
            b.difference(&a).cloned().collect(),
            "{}",
            after.sha
        );
    }

    // Lineage, children and search answer on real rows.
    let file = "api-gateway/src/routes/orders.ts";
    let h = s.history(file).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].path, file);
    let kids = s.children("api-gateway", &At::Now, all).unwrap().rows;
    assert!(
        kids.iter().any(|k| k.path == "api-gateway/src/routes"),
        "{kids:?}"
    );
    let top = s.children("workspace", &At::Now, all).unwrap().rows;
    assert!(
        top.iter().any(|k| k.name == "@northwind/api-gateway"),
        "{top:?}"
    );
    let order = s.reading_order("billing", &At::Now, all);
    assert!(order.is_err(), "billing isn't a node name in the index");
    let order = s
        .reading_order("billing-service", &At::Now, all)
        .unwrap()
        .rows;
    assert!(!order.is_empty());
    let d = s.link_detail(
        &s.neighbors(file, &At::Now, all).unwrap().rows[0].id.0,
        &At::Now,
    );
    assert!(d.unwrap().link.confidence.evidence[0].file.is_some());
}
