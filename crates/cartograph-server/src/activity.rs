//! FE-16's activity store: rows built from what each sync added to the index, kept in the
//! workspace's `ask.db` (I5: "jobs and activity in sibling tables"), which survives a rebuild.
//!
//! Whoever ran a sync records it when the run ends: a server job, MCP, the CLI. A run is recorded
//! as one diff of each repo's indexed commit against the commit last recorded for it, so a burst of
//! batches and rounds is one row per repo per run. The server's ticker records what nobody else did
//! (a writer that predates this store) once the writer lock is free, and sends every row it hasn't
//! sent yet as `activity.new`. Rows hold repo and author names, never commit subjects or file
//! contents. The last 500 rows per workspace are kept.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cartograph_api::types::{ActivityItem, ActivityKind, Actor, JobKind, JobState, Problem};
use cartograph_store::rusqlite::{self, Connection, OpenFlags, OptionalExtension, params};

/// Rows kept per workspace.
pub const KEEP: i64 = 500;
/// Rows per page of `GET /api/v1/activity`.
pub const PAGE: usize = 50;
/// Untrusted text is cut to this many characters.
pub const MAX_TEXT: usize = 120;

const FILE: &str = "ask.db";
/// "CASK": ask.db's own application id, not the index's.
const APPLICATION_ID: i64 = 0x4341_534b;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS activity (
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  kind TEXT NOT NULL,
  text TEXT NOT NULL,
  detail TEXT NOT NULL,
  href TEXT NOT NULL,
  unread INTEGER NOT NULL DEFAULT 1
) STRICT;
-- The indexed commit last recorded for each repo, and its newest rewrite recorded.
CREATE TABLE IF NOT EXISTS activity_mark (
  repo TEXT PRIMARY KEY,
  sha BLOB NOT NULL,
  rewrite_at INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
";

fn ws_dir(data: &Path, ws: &str) -> PathBuf {
    cartograph_core::paths::workspace_dir(data, ws)
}

/// Opens (and creates) the workspace's ask.db. `None` when the workspace folder doesn't exist.
fn open(data: &Path, ws: &str) -> rusqlite::Result<Option<Connection>> {
    let dir = ws_dir(data, ws);
    if !dir.is_dir() {
        return Ok(None);
    }
    let c = Connection::open(dir.join(FILE))?;
    c.busy_timeout(Duration::from_secs(5))?;
    let id: i64 = c.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    if id == 0 {
        c.execute_batch(&format!(
            "PRAGMA journal_mode = WAL; PRAGMA application_id = {APPLICATION_ID};"
        ))?;
    }
    c.execute_batch(SCHEMA)?;
    Ok(Some(c))
}

/// Opens ask.db only if it exists: a read never creates one.
fn open_existing(data: &Path, ws: &str) -> rusqlite::Result<Option<Connection>> {
    if !ws_dir(data, ws).join(FILE).is_file() {
        return Ok(None);
    }
    open(data, ws)
}

/// Cuts untrusted text at [`MAX_TEXT`] characters.
pub fn cut(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT {
        return s.to_owned();
    }
    let mut t: String = s.chars().take(MAX_TEXT - 1).collect();
    t.push('…');
    t
}

fn who(by: Option<Actor>) -> &'static str {
    match by {
        Some(Actor::App) => "from the app",
        Some(Actor::Cli) => "from the CLI",
        Some(Actor::Mcp) => "by an agent over MCP",
        Some(Actor::Hook) => "by a git hook",
        Some(Actor::Background) => "by the background sync",
        None => "outside the app",
    }
}

fn detail(verb: &str, by: Option<Actor>) -> String {
    format!(
        "{verb} {} · cartograph {}",
        who(by),
        env!("CARGO_PKG_VERSION")
    )
}

fn kind_str(k: ActivityKind) -> &'static str {
    match k {
        ActivityKind::Index => "index",
        ActivityKind::Sync => "sync",
        ActivityKind::Correction => "correction",
        ActivityKind::Warning => "warning",
    }
}

fn kind_of(s: &str) -> ActivityKind {
    match s {
        "index" => ActivityKind::Index,
        "sync" => ActivityKind::Sync,
        "correction" => ActivityKind::Correction,
        _ => ActivityKind::Warning,
    }
}

fn hex7(sha: &[u8]) -> String {
    sha.iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..7]
        .to_owned()
}

/// "3 commits by A and B", "1 commit by A", "9 commits by A, B and 4 others". Authors in the
/// order of their newest commit.
fn commits_by(authors: &[String]) -> String {
    let n = authors.len();
    let mut names: Vec<&str> = Vec::new();
    for a in authors {
        if !names.contains(&a.as_str()) {
            names.push(a);
        }
    }
    let by = match names.as_slice() {
        [] => String::new(),
        [a] => format!(" by {a}"),
        [a, b] => format!(" by {a} and {b}"),
        [a, b, c] => format!(" by {a}, {b} and {c}"),
        [a, b, rest @ ..] => format!(" by {a}, {b} and {} others", rest.len()),
    };
    format!("{n} commit{}{by}", if n == 1 { "" } else { "s" })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

struct Row {
    kind: ActivityKind,
    text: String,
    detail: String,
    href: String,
}

fn insert(c: &Connection, rows: &[Row]) -> rusqlite::Result<()> {
    let at = now();
    for r in rows {
        c.execute(
            "INSERT INTO activity (at, kind, text, detail, href) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at, kind_str(r.kind), cut(&r.text), cut(&r.detail), r.href],
        )?;
    }
    c.execute(
        "DELETE FROM activity WHERE id NOT IN (SELECT id FROM activity ORDER BY id DESC LIMIT ?1)",
        [KEEP],
    )?;
    Ok(())
}

fn map_href(ws: &str) -> String {
    format!("/#ws={ws}")
}

/// Records what the index gained since the last record: per repo, one sync row (or an index row
/// for a repo first seen, or every repo after a rebuild) and a row per rewritten history. Returns
/// the number of rows added. Call it when a run has ended, never while its writer holds the lock.
pub fn record(data: &Path, ws: &str, by: Option<Actor>, rebuilt: bool) -> rusqlite::Result<usize> {
    let index = ws_dir(data, ws).join("index.cartograph");
    if !index.is_file() {
        return Ok(0);
    }
    let Some(mut c) = open(data, ws)? else {
        return Ok(0);
    };
    let ix = Connection::open_with_flags(
        &index,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    ix.busy_timeout(Duration::from_secs(5))?;
    // IMMEDIATE: the CLI and the server may both record the same run; the second sees the marks.
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut rows = Vec::new();
    let repos: Vec<(i64, String, Vec<u8>, i64)> = ix
        .prepare(
            "SELECT id, name, indexed_commit, indexed_seq FROM repo
             WHERE removed = 0 AND indexed_commit IS NOT NULL ORDER BY name",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    for (id, name, sha, seq) in repos {
        let mark: Option<(Vec<u8>, i64)> = tx
            .query_row(
                "SELECT sha, rewrite_at FROM activity_mark WHERE repo = ?1",
                [&name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let authors = |from: i64| -> rusqlite::Result<Vec<String>> {
            ix.prepare_cached(
                "SELECT author FROM commits WHERE repo = ?1 AND seq > ?2 AND seq <= ?3
                 ORDER BY seq DESC",
            )?
            .query_map(params![id, from, seq], |r| r.get(0))?
            .collect()
        };
        let newest_rewrite: i64 = ix.query_row(
            "SELECT coalesce(max(recorded_at), 0) FROM rewrite WHERE repo = ?1",
            [id],
            |r| r.get(0),
        )?;
        match &mark {
            Some((old, _)) if *old == sha && !rebuilt => {}
            Some(_) if !rebuilt => {
                let from: Option<i64> = ix
                    .query_row(
                        "SELECT seq FROM commits WHERE repo = ?1 AND sha = ?2",
                        params![id, mark.as_ref().map(|m| &m.0)],
                        |r| r.get(0),
                    )
                    .optional()?;
                // An old tip no longer in the index (a rewrite replayed past it): no count.
                let count = match from {
                    Some(f) => format!(" · {}", commits_by(&authors(f)?)),
                    None => String::new(),
                };
                rows.push(Row {
                    kind: ActivityKind::Sync,
                    text: format!("{name} synced to {}{count}", hex7(&sha)),
                    detail: detail("Indexed", by),
                    href: map_href(ws),
                });
            }
            _ => {
                let n: i64 = ix.query_row(
                    "SELECT count(*) FROM commits WHERE repo = ?1 AND seq <= ?2",
                    params![id, seq],
                    |r| r.get(0),
                )?;
                let verb = if rebuilt { "rebuilt" } else { "indexed" };
                rows.push(Row {
                    kind: ActivityKind::Index,
                    text: format!(
                        "{name} {verb} at {} · {n} commit{}",
                        hex7(&sha),
                        if n == 1 { "" } else { "s" }
                    ),
                    detail: detail("Indexed", by),
                    href: map_href(ws),
                });
            }
        }
        // Rewrites recorded since the last look; a repo first seen starts from its newest.
        if let Some((_, seen)) = &mark {
            let rewrites: Vec<(Option<Vec<u8>>, i64)> = ix
                .prepare_cached(
                    "SELECT base, archived FROM rewrite WHERE repo = ?1 AND recorded_at > ?2
                     ORDER BY recorded_at",
                )?
                .query_map(params![id, seen], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            for (base, archived) in rewrites {
                let from = base.map_or_else(
                    || "with no common commit".to_owned(),
                    |b| format!("replayed from {}", hex7(&b)),
                );
                rows.push(Row {
                    kind: ActivityKind::Warning,
                    text: format!(
                        "{name}: history was rewritten, {from} · {archived} links archived"
                    ),
                    detail: detail("Found", by),
                    href: map_href(ws),
                });
            }
        }
        tx.execute(
            "INSERT INTO activity_mark (repo, sha, rewrite_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (repo) DO UPDATE SET sha = ?2, rewrite_at = max(rewrite_at, ?3)",
            params![name, sha, newest_rewrite],
        )?;
    }
    insert(&tx, &rows)?;
    tx.commit()?;
    Ok(rows.len())
}

/// Records a run that failed: one warning row with the run's own explanation.
pub fn record_failure(
    data: &Path,
    ws: &str,
    by: Option<Actor>,
    what: &str,
    p: &Problem,
) -> rusqlite::Result<()> {
    let Some(c) = open(data, ws)? else {
        return Ok(());
    };
    let name = cartograph_discovery::Registry::load(data)
        .ok()
        .and_then(|(r, _)| r.workspaces.into_iter().find(|w| w.id == ws))
        .map_or_else(|| ws.to_owned(), |w| w.name);
    insert(
        &c,
        &[Row {
            kind: ActivityKind::Warning,
            text: format!("{name}: the {what} failed. {}", p.what),
            detail: detail("Run", by),
            href: "/settings/indexing".into(),
        }],
    )
}

/// Records a job's end: what it indexed (a cancelled or failed sync keeps the batches it wrote),
/// and a warning row when it failed. Errors are logged; activity never fails a run.
pub fn after_run(
    data: &Path,
    ws: &str,
    kind: JobKind,
    by: Actor,
    state: JobState,
    problem: Option<&Problem>,
) {
    let rebuilt = matches!(kind, JobKind::Rebuild) && state == JobState::Done;
    let mut r = record(data, ws, Some(by), rebuilt).map(|_| ());
    if let (JobState::Failed, Some(p)) = (state, problem) {
        let what = if matches!(kind, JobKind::Rebuild) {
            "rebuild"
        } else {
            "sync"
        };
        r = r.and(record_failure(data, ws, Some(by), what, p));
    }
    if let Err(e) = r {
        tracing::warn!(ws, error = %e, "can't record activity");
    }
}

/// Records a `cartograph sync` that ended (the hooks' background syncs run the same command).
pub fn after_cli_sync(data: &Path, ws: &str) {
    if let Err(e) = record(data, ws, Some(Actor::Cli), false) {
        tracing::warn!(ws, error = %e, "can't record activity");
    }
}

fn item(ws: &str, r: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityItem> {
    let id: i64 = r.get(0)?;
    let kind: String = r.get(2)?;
    Ok(ActivityItem {
        id: format!("{ws}:{id}"),
        ws: ws.to_owned(),
        at: cartograph_query::rfc3339(r.get(1)?),
        kind: kind_of(&kind),
        text: r.get(3)?,
        detail: r.get(4)?,
        href: r.get(5)?,
        unread: r.get::<_, i64>(6)? != 0,
    })
}

const COLUMNS: &str = "SELECT id, at, kind, text, detail, href, unread FROM activity";

/// The workspace's rows, newest first.
pub fn list(data: &Path, ws: &str) -> rusqlite::Result<Vec<ActivityItem>> {
    let Some(c) = open_existing(data, ws)? else {
        return Ok(Vec::new());
    };
    c.prepare(&format!("{COLUMNS} ORDER BY id DESC"))?
        .query_map([], |r| item(ws, r))?
        .collect()
}

/// Rows after row `after`, oldest first, and the newest row id (`after` when there are none).
pub fn since(data: &Path, ws: &str, after: i64) -> rusqlite::Result<(Vec<ActivityItem>, i64)> {
    let Some(c) = open_existing(data, ws)? else {
        return Ok((Vec::new(), after));
    };
    let rows: Vec<ActivityItem> = c
        .prepare(&format!("{COLUMNS} WHERE id > ?1 ORDER BY id"))?
        .query_map([after], |r| item(ws, r))?
        .collect::<Result<_, _>>()?;
    let last = c.query_row("SELECT coalesce(max(id), ?1) FROM activity", [after], |r| {
        r.get(0)
    })?;
    Ok((rows, last))
}

/// Marks rows read: the given row numbers, or every row when `ids` is `None`.
pub fn mark_read(data: &Path, ws: &str, ids: Option<&[i64]>) -> rusqlite::Result<()> {
    let Some(c) = open_existing(data, ws)? else {
        return Ok(());
    };
    match ids {
        None => {
            c.execute("UPDATE activity SET unread = 0 WHERE unread = 1", [])?;
        }
        Some(ids) => {
            let mut st = c.prepare("UPDATE activity SET unread = 0 WHERE id = ?1")?;
            for id in ids {
                st.execute([id])?;
            }
        }
    }
    Ok(())
}

/// Splits a row id, `<workspace>:<row>`.
pub fn parse_id(id: &str) -> Option<(&str, i64)> {
    let (ws, n) = id.rsplit_once(':')?;
    Some((ws, n.parse().ok()?))
}

// ---------------------------------------------------------------- routes

#[derive(serde::Deserialize)]
pub(crate) struct ListParams {
    limit: Option<usize>,
    cursor: Option<String>,
}

/// `GET /api/v1/activity`: every workspace's rows, newest first, [`PAGE`] at a time. The cursor
/// is the number of rows already sent.
pub(crate) async fn get_activity(
    axum::extract::State(app): axum::extract::State<std::sync::Arc<crate::routes::App>>,
    q: Result<axum::extract::Query<ListParams>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    let Ok(axum::extract::Query(p)) = q else {
        return crate::routes::error(&cartograph_api::types::ApiError::new(
            cartograph_api::types::ErrorCode::BadRequest,
            "the query string is malformed",
            "limit is a number and cursor is the one the previous page gave",
            "leave both out for the first page",
        ));
    };
    let skip = p.cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
    let limit = p.limit.unwrap_or(PAGE).clamp(1, PAGE);
    let rows = tokio::task::spawn_blocking(move || {
        let mut all: Vec<ActivityItem> = app
            .workspace_ids()
            .iter()
            .flat_map(|ws| list(&app.data_dir, ws).unwrap_or_default())
            .collect();
        all.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| row_of(b).cmp(&row_of(a))));
        all
    })
    .await
    .unwrap_or_default();
    let page: Vec<ActivityItem> = rows.iter().skip(skip).take(limit).cloned().collect();
    let next = (skip + page.len() < rows.len()).then(|| (skip + page.len()).to_string());
    let mut env = cartograph_api::types::Envelope::new(0, page);
    env.next_cursor = next;
    crate::routes::json(axum::http::StatusCode::OK, &env)
}

fn row_of(a: &ActivityItem) -> i64 {
    parse_id(&a.id).map_or(0, |(_, n)| n)
}

/// `POST /api/v1/activity/read`: marks rows read on the server, so every tab agrees, and tells
/// the other tabs with `activity.read`.
pub(crate) async fn post_read(
    axum::extract::State(app): axum::extract::State<std::sync::Arc<crate::routes::App>>,
    bytes: axum::body::Bytes,
) -> axum::response::Response {
    let req: cartograph_api::types::ActivityRead = match crate::routes::body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let a = app.clone();
    let ids = req.ids.clone();
    let done = tokio::task::spawn_blocking(move || {
        let known = a.workspace_ids();
        if ids.is_empty() {
            for ws in &known {
                mark_read(&a.data_dir, ws, None)?;
            }
            return Ok(());
        }
        let mut by_ws: std::collections::BTreeMap<&str, Vec<i64>> = Default::default();
        // Ids of a workspace that isn't registered, or that don't parse, are ignored.
        for (ws, n) in ids.iter().filter_map(|i| parse_id(i)) {
            if known.iter().any(|k| k == ws) {
                by_ws.entry(ws).or_default().push(n);
            }
        }
        for (ws, ns) in by_ws {
            mark_read(&a.data_dir, ws, Some(&ns))?;
        }
        Ok::<_, rusqlite::Error>(())
    })
    .await;
    if !matches!(done, Ok(Ok(()))) {
        return crate::routes::error(&cartograph_api::types::ApiError::new(
            cartograph_api::types::ErrorCode::Internal,
            "can't save the read state",
            "the workspace's ask.db couldn't be written",
            "check the data folder's permissions and free space",
        ));
    }
    app.live
        .emit(cartograph_api::events::ServerEvent::ActivityRead { ids: req.ids });
    let mut res = axum::response::Response::new(axum::body::Body::empty());
    *res.status_mut() = axum::http::StatusCode::NO_CONTENT;
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use cartograph_git::fixture::Fixture;

    /// A registered one-repo workspace, synced once; returns (tempdir, data, ws, fixture).
    fn workspace() -> (tempfile::TempDir, PathBuf, String, Fixture) {
        let d = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let mut f = Fixture::init(&root.join("billing-service"));
        f.write("src/a.ts", "export const a = 1;\n");
        f.commit("one");
        let data = root.join("data");
        let plan = cartograph_discovery::setup::plan(&f.dir, None).unwrap();
        let ws = cartograph_discovery::setup::apply(&plan, &data)
            .unwrap()
            .workspace_id;
        sync(&data, &ws);
        (d, data, ws, f)
    }

    fn sync(data: &Path, ws: &str) {
        crate::sync::run(
            data,
            ws,
            cartograph_api::types::JobKind::Sync,
            |_, _, _| {},
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    }

    #[test]
    fn a_run_is_one_row_per_repo_with_its_commits_and_authors() {
        let (_d, data, ws, mut f) = workspace();
        assert_eq!(record(&data, &ws, Some(Actor::Cli), false).unwrap(), 1);
        let first = list(&data, &ws).unwrap();
        assert_eq!(first.len(), 1);
        assert!(matches!(first[0].kind, ActivityKind::Index));
        assert!(
            first[0].text.starts_with("billing-service indexed at "),
            "{}",
            first[0].text
        );
        // Two syncs before anyone records (a burst of runs and batches): still one row.
        for (i, who) in ["Tom Ellery", "Jon Reyes", "Tom Ellery"].iter().enumerate() {
            f.author.0 = (*who).into();
            f.write(&format!("src/x{i}.ts"), "export const x = 1;\n");
            f.commit(&format!("change {i}"));
            if i == 1 {
                sync(&data, &ws);
            }
        }
        sync(&data, &ws);
        assert_eq!(
            record(&data, &ws, Some(Actor::Background), false).unwrap(),
            1
        );
        assert_eq!(record(&data, &ws, None, false).unwrap(), 0, "nothing new");
        let rows = list(&data, &ws).unwrap();
        assert_eq!(rows.len(), 2);
        let head = f.git(&["rev-parse", "--short=7", "HEAD"]);
        assert_eq!(
            rows[0].text,
            format!("billing-service synced to {head} · 3 commits by Tom Ellery and Jon Reyes")
        );
        assert!(
            rows[0]
                .detail
                .starts_with("Indexed by the background sync · cartograph "),
            "{}",
            rows[0].detail
        );
        assert_eq!(rows[0].href, format!("/#ws={ws}"));
        assert!(rows[0].unread);
        assert_eq!(rows[0].id, format!("{ws}:2"));
    }

    #[test]
    fn read_state_is_kept_per_row_and_for_all() {
        let (_d, data, ws, _f) = workspace();
        record(&data, &ws, None, false).unwrap();
        record_failure(
            &data,
            &ws,
            Some(Actor::App),
            "sync",
            &Problem {
                what: "no space left".into(),
                why: String::new(),
                action: String::new(),
            },
        )
        .unwrap();
        let rows = list(&data, &ws).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].text.ends_with(": the sync failed. no space left"));
        let (_, n) = parse_id(&rows[0].id).unwrap();
        mark_read(&data, &ws, Some(&[n])).unwrap();
        let unread: Vec<bool> = list(&data, &ws).unwrap().iter().map(|r| r.unread).collect();
        assert_eq!(unread, [false, true]);
        mark_read(&data, &ws, None).unwrap();
        assert!(list(&data, &ws).unwrap().iter().all(|r| !r.unread));
        let (new, last) = since(&data, &ws, 1).unwrap();
        assert_eq!((new.len(), last), (1, 2));
    }

    #[test]
    fn only_the_last_500_rows_are_kept() {
        let (_d, data, ws, _f) = workspace();
        let p = Problem {
            what: "x".into(),
            why: String::new(),
            action: String::new(),
        };
        for _ in 0..510 {
            record_failure(&data, &ws, None, "sync", &p).unwrap();
        }
        let rows = list(&data, &ws).unwrap();
        assert_eq!(rows.len(), 500);
        assert_eq!(rows[0].id, format!("{ws}:510"));
        assert_eq!(rows[499].id, format!("{ws}:11"));
    }

    #[test]
    fn untrusted_text_is_cut_at_120_characters() {
        let long = "é".repeat(300);
        let c = cut(&long);
        assert_eq!(c.chars().count(), 120);
        assert!(c.ends_with('…'));
        assert_eq!(cut("short"), "short");
        assert_eq!(
            commits_by(&["a".into(), "b".into(), "c".into(), "d".into()]),
            "4 commits by a, b and 2 others"
        );
        assert_eq!(commits_by(&["a".into()]), "1 commit by a");
        assert_eq!(parse_id("ws-1:42"), Some(("ws-1", 42)));
        assert_eq!(parse_id("nope"), None);
    }
}
