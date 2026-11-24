//! The post-pass of XS-03: a captured node becomes an attribute value. A plain string stays a
//! string; a template string or a `+` concatenation becomes parts, so a matcher can see which
//! segments the code spells out (`"/orders/"`) and which it doesn't (`id`).

use cartograph_parse::syntax::text;
use tree_sitter::Node;

use crate::schema::{AttrValue, Capped, MAX_PARTS, Part, Parts};

/// Source text to a [`Part`], cut to the cap.
fn lit(s: &str) -> Part {
    Part::Lit(Capped::truncated(s))
}

fn expr(s: &str) -> Part {
    Part::Expr(Capped::truncated(s))
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
fn push_lit(out: &mut Vec<Part>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Part::Lit(last)) = out.last_mut() {
        *last = Capped::truncated(&format!("{}{s}", last.as_str()));
    } else {
        out.push(lit(s));
    }
}

fn collect(n: Node, src: &[u8], out: &mut Vec<Part>) {
    match n.kind() {
        "string" => push_lit(out, unquote(text(n, src))),
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
        "binary_expression"
            if n.child_by_field_name("operator")
                .is_some_and(|o| text(o, src) == "+") =>
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

/// The value of a captured node: a string when the code spells it out, parts otherwise.
pub fn value_of(n: Node, src: &[u8]) -> AttrValue {
    let mut parts = Vec::new();
    collect(n, src, &mut parts);
    parts.truncate(MAX_PARTS);
    match parts.as_slice() {
        [] => AttrValue::Str(Capped::truncated("")),
        [Part::Lit(s)] => AttrValue::Str(s.clone()),
        _ => AttrValue::Parts {
            parts: Parts::new(parts).expect("truncated to the cap"),
        },
    }
}

/// True when every piece of the value is spelled out in the source.
pub fn is_literal(v: &AttrValue) -> bool {
    matches!(v, AttrValue::Str(_))
}
