//! Shared by the property tests and the performance gates: a history written with one
//! `git fast-import`, so a generated case costs one git process, not five per commit.

#![allow(dead_code)] // each test binary uses part of this

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use cartograph_git::fixture::Fixture;

/// One commit: each path with its new content, or `None` to delete it.
pub type Commit = BTreeMap<String, Option<String>>;

/// Writes `commits` as the first-parent history of `main` in a new repository at `dir`.
pub fn import(dir: &Path, commits: &[Commit]) {
    let f = Fixture::init(dir);
    let mut s = String::new();
    for (i, c) in commits.iter().enumerate() {
        let t = 1_600_000_000 + i as i64 * 60;
        let msg = format!("c{i}\n");
        let _ = write!(
            s,
            "commit refs/heads/main\ncommitter Ada <ada@example.com> {t} +0000\ndata {}\n{msg}",
            msg.len()
        );
        for (path, body) in c {
            match body {
                Some(b) => {
                    let _ = write!(s, "M 100644 inline {path}\ndata {}\n{b}\n", b.len());
                }
                None => {
                    let _ = writeln!(s, "D {path}");
                }
            }
        }
        s.push('\n');
    }
    s.push_str("done\n");
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&f.dir)
        .args(["fast-import", "--quiet", "--done"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .stdin(Stdio::piped())
        .spawn()
        .expect("git fast-import runs");
    child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success(), "fast-import");
}

/// A tree of `files` TypeScript files in one package, then `commits` commits that each rewrite one
/// file with two imports. Every commit's work is the same whatever `files` is, so replay time per
/// commit must not grow with the tree.
pub fn wide_history(files: usize, commits: usize) -> Vec<Commit> {
    let body = |i: usize, rev: usize| {
        format!(
            "import {{ v }} from './f{}';\nimport {{ w }} from './f{}';\nexport const v{i} = {rev};\n",
            (i * 7 + rev) % files,
            (i * 13 + rev + 1) % files
        )
    };
    let mut first = Commit::new();
    first.insert(
        "package.json".into(),
        Some(r#"{"name":"wide","private":true}"#.into()),
    );
    for i in 0..files {
        first.insert(format!("src/f{i}.ts"), Some(body(i, 0)));
    }
    let mut out = vec![first];
    for c in 1..=commits {
        let i = (c * 31) % files;
        out.push(Commit::from([(format!("src/f{i}.ts"), Some(body(i, c)))]));
    }
    out
}

/// remotion's size without a remotion clone: 9,000 TypeScript files in 90 packages behind a root
/// tsconfig with `paths`, five references each (relative, `.js` to `.ts`, a workspace package, a
/// `paths` alias, an npm package), so about 45k resolutions per tree. Then `commits` commits that
/// each rewrite two files.
pub fn monorepo_history(commits: usize) -> Vec<Commit> {
    const PACKAGES: usize = 90;
    const PER_PACKAGE: usize = 100;
    let body = |p: usize, i: usize, rev: usize| {
        let q = (p * 7 + rev + 1) % PACKAGES;
        format!(
            "import {{ a }} from './f{}';\nimport {{ b }} from '../src/f{}.js';\n\
             import {{ c }} from '@r/p{q}';\nimport {{ d }} from '~/p{q}/src/f{}';\n\
             import React from 'react';\nexport const v{i} = {rev};\n",
            (i * 7 + rev) % PER_PACKAGE,
            (i * 13 + rev + 1) % PER_PACKAGE,
            (i + rev) % PER_PACKAGE,
        )
    };
    let mut first = Commit::new();
    first.insert(
        "tsconfig.json".into(),
        Some(r#"{"compilerOptions":{"baseUrl":".","paths":{"~/*":["packages/*"]}}}"#.into()),
    );
    for p in 0..PACKAGES {
        first.insert(
            format!("packages/p{p}/package.json"),
            Some(format!(
                r#"{{"name":"@r/p{p}","exports":{{".":"./src/f0.ts"}}}}"#
            )),
        );
        for i in 0..PER_PACKAGE {
            first.insert(format!("packages/p{p}/src/f{i}.ts"), Some(body(p, i, 0)));
        }
    }
    let mut out = vec![first];
    for c in 1..=commits {
        let mut commit = Commit::new();
        for k in 0..2 {
            let (p, i) = ((c * 31 + k * 17) % PACKAGES, (c * 13 + k) % PER_PACKAGE);
            commit.insert(format!("packages/p{p}/src/f{i}.ts"), Some(body(p, i, c)));
        }
        out.push(commit);
    }
    out
}
