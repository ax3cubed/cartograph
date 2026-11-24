//! IDX-20: services, libraries and modules over the lineages, by C6's rules.

use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::Repo;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::{Root, ServiceSpec, detect, summary_counts};
use cartograph_index::sync::{SyncOptions, SyncOutcome, SyncSummary, Workspace, restructure, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::{Index, fact_hash};

fn run(w: &Workspace, cache: &mut FactCache) -> SyncSummary {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

fn one(f: &Fixture, dir: &Path) -> Workspace {
    Workspace {
        dir: dir.to_owned(),
        repos: vec![RepoSpec {
            name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// `name kind detected_by` per root, as detection found them.
fn roots(f: &Fixture, services: &[ServiceSpec], scopes: &[String]) -> Vec<String> {
    let repo = Repo::open(&f.dir).unwrap();
    let name = f.dir.file_name().unwrap().to_string_lossy().into_owned();
    detect(&repo, repo.head().unwrap(), &name, scopes, services, &[])
        .unwrap()
        .roots
        .into_iter()
        .map(
            |Root {
                 path,
                 name,
                 kind,
                 detected_by,
                 ..
             }| format!("{path}={name} {kind} {detected_by}"),
        )
        .collect()
}

/// The node tree at the last step: services, their modules, their files, indented.
fn tree(ix: &Index) -> String {
    let mut out = String::new();
    let mut st = ix
        .conn
        .prepare(
            "SELECT n.id, n.kind, n.name, n.detected_by FROM node n
             WHERE n.kind IN ('service', 'library')
               AND EXISTS (SELECT 1 FROM node_span s WHERE s.node = n.id AND s.to_seq IS NULL)
             ORDER BY n.name",
        )
        .unwrap();
    let services: Vec<(i64, String, String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (id, kind, name, by) in services {
        out += &format!("{kind} {name} ({by})\n");
        let children: Vec<String> = ix
            .conn
            .prepare(
                "SELECT CASE n.kind WHEN 'module' THEN 'module ' || n.name
                        ELSE 'file ' || fp.path END
                 FROM node n LEFT JOIN file_path fp ON fp.file = n.file AND fp.to_seq IS NULL
                 WHERE n.parent = ?1 AND n.id != ?1
                   AND (fp.path IS NOT NULL OR EXISTS (SELECT 1 FROM node_span s
                        WHERE s.node = n.id AND s.to_seq IS NULL))
                 ORDER BY 1",
            )
            .unwrap()
            .query_map([id], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for c in children {
            out += &format!("  {c}\n");
            if let Some(m) = c.strip_prefix("module ") {
                let files: Vec<String> = ix
                    .conn
                    .prepare(
                        "SELECT fp.path FROM node n JOIN node m ON m.id = n.parent
                         JOIN file_path fp ON fp.file = n.file AND fp.to_seq IS NULL
                         WHERE m.name = ?1 AND m.service = ?2 ORDER BY 1",
                    )
                    .unwrap()
                    .query_map(cartograph_store::rusqlite::params![m, id], |r| r.get(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                for f in files {
                    out += &format!("    {f}\n");
                }
            }
        }
    }
    out
}

fn pnpm(root: &Path) -> Fixture {
    let mut f = Fixture::init(root);
    f.write("package.json", r#"{"name":"mono","private":true}"#);
    f.write(
        "pnpm-workspace.yaml",
        "packages:\n  - 'packages/*'\n  - 'apps/*'\n  - '!**/test'\n",
    );
    f.write("apps/web/package.json", r#"{"name":"@m/web"}"#);
    f.write("apps/web/Dockerfile", "FROM node:22\n");
    f.write("apps/web/src/index.ts", "import { a } from '@m/core';\n");
    f.write(
        "packages/core/package.json",
        r#"{"name":"@m/core","main":"src/a.ts"}"#,
    );
    f.write(
        "packages/core/src/a.ts",
        "import './util/b';\nexport const a = 1;\n",
    );
    f.write("packages/core/src/util/b.ts", "export const b = 1;\n");
    f.write("packages/ui/package.json", r#"{"name":"@m/ui"}"#);
    f.write("packages/ui/src/button.ts", "import '@m/core';\n");
    f.write("scripts/release.ts", "export {};\n");
    f.commit("first");
    f
}

#[test]
fn a_pnpm_monorepo_splits_into_its_packages() {
    let d = tempfile::tempdir().unwrap();
    let f = pnpm(&d.path().join("mono"));
    let w = one(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    // The golden node tree.
    assert_eq!(
        tree(&ix),
        "library @m/core (pnpm)
  file packages/core/package.json
  module src
    packages/core/src/a.ts
  module src/util
    packages/core/src/util/b.ts
library @m/ui (pnpm)
  file packages/ui/package.json
  module src
    packages/ui/src/button.ts
service @m/web (pnpm)
  file apps/web/package.json
  module src
    apps/web/src/index.ts
service mono (repo)
  module scripts
    scripts/release.ts
"
    );
    // WorkspaceSummary's counts in one query; the root package.json belongs to no service.
    assert_eq!(summary_counts(&ix.conn).unwrap(), (4, 5, 9));
    let fan_in: i64 = ix
        .conn
        .query_row(
            "SELECT n.fan_in FROM node n JOIN file_path fp ON fp.file = n.file
             WHERE fp.path = 'packages/core/src/a.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fan_in, 2, "web and ui import core's entry point");
    let counts: (i64, i64, i64, i64) = ix
        .conn
        .query_row(
            "SELECT c.modules, c.files, c.links_in, c.links_inside FROM scope_count c
             JOIN node n ON n.id = c.node WHERE n.name = '@m/core'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(counts, (2, 3, 2, 1));
}

/// The manifest's example: platform is one repo with two scopes, one of them named by a
/// `services:` entry.
#[test]
fn the_spec_example_is_one_repo_with_two_scopes() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("platform"));
    f.write("package.json", r#"{"name":"platform"}"#);
    f.write("packages/auth/src/a.ts", "import '../../events/src/e';\n");
    f.write("packages/events/src/e.ts", "export {};\n");
    f.write("apps/web/w.ts", "export {};\n");
    f.commit("first");
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "platform".into(),
            path: f.dir.clone(),
            scopes: vec!["packages/auth".into(), "packages/events".into()],
            ..Default::default()
        }],
        services: vec![ServiceSpec {
            name: "auth".into(),
            repo: "platform".into(),
            path: Some("packages/auth".into()),
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(
        tree(&ix),
        "service auth (manifest)
  module src
    packages/auth/src/a.ts
service events (scope)
  module src
    packages/events/src/e.ts
"
    );
    assert_eq!(summary_counts(&ix.conn).unwrap(), (2, 2, 3));
}

/// One fixture per C6 rule, and the precedence between them.
#[test]
fn each_c6_rule_and_their_precedence() {
    let d = tempfile::tempdir().unwrap();
    let mk = |name: &str, files: &[(&str, &str)]| {
        let mut f = Fixture::init(&d.path().join(name));
        for (p, c) in files {
            f.write(p, c);
        }
        f.commit("first");
        f
    };
    let backstage = mk(
        "backstage",
        &[
            (
                "svc/a/catalog-info.yaml",
                "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: alpha\n---\nkind: API\nmetadata:\n  name: not-a-component\n",
            ),
            ("svc/a/x.ts", ""),
            ("pnpm-workspace.yaml", "packages: ['libs/*']\n"),
            ("libs/l/package.json", "{}"),
        ],
    );
    assert_eq!(
        roots(&backstage, &[], &[]),
        ["svc/a=alpha service backstage"],
        "before workspaces"
    );
    let manifest = [ServiceSpec {
        name: "mine".into(),
        repo: "backstage".into(),
        path: Some("svc".into()),
    }];
    assert_eq!(
        roots(&backstage, &manifest, &[]),
        ["svc=mine service manifest"],
        "before Backstage"
    );

    let npm = mk(
        "npm",
        &[
            ("package.json", r#"{"workspaces":{"packages":["pkgs/*"]}}"#),
            ("pkgs/api/package.json", r#"{"name":"api"}"#),
            ("pkgs/api/Procfile", "web: node ."),
            ("pkgs/lib/package.json", r#"{"name":"lib"}"#),
        ],
    );
    assert_eq!(
        roots(&npm, &[], &[]),
        [
            "pkgs/api=api service npm-workspaces",
            "pkgs/lib=lib library npm-workspaces"
        ]
    );

    let go = mk(
        "go",
        &[
            ("go.work", "go 1.22\nuse (\n  ./billing\n  ./shared\n)\n"),
            ("billing/go.mod", "module example.com/billing\n"),
            ("billing/serverless.yml", "service: billing\n"),
            ("shared/go.mod", "module example.com/shared\n"),
        ],
    );
    assert_eq!(
        roots(&go, &[], &[]),
        [
            "billing=billing service go.work",
            "shared=shared library go.work"
        ]
    );

    let nx = mk(
        "nx",
        &[
            ("apps/shop/project.json", r#"{"name":"shop"}"#),
            ("apps/shop/Dockerfile", ""),
        ],
    );
    assert_eq!(roots(&nx, &[], &[]), ["apps/shop=shop service nx"]);

    let cargo = mk(
        "cargo",
        &[
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
            ("crates/core/Cargo.toml", "[package]\nname = \"core-lib\"\n"),
            (
                "docker-compose.yml",
                "services:\n  core:\n    build: ./crates/core\n",
            ),
        ],
    );
    let repo = Repo::open(&cargo.dir).unwrap();
    let found = detect(&repo, repo.head().unwrap(), "cargo", &[], &[], &[]).unwrap();
    assert_eq!(found.roots.len(), 1);
    assert_eq!(
        (found.roots[0].name.as_str(), found.roots[0].kind),
        ("core-lib", "service"),
        "a compose build is deployable"
    );
    assert_eq!(
        found.roots[0].source.as_deref(),
        Some("Cargo.toml"),
        "the rule's input is the workspace file"
    );

    let composer = mk(
        "composer",
        &[
            (
                "composer.json",
                r#"{"repositories":[{"type":"path","url":"packages/*"}]}"#,
            ),
            (
                "packages/billing/composer.json",
                r#"{"name":"acme/billing"}"#,
            ),
        ],
    );
    assert_eq!(
        roots(&composer, &[], &[]),
        ["packages/billing=acme/billing library composer"]
    );

    let plain = mk("plain", &[("src/a.ts", "")]);
    assert_eq!(roots(&plain, &[], &[]), ["=plain service repo"]);
}

/// C7: a split re-parents nodes and leaves facts and links as they were.
#[test]
fn a_split_restructures_one_repo_without_a_replay() {
    let d = tempfile::tempdir().unwrap();
    let f = pnpm(&d.path().join("mono"));
    let mut w = one(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let before = {
        let ix = Index::open_read(&w.index_path()).unwrap();
        fact_hash(&ix.conn).unwrap().to_string()
    };
    w.services.push(ServiceSpec {
        name: "scripts".into(),
        repo: "mono".into(),
        path: Some("scripts".into()),
    });
    let d2 = restructure(&w, "mono").unwrap();
    assert_eq!(d2.structures, 1);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(
        fact_hash(&ix.conn).unwrap().to_string(),
        before,
        "links untouched"
    );
    assert!(
        tree(&ix).contains("service scripts (manifest)\n  module .\n    scripts/release.ts\n"),
        "{}",
        tree(&ix)
    );
    // The manifest entry wins (first match), so the packages are no longer split out.
    assert!(!tree(&ix).contains("@m/core"));
    // And an unchanged sync after it rebuilds nothing.
    let again = run(&w, &mut cache).derived;
    assert_eq!(again.rebuilt(), 0, "{again:?}");
}

/// Structure nodes have validity intervals: a module whose files are all deleted closes.
#[test]
fn a_module_lives_while_it_holds_files() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("src/a.ts", "export {};\n");
    f.write("old/b.ts", "export {};\n");
    f.commit("one");
    f.remove("old/b.ts");
    f.commit("two");
    let w = one(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    let spans: Vec<String> = ix
        .conn
        .prepare(
            "SELECT n.name || ' ' || s.from_seq || '-' || coalesce(s.to_seq, 'open') FROM node_span s
             JOIN node n ON n.id = s.node WHERE n.kind = 'module' ORDER BY 1",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(spans, ["old 0-1", "src 0-open"]);
}

/// A repository dropped from the workspace keeps its nodes, and its services close with its files.
#[test]
fn a_removed_repo_s_services_close() {
    let d = tempfile::tempdir().unwrap();
    let mut a = Fixture::init(&d.path().join("a"));
    a.write("src/x.ts", "export {};\n");
    a.commit("a");
    let mut b = Fixture::init(&d.path().join("b"));
    b.write("src/y.ts", "export {};\n");
    b.commit("b");
    let spec = |f: &Fixture| RepoSpec {
        name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
        path: f.dir.clone(),
        ..Default::default()
    };
    let mut w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![spec(&a), spec(&b)],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let counts =
        |w: &Workspace| summary_counts(&Index::open_read(&w.index_path()).unwrap().conn).unwrap();
    assert_eq!(counts(&w), (2, 2, 2));
    w.repos.pop();
    run(&w, &mut cache);
    assert_eq!(counts(&w), (1, 1, 1));
}
