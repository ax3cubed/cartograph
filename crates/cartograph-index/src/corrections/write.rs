//! XS-13: the one writer of `cartograph.overrides.yaml`, behind `POST /api/v1/w/{ws}/corrections`,
//! `DELETE …/corrections/{id}` and the MCP `correct` tool.
//!
//! The file is the user's and is committed, so a write is text: an entry is appended at the end,
//! or its lines are cut out, and nothing else moves (comments, order and spelling stay). The file
//! is never parsed and written back. Every write goes through a temporary file beside it and a
//! rename, under one lock, and is checked by parsing the result with the reader the index uses.
//!
//! What goes in is data (F8). Every string is written as a double-quoted scalar, a field with a
//! control character or a line separator is refused, and the new text must parse to exactly the
//! entry asked for, so no text in a field can add a key or an entry of its own. The path is the
//! workspace's, given by the caller from the registry, never from a request.

use std::path::Path;
use std::sync::Mutex;

use cartograph_discovery::overrides::{self, Action, Entry, FILE_NAME, Kind, MAX_ENTRIES};
use cartograph_store::rusqlite::{self, Connection, OptionalExtension};

use super::{Ctx, End, matches, overlap, spans};

static WRITING: Mutex<()> = Mutex::new(());

/// What a write did, or would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// A unified diff of the file.
    pub diff: String,
    /// False for a dry run.
    pub applied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The request or the file can't be used as it is.
    Invalid,
    /// No such entry.
    Missing,
    /// The disk.
    Io,
}

/// A write that didn't happen: what, why and what to do, as E1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub why: Why,
    pub what: String,
    pub reason: String,
    pub action: String,
}

fn refuse(why: Why, what: &str, reason: impl Into<String>, action: &str) -> Refused {
    Refused {
        why,
        what: what.into(),
        reason: reason.into(),
        action: action.into(),
    }
}

fn io(e: &std::io::Error) -> Refused {
    refuse(
        Why::Io,
        "can't write cartograph.overrides.yaml",
        e.to_string(),
        "check the folder's permissions and free space",
    )
}

/// Text a field can hold: no control character, and none of the line and paragraph separators and
/// the byte order mark that YAML readers treat as breaks or skip.
fn plain(s: &str) -> bool {
    !s.chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}'))
}

fn quoted(s: &str) -> String {
    // A JSON string is a YAML double-quoted scalar.
    serde_json::to_string(s).expect("a string serialises")
}

/// The entry as the lines appended to the file, newline-terminated, its `-` indented `at` spaces
/// (as the entries already there are).
fn block(e: &Entry, at: usize) -> String {
    let (dash, body) = (" ".repeat(at), " ".repeat(at + 2));
    let k = &e.key;
    let mut key = format!(
        "kind: {}, from: {}, to: {}",
        k.kind.as_str(),
        quoted(&k.from),
        quoted(&k.to)
    );
    for (name, v) in [("op", &k.op), ("topic", &k.topic), ("table", &k.table)] {
        if let Some(v) = v {
            key += &format!(", {name}: {}", quoted(v));
        }
    }
    let mut out = format!(
        "{dash}- action: {}\n{body}key: {{{key}}}\n",
        e.action.as_str()
    );
    if let Some(to) = &e.to {
        out += &format!("{body}to: {}\n", quoted(to));
    }
    if let Some(c) = e.confidence {
        out += &format!("{body}confidence: {}\n", c.as_str());
    }
    out + &format!("{body}reason: {}\n", quoted(&e.reason))
}

/// A unified diff with three lines of context. Appends and cuts change one run of lines, so the
/// common head and tail are trimmed and the middle is shown as it is.
pub fn unified(old: &str, new: &str) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    if head == a.len() && head == b.len() {
        return String::new();
    }
    let (ca, cb) = (head.min(3), tail.min(3));
    let (a_mid, b_mid) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let start = |n: usize, len: usize| if len == 0 { n } else { n + 1 };
    let (a_len, b_len) = (ca + a_mid.len() + cb, ca + b_mid.len() + cb);
    let from = if old.is_empty() {
        "/dev/null".to_owned()
    } else {
        format!("a/{FILE_NAME}")
    };
    let mut out = format!(
        "--- {from}\n+++ b/{FILE_NAME}\n@@ -{},{a_len} +{},{b_len} @@\n",
        start(head - ca, a_len),
        start(head - ca, b_len)
    );
    for l in &a[head - ca..head] {
        out += &format!(" {l}\n");
    }
    for l in a_mid {
        out += &format!("-{l}\n");
    }
    for l in b_mid {
        out += &format!("+{l}\n");
    }
    for l in &a[a.len() - tail..a.len() - tail + cb] {
        out += &format!(" {l}\n");
    }
    out
}

/// A merge in progress leaves these at the start of a line.
fn conflicted(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>") || l == "=======")
}

/// The file's text, `None` when there is none. A file that isn't a regular file, one a merge left
/// conflicted and one that doesn't read are refused: a write would hide the problem or add to it.
fn load(path: &Path) -> Result<Option<String>, Refused> {
    let (text, parsed) = match overrides::read(path) {
        Ok(None) => return Ok(None),
        Ok(Some(r)) => r,
        Err(e) => return Err(io(&e)),
    };
    if conflicted(&text) {
        return Err(refuse(
            Why::Invalid,
            "cartograph.overrides.yaml has merge-conflict markers",
            "a merge left <<<<<<< or ======= lines in it",
            "resolve the merge, then make the change again",
        ));
    }
    if let Err(e) = parsed {
        return Err(refuse(
            Why::Invalid,
            "cartograph.overrides.yaml doesn't read",
            e.to_string(),
            &e.fix,
        ));
    }
    Ok(Some(text))
}

/// Writes through a temporary file beside the target and a rename.
fn store(path: &Path, text: &str) -> Result<(), Refused> {
    let tmp = path.with_file_name(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    let done = std::fs::write(&tmp, text)
        .and_then(|()| match std::fs::metadata(path) {
            Ok(m) => std::fs::set_permissions(&tmp, m.permissions()),
            Err(_) => Ok(()),
        })
        .and_then(|()| std::fs::rename(&tmp, path));
    done.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(&e)
    })
}

/// Whether the index knows what the entry is about: its ends exist, and a link (or, for `add`,
/// a time both files exist) is there for it to change.
fn bound(conn: &Connection, e: &Entry) -> Result<(), String> {
    let sql = |e: rusqlite::Error| format!("the index can't be read: {e}");
    if e.key.kind != Kind::Http {
        return Err("only http links can be corrected so far".into());
    }
    let tip: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'watermark_seq'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or("the index has no history yet")?;
    let mut ctx = Ctx::load(conn).map_err(sql)?;
    let end = |spec: &str, name: &str| -> Result<End, String> {
        match ctx.end(conn, spec, tip, tip).map_err(sql)? {
            End::None => Err(format!(
                "{name} `{spec}` names no service or file the index has"
            )),
            found => Ok(found),
        }
    };
    let from = end(&e.key.from, "key.from")?;
    let to = end(&e.key.to, "key.to")?;
    let new = e.to.as_deref().map(|t| end(t, "to")).transpose()?;
    if e.action == Action::Add {
        let (Some(a), Some(b)) = (from.single(), to.single()) else {
            return Err("add links two files: each end must name one file".into());
        };
        let both = overlap(&spans(conn, a).map_err(sql)?, &spans(conn, b).map_err(sql)?);
        return if both.iter().any(|s| s.1.is_none()) {
            Ok(())
        } else {
            Err("the two files don't exist together now".into())
        };
    }
    let hits = matches(conn, &mut ctx, "http", &from, &to, e.key.op.as_deref()).map_err(sql)?;
    if !hits.iter().any(|h| h.1) {
        return Err("no live link matches this key".into());
    }
    if new.is_some_and(|n| n.single().is_none()) {
        return Err("`to` must name one file".into());
    }
    Ok(())
}

/// Appends `entry` to the end of the file at `path` (created when there is none), after checking
/// it against the index. `dry_run` returns the diff and writes nothing. The caller starts the
/// index's apply pass: the next sync reads the file.
pub fn append(
    path: &Path,
    conn: &Connection,
    entry: &Entry,
    dry_run: bool,
) -> Result<Diff, Refused> {
    let bad = |what: &str, why: String| refuse(Why::Invalid, what, why, "change the request");
    if !plain(&entry.reason) || [&entry.key.from, &entry.key.to].iter().any(|s| !plain(s)) {
        return Err(bad(
            "the correction can't be written",
            "its text holds a control character or a line break".into(),
        ));
    }
    bound(conn, entry).map_err(|why| bad("this correction matches nothing in the index", why))?;
    let _writing = WRITING.lock().unwrap_or_else(|p| p.into_inner());
    let old = load(path)?;
    let old_text = old.as_deref().unwrap_or("");
    let was = overrides::parse(old_text).ok();
    let before = was.as_ref().map(|p| p.overrides.corrections.len());
    if before.is_some_and(|n| n >= MAX_ENTRIES) {
        return Err(bad(
            "the file is full",
            format!("it holds {MAX_ENTRIES} corrections"),
        ));
    }
    let mut new = String::from(old_text);
    if old.is_none() {
        new += "version: 1\n";
    }
    let at = was.and_then(|p| p.lines.last().copied()).map_or(2, |l| {
        let line = old_text.lines().nth(l as usize - 1).unwrap_or("");
        line.len() - line.trim_start().len()
    });
    let mut add = block(entry, at);
    if !new.is_empty() && !new.ends_with('\n') {
        // The last line has no newline: the one added now is cut again by an undo.
        new.push('\n');
        add.pop();
    }
    if !new.lines().any(|l| l.starts_with("corrections:")) {
        new += "corrections:\n";
    }
    let first = new.lines().count() + 1;
    new += &add;
    // The text must read back as the old entries and this one, starting where it was put.
    let read = overrides::parse(&new).map_err(|e| {
        refuse(
            Why::Invalid,
            "the correction can't be added to this file",
            e.to_string(),
            "move `corrections:` to the end of the file, or add the entry by hand",
        )
    })?;
    if read.overrides.corrections.last() != Some(entry)
        || read.lines.last() != Some(&(first as u32))
        || before.is_some_and(|b| b + 1 != read.lines.len())
    {
        return Err(bad(
            "the correction can't be added to this file",
            "the corrections don't end the file, so the new one wouldn't be the last".into(),
        ));
    }
    let diff = unified(old_text, &new);
    if !dry_run {
        store(path, &new)?;
    }
    Ok(Diff {
        diff,
        applied: !dry_run,
    })
}

/// Removes entry `id` (its position in the file, from 0): the lines from its `-` to its last line
/// go, and nothing else. One parse locates it.
/// Removes the entry with fingerprint `fp`: the one at `id` when it still matches, else the only
/// other one that does. Nothing is removed when the file no longer holds exactly that entry.
pub fn remove(path: &Path, id: usize, fp: &str, dry_run: bool) -> Result<Diff, Refused> {
    let _writing = WRITING.lock().unwrap_or_else(|p| p.into_inner());
    let Some(text) = load(path)? else {
        return Err(refuse(
            Why::Missing,
            "there is no cartograph.overrides.yaml",
            "nothing to undo",
            "reload the corrections",
        ));
    };
    let parsed = overrides::parse(&text).expect("load parsed it");
    let entries = &parsed.overrides.corrections;
    let same: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].fingerprint() == fp)
        .collect();
    let id = if same.contains(&id) {
        id
    } else if let [only] = same[..] {
        only
    } else {
        return Err(refuse(
            Why::Missing,
            "no such correction",
            if same.is_empty() {
                "the file no longer holds that entry".to_string()
            } else {
                "the file holds that entry more than once, elsewhere".to_string()
            },
            "reload the corrections; the file changed",
        ));
    };
    let Some(&line) = parsed.lines.get(id) else {
        return Err(refuse(
            Why::Missing,
            "no such correction",
            format!("the file holds {}", parsed.lines.len()),
            "reload the corrections; the file changed",
        ));
    };
    if parsed.lines.windows(2).any(|w| w[0] >= w[1]) {
        return Err(refuse(
            Why::Invalid,
            "this entry can't be removed from the app",
            "the file writes its corrections in flow style, so no entry has lines of its own",
            "delete it by hand",
        ));
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    let first = line as usize - 1;
    let at = indent(lines[first]);
    let mut last = first + 1;
    for (i, l) in lines.iter().enumerate().skip(first + 1) {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if indent(l) <= at {
            break;
        }
        last = i + 1;
    }
    let offset = |n: usize| lines[..n].iter().map(|l| l.len()).sum::<usize>();
    let (mut from, to) = (offset(first), offset(last));
    if to == text.len() && !text.ends_with('\n') && from > 0 {
        from -= 1;
    }
    let new = format!("{}{}", &text[..from], &text[to..]);
    let diff = unified(&text, &new);
    if !dry_run {
        store(path, &new)?;
    }
    Ok(Diff {
        diff,
        applied: !dry_run,
    })
}
