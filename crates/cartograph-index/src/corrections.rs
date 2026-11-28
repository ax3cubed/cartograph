//! Corrections (XS-12): the entries of `cartograph.overrides.yaml` bound to the index and applied
//! to the links the HTTP pass wrote.
//!
//! **Binding.** An end (`service`, `service:path`, `service:path#symbol`) resolves to nodes as of
//! the entry's anchor: the commit that added the entry to the file, found by `git blame` (an entry
//! that isn't committed anchors at the tip), then at the tip when the anchor finds nothing. A
//! file's node is the same node under every path it had, so a binding made before a rename holds
//! after it. Links are found by the natural key (`kind`, `src`, `dst`) through the indexes on it,
//! never by a scan; `#symbol` is accepted and ignored, because links join files.
//!
//! **Apply.** After the HTTP pass and before the derived rows. `remove` moves the link's intervals
//! to `correction_interval`, `retarget` rewrites its `dst`, `set` fixes its level to the midpoint of
//! the band, `add` opens a link over the time both files exist. Each changed link gets
//! `layer = 'correction'` and its base values in `correction_base`; every apply starts by undoing
//! the last one, so the result is a function of the base links and the file (idempotent), and
//! deleting an entry restores its link. Only `http` links are corrected: the pass that writes them
//! rebuilds them whole, which is what lets the layer sit on top.
//!
//! **Cost.** The inputs key is the file's hash, the HTTP pass's key and, while some entry isn't
//! committed, the tip. An unchanged file and unchanged links cost one key lookup and no `git`.
//!
//! The file is untrusted (F8): its text is only ever stored as data, capped, and the index holds no
//! path it names.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::{Command, Stdio};

use cartograph_discovery::overrides::{self, Action, Entry, Kind, Level, Parsed, Stale, split_end};
use cartograph_store::derived::{changed, record, stored};
use cartograph_store::rusqlite::{self, Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::derive::key;
use crate::rows::cut;

/// Bump when what the rows hold changes, so every index rebuilds them.
const VERSION: &str = "corrections 1";
/// `meta`: 1 while some entry isn't committed, so its anchor (the tip) moves with the tip.
const PENDING: &str = "corrections_pending";
/// `meta`: the file's `stale:` mode as of the last apply.
const MODE: &str = "corrections_stale";
/// Nodes one end may stand for (a directory).
const MAX_NODES: i64 = 10_000;

/// Who added a line and when, from `git blame`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blame {
    /// The commit, hex; `None` for a line that isn't committed.
    pub sha: Option<String>,
    pub by: Option<String>,
    pub at: Option<i64>,
}

/// One entry as the index holds it, for `check` and the API.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: i64,
    pub line: i64,
    pub action: String,
    pub kind: String,
    /// `from -> to op`: the key as written, untrusted text.
    pub key: String,
    /// bound, stale or ambiguous.
    pub status: String,
    pub links: i64,
    /// Untrusted text.
    pub reason: String,
    /// Untrusted text.
    pub by: Option<String>,
    pub at: Option<i64>,
}

/// The entries, in file order.
pub fn rows(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    conn.prepare(
        "SELECT id, line, action, kind, key, status, links, reason, by, at FROM correction
         ORDER BY id",
    )?
    .query_map([], |r| {
        Ok(Row {
            id: r.get(0)?,
            line: r.get(1)?,
            action: r.get(2)?,
            kind: r.get(3)?,
            key: r.get(4)?,
            status: r.get(5)?,
            links: r.get(6)?,
            reason: r.get(7)?,
            by: r.get(8)?,
            at: r.get(9)?,
        })
    })?
    .collect()
}

/// The file's `stale:` mode as of the last apply.
pub fn stale_mode(conn: &Connection) -> rusqlite::Result<Stale> {
    let v: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [MODE], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(match v.as_deref() {
        Some("warn") => Stale::Warn,
        Some("off") => Stale::Off,
        _ => Stale::Error,
    })
}

/// Reads the overrides file and applies it, when it or the links changed since the last run.
/// Returns the entries applied.
pub(crate) fn refresh(conn: &Connection, path: Option<&Path>, tip: i64) -> rusqlite::Result<usize> {
    let read = path.and_then(|p| match overrides::read(p) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("can't read {}: {e}", overrides::FILE_NAME);
            None
        }
    });
    let pending: i64 = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [PENDING], |r| {
            r.get(0)
        })
        .optional()?
        .unwrap_or(0);
    let mut inputs = vec![match &read {
        None => b"absent".to_vec(),
        Some((text, _)) => Sha256::digest(text.as_bytes()).to_vec(),
    }];
    inputs.push(stored(conn, "http", "workspace")?.unwrap_or_default());
    if pending != 0 {
        inputs.push(tip.to_string().into_bytes());
    }
    let k = key("corrections", VERSION, inputs, None);
    if !changed(conn, "corrections", "workspace", &k)? {
        return Ok(0);
    }
    let mut n = 0;
    let mut pending = 0;
    let mut mode = Stale::Error;
    if let (Some(path), Some((text, Ok(p)))) = (path, &read) {
        // The blame of each entry is the blame of the line it starts on.
        let per_line = blame(path, text.lines().count());
        let blames: Vec<Blame> = p
            .lines
            .iter()
            .map(|l| per_line.get(*l as usize - 1).cloned().unwrap_or_default())
            .collect();
        pending = i64::from(blames.iter().any(|b| b.sha.is_none()));
        mode = p.overrides.stale;
        n = apply(conn, p, &blames, tip)?;
    } else {
        if let Some((_, Err(e))) = &read {
            tracing::warn!("{} isn't applied: {e}", overrides::FILE_NAME);
        }
        // No file, or one that doesn't read: nothing is corrected (`check` says why).
        clear(conn)?;
    }
    for (k, v) in [(PENDING, pending), (MODE, mode as i64)] {
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![k, v],
        )?;
    }
    record(conn, "corrections", "workspace", &k)?;
    Ok(n)
}

/// Undoes the last apply and forgets its entries: every link is as the passes wrote it.
pub fn clear(conn: &Connection) -> rusqlite::Result<()> {
    unapply(conn)?;
    conn.execute("DELETE FROM correction", []).map(drop)
}

/// Puts every link a correction changed back as the passes wrote it, newest change first.
fn unapply(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO interval (link, from_seq, to_seq)
         SELECT link, from_seq, to_seq FROM correction_interval",
        [],
    )?;
    conn.execute("DELETE FROM correction_interval", [])?;
    type Base = (i64, i64, bool, i64, Option<String>, Option<f64>);
    let done: Vec<Base> = conn
        .prepare("SELECT id, link, added, dst, level, score FROM correction_base ORDER BY id DESC")?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    for (id, link, added, dst, level, score) in done {
        conn.prepare_cached("DELETE FROM correction_base WHERE id = ?1")?
            .execute([id])?;
        if added {
            conn.prepare_cached("DELETE FROM interval WHERE link = ?1")?
                .execute([link])?;
            conn.prepare_cached("DELETE FROM link WHERE id = ?1")?
                .execute([link])?;
        } else {
            conn.prepare_cached(
                "UPDATE link SET dst = ?2, level = ?3, score = ?4, layer = NULL WHERE id = ?1",
            )?
            .execute(params![link, dst, level, score])?;
        }
    }
    Ok(())
}

/// `git blame` of the file, one [`Blame`] per line; empty when git can't say (not a work tree, not
/// tracked), which anchors every entry at the tip.
fn blame(path: &Path, lines: usize) -> Vec<Blame> {
    let none = || vec![Blame::default(); lines];
    if lines == 0 {
        return Vec::new();
    }
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return none();
    };
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        // No index refresh written into the user's repo, and no fsmonitor hook run.
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "blame",
            "--line-porcelain",
            "--",
        ])
        .arg(name)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => parse_blame(&String::from_utf8_lossy(&o.stdout)),
        _ => none(),
    }
}

fn parse_blame(text: &str) -> Vec<Blame> {
    let mut out: Vec<Blame> = Vec::new();
    let mut cur = Blame::default();
    let mut line = 0usize;
    for l in text.lines() {
        if l.starts_with('\t') {
            if line > 0 {
                out.resize(line, Blame::default());
                out[line - 1] = std::mem::take(&mut cur);
            }
            continue;
        }
        let mut f = l.split(' ');
        match (f.next(), f.next(), f.next()) {
            (Some(sha), Some(_), Some(fin))
                if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                line = fin.parse().unwrap_or(0);
                cur = Blame::default();
                cur.sha = (sha.bytes().any(|b| b != b'0')).then(|| sha.to_owned());
            }
            _ => {
                if let Some(a) = l.strip_prefix("author ") {
                    cur.by = Some(cut(&a
                        .chars()
                        .filter(|c| !c.is_control())
                        .collect::<String>()));
                } else if let Some(t) = l.strip_prefix("author-time ") {
                    cur.at = t.parse().ok();
                }
            }
        }
    }
    for b in &mut out {
        if b.sha.is_none() {
            *b = Blame::default();
        }
    }
    out
}

/// What an end of a key bound to.
enum End {
    /// Nothing (an unknown service, a path that no file had).
    None,
    /// A whole service.
    Service(i64),
    /// File nodes: one file, or the files under a directory.
    Files(BTreeSet<i64>),
}

impl End {
    fn single(&self) -> Option<i64> {
        match self {
            End::Files(f) if f.len() == 1 => f.first().copied(),
            _ => None,
        }
    }
}

struct Ctx {
    /// Service and library nodes by name; the first of a name wins.
    services: HashMap<String, i64>,
    service_of: HashMap<i64, Option<i64>>,
}

impl Ctx {
    fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut services = HashMap::new();
        let mut st = conn.prepare(
            "SELECT name, id FROM node WHERE kind IN ('service', 'library') AND name IS NOT NULL
             ORDER BY id DESC",
        )?;
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (n, id) = r?;
            services.insert(n, id);
        }
        Ok(Self {
            services,
            service_of: HashMap::new(),
        })
    }

    fn holds(&mut self, conn: &Connection, end: &End, node: i64) -> rusqlite::Result<bool> {
        Ok(match end {
            End::None => false,
            End::Files(f) => f.contains(&node),
            End::Service(s) => {
                let of = match self.service_of.get(&node) {
                    Some(v) => *v,
                    None => {
                        let v = conn
                            .prepare_cached("SELECT service FROM node WHERE id = ?1")?
                            .query_row([node], |r| r.get(0))
                            .optional()?
                            .flatten();
                        self.service_of.insert(node, v);
                        v
                    }
                };
                of == Some(*s)
            }
        })
    }

    /// The nodes `spec` names as the index had them at `anchor`, else at `tip`.
    fn end(&self, conn: &Connection, spec: &str, anchor: i64, tip: i64) -> rusqlite::Result<End> {
        let (service, path, _symbol) = split_end(spec);
        let Some(&sid) = self.services.get(service) else {
            return Ok(End::None);
        };
        let Some(path) = path else {
            return Ok(End::Service(sid));
        };
        // The file; failing that, every file under it as a directory. Both walk the path index
        // (CROSS JOIN fixes the order: the path first, then its node).
        let (dir_from, dir_to) = (format!("{path}/"), format!("{path}0"));
        for seq in [anchor, tip] {
            for (lo, hi) in [(path, path), (dir_from.as_str(), dir_to.as_str())] {
                let found: BTreeSet<i64> = conn
                    .prepare_cached(
                        "SELECT n.id FROM file_path fp CROSS JOIN node n ON n.file = fp.file
                         WHERE fp.path >= ?1 AND fp.path <= ?2 AND fp.path != ?3
                           AND fp.from_seq <= ?4 AND (fp.to_seq IS NULL OR fp.to_seq > ?4)
                           AND n.kind = 'file' AND n.service = ?5 LIMIT ?6",
                    )?
                    .query_map(
                        params![lo, hi, if lo == hi { "" } else { hi }, seq, sid, MAX_NODES],
                        |r| r.get(0),
                    )?
                    .collect::<Result<_, _>>()?;
                if !found.is_empty() {
                    return Ok(End::Files(found));
                }
            }
            if anchor == tip {
                break;
            }
        }
        Ok(End::None)
    }
}

/// `GET /orders/{id}`, `get /orders/:id` and `get /orders/{}` are one operation.
fn norm_op(op: &str) -> String {
    let (method, path) = op
        .trim()
        .split_once(char::is_whitespace)
        .unwrap_or(("", op.trim()));
    let path: Vec<&str> = path
        .trim()
        .split('/')
        .map(|s| {
            if s.starts_with(':') || (s.starts_with('{') && s.ends_with('}')) {
                "{}"
            } else {
                s
            }
        })
        .collect();
    format!("{} {}", method.to_ascii_lowercase(), path.join("/"))
}

/// The links of `kind` between the ends (and carrying `op`), each with whether it is live at the tip.
fn matches(
    conn: &Connection,
    ctx: &mut Ctx,
    kind: &str,
    from: &End,
    to: &End,
    op: Option<&str>,
) -> rusqlite::Result<Vec<(i64, bool)>> {
    let mut cand: Vec<(i64, i64, i64)> = Vec::new();
    match (from, to) {
        (End::None, _) | (_, End::None) => {}
        (End::Files(f), _) => {
            for n in f {
                let mut st = conn
                    .prepare_cached("SELECT id, src, dst FROM link WHERE kind = ?1 AND src = ?2")?;
                for r in
                    st.query_map(params![kind, n], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                {
                    cand.push(r?);
                }
            }
        }
        (_, End::Files(t)) => {
            for n in t {
                let mut st = conn
                    .prepare_cached("SELECT id, src, dst FROM link WHERE kind = ?1 AND dst = ?2")?;
                for r in
                    st.query_map(params![kind, n], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                {
                    cand.push(r?);
                }
            }
        }
        (End::Service(s), End::Service(_)) => {
            let mut st = conn.prepare_cached(
                "SELECT l.id, l.src, l.dst FROM node n JOIN link l ON l.src = n.id AND l.kind = ?1
                 WHERE n.service = ?2 AND n.kind = 'file'",
            )?;
            for r in st.query_map(params![kind, s], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
                cand.push(r?);
            }
        }
    }
    let want = op.map(norm_op);
    let mut out = Vec::new();
    for (id, src, dst) in cand {
        if !(ctx.holds(conn, from, src)? && ctx.holds(conn, to, dst)?) {
            continue;
        }
        if let Some(w) = &want {
            let mut st = conn.prepare_cached(
                "SELECT operation FROM evidence WHERE link = ?1 AND operation IS NOT NULL",
            )?;
            let mut ops = st.query_map([id], |r| r.get::<_, String>(0))?;
            if !ops.any(|o| o.is_ok_and(|o| norm_op(&o) == *w)) {
                continue;
            }
        }
        let live: bool = conn
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM interval WHERE link = ?1 AND to_seq IS NULL)",
            )?
            .query_row([id], |r| r.get(0))?;
        out.push((id, live));
    }
    Ok(out)
}

/// The steps a file node has a path, joined across renames.
fn spans(conn: &Connection, node: i64) -> rusqlite::Result<Vec<(i64, Option<i64>)>> {
    let mut v: Vec<(i64, Option<i64>)> = conn
        .prepare_cached(
            "SELECT fp.from_seq, fp.to_seq FROM file_path fp JOIN node n ON n.file = fp.file
             WHERE n.id = ?1 ORDER BY fp.from_seq",
        )?
        .query_map([node], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut out: Vec<(i64, Option<i64>)> = Vec::new();
    for (f, t) in v.drain(..) {
        match out.last_mut() {
            Some(last) if last.1.is_none_or(|e| f <= e) => {
                last.1 = match (last.1, t) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
            }
            _ => out.push((f, t)),
        }
    }
    Ok(out)
}

/// Where two lists of spans overlap.
fn overlap(a: &[(i64, Option<i64>)], b: &[(i64, Option<i64>)]) -> Vec<(i64, Option<i64>)> {
    let mut out = Vec::new();
    for (af, at) in a {
        for (bf, bt) in b {
            let from = *af.max(bf);
            let to = match (at, bt) {
                (Some(x), Some(y)) => Some(*x.min(y)),
                (Some(x), None) | (None, Some(x)) => Some(*x),
                (None, None) => None,
            };
            if to.is_none_or(|t| from < t) {
                out.push((from, to));
            }
        }
    }
    out
}

/// Keeps the link's base values the first time an entry changes it.
fn stash(conn: &Connection, link: i64, entry: i64, added: bool) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT OR IGNORE INTO correction_base (link, correction, added, dst, level, score)
         SELECT id, ?2, ?3, dst, level, score FROM link WHERE id = ?1",
    )?
    .execute(params![link, entry, added])
    .map(drop)
}

fn hide(conn: &Connection, link: i64, entry: i64) -> rusqlite::Result<()> {
    stash(conn, link, entry, false)?;
    conn.prepare_cached(
        "INSERT INTO correction_interval (link, from_seq, to_seq)
         SELECT link, from_seq, to_seq FROM interval WHERE link = ?1",
    )?
    .execute([link])?;
    conn.prepare_cached("DELETE FROM interval WHERE link = ?1")?
        .execute([link])?;
    conn.prepare_cached("UPDATE link SET layer = 'correction' WHERE id = ?1")?
        .execute([link])
        .map(drop)
}

fn fix(conn: &Connection, link: i64, entry: i64, level: Level) -> rusqlite::Result<()> {
    stash(conn, link, entry, false)?;
    conn.prepare_cached(
        "UPDATE link SET level = ?2, score = ?3, layer = 'correction' WHERE id = ?1",
    )?
    .execute(params![link, level.as_str(), level.score()])
    .map(drop)
}

/// Binds the entries and applies them, in file order, to the links as the passes wrote them (the
/// last apply is undone first, so applying twice is applying once). `blames` is one per entry (a
/// missing one anchors at the tip). Returns how many are bound.
pub fn apply(
    conn: &Connection,
    parsed: &Parsed,
    blames: &[Blame],
    tip: i64,
) -> rusqlite::Result<usize> {
    clear(conn)?;
    let mut ctx = Ctx::load(conn)?;
    let mut bound = 0;
    for (i, e) in parsed.overrides.corrections.iter().enumerate() {
        let id = i as i64;
        let b = blames.get(i).cloned().unwrap_or_default();
        let anchor = match b.sha.as_deref().and_then(unhex) {
            Some(sha) => conn
                .prepare_cached("SELECT seq FROM commits WHERE sha = ?1 LIMIT 1")?
                .query_row([sha], |r| r.get(0))
                .optional()?
                .unwrap_or(tip),
            None => tip,
        };
        let kind = match e.key.kind {
            Kind::Http => Some("http"),
            // Only the HTTP pass writes links that a pass rebuilds whole (see the module doc).
            _ => None,
        };
        let from = ctx.end(conn, &e.key.from, anchor, tip)?;
        let to = ctx.end(conn, &e.key.to, anchor, tip)?;
        let new = match &e.to {
            Some(t) => Some(ctx.end(conn, t, anchor, tip)?),
            None => None,
        };
        let op = e.key.op.as_deref();
        let mut hits = Vec::new();
        let mut status = "bound";
        let mut add_spans = Vec::new();
        match (e.action, kind) {
            (_, None) => status = "stale",
            (Action::Add, Some(_)) => {
                add_spans = match (from.single(), to.single()) {
                    (Some(a), Some(b)) => overlap(&spans(conn, a)?, &spans(conn, b)?),
                    _ => Vec::new(),
                };
                if matches!(from, End::Files(ref f) if f.len() > 1)
                    || matches!(to, End::Files(ref f) if f.len() > 1)
                {
                    status = "ambiguous";
                } else if add_spans.is_empty() || !add_spans.iter().any(|s| s.1.is_none()) {
                    status = "stale";
                }
            }
            (a, Some(k)) => {
                hits = matches(conn, &mut ctx, k, &from, &to, op)?;
                if !hits.iter().any(|h| h.1) {
                    status = "stale";
                } else if a == Action::Retarget
                    && new
                        .as_ref()
                        .is_some_and(|n| matches!(n, End::Files(f) if f.len() > 1))
                {
                    status = "ambiguous";
                } else if a == Action::Retarget && new.as_ref().and_then(End::single).is_none() {
                    status = "stale";
                }
            }
        }
        bound += usize::from(status == "bound");
        let key_text = cut(&format!(
            "{} -> {}{}",
            e.key.from,
            e.key.to,
            e.key.carried().map(|c| format!(" {c}")).unwrap_or_default()
        ));
        let n_links = if e.action == Action::Add {
            i64::from(status == "bound")
        } else {
            hits.len() as i64
        };
        conn.prepare_cached(
            "INSERT INTO correction (id, line, action, kind, key, src, dst, new_dst, confidence,
               reason, anchor_seq, status, links, by, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?
        .execute(params![
            id,
            parsed.lines.get(i).copied().unwrap_or(0),
            e.action.as_str(),
            e.key.kind.as_str(),
            key_text,
            from.single(),
            to.single(),
            new.as_ref().and_then(End::single),
            e.confidence.map(Level::as_str),
            e.reason,
            anchor,
            status,
            n_links,
            b.by,
            b.at,
        ])?;
        if status == "ambiguous" || kind.is_none() {
            continue;
        }
        change(
            conn,
            id,
            e,
            &hits,
            new.as_ref().and_then(End::single),
            from.single().zip(to.single()),
            &add_spans,
        )?;
    }
    Ok(bound)
}

fn change(
    conn: &Connection,
    id: i64,
    e: &Entry,
    hits: &[(i64, bool)],
    new_dst: Option<i64>,
    pair: Option<(i64, i64)>,
    add_spans: &[(i64, Option<i64>)],
) -> rusqlite::Result<()> {
    match e.action {
        Action::Remove => {
            for (l, _) in hits {
                hide(conn, *l, id)?;
            }
        }
        Action::Set => {
            for (l, _) in hits {
                fix(conn, *l, id, e.confidence.unwrap_or(Level::Likely))?;
            }
        }
        Action::Retarget => {
            let Some(new) = new_dst else { return Ok(()) };
            for (l, _) in hits {
                let src: i64 = conn
                    .prepare_cached("SELECT src FROM link WHERE id = ?1")?
                    .query_row([l], |r| r.get(0))?;
                let taken: Option<i64> = conn
                    .prepare_cached(
                        "SELECT id FROM link WHERE kind = 'http' AND src = ?1 AND dst = ?2",
                    )?
                    .query_row(params![src, new], |r| r.get(0))
                    .optional()?;
                if taken.is_some() {
                    // The link the far end already has stands for it.
                    hide(conn, *l, id)?;
                } else {
                    stash(conn, *l, id, false)?;
                    conn.prepare_cached(
                        "UPDATE link SET dst = ?2, layer = 'correction' WHERE id = ?1",
                    )?
                    .execute(params![l, new])?;
                }
            }
        }
        Action::Add => {
            let (Some((a, b)), false) = (pair, add_spans.is_empty()) else {
                return Ok(());
            };
            let level = e.confidence.unwrap_or(Level::Likely);
            let made = conn
                .prepare_cached(
                    "INSERT INTO link (kind, src, dst, level, score, layer)
                     VALUES ('http', ?1, ?2, ?3, ?4, 'correction') ON CONFLICT DO NOTHING",
                )?
                .execute(params![a, b, level.as_str(), level.score()])?;
            let link: i64 = conn
                .prepare_cached(
                    "SELECT id FROM link WHERE kind = 'http' AND src = ?1 AND dst = ?2",
                )?
                .query_row(params![a, b], |r| r.get(0))?;
            if made == 0 {
                fix(conn, link, id, level)?;
            } else {
                stash(conn, link, id, true)?;
                for (f, t) in add_spans {
                    conn.prepare_cached(
                        "INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)",
                    )?
                    .execute(params![link, f, t])?;
                }
            }
        }
    }
    Ok(())
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    s.len()
        .is_multiple_of(2)
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_compare_by_shape() {
        assert_eq!(norm_op("GET /orders/{id}"), norm_op("get /orders/:id"));
        assert_eq!(norm_op("GET /orders/{id}"), "get /orders/{}");
        assert_ne!(norm_op("GET /a"), norm_op("POST /a"));
    }

    #[test]
    fn blame_is_read_per_line() {
        let sha = "a".repeat(40);
        let text = format!(
            "{sha} 1 1 1\nauthor Ada L\nauthor-time 100\nsummary x\n\tversion: 1\n{} 2 2 1\nauthor Not Committed Yet\nauthor-time 5\n\tmore\n",
            "0".repeat(40)
        );
        let b = parse_blame(&text);
        assert_eq!(
            b[0],
            Blame {
                sha: Some(sha),
                by: Some("Ada L".into()),
                at: Some(100)
            }
        );
        assert_eq!(b[1], Blame::default());
    }
}
