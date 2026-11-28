use proptest::prelude::*;

use super::*;
use crate::schema::FactKind;

const ORDERS_SERVER: &str = r#"import express from "express";
import { ordersRouter } from "./orders";

const app = express();
app.use("/api", ordersRouter);
app.get("/orders/:id", (req, res) => {
  res.json({ id: req.params.id });
});
app.post(`/orders`, create);
const cache = new Map();
cache.get("k", 1);
"#;

const WEB_API: &str = r#"const base = "http://orders:8080";
export async function load(id: string) {
  const r = await fetch(`http://orders:8080/orders/${id}`);
  return r.json();
}
export const save = (o: Order) =>
  fetch(base + "/orders", { method: "post", body: JSON.stringify(o) });
export const client = axios.create({ baseURL: process.env.ORDERS_URL });
export const one = (id: string) => axios.get("/orders/" + id);
"#;

fn cache() -> (tempfile::TempDir, FactCache) {
    let d = tempfile::tempdir().unwrap();
    let c = FactCache::open(&d.path().join("facts.db")).unwrap();
    (d, c)
}

fn run(c: &mut FactCache, blob: u8, path: &str, src: &str) -> Extracted {
    AdapterSet::builtin().extract(
        c,
        &[blob; 20],
        path,
        || Some(src.as_bytes().to_vec()),
        &Limits::default(),
    )
}

fn all(e: &Extracted) -> Vec<&BlobFact> {
    e.facts.iter().flat_map(|a| &a.facts).collect()
}

fn pretty(e: &Extracted) -> String {
    let mut out = Vec::new();
    for a in &e.facts {
        for f in &a.facts {
            out.push(format!("{} {}", a.name, serde_json::to_string(f).unwrap()));
        }
    }
    out.join("\n")
}

#[test]
fn the_builtin_adapters_load() {
    let names: Vec<_> = AdapterSet::builtin()
        .adapters()
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["express", "fetch"]);
    // The built-in set compiles lazily: this is where a bad built-in query fails.
    assert!(compile(AdapterSet::builtin().adapters()).is_ok());
}

/// The Done-when of XS-03: on a two-service fixture the Express route and the fetch call each
/// produce one fact with the right span, and the second run is served from the cache.
#[test]
fn a_route_and_a_call_are_one_fact_each_and_the_second_run_parses_nothing() {
    let (_d, mut c) = cache();
    let orders = "app.get('/orders/:id', (req, res) => res.json({}));\n";
    let web = "const r = await fetch(`http://orders:8080/orders/${id}`);\n";
    let route = run(
        &mut c,
        1,
        "orders/src/server.ts",
        &format!("import express from 'express';\n{orders}"),
    );
    let call = run(&mut c, 2, "web/src/api.ts", web);
    assert!(route.parsed && call.parsed);

    let routes = all(&route);
    assert_eq!(routes.len(), 1, "{}", pretty(&route));
    let r = routes[0];
    assert_eq!(r.kind, FactKind::Route);
    assert_eq!(r.span.0, [2, 0, 2, 50]);
    assert_eq!(
        r.attrs.get("method"),
        Some(&AttrValue::Str(Capped::new("GET").unwrap()))
    );
    assert_eq!(
        r.attrs.get("path"),
        Some(&AttrValue::Str(Capped::new("/orders/:id").unwrap()))
    );

    let calls = all(&call);
    assert_eq!(calls.len(), 1, "{}", pretty(&call));
    let k = calls[0];
    assert_eq!(k.kind, FactKind::HttpCall);
    assert_eq!(k.span.0, [1, 16, 1, 56]);
    assert_eq!(
        k.attrs.get("method"),
        Some(&AttrValue::Str(Capped::new("GET").unwrap()))
    );
    assert_eq!(k.evidence.kind, EvidenceType::Template);

    // Again, with a reader that must not be called: nothing is read, nothing parsed.
    let reads = Cell::new(0);
    for (blob, path, want) in [
        (1u8, "orders/src/server.ts", &route),
        (2, "web/src/api.ts", &call),
    ] {
        let again = AdapterSet::builtin().extract(
            &mut c,
            &[blob; 20],
            path,
            || {
                reads.set(reads.get() + 1);
                None
            },
            &Limits::default(),
        );
        assert!(!again.parsed);
        assert_eq!(again.facts, want.facts);
    }
    assert_eq!(reads.get(), 0);
}

#[test]
fn express_golden() {
    let (_d, mut c) = cache();
    insta::assert_snapshot!(pretty(&run(
        &mut c,
        3,
        "orders/src/server.ts",
        ORDERS_SERVER
    )));
}

#[test]
fn fetch_golden() {
    let (_d, mut c) = cache();
    insta::assert_snapshot!(pretty(&run(&mut c, 4, "web/src/api.ts", WEB_API)));
}

const ENV_SRC: &str = r#"const CATALOG = process.env.CATALOG_URL ?? "http://localhost:3000";
const PRICING = process.env["PRICING_URL"];
const viteBase = import.meta.env.VITE_API;
export const api = axios.create({ baseURL: process.env.API_URL });
export const list = () => api.get("/items");
export const find = (id: string) => fetch(`${CATALOG}/items/${id}`);
const seen = new Map();
seen.get("k");
"#;

/// XS-09: env.read facts (with the name a variable binds them to) and named clients.
#[test]
fn env_golden() {
    let (_d, mut c) = cache();
    let e = run(&mut c, 40, "web/src/config.ts", ENV_SRC);
    insta::assert_snapshot!(pretty(&e));
}

#[test]
fn a_template_is_parts_and_a_concatenation_too() {
    let (_d, mut c) = cache();
    let e = run(&mut c, 5, "a.js", WEB_API);
    let parts = |kind: FactKind, key: &str| -> Vec<String> {
        all(&e)
            .into_iter()
            .filter(|f| f.kind == kind)
            .filter_map(|f| match f.attrs.get(key)? {
                AttrValue::Parts { parts } => Some(
                    parts
                        .as_slice()
                        .iter()
                        .map(|p| match p {
                            crate::schema::Part::Lit(s) => format!("lit:{}", s.as_str()),
                            crate::schema::Part::Expr(s) => format!("expr:{}", s.as_str()),
                        })
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                AttrValue::Str(_) => None,
            })
            .collect()
    };
    let urls = parts(FactKind::HttpCall, "url");
    assert!(
        urls.contains(&"lit:http://orders:8080/orders/,expr:id".to_owned()),
        "{urls:?}"
    );
    assert!(
        urls.contains(&"expr:base,lit:/orders".to_owned()),
        "{urls:?}"
    );
    assert!(
        urls.contains(&"lit:/orders/,expr:id".to_owned()),
        "{urls:?}"
    );
    assert_eq!(
        parts(FactKind::ClientBase, "url"),
        ["expr:process.env.ORDERS_URL"]
    );
}

#[test]
fn an_adapter_is_skipped_when_its_hints_are_absent() {
    let (_d, mut c) = cache();
    // No `express` in the file: the express adapter says nothing, and with no fetch hint either
    // nothing is parsed.
    let e = run(&mut c, 6, "x.ts", "app.get('/a', h);\nconst y = 1;\n");
    assert!(!e.parsed);
    assert!(all(&e).is_empty());
    // And the empty answer is cached: no second read.
    let again = AdapterSet::builtin().extract(
        &mut c,
        &[6; 20],
        "x.ts",
        || panic!("read"),
        &Limits::default(),
    );
    assert_eq!(again.facts, e.facts);
}

#[test]
fn a_path_no_adapter_handles_costs_nothing() {
    let (_d, mut c) = cache();
    let e = AdapterSet::builtin().extract(
        &mut c,
        &[7; 20],
        "main.go",
        || panic!("read"),
        &Limits::default(),
    );
    assert_eq!(e, Extracted::default());
}

fn adapter(scm: &str) -> Result<Adapter, LoadError> {
    Adapter::load(
        "name = \"t\"\nversion = \"1\"\nlanguage = \"javascript\"\nfiles = [\"*.js\"]\n",
        &[("t.scm", scm)],
    )
}

fn facts_of(scm: &str, src: &str) -> Vec<BlobFact> {
    let set = AdapterSet::new(vec![adapter(scm).unwrap()]).unwrap();
    let (_d, mut c) = cache();
    let e = set.extract(
        &mut c,
        &[9; 20],
        "a.js",
        || Some(src.as_bytes().to_vec()),
        &Limits::default(),
    );
    e.facts.into_iter().flat_map(|a| a.facts).collect()
}

#[test]
fn every_supported_predicate_filters_and_sets() {
    let src = "call(a); call(b); call(c); other(a);\n";
    let q = |pred: &str| {
        format!(
            "((call_expression function: (identifier) @_f arguments: (arguments (identifier) @symbol.ref.name)) @symbol.ref {pred})"
        )
    };
    let names = |pred: &str| -> Vec<String> {
        facts_of(&q(pred), src)
            .iter()
            .map(|f| match f.attrs.get("name") {
                Some(AttrValue::Parts { parts }) => format!("{:?}", parts.as_slice().len()),
                _ => String::new(),
            })
            .collect()
    };
    assert_eq!(names("(#eq? @_f \"call\")").len(), 3);
    assert_eq!(names("(#not-eq? @_f \"call\")").len(), 1);
    assert_eq!(names("(#any-of? @_f \"other\" \"nope\")").len(), 1);
    assert_eq!(names("(#match? @_f \"^c.*l$\")").len(), 3);
    let set = facts_of(&q("(#set! lang \"js\")"), src);
    assert_eq!(set.len(), 4);
    assert!(
        set.iter()
            .all(|f| f.attrs.get("lang") == Some(&AttrValue::Str(Capped::new("js").unwrap())))
    );
}

#[test]
fn a_hostile_or_malformed_adapter_is_refused_at_load() {
    for scm in [
        "((identifier) @nonsense)",                         // names no fact kind
        "((identifier) @route.)",                           // empty attribute
        "((no_such_node) @route)",                          // not in the grammar
        "((identifier) @route (#lua-match? @route \"x\"))", // predicate outside the set
        "((identifier) @route (#match? @route \"(a{1000}){1000}\"))", // regex over the size limit
        "(((((",                                            // not a query
    ] {
        assert!(adapter(scm).is_err(), "{scm}");
    }
    let manifest = |extra: &str| {
        Adapter::load(
            &format!(
                "name = \"t\"\nversion = \"1\"\nlanguage = \"javascript\"\nfiles = [\"*.js\"]\n{extra}"
            ),
            &[("t.scm", "((identifier) @symbol.ref)")],
        )
    };
    assert!(manifest("").is_ok());
    assert!(
        manifest("command = \"sh\"").is_err(),
        "no subprocesses in XS-03"
    );
    assert!(
        Adapter::load(
            "name = \"t\"\nversion = \"1\"\nlanguage = \"cobol\"\nfiles = [\"*\"]\n",
            &[("t.scm", "((identifier) @symbol.ref)")]
        )
        .is_err(),
        "only grammars compiled in"
    );
}

#[test]
fn a_pathological_query_stops_at_the_budget_and_is_not_cached() {
    // Every pair of identifiers in the file is a match: quadratic work and memory.
    let scm = "((identifier) @symbol.ref . (identifier) @_b)\n(program (_)* @_a (_)* @_b (_)* @_c) @symbol.def";
    let a = adapter(scm).unwrap();
    let set = AdapterSet::new(vec![a]).unwrap();
    let (_d, mut c) = cache();
    let src = "a b c d e f g h;\n".repeat(1_000);
    let start = Instant::now();
    let limits = Limits {
        time_budget: Duration::from_millis(5),
        match_limit: 64,
        ..Limits::default()
    };
    let e = set.extract(
        &mut c,
        &[8; 20],
        "a.js",
        || Some(src.clone().into_bytes()),
        &limits,
    );
    assert!(e.parsed);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    // A pass that ran out is not cached, so a run with a bigger budget can still finish it.
    assert!(c.get_interface(&[8; 20], "adapter-t/js", "1").is_none());
}

#[test]
fn a_blob_over_the_input_cap_gets_no_facts() {
    let (_d, mut c) = cache();
    let big = format!(
        "import express from 'express'; app.get('/a', h);\n{}",
        "x".repeat(1_000_001)
    );
    let e = run(&mut c, 10, "big.js", &big);
    assert!(all(&e).is_empty());
    assert!(!e.parsed);
}

proptest! {
    /// The query loader refuses or accepts any text; it never panics.
    #[test]
    fn the_scm_loader_never_panics(s in "\\PC{0,200}") {
        let _ = adapter(&s);
        let _ = query::check(&s);
    }

    #[test]
    fn the_scm_loader_survives_query_shaped_text(s in "[()@#.?!*+\\[\\]_ a-z\"\\\\;\\n-]{0,120}") {
        let _ = adapter(&s);
    }
}

/// XS-10: a secret in a URL, in evidence or in a concatenation never reaches a fact, so never the
/// cache either.
#[test]
fn a_planted_secret_is_masked_in_attributes_and_evidence() {
    let (_d, mut c) = cache();
    let src = r#"
fetch("postgres://user:pw@db/app");
fetch("https://x.test/?k=" + "AKIAIOSFODNN7" + "EXAMPLE");
fetch(`https://x.test/${id}`, { headers: { password: "hunter2hunter2" } });
axios.get("https://api.test/", { headers: { Authorization: "Bearer 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" } });
"#;
    let e = run(&mut c, 11, "leak.js", src);
    assert_eq!(all(&e).len(), 4, "{}", pretty(&e));
    let facts = pretty(&e);
    let cached =
        serde_json::to_string(&c.get_interface(&[11; 20], "adapter-fetch/js", "1.1.0")).unwrap();
    for planted in [
        "user:pw",
        "AKIAIOSFODNN7EXAMPLE",
        "hunter2hunter2",
        "ghp_wWPw5k4a",
    ] {
        assert!(!facts.contains(planted), "{planted} in {facts}");
        assert!(!cached.contains(planted), "{planted} in the cache");
    }
    assert!(facts.contains("[REDACTED]"));
}
