use proptest::prelude::*;
use serde_json::json;

use super::*;

/// The sample line of H4 (decisions.html), unchanged.
const H4_LINE: &str = r#"{"schema":1,"kind":"http.call","repo":"web","commit":"77c0e2b","path":"src/api.ts",
"span":[12,4,12,38],"attrs":{"method":"GET","url":"/orders/{id}"},
"evidence":{"type":"literal","text":"fetch(`/orders/${id}`)"},"extractor":{"name":"fetch","version":"1.2.0"}}"#;

fn schema_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fact.schema.json")
}

fn render() -> String {
    let mut s = serde_json::to_string_pretty(&json_schema()).unwrap();
    s.push('\n');
    s
}

/// The checked-in schema is what the types generate (the CI check). `BLESS=1 cargo test -p
/// cartograph-facts schema_file` rewrites it.
#[test]
fn schema_file_matches_the_types() {
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(schema_path(), render()).unwrap();
    }
    let have = std::fs::read_to_string(schema_path()).unwrap_or_default();
    assert!(
        have == render(),
        "crates/cartograph-facts/fact.schema.json is out of date: run `BLESS=1 cargo test -p cartograph-facts schema_file`"
    );
}

#[test]
fn the_h4_sample_line_validates_and_round_trips_unchanged() {
    let validator = jsonschema::validator_for(&json_schema()).unwrap();
    let line: serde_json::Value = serde_json::from_str(H4_LINE).unwrap();
    assert!(
        validator.is_valid(&line),
        "{:?}",
        validator.iter_errors(&line).next()
    );
    let fact: Fact = serde_json::from_str(H4_LINE).unwrap();
    assert_eq!(fact.kind, FactKind::HttpCall);
    assert_eq!(serde_json::to_value(&fact).unwrap(), line);
}

#[test]
fn the_schema_rejects_what_the_types_reject() {
    let validator = jsonschema::validator_for(&json_schema()).unwrap();
    let mut line: serde_json::Value = serde_json::from_str(H4_LINE).unwrap();
    for (field, bad) in [
        ("kind", json!("http.cal")),
        ("schema", json!(2)),
        ("path", json!("p".repeat(MAX_PATH + 1))),
        ("span", json!([1, 2, 3])),
    ] {
        let good = line[field].clone();
        line[field] = bad;
        assert!(!validator.is_valid(&line), "{field}");
        assert!(
            serde_json::from_value::<Fact>(line.clone()).is_err(),
            "{field}"
        );
        line[field] = good;
    }
}

#[test]
fn an_unknown_kind_is_an_error_not_a_fact() {
    let line = H4_LINE.replace("http.call", "carrier.pigeon");
    assert!(serde_json::from_str::<Fact>(&line).is_err());
    assert!(FactKind::parse("carrier.pigeon").is_none());
    for k in FactKind::ALL {
        assert_eq!(FactKind::parse(k.as_str()), Some(k));
        // The serde name and the as_str name are one.
        assert_eq!(serde_json::to_value(k).unwrap(), json!(k.as_str()));
    }
    assert!(serde_json::from_str::<Predicate>("\"befriends\"").is_err());
    assert!(serde_json::from_str::<NodeClass>("\"Cheese\"").is_err());
    assert!(serde_json::from_str::<OriginLayer>("\"hunch\"").is_err());
}

#[test]
fn an_unknown_field_is_an_error() {
    let line = H4_LINE.replace("\"schema\":1,", "\"schema\":1,\"extra\":true,");
    assert!(serde_json::from_str::<Fact>(&line).is_err());
}

#[test]
fn every_string_has_a_cap_in_its_type() {
    let long = |n: usize| "x".repeat(n);
    let with = |field: &str, value: serde_json::Value| {
        let mut v: serde_json::Value = serde_json::from_str(H4_LINE).unwrap();
        match field {
            "attr" => v["attrs"]["url"] = value,
            "evidence" => v["evidence"]["text"] = value,
            "repo" | "commit" | "path" => v[field] = value,
            "ext" => v["extractor"]["name"] = value,
            _ => unreachable!(),
        }
        serde_json::from_value::<Fact>(v).is_ok()
    };
    assert!(with("path", json!(long(MAX_PATH))));
    assert!(!with("path", json!(long(MAX_PATH + 1))));
    assert!(with("attr", json!(long(MAX_ATTR_VALUE))));
    assert!(!with("attr", json!(long(MAX_ATTR_VALUE + 1))));
    assert!(with("evidence", json!(long(MAX_EVIDENCE))));
    assert!(!with("evidence", json!(long(MAX_EVIDENCE + 1))));
    assert!(!with("repo", json!(long(257))));
    assert!(!with("commit", json!(long(65))));
    assert!(!with("ext", json!(long(65))));
    // Characters, not bytes: 200 four-byte characters are within the evidence cap.
    assert!(with("evidence", json!("𝄞".repeat(MAX_EVIDENCE))));
    // And the other containers.
    let many: BTreeMap<_, _> = (0..=MAX_ATTRS)
        .map(|i| {
            (
                Capped::new(format!("k{i}")).unwrap(),
                AttrValue::Str(Capped::new("v").unwrap()),
            )
        })
        .collect();
    assert!(Attrs::new(many).is_err());
    assert!(Parts::new(vec![Part::Lit(Capped::new("a").unwrap()); MAX_PARTS + 1]).is_err());
}

#[test]
fn a_template_attribute_is_a_list_of_parts() {
    let v = json!({
        "kind": "http.call", "span": [1, 0, 1, 9],
        "attrs": {"url": {"parts": [{"lit": "/orders/"}, {"expr": "id"}]}},
        "evidence": {"type": "template", "text": "fetch(`/orders/${id}`)"}
    });
    let f: BlobFact = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(serde_json::to_value(&f).unwrap(), v);
    match f.attrs.get("url") {
        Some(AttrValue::Parts { parts }) => assert_eq!(parts.as_slice().len(), 2),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_typical_cached_fact_is_under_150_bytes() {
    let f = BlobFact {
        kind: FactKind::Route,
        span: FactSpan([12, 2, 14, 4]),
        attrs: serde_json::from_value(json!({"method": "GET", "path": "/orders/:id"})).unwrap(),
        evidence: Evidence {
            kind: EvidenceType::Literal,
            text: Capped::new("app.get('/orders/:id', h)").unwrap(),
        },
    };
    let n = serde_json::to_vec(&f).unwrap().len();
    assert!(n <= 150, "{n} bytes");
}

#[test]
fn facts_round_trip_through_the_cache_as_one_row_per_blob() {
    let d = tempfile::tempdir().unwrap();
    let mut cache = crate::FactCache::open(&d.path().join("facts.db")).unwrap();
    let f = BlobFact {
        kind: FactKind::EnvRead,
        span: FactSpan([1, 0, 1, 20]),
        attrs: Attrs::default(),
        evidence: Evidence {
            kind: EvidenceType::Literal,
            text: Capped::new("process.env.PORT").unwrap(),
        },
    };
    let blob = [7u8; 20];
    assert!(cache.get_interface(&blob, "x/ts", "1").is_none());
    cache
        .put_interface_many([(&blob[..], "x/ts", "1", &[f.clone(), f.clone()][..])])
        .unwrap();
    assert_eq!(
        cache.get_interface(&blob, "x/ts", "1"),
        Some(vec![f.clone(), f])
    );
    // Another version is another key.
    assert!(cache.get_interface(&blob, "x/ts", "2").is_none());
    // A payload this build can't read is a miss, not an error.
    cache
        .put_interface_many([(&blob[..], "bad", "1", &[][..])])
        .unwrap();
    assert_eq!(cache.get_interface(&blob, "bad", "1"), Some(vec![]));
}

fn capped<const N: usize>() -> impl Strategy<Value = Capped<N>> {
    prop::collection::vec(any::<char>(), 0..12)
        .prop_map(|c| Capped::truncated(&c.into_iter().collect::<String>()))
}

fn attr_value() -> impl Strategy<Value = AttrValue> {
    prop_oneof![
        capped().prop_map(AttrValue::Str),
        prop::collection::vec(
            prop_oneof![capped().prop_map(Part::Lit), capped().prop_map(Part::Expr)],
            0..5
        )
        .prop_map(|p| AttrValue::Parts {
            parts: Parts::new(p).unwrap()
        }),
    ]
}

fn any_fact() -> impl Strategy<Value = Fact> {
    let kind = prop::sample::select(FactKind::ALL.to_vec());
    let ev = prop::sample::select(vec![
        EvidenceType::Literal,
        EvidenceType::Template,
        EvidenceType::Inferred,
    ]);
    let attrs = prop::collection::btree_map(capped(), attr_value(), 0..5)
        .prop_map(|m| Attrs::new(m).unwrap());
    (
        kind,
        (capped(), capped(), capped()),
        any::<[u32; 4]>(),
        attrs,
        (ev, capped()),
        (capped(), capped()),
    )
        .prop_map(
            |(kind, (repo, commit, path), span, attrs, (et, text), (name, version))| Fact {
                schema: SchemaTag,
                kind,
                repo,
                commit,
                path,
                span: FactSpan(span),
                attrs,
                evidence: Evidence { kind: et, text },
                extractor: ExtractorRef { name, version },
            },
        )
}

proptest! {
    /// Any valid fact serialises and parses back to itself, and the schema accepts what it wrote.
    #[test]
    fn a_valid_fact_round_trips(f in any_fact()) {
        let text = serde_json::to_string(&f).unwrap();
        prop_assert_eq!(serde_json::from_str::<Fact>(&text).unwrap(), f.clone());
        let validator = jsonschema::validator_for(&json_schema()).unwrap();
        prop_assert!(validator.is_valid(&serde_json::from_str(&text).unwrap()));
        // The cached form too.
        let b = BlobFact { kind: f.kind, span: f.span, attrs: f.attrs, evidence: f.evidence };
        prop_assert_eq!(decode(&encode(std::slice::from_ref(&b))), Some(vec![b]));
    }
}
