//! IDX-25: positions at index time, deterministic, stable and nested.

use std::collections::BTreeMap;
use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::layout::{Circle, Group, place};
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;
use proptest::prelude::*;

fn ws(f: &Fixture, dir: &Path) -> Workspace {
    Workspace {
        dir: dir.to_owned(),
        repos: vec![RepoSpec {
            name: "mono".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn run(w: &Workspace, cache: &mut FactCache) {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

/// Node key -> (circle, parent key), for the nodes placed at the last run.
fn positions(w: &Workspace) -> BTreeMap<String, (Circle, Option<String>)> {
    let ix = Index::open_read(&w.index_path()).unwrap();
    let key = "coalesce(n.key, 'file:' || (SELECT path FROM file_path p WHERE p.file = n.file
                ORDER BY from_seq, path LIMIT 1))";
    let sql = format!(
        "SELECT {key}, l.x, l.y, l.r,
                (SELECT {} FROM node n WHERE n.id = (SELECT parent FROM node WHERE id = l.node)
                   AND n.id != l.node)
         FROM layout l JOIN node n ON n.id = l.node WHERE l.to_seq IS NULL",
        key
    );
    ix.conn
        .prepare(&sql)
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    Circle {
                        x: r.get(1)?,
                        y: r.get(2)?,
                        r: r.get(3)?,
                    },
                    r.get::<_, Option<String>>(4)?,
                ),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn monorepo(root: &Path) -> Fixture {
    let mut f = Fixture::init(root);
    f.write("pnpm-workspace.yaml", "packages: ['packages/*']\n");
    for p in ["api", "web", "core"] {
        f.write(
            &format!("packages/{p}/package.json"),
            &format!(r#"{{"name":"@m/{p}"}}"#),
        );
        for m in ["src", "src/lib", "test"] {
            for i in 0..6 {
                f.write(
                    &format!("packages/{p}/{m}/f{i}.ts"),
                    &format!("import './f{}';\nimport '@m/core';\n", (i + 1) % 6),
                );
            }
        }
    }
    f.commit("first");
    f
}

fn inside(c: &Circle, p: &Circle) -> bool {
    ((c.x - p.x).powi(2) + (c.y - p.y).powi(2)).sqrt() + c.r <= p.r + 1e-6
}

#[test]
fn layout_is_deterministic_stable_and_nested() {
    let d = tempfile::tempdir().unwrap();
    let mut f = monorepo(&d.path().join("mono"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let a = ws(&f, &d.path().join("a"));
    run(&a, &mut cache);
    let first = positions(&a);
    assert_eq!(first.len(), 3 + 9 + 3 * 19, "services, modules, files");
    // Two indexes of the same history place every node alike, to the bit.
    let b = ws(&f, &d.path().join("b"));
    run(&b, &mut cache);
    assert_eq!(positions(&b), first);
    // Every child inside its parent.
    for (k, (c, parent)) in &first {
        if let Some(p) = parent {
            assert!(inside(c, &first[p].0), "{k} outside {p}");
        }
    }
    // One file added: nothing that existed moves, and the new file is inside its module.
    f.write("packages/web/src/new.ts", "import './f0';\n");
    f.commit("one more file");
    run(&a, &mut cache);
    let after = positions(&a);
    for (k, v) in &first {
        assert_eq!(after.get(k), Some(v), "{k} moved");
    }
    let (c, parent) = &after["file:packages/web/src/new.ts"];
    assert!(inside(c, &after[parent.as_ref().unwrap()].0));
    // A rebuild carries positions over by node key.
    let rebuilt = d.path().join("rebuilt/index.cartograph");
    std::fs::create_dir_all(rebuilt.parent().unwrap()).unwrap();
    std::fs::copy(a.index_path(), &rebuilt).unwrap();
    let lock = cartograph_store::writer::WriterLock::try_acquire(rebuilt.parent().unwrap())
        .unwrap()
        .unwrap();
    cartograph_index::rebuild_single(&lock, &f.dir, None, &rebuilt, &mut cache, 2).unwrap();
    let ix = Index::open_read(&rebuilt).unwrap();
    let carried = cartograph_index::layout::positions_by_key(&ix.conn).unwrap();
    for (k, (c, _)) in &after {
        assert_eq!(carried.get(k), Some(c), "{k} after a rebuild");
    }
}

proptest! {
    /// Placement is a pure function of its input, keeps pinned children where they are and every
    /// child inside the parent.
    #[test]
    fn placement_is_pure_pinned_and_contained(
        n in 1usize..80,
        pins in proptest::collection::vec(any::<bool>(), 80),
        links in proptest::collection::vec((0usize..80, 0usize..80), 0..60),
    ) {
        let area = Circle { x: 3.0, y: -2.0, r: 40.0 };
        let children: Vec<_> = (0..n)
            .map(|i| {
                let pin = pins[i].then(|| (3.0 + (i as f64 % 7.0) * 2.0, -2.0 + (i as f64 / 7.0)));
                (format!("k{i:03}"), 1.0 + (i % 3) as f64, pin)
            })
            .collect();
        let springs = links.iter().filter(|(a, b)| *a < n && *b < n && a != b).map(|(a, b)| (*a, *b, 1.0)).collect();
        let g = Group { area, children, springs };
        let p = place("parent", &g);
        prop_assert_eq!(&p, &place("parent", &g));
        for (i, (x, y)) in p.iter().enumerate() {
            let r = g.children[i].1;
            if let Some(pin) = g.children[i].2 {
                prop_assert_eq!((*x, *y), pin);
            } else {
                let c = Circle { x: *x, y: *y, r };
                prop_assert!(inside(&c, &area), "child {} outside", i);
            }
        }
    }
}

#[test]
fn a_huge_folder_is_packed_on_a_grid() {
    let n = cartograph_index::layout::GRID_ABOVE + 1;
    let g = Group {
        area: Circle {
            x: 0.0,
            y: 0.0,
            r: cartograph_index::layout::radius_for(std::iter::repeat_n(1.0, n)),
        },
        children: (0..n).map(|i| (format!("{i}"), 1.0, None)).collect(),
        springs: Vec::new(),
    };
    let t = std::time::Instant::now();
    let p = place("big", &g);
    assert_eq!(p.len(), n);
    assert!(t.elapsed().as_secs() < 2, "grid packing is linear");
}
