//! Checks on a `.scm` file before tree-sitter sees it (XS-03). A query is data, but a hostile one
//! can still explode, so the loader allows only the predicates the runtime supports and compiles
//! every `#match?` pattern with the `regex` crate under a size limit: linear time, no
//! backtracking.

use regex::RegexBuilder;

/// A query file over this size is refused.
pub const MAX_QUERY_BYTES: usize = 64 * 1024;
/// A `#match?` pattern compiles within this (1 MiB).
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// The predicates a query may use. `#set!` gives a constant attribute.
const ALLOWED: [&str; 5] = ["eq?", "not-eq?", "any-of?", "match?", "set!"];

/// What was wrong with a query file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError(pub String);

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One `#name arg…` predicate: its name and the string literals among its arguments.
struct Predicate {
    name: String,
    strings: Vec<String>,
}

/// Reads the predicates of an S-expression query, skipping comments and the inside of strings.
fn predicates(src: &str) -> Vec<Predicate> {
    let mut out: Vec<Predicate> = Vec::new();
    let mut open = false; // inside a predicate's argument list
    let mut chars = src.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            ';' => {
                for (_, c) in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut s = String::new();
                while let Some((_, c)) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some((_, 'n')) => s.push('\n'),
                            Some((_, 't')) => s.push('\t'),
                            Some((_, 'r')) => s.push('\r'),
                            Some((_, c)) => s.push(c),
                            None => break,
                        },
                        c => s.push(c),
                    }
                }
                if let (true, Some(p)) = (open, out.last_mut()) {
                    p.strings.push(s);
                }
            }
            '#' => {
                let rest = &src[i + 1..];
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == ')' || c == '(')
                    .unwrap_or(rest.len());
                out.push(Predicate {
                    name: rest[..end].to_owned(),
                    strings: Vec::new(),
                });
                for _ in 0..rest[..end].chars().count() {
                    chars.next();
                }
                open = true;
            }
            ')' => open = false,
            _ => {}
        }
    }
    out
}

/// Refuses a query that is too big, uses a predicate outside [`ALLOWED`] or carries a `#match?`
/// pattern that doesn't compile within the size limit.
pub fn check(src: &str) -> Result<(), QueryError> {
    if src.len() > MAX_QUERY_BYTES {
        return Err(QueryError(format!("query over {MAX_QUERY_BYTES} bytes")));
    }
    for p in predicates(src) {
        if !ALLOWED.contains(&p.name.as_str()) {
            return Err(QueryError(format!("unsupported predicate #{}", p.name)));
        }
        if p.name == "match?" {
            for pattern in &p.strings {
                RegexBuilder::new(pattern)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|e| QueryError(format!("#match? pattern: {e}")))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supported_predicates_pass() {
        let q = r#"((identifier) @a (#eq? @a "x") (#not-eq? @a "y") (#any-of? @a "p" "q")
                    (#match? @a "^a+$") (#set! method "GET")) ; a #comment? here"#;
        assert_eq!(check(q), Ok(()));
    }

    #[test]
    fn an_unsupported_predicate_is_refused() {
        let e = check(r#"((identifier) @a (#lua-match? @a "x"))"#).unwrap_err();
        assert!(e.0.contains("#lua-match?"), "{e}");
        // A `#` inside a string is not a predicate.
        assert_eq!(check(r##"((string) @a (#eq? @a "#nope"))"##), Ok(()));
    }

    #[test]
    fn a_match_pattern_is_compiled_with_a_size_limit() {
        assert!(check(r#"((identifier) @a (#match? @a "("))"#).is_err());
        assert!(check(r#"((identifier) @a (#match? @a "(a{1000}){1000}"))"#).is_err());
    }

    #[test]
    fn a_big_query_is_refused() {
        assert!(check(&" ".repeat(MAX_QUERY_BYTES + 1)).is_err());
    }
}
