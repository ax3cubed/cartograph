//! Search over node names, split identifiers and paths (AI-15): the FTS5 table `search`, written at
//! index time, and the one query that reads it, which SRV-09's route and MCP `find` wrap.
//!
//! The table is contentless (the text lives in `node` and `file_path`), rowid = node id. Query text
//! from a person or a model never reaches FTS syntax: it is cut into words and each word is quoted
//! here, so `" or NEAR(` is three plain words, never an operator or a SQL error.
//!
//! ponytail: docstrings and summaries have columns but stay empty until the secret scan (XS-10,
//! phase 4) and summaries land; the contract-operation table comes with contracts.

use rusqlite::{Connection, params};

/// bm25 column weights: name > split name > path > docstring, summary (AI-15).
const WEIGHTS: &str = "10.0, 5.0, 2.0, 1.0, 1.0";

/// The most words a query keeps, and the longest word.
const MAX_WORDS: usize = 16;
const MAX_WORD: usize = 64;

/// `InvoiceRepo.insert` -> `invoice repo insert`; `HTTPServer2` -> `http server 2`.
pub fn split_identifier(s: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if !cur.is_empty() {
            // The previous character as written; `cur` holds it lowercased.
            let p = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = (p.is_lowercase() && c.is_uppercase())
                || (p.is_uppercase() && c.is_uppercase() && next_lower)
                || (p.is_alphabetic() != c.is_alphabetic());
            if boundary {
                words.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.join(" ")
}

/// The FTS5 query for `text`: each word quoted, all required, the last one as a prefix (what is
/// still being typed). `None` when the text has no word, which matches nothing.
pub fn fts_query(text: &str) -> Option<String> {
    let mut words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(MAX_WORDS)
        .map(|w| {
            let w: String = w.chars().take(MAX_WORD).collect::<String>().to_lowercase();
            format!("\"{}\"", w.replace('"', "\"\""))
        })
        .collect();
    if let Some(last) = words.last_mut() {
        last.push('*');
    }
    (!words.is_empty()).then(|| words.join(" "))
}

/// One search hit: a node and its bm25 score (lower is better; 0 for a substring hit).
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub node: i64,
    pub score: f64,
}

/// What a search is limited to besides its text.
#[derive(Debug, Clone, Default)]
pub struct Filter<'a> {
    /// Only nodes alive at this step.
    pub at: Option<i64>,
    /// Only these `node.kind` values; empty keeps every kind.
    pub kinds: &'a [&'a str],
    /// Only this node and what lies below it (its `parent` or `service`).
    pub scope: Option<i64>,
}

/// The kinds as one bound string, `,file,module,`, so the statement stays static.
fn kinds_arg(kinds: &[&str]) -> Option<String> {
    (!kinds.is_empty()).then(|| format!(",{},", kinds.join(",")))
}

const FILTER: &str = "(?2 IS NULL OR EXISTS (
             SELECT 1 FROM file_path fp WHERE fp.file = n.file AND fp.from_seq <= ?2
               AND (fp.to_seq IS NULL OR fp.to_seq > ?2))
           OR EXISTS (SELECT 1 FROM node_span ns WHERE ns.node = n.id AND ns.from_seq <= ?2
               AND (ns.to_seq IS NULL OR ns.to_seq > ?2)))
         AND (?4 IS NULL OR instr(?4, ',' || n.kind || ',') > 0)
         AND (?5 IS NULL OR n.id = ?5 OR n.parent = ?5 OR n.service = ?5)";

/// Nodes matching `text`, best first: bm25 with the weights above, then fan-in (F1), then id. With
/// `at`, only nodes alive at that step.
pub fn search(
    conn: &Connection,
    text: &str,
    at: Option<i64>,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    search_in(
        conn,
        text,
        &Filter {
            at,
            ..Default::default()
        },
        limit,
    )
}

/// [`search`] with kind and scope filters.
pub fn search_in(
    conn: &Connection,
    text: &str,
    f: &Filter<'_>,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    let Some(q) = fts_query(text) else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "SELECT s.rowid, bm25(search, {WEIGHTS}) AS score FROM search s
         JOIN node n ON n.id = s.rowid
         WHERE search MATCH ?1 AND {FILTER}
         ORDER BY score, n.fan_in DESC, s.rowid LIMIT ?3"
    );
    let mut st = conn.prepare_cached(&sql)?;
    let rows = st.query_map(
        params![q, f.at, limit as i64, kinds_arg(f.kinds), f.scope],
        |r| {
            Ok(Hit {
                node: r.get(0)?,
                score: r.get(1)?,
            })
        },
    )?;
    rows.collect()
}

/// The trigram query for `text`: each word of three or more characters quoted, all required.
/// `None` when no word is that long (a trigram index can't match a shorter one).
pub fn substring_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .take(MAX_WORDS)
        .map(|w| {
            let w: String = w.chars().take(MAX_WORD).collect::<String>().to_lowercase();
            format!("\"{}\"", w.replace('"', "\"\""))
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// Nodes whose name or path contains every word of `text` of three or more characters anywhere,
/// not only at a word start (SRV-09: what the palette's `includes` did). Ranked by fan-in, then
/// the node id. An index without the trigram table answers nothing.
pub fn substring(
    conn: &Connection,
    text: &str,
    f: &Filter<'_>,
    limit: usize,
) -> rusqlite::Result<Vec<Hit>> {
    let Some(q) = substring_query(text) else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "SELECT s.rowid FROM search_sub s JOIN node n ON n.id = s.rowid
         WHERE search_sub MATCH ?1 AND {FILTER}
         ORDER BY n.fan_in DESC, s.rowid LIMIT ?3"
    );
    let mut st = match conn.prepare_cached(&sql) {
        Ok(st) => st,
        Err(rusqlite::Error::SqliteFailure(_, Some(m))) if m.contains("no such table") => {
            return Ok(Vec::new());
        }
        Err(e) => return Err(e),
    };
    let rows = st.query_map(
        params![q, f.at, limit as i64, kinds_arg(f.kinds), f.scope],
        |r| {
            Ok(Hit {
                node: r.get(0)?,
                score: 0.0,
            })
        },
    )?;
    rows.collect()
}

/// Writes (or rewrites) a node's search rows.
pub fn put(conn: &Connection, node: i64, name: &str, path: &str) -> rusqlite::Result<()> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    conn.prepare_cached("DELETE FROM search WHERE rowid = ?1")?
        .execute([node])?;
    conn.prepare_cached(
        "INSERT INTO search (rowid, name, split, path, docstring, summary)
         VALUES (?1, ?2, ?3, ?4, '', '')",
    )?
    .execute(params![node, name, split_identifier(stem), path])?;
    conn.prepare_cached("DELETE FROM search_sub WHERE rowid = ?1")?
        .execute([node])?;
    conn.prepare_cached("INSERT INTO search_sub (rowid, name, path) VALUES (?1, ?2, ?3)")?
        .execute(params![node, name, path])
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn identifiers_split_into_words() {
        assert_eq!(
            split_identifier("InvoiceRepo.insert"),
            "invoice repo insert"
        );
        assert_eq!(split_identifier("HTTPServer2"), "http server 2");
        assert_eq!(split_identifier("snake_case-name"), "snake case name");
        assert_eq!(split_identifier(""), "");
    }

    #[test]
    fn query_text_is_quoted_word_by_word() {
        assert_eq!(fts_query("invoice repo").unwrap(), "\"invoice\" \"repo\"*");
        assert_eq!(fts_query("\" or NEAR(").unwrap(), "\"or\" \"near\"*");
        assert_eq!(fts_query("  ()\"*^:"), None);
    }

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE node (id INTEGER PRIMARY KEY, kind TEXT, file INTEGER, parent INTEGER,
               service INTEGER, fan_in INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE file_path (file INTEGER, path TEXT, from_seq INTEGER, to_seq INTEGER);
             CREATE TABLE node_span (node INTEGER, from_seq INTEGER, to_seq INTEGER);
             CREATE VIRTUAL TABLE search USING fts5 (name, split, path, docstring, summary,
               content = '', contentless_delete = 1, prefix = '2 3', tokenize = 'unicode61');
             CREATE VIRTUAL TABLE search_sub USING fts5 (name, path,
               content = '', contentless_delete = 1, tokenize = 'trigram');",
        )
        .unwrap();
        c
    }

    #[test]
    fn ranking_prefers_the_name_and_slices_by_time() {
        let c = db();
        let files = [
            (1, "src/billing/InvoiceRepo.ts", 0, None),
            (2, "src/invoice/repo/notes.ts", 0, None),
            (3, "src/billing/InvoiceRepository.ts", 0, Some(5)),
        ];
        for (id, path, from, to) in files {
            c.execute(
                "INSERT INTO node (id, kind, file) VALUES (?1, 'file', ?1)",
                [id],
            )
            .unwrap();
            c.execute(
                "INSERT INTO file_path VALUES (?1, ?2, ?3, ?4)",
                params![id, path, from, to],
            )
            .unwrap();
            put(&c, id, path.rsplit('/').next().unwrap(), path).unwrap();
        }
        let hits = search(&c, "invoice repo", None, 3).unwrap();
        assert_eq!(hits[0].node, 1, "{hits:?}");
        let later = search(&c, "invoice repo", Some(7), 3).unwrap();
        assert!(later.iter().all(|h| h.node != 3), "gone at 7");
        put(&c, 1, "Renamed.ts", "src/Renamed.ts").unwrap();
        assert!(
            search(&c, "invoice repo", None, 3)
                .unwrap()
                .iter()
                .all(|h| h.node != 1)
        );
    }

    #[test]
    fn substrings_match_inside_words_and_filters_apply() {
        let c = db();
        for (id, kind, path) in [
            (1, "file", "src/billing/InvoiceRepo.ts"),
            (2, "module", "src/billing"),
        ] {
            c.execute(
                "INSERT INTO node (id, kind, file) VALUES (?1, ?2, ?1)",
                params![id, kind],
            )
            .unwrap();
            c.execute(
                "INSERT INTO file_path VALUES (?1, ?2, 0, NULL)",
                params![id, path],
            )
            .unwrap();
            put(&c, id, path.rsplit('/').next().unwrap(), path).unwrap();
        }
        let f = Filter::default();
        assert!(
            search_in(&c, "voice", &f, 10).unwrap().is_empty(),
            "not a word start"
        );
        let hits: Vec<i64> = substring(&c, "voice", &f, 10)
            .unwrap()
            .iter()
            .map(|h| h.node)
            .collect();
        assert_eq!(hits, [1]);
        let both: Vec<i64> = substring(&c, "illi", &f, 10)
            .unwrap()
            .iter()
            .map(|h| h.node)
            .collect();
        assert_eq!(both, [1, 2]);
        let files = Filter {
            kinds: &["file"],
            ..Default::default()
        };
        assert_eq!(substring(&c, "illi", &files, 10).unwrap().len(), 1);
        assert!(
            substring(&c, "ab", &f, 10).unwrap().is_empty(),
            "under three characters"
        );
        assert!(substring(&c, "\" OR NEAR(", &f, 10).is_ok());
    }

    proptest! {
        /// Whatever the text, a query returns rows or nothing, never an error.
        #[test]
        fn any_text_queries_without_error(text in "\\PC{0,40}") {
            let c = db();
            put(&c, 1, "InvoiceRepo.ts", "a/InvoiceRepo.ts").unwrap();
            prop_assert!(search(&c, &text, None, 10).is_ok());
            prop_assert!(search(&c, &text, Some(0), 10).is_ok());
            prop_assert!(substring(&c, &text, &Filter::default(), 10).is_ok());
        }
    }
}
