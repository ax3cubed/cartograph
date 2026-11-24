//! What every tree-sitter extractor shares: one parser per grammar per thread, made the first time
//! a file of that grammar shows up (IDX-23: a workspace with no PHP never builds a PHP parser), a
//! time limit per parse, and a depth-limited walk. Adapters (XS-03) parse through it too.

use std::cell::RefCell;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use tree_sitter::{Language, Node, Parser, Tree};

use crate::Span;

/// Per-blob parse time limit (proposed).
const TIME_LIMIT: Duration = Duration::from_secs(2);
/// The syntax walk doesn't descend deeper than this (proposed).
const MAX_DEPTH: usize = 2_000;

thread_local! {
    /// One parser per grammar per thread, reused.
    static PARSERS: RefCell<Vec<(&'static str, Parser)>> = const { RefCell::new(Vec::new()) };
}

/// Parses `bytes` with the grammar of `kind`; `None` when the time limit stopped it.
pub fn parse(kind: &'static str, language: fn(&str) -> Language, bytes: &[u8]) -> Option<Tree> {
    PARSERS.with_borrow_mut(|ps| {
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
    })
}

/// Visits every node, depth first, down to [`MAX_DEPTH`].
pub fn walk(tree: &Tree, mut visit: impl FnMut(Node)) {
    let mut cursor = tree.walk();
    let mut depth = 0usize;
    loop {
        visit(cursor.node());
        if depth < MAX_DEPTH && cursor.goto_first_child() {
            depth += 1;
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
            depth -= 1;
        }
    }
}

pub fn span(n: Node) -> Span {
    let (s, e) = (n.start_position(), n.end_position());
    Span {
        start_line: s.row as u32 + 1,
        start_col: s.column as u32,
        end_line: e.row as u32 + 1,
        end_col: e.column as u32,
    }
}

/// A node's source text.
pub fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(src.get(n.start_byte()..n.end_byte()).unwrap_or_default()).unwrap_or("")
}
