//! ts-imports: import facts from TypeScript and JavaScript, on tree-sitter.

use std::cell::RefCell;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use tree_sitter::{Language, Node, Parser};

use crate::{BlobFacts, Extractor, Fact, ImportStyle, SkipReason, Span};

pub struct TsImports;

/// Per-blob parse time limit (proposed).
const TIME_LIMIT: Duration = Duration::from_secs(2);
/// The syntax walk doesn't descend deeper than this (proposed).
const MAX_DEPTH: usize = 2_000;

const TS: &str = "ts-imports/ts";
const TSX: &str = "ts-imports/tsx";
const JS: &str = "ts-imports/js";

thread_local! {
    /// One parser per grammar per thread, reused.
    static PARSERS: RefCell<Vec<(&'static str, Parser)>> = const { RefCell::new(Vec::new()) };
}

fn language(kind: &str) -> Language {
    match kind {
        TS => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        TSX => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    }
}

impl Extractor for TsImports {
    fn name(&self) -> &'static str {
        "ts-imports"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        let ext = path.rsplit_once('.')?.1;
        match ext {
            "ts" => Some(TS),
            "tsx" => Some(TSX),
            "js" | "jsx" | "mjs" | "cjs" => Some(JS),
            _ => None,
        }
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        if let Some(r) = crate::skip_reason(bytes) {
            return BlobFacts::Skipped(r);
        }
        let tree = PARSERS.with_borrow_mut(|ps| {
            let i = match ps.iter().position(|(k, _)| *k == kind) {
                Some(i) => i,
                None => {
                    let mut p = Parser::new();
                    p.set_language(&language(kind)).expect("grammar matches");
                    ps.push((kind, p));
                    ps.len() - 1
                }
            };
            let start = Instant::now();
            let mut over = |_: &tree_sitter::ParseState| {
                if start.elapsed() > TIME_LIMIT {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let opts = tree_sitter::ParseOptions::new().progress_callback(&mut over);
            let mut read = |i: usize, _| bytes.get(i..).unwrap_or_default();
            let tree = ps[i].1.parse_with_options(&mut read, None, Some(opts));
            ps[i].1.reset();
            tree
        });
        let Some(tree) = tree else {
            return BlobFacts::Skipped(SkipReason::Timeout);
        };
        let mut facts = Vec::new();
        let mut cursor = tree.walk();
        let mut depth = 0usize;
        loop {
            visit(cursor.node(), bytes, &mut facts);
            if depth < MAX_DEPTH && cursor.goto_first_child() {
                depth += 1;
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    facts.sort();
                    return BlobFacts::Facts(facts);
                }
                depth -= 1;
            }
        }
    }
}

fn span(n: Node) -> Span {
    let (s, e) = (n.start_position(), n.end_position());
    Span {
        start_line: s.row as u32 + 1,
        start_col: s.column as u32,
        end_line: e.row as u32 + 1,
        end_col: e.column as u32,
    }
}

/// The text of a string literal, or of a template literal with no substitutions.
fn literal(n: Node, src: &[u8]) -> Option<String> {
    let ok = match n.kind() {
        "string" => true,
        "template_string" => {
            let mut c = n.walk();
            !n.named_children(&mut c)
                .any(|k| k.kind() == "template_substitution")
        }
        _ => false,
    };
    let text = src.get(n.start_byte()..n.end_byte())?;
    (ok && text.len() >= 2).then(|| String::from_utf8_lossy(&text[1..text.len() - 1]).into_owned())
}

fn has_child(n: Node, kind: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c).any(|k| k.kind() == kind)
}

fn visit(n: Node, src: &[u8], out: &mut Vec<Fact>) {
    let mut push = |spec: String, style| {
        out.push(Fact::Import {
            spec,
            style,
            span: span(n),
        })
    };
    match n.kind() {
        "import_statement" => {
            let type_only = has_child(n, "type");
            if let Some(s) = n
                .child_by_field_name("source")
                .and_then(|s| literal(s, src))
            {
                push(
                    s,
                    if type_only {
                        ImportStyle::TypeOnly
                    } else {
                        ImportStyle::Esm
                    },
                );
            } else {
                // TS `import x = require("y")`.
                let mut c = n.walk();
                let req = n
                    .named_children(&mut c)
                    .find(|k| k.kind() == "import_require_clause");
                if let Some(s) = req
                    .and_then(|r| r.child_by_field_name("source"))
                    .and_then(|s| literal(s, src))
                {
                    push(s, ImportStyle::Cjs);
                }
            }
        }
        "export_statement" => {
            if let Some(s) = n
                .child_by_field_name("source")
                .and_then(|s| literal(s, src))
            {
                let style = if has_child(n, "type") {
                    ImportStyle::TypeOnly
                } else {
                    ImportStyle::Reexport
                };
                push(s, style);
            }
        }
        "call_expression" => {
            let Some(f) = n.child_by_field_name("function") else {
                return;
            };
            let style = match f.kind() {
                "import" => ImportStyle::Dynamic,
                "identifier" if src.get(f.start_byte()..f.end_byte()) == Some(b"require") => {
                    ImportStyle::Cjs
                }
                _ => return,
            };
            let first = n.child_by_field_name("arguments").and_then(|a| {
                let mut c = a.walk();
                a.named_children(&mut c).next()
            });
            match first.and_then(|a| literal(a, src)) {
                Some(s) => push(s, style),
                None => out.push(Fact::DynamicUnresolved { span: span(n) }),
            }
        }
        _ => {}
    }
}
