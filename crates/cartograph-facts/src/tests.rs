use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use cartograph_parse::{BlobFacts, extractor_for};
use proptest::prelude::*;

use crate::replay::{Change, Interval, Link, LinkKind, Replay, from_scratch};

fn facts(path: &str, src: &str) -> Arc<BlobFacts> {
    let (e, k) = extractor_for(path).expect("tracked");
    Arc::new(e.extract(k, src.as_bytes()))
}

fn put(path: &str, src: &str) -> Change {
    (path.to_owned(), Some(facts(path, src)))
}

fn del(path: &str) -> Change {
    (path.to_owned(), None)
}

fn imp(from: &str, to: &str) -> Link {
    (LinkKind::Import, from.into(), to.into())
}

fn links_of(changes: &[Change]) -> BTreeSet<Link> {
    let mut r = Replay::default();
    r.apply(changes, 0);
    assert_eq!(
        r.links(),
        from_scratch(r.tree()),
        "replay equals from scratch"
    );
    r.links()
}

fn one_link(changes: &[Change], from: &str) -> Option<Link> {
    let l: Vec<Link> = links_of(changes)
        .into_iter()
        .filter(|l| l.1 == from)
        .collect();
    assert!(l.len() <= 1, "{l:?}");
    l.into_iter().next()
}

#[test]
fn relative_paths_extensions_and_index_files() {
    let base = [
        put("src/b/index.ts", ""),
        put("src/c.tsx", ""),
        put("src/d.ts", ""),
    ];
    let with = |src: &str| {
        let mut c = base.to_vec();
        c.push(put("src/a.ts", src));
        one_link(&c, "src/a.ts").map(|l| l.2)
    };
    assert_eq!(with("import './b'"), Some("src/b/index.ts".into()));
    assert_eq!(with("import './c'"), Some("src/c.tsx".into()));
    assert_eq!(
        with("import './d.js'"),
        Some("src/d.ts".into()),
        "the TS ESM rule"
    );
    assert_eq!(with("import '../src/d'"), Some("src/d.ts".into()));
    assert_eq!(with("import './missing'"), None);
}

#[test]
fn tsconfig_paths_through_extends() {
    let c = [
        put(
            "tsconfig.base.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
        ),
        put(
            "tsconfig.json",
            r#"{"extends":"./tsconfig.base.json","compilerOptions":{"strict":true}}"#,
        ),
        put("src/util.ts", ""),
        put("app/main.ts", "import { u } from '@/util';"),
    ];
    assert_eq!(
        one_link(&c, "app/main.ts"),
        Some(imp("app/main.ts", "src/util.ts"))
    );
}

#[test]
fn workspace_packages_exports_and_package_links() {
    let mut c = vec![
        put(
            "pkg/package.json",
            r#"{"name":"@s/pkg","exports":{".":"./src/index.ts","./util":"./src/util.ts"}}"#,
        ),
        put("pkg/src/index.ts", ""),
        put("pkg/src/util.ts", ""),
        put(
            "old/package.json",
            r#"{"name":"old","main":"dist/index.js"}"#,
        ),
        put("app/a.ts", "import '@s/pkg';"),
    ];
    assert_eq!(
        one_link(&c, "app/a.ts"),
        Some(imp("app/a.ts", "pkg/src/index.ts"))
    );
    c.push(put("app/b.ts", "import '@s/pkg/util';"));
    assert_eq!(
        one_link(&c, "app/b.ts"),
        Some(imp("app/b.ts", "pkg/src/util.ts"))
    );
    // Its entry point is a build output that isn't in git: a link to the package itself.
    c.push(put("app/c.ts", "import 'old';"));
    assert_eq!(
        one_link(&c, "app/c.ts"),
        Some((
            LinkKind::Package,
            "app/c.ts".into(),
            "old/package.json".into()
        ))
    );
    // npm dependencies and built-ins are not links.
    c.push(put(
        "app/d.ts",
        "import 'react'; import 'fs'; import 'node:path';",
    ));
    assert_eq!(one_link(&c, "app/d.ts"), None);
}

#[test]
fn exports_conditions_match_in_file_order() {
    let with = |exports: &str| {
        let c = [
            put(
                "p/package.json",
                &format!(r#"{{"name":"p","exports":{{".":{exports}}}}}"#),
            ),
            put("p/a.ts", ""),
            put("p/b.ts", ""),
            put("x.ts", "import 'p';"),
        ];
        one_link(&c, "x.ts").map(|l| l.2)
    };
    assert_eq!(
        with(r#"{"import":"./a.ts","default":"./b.ts"}"#),
        Some("p/a.ts".into())
    );
    assert_eq!(
        with(r#"{"default":"./b.ts","import":"./a.ts"}"#),
        Some("p/b.ts".into())
    );
}

#[test]
fn config_cannot_reach_outside_the_tree() {
    let c = [
        put(
            "tsconfig.json",
            r#"{"extends":"../../../../etc/x","compilerOptions":{"baseUrl":"/","paths":{"*":["../../../etc/*"]}}}"#,
        ),
        put(
            "pkg/package.json",
            r#"{"name":"../../etc","main":"/etc/passwd"}"#,
        ),
        put(
            "a.ts",
            "import '../../../../etc/passwd'; import 'passwd'; import '../../etc';",
        ),
    ];
    assert_eq!(one_link(&c, "a.ts"), None);
}

/// bench.mjs's self-check history, ported.
#[test]
fn research_self_check_history() {
    let mut r = Replay::default();
    r.apply(
        &[put("src/a.ts", "import a from './b'; import l from 'lib';")],
        0,
    );
    assert!(r.links().is_empty(), "b and lib don't exist yet");
    r.apply(
        &[
            put("src/b/index.ts", ""),
            put("lib/package.json", r#"{"name":"lib"}"#),
        ],
        1,
    );
    r.apply(&[put("src/b.ts", "")], 2); // a better candidate appears: the link moves
    r.apply(
        &[
            put("lib/package.json", r#"{"name":"other"}"#),
            put("src/a.ts", "import a from './b';"),
        ],
        3,
    );
    let mut closed = r.closed.clone();
    closed.sort();
    assert_eq!(
        closed,
        vec![
            Interval {
                link: imp("src/a.ts", "src/b/index.ts"),
                from: 1,
                to: 2
            },
            Interval {
                link: (
                    LinkKind::Package,
                    "src/a.ts".into(),
                    "lib/package.json".into()
                ),
                from: 1,
                to: 3
            },
        ]
    );
    assert_eq!(
        r.open().map(|(l, s)| (l.clone(), s)).collect::<Vec<_>>(),
        vec![(imp("src/a.ts", "src/b.ts"), 2)]
    );
    assert_eq!(r.links(), from_scratch(r.tree()));
}

#[test]
fn a_link_that_closes_and_reopens_in_one_step_stays_open() {
    let mut r = Replay::default();
    r.apply(&[put("b.ts", ""), put("a.ts", "import './b';")], 0);
    r.apply(
        &[put("a.ts", "import './b.js';"), put("b.ts", "export {}")],
        1,
    );
    assert!(r.closed.is_empty());
    assert_eq!(r.open().count(), 1);
}

#[test]
fn version_bumps_re_resolve_nothing() {
    let mut r = Replay::default();
    r.apply(
        &[
            put(
                "p/package.json",
                r#"{"name":"p","version":"1.0.0","exports":"./i.ts"}"#,
            ),
            put("p/i.ts", ""),
            put("a.ts", "import 'p';"),
        ],
        0,
    );
    let before = r.stats.resolutions;
    r.apply(
        &[put(
            "p/package.json",
            r#"{"name":"p","version":"1.0.1","exports":"./i.ts"}"#,
        )],
        1,
    );
    assert_eq!(r.stats.resolutions, before);
    r.apply(
        &[
            put("p/package.json", r#"{"name":"p","exports":"./j.ts"}"#),
            put("p/j.ts", ""),
        ],
        2,
    );
    assert_eq!(r.links(), BTreeSet::from([imp("a.ts", "p/j.ts")]));
}

// The first HD-06 property: replay equals direct indexing on generated histories.

const SOURCES: [&str; 8] = [
    "src/a.ts",
    "src/b.ts",
    "src/b/index.ts",
    "src/c.tsx",
    "lib/x.js",
    "pkg/src/index.ts",
    "pkg/src/util.ts",
    "app/main.ts",
];
const SPECS: [&str; 14] = [
    "./b",
    "./b.js",
    "../lib/x",
    "./c",
    "@/b",
    "@/c",
    "pkg",
    "pkg/util",
    "pkg2",
    "react",
    "fs",
    "../pkg/src/util",
    "./index",
    "../src/b",
];
const MANIFESTS: [&str; 4] = [
    "pkg/package.json",
    "pkg2/package.json",
    "tsconfig.json",
    "tsconfig.base.json",
];

fn manifest(path: &str, v: u8) -> String {
    let v = v as usize;
    if path.ends_with("package.json") {
        let name = ["pkg", "pkg2", "other"][v % 3];
        let exports = [
            "",
            r#","exports":{".":"./src/index.ts"}"#,
            r#","exports":{".":"./src/index.ts","./util":"./src/util.ts"}"#,
        ][(v / 3) % 3];
        let main = ["", r#","main":"dist/index.js""#][(v / 9) % 2];
        format!(r#"{{"name":"{name}","version":"1.{v}.0"{exports}{main}}}"#)
    } else {
        let extends = ["", r#""extends":"./tsconfig.base.json","#][v % 2];
        let opts = [
            r#"{"strict":true}"#,
            r#"{"baseUrl":"."}"#,
            r#"{"baseUrl":".","paths":{"@/*":["src/*"]}}"#,
            r#"{"paths":{"@/*":["./src/b/*"]}}"#,
        ][(v / 2) % 4];
        format!(r#"{{{extends}"compilerOptions":{opts}}}"#)
    }
}

#[derive(Debug, Clone)]
enum Op {
    Source(u8, u16),
    Manifest(u8, u8),
    Delete(u8),
}

fn change(op: &Op) -> Change {
    match op {
        Op::Source(f, mask) => {
            let path = SOURCES[*f as usize % SOURCES.len()];
            let src: String = SPECS
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(i, s)| format!("import x{i} from '{s}';\n"))
                .collect();
            put(path, &src)
        }
        Op::Manifest(f, v) => {
            let path = MANIFESTS[*f as usize % MANIFESTS.len()];
            put(path, &manifest(path, *v))
        }
        Op::Delete(f) => {
            let all: Vec<&str> = SOURCES.iter().chain(&MANIFESTS).copied().collect();
            del(all[*f as usize % all.len()])
        }
    }
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => (any::<u8>(), any::<u16>()).prop_map(|(f, m)| Op::Source(f, m)),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(f, v)| Op::Manifest(f, v)),
        2 => any::<u8>().prop_map(Op::Delete),
    ]
}

/// Every interval written, with open ones running to the end.
fn all_intervals(r: &Replay, end: u64) -> Vec<Interval> {
    let mut v = r.closed.clone();
    v.extend(r.open().map(|(l, s)| Interval {
        link: l.clone(),
        from: s,
        to: end,
    }));
    v.sort();
    v
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn replay_equals_direct_indexing(steps in proptest::collection::vec(proptest::collection::vec(op(), 1..4), 1..30)) {
        let mut r = Replay::default();
        let mut ablation = Replay::new(false);
        let mut at_step = Vec::new();
        for (seq, ops) in steps.iter().enumerate() {
            // One change per path per step, as a tree diff gives.
            let mut by_path: HashMap<String, Change> = HashMap::new();
            for o in ops {
                let c = change(o);
                by_path.insert(c.0.clone(), c);
            }
            let changes: Vec<Change> = by_path.into_values().collect();
            r.apply(&changes, seq as u64);
            ablation.apply(&changes, seq as u64);
            prop_assert_eq!(r.links(), from_scratch(r.tree()), "step {}", seq);
            at_step.push(r.links());
        }
        let end = steps.len() as u64;
        let all = all_intervals(&r, end);
        for (s, want) in at_step.iter().enumerate() {
            let s = s as u64;
            let got: BTreeSet<Link> = all.iter().filter(|i| i.from <= s && s < i.to).map(|i| i.link.clone()).collect();
            prop_assert_eq!(&got, want, "intervals reproduce the link set at step {}", s);
        }
        prop_assert_eq!(all, all_intervals(&ablation, end), "the reverse index changes nothing");
        prop_assert!(r.stats.resolutions <= ablation.stats.resolutions);
    }
}
