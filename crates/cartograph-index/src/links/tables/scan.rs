//! Table facts of one blob (XS-17): the tables a file defines (SQL migrations and ORM models) and
//! the tables its SQL string literals read or write. Text in, facts out; nothing here connects to
//! a database. Everything read is untrusted: a literal is parsed only when it starts with a SQL
//! keyword and is under 16 KiB, and the SQL parser's recursion is limited.
//!
//! Fact attributes: `name` (the table), `source` (sql, prisma, typeorm, sqlalchemy, django, gorm,
//! eloquent), `how` (`literal` when the source spells the name, `convention` when an ORM derives it
//! from the class name) and, for a migration, `op` (create, alter, drop).

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::LazyLock;

use cartograph_facts::schema::{
    AttrValue, Attrs, BlobFact, Capped, Evidence, EvidenceType, FactKind, FactSpan,
};
use regex::Regex;
use sqlparser::ast::{ObjectName, ObjectNamePart, ObjectType, Statement, visit_relations};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

/// The most bytes of a SQL string literal that are parsed.
pub const MAX_LITERAL: usize = 16 * 1024;
/// The most bytes of one migration statement that are parsed.
const MAX_STATEMENT: usize = 64 * 1024;
/// How deep the SQL parser may recurse.
const RECURSION: usize = 48;
/// Facts kept per blob.
const MAX_FACTS: usize = 512;

/// Bump when what a blob's facts hold changes.
pub const VERSION: &str = "tables 1";

/// Whether a file of this path can hold table facts.
pub fn applies(path: &str) -> bool {
    matches!(
        path.rsplit_once('.').map(|x| x.1),
        Some(
            "sql"
                | "prisma"
                | "ts"
                | "mts"
                | "cts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "py"
                | "go"
                | "php"
        )
    )
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("a fixed pattern")
}

/// Where a fact stands: a 1-based line, the source text there and how plainly it says so.
struct Site<'a> {
    line: u32,
    text: &'a str,
    kind: EvidenceType,
}

fn fact(kind: FactKind, at: &Site, attrs: &[(&str, &str)]) -> BlobFact {
    let attrs: BTreeMap<_, _> = attrs
        .iter()
        .map(|(k, v)| (Capped::truncated(k), AttrValue::Str(Capped::truncated(v))))
        .collect();
    let one_line = at.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let head: String = one_line.chars().take(512).collect();
    BlobFact {
        kind,
        span: FactSpan([at.line, 0, at.line, 0]),
        attrs: Attrs::new(attrs).expect("few attributes"),
        evidence: Evidence {
            kind: at.kind,
            text: Capped::truncated(&cartograph_core::secrets::mask(&head)),
        },
    }
}

/// A table's name: the last part of a dotted name, unquoted, when it is a plausible identifier.
pub fn clean(raw: &str) -> Option<String> {
    let n = raw
        .rsplit('.')
        .next()?
        .trim_matches(['"', '`', '\'', '[', ']', ' ']);
    (!n.is_empty()
        && n.len() <= 128
        && n != "__expr__"
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'-')))
    .then(|| n.to_owned())
}

/// The facts of one file, sorted and without repeats.
pub fn scan(path: &str, src: &[u8]) -> Vec<BlobFact> {
    let Ok(text) = std::str::from_utf8(src) else {
        return Vec::new();
    };
    let ext = path.rsplit_once('.').map_or("", |x| x.1);
    let mut out = Vec::new();
    match ext {
        "sql" => migration(text, &mut out),
        "prisma" => prisma(text, &mut out),
        _ => {
            orm(ext, path, text, &mut out);
            literals(text, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out.truncate(MAX_FACTS);
    out
}

fn line_of(text: &str, at: usize) -> u32 {
    1 + text.as_bytes()[..at]
        .iter()
        .filter(|b| **b == b'\n')
        .count() as u32
}

fn last_name(name: &ObjectName) -> Option<String> {
    match name.0.last()? {
        ObjectNamePart::Identifier(i) => clean(&i.value),
        _ => None,
    }
}

fn parse(sql: &str) -> Vec<Statement> {
    Parser::new(&GenericDialect {})
        .with_recursion_limit(RECURSION)
        .try_with_sql(sql)
        .and_then(|mut p| p.parse_statements())
        .unwrap_or_default()
}

/// CREATE, ALTER and DROP TABLE of a migration, one statement at a time so one the parser doesn't
/// know costs only itself.
fn migration(text: &str, out: &mut Vec<BlobFact>) {
    let mut at = 0;
    for stmt in text.split_inclusive(';') {
        let start = at;
        at += stmt.len();
        if stmt.len() > MAX_STATEMENT || !stmt.to_ascii_lowercase().contains("table") {
            continue;
        }
        let head = stmt.trim_start();
        let site = Site {
            line: line_of(text, start + stmt.len() - head.len()),
            text: head,
            kind: EvidenceType::Literal,
        };
        for s in parse(stmt) {
            let (op, names) = match &s {
                Statement::CreateTable(c) => ("create", vec![last_name(&c.name)]),
                Statement::AlterTable(a) => ("alter", vec![last_name(&a.name)]),
                Statement::Drop {
                    object_type: ObjectType::Table,
                    names,
                    ..
                } => ("drop", names.iter().map(last_name).collect()),
                _ => continue,
            };
            for n in names.into_iter().flatten() {
                let attrs = [
                    ("name", &*n),
                    ("source", "sql"),
                    ("how", "literal"),
                    ("op", op),
                ];
                out.push(fact(FactKind::DbTableDef, &site, &attrs));
            }
        }
    }
}

fn how_kind(how: &str) -> EvidenceType {
    if how == "literal" {
        EvidenceType::Literal
    } else {
        EvidenceType::Inferred
    }
}

/// `model Name { ... @@map("table") }`: the table is the `@@map` name, else the model's.
fn prisma(text: &str, out: &mut Vec<BlobFact>) {
    static MODEL: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^[ \t]*model\s+(\w+)\s*\{"));
    static MAP: LazyLock<Regex> = LazyLock::new(|| re(r#"@@map\(\s*(?:name:\s*)?"([^"]+)""#));
    for m in MODEL.captures_iter(text) {
        let whole = m.get(0).expect("a match");
        let body = text[whole.end()..].split_once("\n}").map_or("", |b| b.0);
        let (name, how) = match MAP.captures(body) {
            Some(c) => (c[1].to_owned(), "literal"),
            None => (m[1].to_owned(), "convention"),
        };
        let site = Site {
            line: line_of(text, whole.start()),
            text: whole.as_str(),
            kind: how_kind(how),
        };
        if let Some(n) = clean(&name) {
            let attrs = [("name", &*n), ("source", "prisma"), ("how", how)];
            out.push(fact(FactKind::DbTableDef, &site, &attrs));
        }
    }
}

/// `UserProfile` as `user_profile`.
fn snake(s: &str) -> String {
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                o.push('_');
            }
            o.push(c.to_ascii_lowercase());
        } else {
            o.push(c);
        }
    }
    o
}

/// The plural of an English noun, as the ORMs that pluralise do it.
fn plural(s: &str) -> String {
    let mut cs = s.chars().rev();
    match (cs.next(), cs.next()) {
        (Some('y'), Some(p)) if !"aeiou".contains(p) => format!("{}ies", &s[..s.len() - 1]),
        (Some('s' | 'x' | 'z'), _) | (Some('h'), Some('c' | 's')) => format!("{s}es"),
        _ => format!("{s}s"),
    }
}

/// The first quoted string in `s`.
fn quoted(s: &str) -> Option<String> {
    static Q: LazyLock<Regex> = LazyLock::new(|| re(r#"["'`]([^"'`]+)["'`]"#));
    Q.captures(s).and_then(|c| clean(&c[1]))
}

/// ORM models: TypeORM, SQLAlchemy, Django, GORM and Eloquent. A model gives the table's name when
/// it spells one, and the name its ORM derives from the class otherwise.
fn orm(ext: &str, path: &str, text: &str, out: &mut Vec<BlobFact>) {
    static TYPEORM: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"@Entity\s*\(([^)]*)\)(?:\s*@\w+(?:\([^)]*\))?)*\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+(\w+)",
        )
    });
    static SQLALCHEMY: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(?m)^[ \t]*__tablename__\s*=\s*["']([^"']+)["']"#));
    static DJANGO: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?m)^class\s+(\w+)\s*\(\s*(?:models\.)?Model\s*\)\s*:"));
    static DB_TABLE: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(?m)^\s+db_table\s*=\s*["']([^"']+)["']"#));
    static GORM_TYPE: LazyLock<Regex> =
        LazyLock::new(|| re(r#"type\s+(\w+)\s+struct\s*\{[^}]*(?:gorm\.Model|gorm:")"#));
    static GORM_NAME: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r#"func\s*\(\s*(?:\w+\s+)?\*?(\w+)\s*\)\s*TableName\(\)\s*string\s*\{\s*return\s*["`]([^"`]+)["`]"#,
        )
    });
    static ELOQUENT: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"(?m)^[ \t]*(?:abstract\s+|final\s+)?class\s+(\w+)\s+extends\s+(?:Model|Authenticatable|Pivot)\b",
        )
    });
    static TABLE_VAR: LazyLock<Regex> = LazyLock::new(|| re(r#"\$table\s*=\s*["']([^"']+)["']"#));
    let mut def = |at: usize, shown: &str, name: Option<String>, source: &str, how: &str| {
        if let Some(n) = name {
            let site = Site {
                line: line_of(text, at),
                text: shown,
                kind: how_kind(how),
            };
            let attrs = [("name", &*n), ("source", source), ("how", how)];
            out.push(fact(FactKind::DbTableDef, &site, &attrs));
        }
    };
    match ext {
        "ts" | "mts" | "cts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" if text.contains("@Entity") => {
            for c in TYPEORM.captures_iter(text) {
                let m = c.get(0).expect("a match");
                match quoted(&c[1]) {
                    Some(n) => def(m.start(), m.as_str(), Some(n), "typeorm", "literal"),
                    None => {
                        let n = clean(&snake(&c[2]));
                        def(m.start(), m.as_str(), n, "typeorm", "convention");
                    }
                }
            }
        }
        "py" => {
            for c in SQLALCHEMY.captures_iter(text) {
                let m = c.get(0).expect("a match");
                def(m.start(), m.as_str(), clean(&c[1]), "sqlalchemy", "literal");
            }
            // Django: the app is the folder of models.py; a class's Meta may name its table.
            let app = path.rsplit('/').nth(1).unwrap_or("app");
            let classes: Vec<_> = DJANGO.captures_iter(text).collect();
            for (i, c) in classes.iter().enumerate() {
                let m = c.get(0).expect("a match");
                let end = classes
                    .get(i + 1)
                    .map_or(text.len(), |n| n.get(0).expect("a match").start());
                match DB_TABLE.captures(&text[m.end()..end]) {
                    Some(t) => def(m.start(), m.as_str(), clean(&t[1]), "django", "literal"),
                    None => {
                        let n = clean(&format!("{app}_{}", c[1].to_ascii_lowercase()));
                        def(m.start(), m.as_str(), n, "django", "convention");
                    }
                }
            }
        }
        "go" => {
            let mut named = Vec::new();
            for c in GORM_NAME.captures_iter(text) {
                let m = c.get(0).expect("a match");
                named.push(c[1].to_owned());
                def(m.start(), m.as_str(), clean(&c[2]), "gorm", "literal");
            }
            for c in GORM_TYPE.captures_iter(text) {
                if !named.contains(&c[1].to_owned()) {
                    let m = c.get(0).expect("a match");
                    let n = clean(&plural(&snake(&c[1])));
                    def(
                        m.start(),
                        &format!("type {} struct", &c[1]),
                        n,
                        "gorm",
                        "convention",
                    );
                }
            }
        }
        "php" => {
            let literal = TABLE_VAR.captures(text).and_then(|c| clean(&c[1]));
            for c in ELOQUENT.captures_iter(text) {
                let m = c.get(0).expect("a match");
                match &literal {
                    Some(n) => def(
                        m.start(),
                        m.as_str(),
                        Some(n.clone()),
                        "eloquent",
                        "literal",
                    ),
                    None => {
                        let n = clean(&plural(&snake(&c[1])));
                        def(m.start(), m.as_str(), n, "eloquent", "convention");
                    }
                }
            }
        }
        _ => {}
    }
}

/// A quote and then a SQL keyword: where a statement's string literal starts. This is the cheap
/// prefix check, so no literal that starts with anything else is ever parsed.
static STARTS: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)(["'`])\s*(?:select|insert|update|delete|with)\b"#));

/// SQL in the string literals of source code: a read or a write per table a statement names.
fn literals(text: &str, out: &mut Vec<BlobFact>) {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(c) = STARTS.captures_at(text, from) {
        let q = c.get(1).expect("the quote");
        let body = q.end();
        let mut end = body;
        // To the closing quote, past escaped ones, and no further than the cap.
        while end < bytes.len() && end - body <= MAX_LITERAL && bytes[end] != bytes[q.start()] {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        from = (end + 1).max(body);
        if end >= bytes.len() || end - body > MAX_LITERAL || !text.is_char_boundary(end) {
            continue;
        }
        let sql = &text[body..end];
        let site = Site {
            line: line_of(text, q.start()),
            text: sql,
            kind: EvidenceType::Literal,
        };
        for stmt in parse(&EXPR.replace_all(sql, "__expr__")) {
            let write = matches!(
                stmt,
                Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_)
            );
            let ctes: Vec<&str> = match &stmt {
                Statement::Query(q) => q
                    .with
                    .iter()
                    .flat_map(|w| &w.cte_tables)
                    .map(|c| &*c.alias.name.value)
                    .collect(),
                _ => Vec::new(),
            };
            let mut first = true;
            let _ = visit_relations(&stmt, |n| {
                if let Some(t) = last_name(n).filter(|t| !ctes.contains(&t.as_str())) {
                    // The statement's own table is the first one it names.
                    let kind = if write && first {
                        FactKind::DbWrite
                    } else {
                        FactKind::DbRead
                    };
                    out.push(fact(kind, &site, &[("name", &t), ("source", "sql")]));
                }
                first = false;
                ControlFlow::<()>::Continue(())
            });
        }
    }
}

/// What a driver fills in: `${x}` and `%s`, which the SQL parser can't read.
static EXPR: LazyLock<Regex> = LazyLock::new(|| re(r"\$\{[^}]*\}|%\(\w+\)s|%s"));
