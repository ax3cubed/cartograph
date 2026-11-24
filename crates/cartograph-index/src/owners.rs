//! Owners from CODEOWNERS (XS-23, D14).
//!
//! Each repository's CODEOWNERS is the first that exists of `.github/CODEOWNERS`, `CODEOWNERS`,
//! `docs/CODEOWNERS` and `.gitlab/CODEOWNERS` at a commit (a blob fact). [`scan`] records which
//! blob that was over history (`codeowners`, one row per version), reading only the commits it
//! hasn't seen. [`build`] evaluates every file lineage's path intervals against every version and
//! writes `ownership`: who owned a file, by which pattern, from which step to which.
//!
//! Matching is GitHub's: the last line whose pattern matches wins; a line with no owners un-owns.
//! A pattern with no slash but a trailing one floats at any depth, otherwise it is anchored at the
//! root; `*` and `?` never cross a `/`; a pattern naming a folder (or a file name that is a
//! folder somewhere) owns everything under it, except that a last segment holding a wildcard owns
//! only what it matches (`docs/*` is not `docs/a/b.md`, as GitHub documents). Patterns compile once
//! per version into one [`GlobSet`] (never hand-built regexes), at most [`MAX_PATTERNS`] of them.
//!
//! Owner strings are untrusted text (F8): only `@user`, `@org/team` and `user@host` shapes over a
//! small character set, at most [`MAX_OWNER`] bytes, are kept, so a stored owner can't carry
//! markup or control characters. A line with any other token is dropped whole, as GitHub does.
//!
//! ponytail: a repository is re-evaluated whole when its CODEOWNERS versions or its path rows
//! change; owners of a service or module aren't derived (only files have them).

use std::collections::HashMap;

use cartograph_git::{GitError, ObjectId, Repo, read_blob};
use cartograph_store::rusqlite::{self, Connection, OptionalExtension, params};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// Where CODEOWNERS may live, in the order GitHub looks (and GitLab's own folder last).
pub const PATHS: [&str; 4] = [
    ".github/CODEOWNERS",
    "CODEOWNERS",
    "docs/CODEOWNERS",
    ".gitlab/CODEOWNERS",
];
/// Lines past this many valid ones are ignored (XS-23 security).
pub const MAX_PATTERNS: usize = 10_000;
/// An owner longer than this is dropped with its line.
pub const MAX_OWNER: usize = 256;
/// The stored pattern text is cut to this many bytes.
const MAX_PATTERN_TEXT: usize = 512;
/// A line names at most this many owners.
const MAX_OWNERS_PER_LINE: usize = 100;
/// GitHub ignores a CODEOWNERS over 3 MB; so do we.
const MAX_BLOB: u64 = 3_000_000;

/// One owner as CODEOWNERS names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub name: String,
    /// `team` (`@org/team`), `person` (`@user`) or `email`.
    pub kind: &'static str,
}

/// One usable line: its pattern as written and its owners (none: the line un-owns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub pattern: String,
    pub owners: Vec<Owner>,
}

/// A parsed CODEOWNERS: the lines and one compiled set over all their patterns.
pub struct Rules {
    lines: Vec<Line>,
    set: GlobSet,
    /// For each glob of the set, the line it belongs to.
    line_of: Vec<usize>,
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// `@user`, `@org/team` or `user@host.tld`, nothing else.
fn parse_owner(tok: &str) -> Option<Owner> {
    if tok.len() > MAX_OWNER {
        return None;
    }
    let (name, kind) = if let Some(rest) = tok.strip_prefix('@') {
        match rest.split_once('/') {
            Some((org, team)) if !org.is_empty() && !team.is_empty() => {
                if !org.chars().all(name_char) || !team.chars().all(name_char) {
                    return None;
                }
                (tok, "team")
            }
            None if !rest.is_empty() && rest.chars().all(name_char) => (tok, "person"),
            _ => return None,
        }
    } else {
        let (local, host) = tok.split_once('@')?;
        let ok = !local.is_empty()
            && local
                .chars()
                .all(|c| name_char(c) || matches!(c, '%' | '+'))
            && host.contains('.')
            && !host.starts_with('.')
            && !host.ends_with('.')
            && host.chars().all(name_char);
        if !ok {
            return None;
        }
        (tok, "email")
    };
    Some(Owner {
        name: name.to_owned(),
        kind,
    })
}

/// The globs a pattern stands for (see the module docs), or none when it can't be one.
fn globs(pattern: &str) -> Vec<String> {
    let dir_only = pattern.ends_with('/');
    let anchored_start = pattern.starts_with('/');
    let p = pattern.trim_start_matches('/').trim_end_matches('/');
    if p.is_empty() || p.starts_with('!') {
        return Vec::new();
    }
    let anchored = anchored_start || p.contains('/');
    let mut esc = String::with_capacity(p.len() + 4);
    for c in p.chars() {
        if matches!(c, '[' | ']' | '{' | '}' | '\\') {
            esc.push('\\');
        }
        esc.push(c);
    }
    let base = if anchored { esc } else { format!("**/{esc}") };
    let last = p.rsplit('/').next().unwrap_or(p);
    let wild_last = last.contains('*') || last.contains('?');
    let mut v = Vec::new();
    if !dir_only {
        v.push(base.clone());
    }
    if !wild_last || dir_only {
        v.push(format!("{base}/**"));
    }
    v
}

impl Rules {
    /// Parses a CODEOWNERS file. Blank lines, comments, lines with a bad owner and patterns that
    /// don't compile are skipped; it never fails.
    pub fn parse(text: &str) -> Self {
        let mut lines: Vec<Line> = Vec::new();
        let mut builder = GlobSetBuilder::new();
        let mut line_of = Vec::new();
        'lines: for raw in text.lines() {
            if lines.len() >= MAX_PATTERNS {
                break;
            }
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            let mut toks = raw.split_whitespace();
            let Some(first) = toks.next() else { continue };
            // `\#` starts a pattern that begins with a hash.
            let pattern = first.strip_prefix('\\').filter(|p| p.starts_with('#'));
            let pattern = pattern.unwrap_or(first);
            if pattern.starts_with('[') || pattern.starts_with("^[") {
                continue; // a GitLab section header
            }
            let mut owners: Vec<Owner> = Vec::new();
            for t in toks {
                if t.starts_with('#') {
                    break;
                }
                let Some(o) = parse_owner(t) else {
                    continue 'lines;
                };
                if !owners.contains(&o) && owners.len() < MAX_OWNERS_PER_LINE {
                    owners.push(o);
                }
            }
            let mut compiled = Vec::new();
            for g in globs(pattern) {
                match GlobBuilder::new(&g).literal_separator(true).build() {
                    Ok(g) => compiled.push(g),
                    Err(_) => continue 'lines,
                }
            }
            if compiled.is_empty() {
                continue;
            }
            for g in compiled {
                builder.add(g);
                line_of.push(lines.len());
            }
            let mut text = pattern.to_owned();
            if text.len() > MAX_PATTERN_TEXT {
                let mut n = MAX_PATTERN_TEXT;
                while !text.is_char_boundary(n) {
                    n -= 1;
                }
                text.truncate(n);
            }
            lines.push(Line {
                pattern: text,
                owners,
            });
        }
        let set = builder.build().unwrap_or_else(|_| GlobSet::empty());
        // A set that failed to build has no globs; keep the maps in step.
        let line_of = if set.is_empty() { Vec::new() } else { line_of };
        Self {
            lines,
            set,
            line_of,
        }
    }

    /// The last line matching `path` (repository-relative), when it names owners.
    pub fn owners_of(&self, path: &str) -> Option<&Line> {
        let mut hits = Vec::new();
        self.set.matches_into(path, &mut hits);
        let last = hits.into_iter().map(|i| self.line_of[i]).max()?;
        let line = &self.lines[last];
        (!line.owners.is_empty()).then_some(line)
    }

    /// The usable lines, in file order.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }
}

fn sha_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Brings the `codeowners` rows of a repository up to its indexed commits: only commits after the
/// last scanned one are read, and the whole history again if that commit is no longer there
/// (a rewrite replaced it).
pub fn scan<E: From<GitError> + From<rusqlite::Error>>(
    conn: &Connection,
    repo_id: i64,
    repo: &Repo,
) -> Result<(), E> {
    let meta_key = format!("codeowners_scan:{repo_id}");
    let state: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [&meta_key], |r| {
            r.get(0)
        })
        .optional()?;
    let mut after = state
        .as_deref()
        .and_then(|s| s.split_once(':'))
        .and_then(|(seq, sha)| Some((seq.parse::<i64>().ok()?, sha.to_owned())));
    if let Some((seq, sha)) = &after {
        let at: Option<Vec<u8>> = conn
            .query_row(
                "SELECT sha FROM commits WHERE repo = ?1 AND seq = ?2",
                params![repo_id, seq],
                |r| r.get(0),
            )
            .optional()?;
        if at.as_deref().map(sha_hex).as_deref() != Some(sha.as_str()) {
            conn.execute("DELETE FROM codeowners WHERE repo = ?1", [repo_id])?;
            after = None;
        }
    }
    let from = after.as_ref().map_or(-1, |a| a.0);
    let commits: Vec<(i64, Vec<u8>)> = conn
        .prepare_cached("SELECT seq, sha FROM commits WHERE repo = ?1 AND seq > ?2 ORDER BY seq")?
        .query_map(params![repo_id, from], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let Some((last_seq, last_sha)) = commits.last().cloned() else {
        return Ok(());
    };
    let ids: Vec<ObjectId> = commits
        .iter()
        .filter_map(|(_, s)| ObjectId::try_from(s.as_slice()).ok())
        .collect();
    if ids.len() != commits.len() {
        return Ok(());
    }
    let blobs = repo.first_blobs_at(&ids, &PATHS)?;
    let mut open: Option<Vec<u8>> = conn
        .query_row(
            "SELECT blob FROM codeowners WHERE repo = ?1 AND to_seq IS NULL",
            [repo_id],
            |r| r.get(0),
        )
        .optional()?;
    for ((seq, _), blob) in commits.iter().zip(blobs) {
        let now = blob.map(|b| b.as_bytes().to_vec());
        if now == open {
            continue;
        }
        if open.is_some() {
            conn.prepare_cached(
                "UPDATE codeowners SET to_seq = ?2 WHERE repo = ?1 AND to_seq IS NULL",
            )?
            .execute(params![repo_id, seq])?;
        }
        if let Some(b) = &now {
            conn.prepare_cached(
                "INSERT INTO codeowners (repo, blob, from_seq, to_seq) VALUES (?1, ?2, ?3, NULL)",
            )?
            .execute(params![repo_id, b, seq])?;
        }
        open = now;
    }
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![meta_key, format!("{last_seq}:{}", sha_hex(&last_sha))],
    )?;
    Ok(())
}

/// A repository's CODEOWNERS versions as an inputs-key part: empty when it has none.
pub fn versions_key(conn: &Connection, repo_id: i64) -> rusqlite::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut st = conn.prepare_cached(
        "SELECT blob, from_seq, to_seq FROM codeowners WHERE repo = ?1 ORDER BY from_seq",
    )?;
    let mut rows = st.query([repo_id])?;
    while let Some(r) = rows.next()? {
        out.extend(r.get::<_, Vec<u8>>(0)?);
        out.extend(r.get::<_, i64>(1)?.to_le_bytes());
        out.extend(r.get::<_, Option<i64>>(2)?.unwrap_or(-1).to_le_bytes());
    }
    Ok(out)
}

type Version = (Vec<u8>, i64, Option<i64>);

/// The overlap of two `[from, to)` intervals where `None` is open.
fn overlap(a: (i64, Option<i64>), b: (i64, Option<i64>)) -> Option<(i64, Option<i64>)> {
    let lo = a.0.max(b.0);
    let hi = match (a.1, b.1) {
        (None, x) | (x, None) => x,
        (Some(x), Some(y)) => Some(x.min(y)),
    };
    hi.is_none_or(|h| lo < h).then_some((lo, hi))
}

/// Rewrites the `ownership` rows of a repository from its CODEOWNERS versions and path rows.
/// `prefix` is its path prefix in a multi-repository index. Returns the rows written.
pub fn build<E: From<GitError> + From<rusqlite::Error>>(
    conn: &Connection,
    repo_id: i64,
    repo: &Repo,
    prefix: Option<&str>,
) -> Result<usize, E> {
    let versions: Vec<Version> = conn
        .prepare_cached(
            "SELECT blob, from_seq, to_seq FROM codeowners WHERE repo = ?1 ORDER BY from_seq",
        )?
        .query_map([repo_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let had: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM ownership o JOIN file f ON f.id = o.file WHERE f.repo = ?1)",
        [repo_id],
        |r| r.get(0),
    )?;
    if had {
        conn.execute(
            "DELETE FROM ownership WHERE file IN (SELECT id FROM file WHERE repo = ?1)",
            [repo_id],
        )?;
    }
    if versions.is_empty() {
        return Ok(0);
    }
    let git = repo.local();
    let mut compiled: HashMap<Vec<u8>, Rules> = HashMap::new();
    for (blob, _, _) in &versions {
        if compiled.contains_key(blob) {
            continue;
        }
        let rules = ObjectId::try_from(blob.as_slice())
            .ok()
            .and_then(|id| read_blob(&git, id, MAX_BLOB).ok())
            .map_or_else(
                || Rules::parse(""),
                |b| Rules::parse(&String::from_utf8_lossy(&b)),
            );
        compiled.insert(blob.clone(), rules);
    }
    let rel = |p: &'_ str| -> String {
        match prefix {
            Some(x) => p
                .strip_prefix(x)
                .map_or(p, |r| r.trim_start_matches('/'))
                .to_owned(),
            None => p.to_owned(),
        }
    };

    type Row = (String, &'static str, String, i64, Option<i64>);
    let mut written = 0;
    let mut flush = |file: i64, rows: &mut Vec<Row>| -> rusqlite::Result<()> {
        for (owner, kind, pattern, from, to) in rows.drain(..) {
            conn.prepare_cached(
                "INSERT INTO ownership (file, owner, kind, pattern, from_seq, to_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![file, owner, kind, pattern, from, to])?;
            written += 1;
        }
        Ok(())
    };
    let mut st = conn.prepare(
        "SELECT fp.file, fp.path, fp.from_seq, fp.to_seq FROM file_path fp
         JOIN file f ON f.id = fp.file WHERE f.repo = ?1 ORDER BY fp.file, fp.from_seq",
    )?;
    let mut rows = st.query([repo_id])?;
    let mut current: Option<i64> = None;
    let mut out: Vec<Row> = Vec::new();
    while let Some(r) = rows.next()? {
        let (file, path, from, to): (i64, String, i64, Option<i64>) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        if current != Some(file) {
            if let Some(f) = current {
                flush(f, &mut out)?;
            }
            current = Some(file);
        }
        let rp = rel(&path);
        for (blob, vf, vt) in &versions {
            let Some((lo, hi)) = overlap((from, to), (*vf, *vt)) else {
                continue;
            };
            let Some(line) = compiled[blob].owners_of(&rp) else {
                continue;
            };
            for o in &line.owners {
                // The same owner by the same pattern, back to back, is one interval.
                let prev = out
                    .iter_mut()
                    .find(|x| x.0 == o.name && x.2 == line.pattern && x.4 == Some(lo));
                match prev {
                    Some(p) => p.4 = hi,
                    None => out.push((o.name.clone(), o.kind, line.pattern.clone(), lo, hi)),
                }
            }
        }
    }
    if let Some(f) = current {
        flush(f, &mut out)?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn who(r: &Rules, path: &str) -> String {
        r.owners_of(path).map_or_else(
            || "-".to_owned(),
            |l| {
                l.owners
                    .iter()
                    .map(|o| o.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            },
        )
    }

    /// GitHub's documented example, with its documented outcomes for sample paths.
    const DOCS: &str = "\
# This is a comment.
* @global-owner1 @global-owner2

*.js    @js-owner #This is an inline comment.
*.go docs@example.com

/build/logs/ @doctocat

docs/*  docs@example.com
apps/ @octocat

/docs/ @doctocat
/scripts/ @doctocat @octo-org/octocats
/apps/ @octocat
/apps/github
";

    #[test]
    fn github_documented_examples_assign_what_github_does() {
        let r = Rules::parse(DOCS);
        let cases: &[(&str, &str)] = &[
            ("README.md", "@global-owner1 @global-owner2"),
            ("src/deep/er/file.rs", "@global-owner1 @global-owner2"),
            ("a.js", "@js-owner"),
            ("src/lib/a.js", "@js-owner"),
            ("a.go", "docs@example.com"),
            ("pkg/b.go", "docs@example.com"),
            ("build/logs/x.log", "@doctocat"),
            ("build/logs/nested/y.log", "@doctocat"),
            ("build/other.txt", "@global-owner1 @global-owner2"),
            ("src/build/logs/x.log", "@global-owner1 @global-owner2"),
            // `docs/*` matches a file in docs, but /docs/ later wins for the whole folder.
            ("docs/getting-started.md", "@doctocat"),
            ("docs/build-app/troubleshooting.md", "@doctocat"),
            // A slash in the middle anchors `docs/*` at the root.
            (
                "sub/docs/getting-started.md",
                "@global-owner1 @global-owner2",
            ),
            ("apps/x/y.ts", "@octocat"),
            ("apps/github", "@octocat"),
            ("apps/github/a.rs", "@octocat"),
            ("other/apps/z.ts", "@octocat"),
            ("scripts/run.sh", "@doctocat @octo-org/octocats"),
            ("scripts/deep/run.js", "@doctocat @octo-org/octocats"),
        ];
        for (path, want) in cases {
            // apps/github has no owner (the last line), so it un-owns what /apps/ gave.
            let want = if path.starts_with("apps/github") {
                "-"
            } else {
                want
            };
            assert_eq!(who(&r, path), want, "{path}");
        }
    }

    #[test]
    fn fifty_paths_across_overlapping_patterns_follow_last_match_wins() {
        let r = Rules::parse(
            "\
* @all
/src/ @src
/src/api/ @api
*.md @docs
/src/api/*.proto @proto
docs/** @docs-team
/docs/internal/ @sec
/src/api/legacy/ @all
**/vendor/ @vend
/src/*.rs @rs
",
        );
        let cases: Vec<(String, &str)> = [
            ("README.md", "@docs"),
            ("LICENSE", "@all"),
            ("src/main.rs", "@rs"),
            ("src/util/mod.rs", "@src"),
            ("src/util/notes.md", "@docs"),
            ("src/api/server.ts", "@api"),
            ("src/api/spec.proto", "@proto"),
            ("src/api/v1/spec.proto", "@api"),
            ("src/api/readme.md", "@docs"),
            ("src/api/legacy/old.ts", "@all"),
            ("src/api/legacy/old.md", "@all"),
            ("docs/index.md", "@docs-team"),
            ("docs/a/b/c.txt", "@docs-team"),
            ("docs/internal/plan.md", "@sec"),
            ("docs/internal/deep/x", "@sec"),
            ("other/docs/x.txt", "@all"),
            ("vendor/x/y.js", "@vend"),
            ("src/vendor/y.js", "@vend"),
            ("src/api/vendor/z.proto", "@vend"),
            ("a/b/vendor/c/d.rs", "@vend"),
            ("vendors/x.js", "@all"),
            ("srcx/a.ts", "@all"),
            ("src", "@all"),
            ("lib/src/x.c", "@all"),
            ("tests/src/a.ts", "@all"),
            ("Cargo.toml", "@all"),
            ("src/lib.rs", "@rs"),
            ("src/a/lib.rs", "@src"),
            ("src/api/a/b/c/d.txt", "@api"),
            ("docs", "@all"),
            ("docs/README.md", "@docs-team"),
            ("x/y/z/w.md", "@docs"),
            ("src/api/legacy", "@api"),
            ("src/api/legacy/sub/f.go", "@all"),
            ("src/api/legacyx/f.go", "@api"),
            ("src/apix/f.go", "@src"),
            ("a.rs", "@all"),
            ("src/b.rs", "@rs"),
            ("src/b.rs.bak", "@src"),
            ("src/api/x.proto.md", "@docs"),
            ("docs/internal", "@docs-team"),
            ("docs/internals/x", "@docs-team"),
            ("one/two/three/four/five.txt", "@all"),
            ("build/vendor", "@all"),
            ("build/vendor/x", "@vend"),
            ("src/api/a.proto", "@proto"),
            ("src/api/a.proto/b", "@api"),
            ("internal/docs/x.md", "@docs"),
            ("docs/x.md", "@docs-team"),
            ("zzz", "@all"),
        ]
        .iter()
        .map(|(p, o)| ((*p).to_owned(), *o))
        .collect();
        assert!(cases.len() >= 50);
        for (path, want) in &cases {
            // A path is a file; the cases naming a bare folder test that a folder isn't owned as
            // a file unless a pattern names it.
            assert_eq!(who(&r, path), *want, "{path}");
        }
    }

    #[test]
    fn a_line_without_owners_unowns_and_bad_lines_are_skipped() {
        let r = Rules::parse("* @a\n/x/\n/y/ <script>alert(1)</script>\n/z/ @b @ok\n");
        assert_eq!(who(&r, "x/f"), "-");
        // The bad owner drops the whole line, so /y/ stays with `*`.
        assert_eq!(who(&r, "y/f"), "@a");
        assert_eq!(who(&r, "z/f"), "@b @ok");
    }

    #[test]
    fn owner_strings_are_validated_and_capped() {
        assert_eq!(parse_owner("@org/team").unwrap().kind, "team");
        assert_eq!(parse_owner("@user").unwrap().kind, "person");
        assert_eq!(parse_owner("a.b+c@example.com").unwrap().kind, "email");
        for bad in [
            "@",
            "@/x",
            "@a/",
            "@a/b/c",
            "x",
            "a@b",
            "<b>@x.io",
            "@a\u{7}",
            "a@b.c\u{202e}",
            "@a;drop",
            "\"@a\"",
        ] {
            assert!(parse_owner(bad).is_none(), "{bad:?}");
        }
        let long = format!("@{}", "a".repeat(MAX_OWNER));
        assert!(parse_owner(&long).is_none());
        let r = Rules::parse(&format!("* {long}\n"));
        assert!(r.lines().is_empty() || r.owners_of("x").is_none());
    }

    #[test]
    fn the_pattern_count_is_capped() {
        let text: String = (0..MAX_PATTERNS + 50)
            .map(|i| format!("/f{i}.txt @o\n"))
            .collect();
        let r = Rules::parse(&text);
        assert_eq!(r.lines().len(), MAX_PATTERNS);
        assert!(r.owners_of("f0.txt").is_some());
        assert!(r.owners_of(&format!("f{}.txt", MAX_PATTERNS + 1)).is_none());
    }

    proptest! {
        /// Appending a catch-all as the last line makes it the owner of every path.
        #[test]
        fn a_final_catch_all_owns_everything(
            lines in proptest::collection::vec("[a-z*/.]{1,12} @[a-z]{1,6}", 0..12),
            path in "[a-z./]{1,24}",
        ) {
            let mut text = lines.join("\n");
            text.push_str("\n* @everyone\n");
            let r = Rules::parse(&text);
            prop_assert_eq!(who(&r, &path), "@everyone");
        }
    }
}
