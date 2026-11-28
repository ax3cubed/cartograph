//! The post-pass of XS-03: a captured node becomes an attribute value. A plain string stays a
//! string; a template string or a `+` concatenation becomes parts, so a matcher can see which
//! segments the code spells out (`"/orders/"`) and which it doesn't (`id`).

use cartograph_core::secrets::mask;
use cartograph_parse::syntax::text;
use tree_sitter::Node;

use crate::schema::{AttrValue, Capped, MAX_PARTS, Part, Parts};

/// A piece of a value before it is masked and cut: both come last, so a secret can't be cut in half
/// and slip past the rules, and text the code concatenates is scanned as one string.
enum Seg {
    Lit(String),
    Expr(String),
}

fn lit(s: &str) -> Seg {
    Seg::Lit(s.to_owned())
}

fn expr(s: &str) -> Seg {
    Seg::Expr(s.to_owned())
}

/// The text of a `string` node without its quotes.
fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    match b.first() {
        Some(b'"' | b'\'' | b'`') if s.len() >= 2 => &s[1..s.len() - 1],
        _ => s,
    }
}

/// Appends `s` to the last part when that is text too, so adjacent literals are one part.
fn push_lit(out: &mut Vec<Seg>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Seg::Lit(last)) = out.last_mut() {
        last.push_str(s);
    } else {
        out.push(lit(s));
    }
}

fn collect(n: Node, src: &[u8], out: &mut Vec<Seg>) {
    match n.kind() {
        // Python's strings hold their own pieces: `f"/orders/{id}"` is text and an interpolation.
        "string" if n.child(0).is_some_and(|c| c.kind() == "string_start") => {
            let mut c = n.walk();
            for k in n.named_children(&mut c) {
                match k.kind() {
                    "string_content" => push_lit(out, text(k, src)),
                    "interpolation" => {
                        out.push(expr(k.named_child(0).map_or("", |i| text(i, src))))
                    }
                    _ => {}
                }
            }
        }
        "string"
        | "encapsed_string"
        | "interpreted_string_literal"
        | "raw_string_literal"
        | "string_scalar"
        | "double_quote_scalar"
        | "single_quote_scalar" => push_lit(out, unquote(text(n, src))),
        // A name the code spells out (`.get` in `axios.get(…)`), not a value worked out at run time.
        "property_identifier" => push_lit(out, text(n, src)),
        "template_string" => {
            let mut c = n.walk();
            for k in n.named_children(&mut c) {
                match k.kind() {
                    "template_substitution" => {
                        let mut kc = k.walk();
                        let inner = k.named_children(&mut kc).next();
                        out.push(expr(inner.map_or("", |i| text(i, src))));
                    }
                    _ => push_lit(out, text(k, src)),
                }
            }
        }
        "parenthesized_expression" => {
            let mut c = n.walk();
            match n.named_children(&mut c).next() {
                Some(inner) => collect(inner, src, out),
                None => out.push(expr(text(n, src))),
            }
        }
        // `+` in JS, Go and Python; `.` in PHP.
        "binary_expression" | "binary_operator"
            if n.child_by_field_name("operator")
                .is_some_and(|o| matches!(text(o, src), "+" | ".")) =>
        {
            for side in ["left", "right"] {
                match n.child_by_field_name(side) {
                    Some(k) => collect(k, src, out),
                    None => out.push(expr(text(n, src))),
                }
            }
        }
        _ => out.push(expr(text(n, src))),
    }
}

/// The value of a captured node: a string when the code spells it out, parts otherwise. Secrets
/// are masked (XS-10).
pub fn value_of(n: Node, src: &[u8]) -> AttrValue {
    let mut segs = Vec::new();
    collect(n, src, &mut segs);
    segs.truncate(MAX_PARTS);
    let mut parts: Vec<Part> = segs
        .into_iter()
        .map(|s| match s {
            Seg::Lit(s) => Part::Lit(Capped::truncated(&mask(&s))),
            Seg::Expr(s) => Part::Expr(Capped::truncated(&mask(&s))),
        })
        .collect();
    match parts.as_slice() {
        [] => AttrValue::Str(Capped::truncated("")),
        [Part::Lit(_)] => match parts.pop() {
            Some(Part::Lit(s)) => AttrValue::Str(s),
            _ => unreachable!("matched a single literal"),
        },
        _ => AttrValue::Parts {
            parts: Parts::new(parts).expect("truncated to the cap"),
        },
    }
}

/// True when every piece of the value is spelled out in the source.
pub fn is_literal(v: &AttrValue) -> bool {
    matches!(v, AttrValue::Str(_))
}

/// The text of a value, its parts joined.
pub fn plain(v: &AttrValue) -> String {
    match v {
        AttrValue::Str(s) => s.as_str().to_owned(),
        AttrValue::Parts { parts } => parts
            .as_slice()
            .iter()
            .map(|p| match p {
                Part::Lit(s) | Part::Expr(s) => s.as_str(),
            })
            .collect(),
    }
}
