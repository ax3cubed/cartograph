//! go-imports, py-imports and php-imports (IDX-23): import facts from Go, Python and PHP, on
//! tree-sitter. As for TypeScript, only literal targets are facts; a computed one (Python's
//! `importlib.import_module(x)`, PHP's `require $x`) is kept as unresolved for coverage and never
//! evaluated.
//!
//! Specifiers:
//! - Go: the import path (`example.com/billing/db`).
//! - Python: `a.b` for `import a.b`; `a.b:c` for each name of `from a.b import c` (the name may be a
//!   submodule); relative levels stay as leading dots (`..pkg:x`, `.:x`).
//! - PHP: the class for `use A\B\C` (a group use gives one per class; `use function` and
//!   `use const` name no file); the path for `require`/`include` of a literal, `./`-prefixed when
//!   written as `__DIR__ . '/x.php'`.

use tree_sitter::{Language, Node};

use crate::syntax::{parse, span, text, walk};
use crate::{BlobFacts, Extractor, Fact, ImportStyle, SkipReason};

pub struct GoImports;
pub struct PyImports;
pub struct PhpImports;

const GO: &str = "go-imports/go";
const PY: &str = "py-imports/py";
const PHP: &str = "php-imports/php";

fn language(kind: &str) -> Language {
    match kind {
        GO => tree_sitter_go::LANGUAGE.into(),
        PY => tree_sitter_python::LANGUAGE.into(),
        _ => tree_sitter_php::LANGUAGE_PHP.into(),
    }
}

fn ext(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(_, e)| e)
}

/// Parses and visits, as every extractor here does.
fn run(kind: &'static str, bytes: &[u8], visit: fn(Node, &[u8], &mut Vec<Fact>)) -> BlobFacts {
    if let Some(r) = crate::skip_reason(bytes) {
        return BlobFacts::Skipped(r);
    }
    let Some(tree) = parse(kind, language, bytes) else {
        return BlobFacts::Skipped(SkipReason::Timeout);
    };
    let mut facts = Vec::new();
    walk(&tree, |n| visit(n, bytes, &mut facts));
    facts.sort();
    facts.dedup();
    BlobFacts::Facts(facts)
}

fn import(n: Node, spec: String, style: ImportStyle) -> Fact {
    Fact::Import {
        spec,
        style,
        span: span(n),
    }
}

impl Extractor for GoImports {
    fn name(&self) -> &'static str {
        "go-imports"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[GO]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        (ext(path) == "go").then_some(GO)
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        run(kind, bytes, go_visit)
    }
}

fn go_visit(n: Node, src: &[u8], out: &mut Vec<Fact>) {
    if n.kind() != "import_spec" {
        return;
    }
    let Some(p) = n.child_by_field_name("path") else {
        return;
    };
    let t = text(p, src);
    if t.len() >= 2 {
        out.push(import(n, t[1..t.len() - 1].to_owned(), ImportStyle::Go));
    }
}

impl Extractor for PyImports {
    fn name(&self) -> &'static str {
        "py-imports"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[PY]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        (ext(path) == "py").then_some(PY)
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        run(kind, bytes, py_visit)
    }
}

/// The dotted name of an import target, without an alias.
fn dotted<'a>(n: Node, src: &'a [u8]) -> Option<&'a str> {
    match n.kind() {
        "dotted_name" => Some(text(n, src)),
        "aliased_import" => n.child_by_field_name("name").map(|x| text(x, src)),
        _ => None,
    }
}

fn py_visit(n: Node, src: &[u8], out: &mut Vec<Fact>) {
    match n.kind() {
        "import_statement" => {
            let mut c = n.walk();
            for name in n.children_by_field_name("name", &mut c) {
                if let Some(d) = dotted(name, src) {
                    out.push(import(n, d.to_owned(), ImportStyle::Python));
                }
            }
        }
        "import_from_statement" => {
            let Some(m) = n.child_by_field_name("module_name") else {
                return;
            };
            let module: String = text(m, src)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let mut c = n.walk();
            let names: Vec<String> = n
                .children_by_field_name("name", &mut c)
                .filter_map(|x| dotted(x, src).map(str::to_owned))
                .collect();
            if names.is_empty() {
                // `from m import *`.
                out.push(import(n, format!("{module}:*"), ImportStyle::Python));
            }
            for name in names {
                out.push(import(n, format!("{module}:{name}"), ImportStyle::Python));
            }
        }
        "call" => {
            let Some(f) = n.child_by_field_name("function") else {
                return;
            };
            let callee = text(f, src);
            if !matches!(
                callee,
                "importlib.import_module" | "import_module" | "__import__"
            ) {
                return;
            }
            let first = n.child_by_field_name("arguments").and_then(|a| {
                let mut c = a.walk();
                a.named_children(&mut c).next()
            });
            // A literal names a module; anything else is never evaluated.
            let plain = |a: &Node| {
                let mut c = a.walk();
                a.kind() == "string"
                    && !a
                        .named_children(&mut c)
                        .any(|k| k.kind() == "interpolation")
            };
            match first.filter(plain) {
                Some(a) => {
                    let t = text(a, src);
                    let s = t.trim_matches(|c| c == '"' || c == '\'');
                    if s.is_empty() {
                        out.push(Fact::DynamicUnresolved { span: span(n) });
                    } else {
                        out.push(import(n, s.to_owned(), ImportStyle::Dynamic));
                    }
                }
                None => out.push(Fact::DynamicUnresolved { span: span(n) }),
            }
        }
        _ => {}
    }
}

impl Extractor for PhpImports {
    fn name(&self) -> &'static str {
        "php-imports"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[PHP]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        (ext(path) == "php").then_some(PHP)
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        run(kind, bytes, php_visit)
    }
}

/// A PHP string literal's text, when it has no interpolation.
fn php_literal(n: Node, src: &[u8]) -> Option<String> {
    let t = text(n, src);
    let ok = match n.kind() {
        "string" => true,
        "encapsed_string" => !t.contains('$'),
        _ => false,
    };
    (ok && t.len() >= 2).then(|| t[1..t.len() - 1].to_owned())
}

fn php_visit(n: Node, src: &[u8], out: &mut Vec<Fact>) {
    match n.kind() {
        "namespace_use_declaration" => {
            // `use function` and `use const` name no class, so no file.
            if n.child_by_field_name("type").is_some() {
                return;
            }
            let mut c = n.walk();
            let prefix = n
                .named_children(&mut c)
                .find(|k| k.kind() == "namespace_name")
                .map(|p| text(p, src).trim_start_matches('\\').to_owned());
            let mut clauses = Vec::new();
            let mut c = n.walk();
            for k in n.named_children(&mut c) {
                match k.kind() {
                    "namespace_use_clause" => clauses.push(k),
                    "namespace_use_group" => {
                        let mut g = k.walk();
                        clauses.extend(
                            k.named_children(&mut g)
                                .filter(|x| x.kind() == "namespace_use_clause"),
                        );
                    }
                    _ => {}
                }
            }
            for cl in clauses {
                if cl.child_by_field_name("type").is_some() {
                    continue;
                }
                let mut c = cl.walk();
                let Some(name) = cl
                    .named_children(&mut c)
                    .find(|x| matches!(x.kind(), "name" | "qualified_name"))
                else {
                    continue;
                };
                let name = text(name, src).trim_start_matches('\\');
                let full = match &prefix {
                    Some(p) => format!("{p}\\{name}"),
                    None => name.to_owned(),
                };
                out.push(import(n, full, ImportStyle::PhpUse));
            }
        }
        "require_expression"
        | "require_once_expression"
        | "include_expression"
        | "include_once_expression" => {
            let Some(arg) = n.named_child(0) else {
                return;
            };
            let arg = if arg.kind() == "parenthesized_expression" {
                arg.named_child(0).unwrap_or(arg)
            } else {
                arg
            };
            let spec = match arg.kind() {
                "binary_expression" => {
                    // `__DIR__ . '/x.php'`: relative to the file's own folder.
                    let (l, r) = (
                        arg.child_by_field_name("left"),
                        arg.child_by_field_name("right"),
                    );
                    match (l, r) {
                        (Some(l), Some(r)) if text(l, src) == "__DIR__" => {
                            php_literal(r, src).map(|s| format!(".{s}"))
                        }
                        _ => None,
                    }
                }
                _ => php_literal(arg, src),
            };
            match spec {
                Some(s) if !s.is_empty() => out.push(import(n, s, ImportStyle::PhpInclude)),
                _ => out.push(Fact::DynamicUnresolved { span: span(n) }),
            }
        }
        _ => {}
    }
}
