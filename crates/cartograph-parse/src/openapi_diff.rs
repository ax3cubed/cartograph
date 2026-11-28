//! What changed between two versions of a spec (XS-19): a structural diff of [`Contract`]s, with
//! the breaking subset of oasdiff's rules reimplemented over the XS-08 model. The model keeps no
//! direction (request or response), so a schema field is judged both ways: a removed field, a
//! new or newly required field, a narrowed type and a removed enum value are all breaking.
//!
//! Summary strings are built here from the diff; no model writes them. Descriptions, summaries,
//! titles and line numbers are not structure and never show up as a change.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::openapi::{Contract, SchemaField};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    OperationAdded,
    OperationRemoved,
    SchemaAdded,
    SchemaRemoved,
    FieldAdded,
    FieldRemoved,
    FieldRequired,
    FieldOptional,
    TypeNarrowed,
    TypeWidened,
    EnumValueRemoved,
    EnumValueAdded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    /// `post /v1/orders` for an operation, `Order.id` for a field, `Order` for a schema.
    pub subject: String,
    pub breaking: bool,
    /// `Removed POST /v1/orders`.
    pub summary: String,
}

/// (method, path with its base).
type Ops = BTreeSet<(String, String)>;

fn ops(c: &Contract) -> Ops {
    let none = [String::new()];
    let bases = if c.bases.is_empty() {
        &none[..]
    } else {
        &c.bases[..]
    };
    c.operations
        .iter()
        .flat_map(|o| {
            bases
                .iter()
                .map(move |b| (o.method.clone(), format!("{b}{}", o.path)))
        })
        .collect()
}

type Fields<'a> = BTreeMap<&'a str, &'a SchemaField>;

fn schemas(c: &Contract) -> BTreeMap<&str, Fields<'_>> {
    c.schemas
        .iter()
        .map(|s| {
            (
                s.name.as_str(),
                s.fields.iter().map(|f| (f.name.as_str(), f)).collect(),
            )
        })
        .collect()
}

/// A type as `string`, `integer|null`, `string{a,b}` (an enum): its alternatives and its values.
fn parts(ty: &str) -> (BTreeSet<&str>, BTreeSet<&str>) {
    let (base, values) = ty.split_once('{').unwrap_or((ty, ""));
    (
        base.split('|').collect(),
        values
            .trim_end_matches('}')
            .split(',')
            .filter(|v| !v.is_empty())
            .collect(),
    )
}

fn base(ty: &str) -> &str {
    ty.split('{').next().unwrap_or_default()
}

/// `post /v1/orders` as `POST /v1/orders`.
fn shown(op: &(String, String)) -> String {
    format!("{} {}", op.0.to_ascii_uppercase(), op.1)
}

/// The text a spec is hashed by: operations and schemas, sorted, nothing else.
pub fn canonical(c: &Contract) -> Vec<u8> {
    let mut out = String::new();
    for (m, p) in ops(c) {
        out += &format!("op {m} {p}\n");
    }
    let mut ids: Vec<String> = c
        .operations
        .iter()
        .filter_map(|o| Some(format!("id {} {} {}\n", o.method, o.path, o.operation_id.as_ref()?)))
        .collect();
    ids.sort();
    out += &ids.concat();
    for (name, fields) in schemas(c) {
        out += &format!("schema {name}\n");
        for f in fields.values() {
            out += &format!("field {} {} {}\n", f.name, f.ty, f.required);
        }
    }
    out.into_bytes()
}

/// Every change from `old` to `new`, in a fixed order.
pub fn diff(old: &Contract, new: &Contract) -> Vec<Change> {
    let mut out = Vec::new();
    let mut push = |kind, subject: &str, breaking, summary: String| {
        out.push(Change {
            kind,
            subject: subject.to_owned(),
            breaking,
            summary,
        });
    };
    let (a, b) = (ops(old), ops(new));
    for o in b.difference(&a) {
        push(
            ChangeKind::OperationAdded,
            &format!("{} {}", o.0, o.1),
            false,
            format!("Added {}", shown(o)),
        );
    }
    for o in a.difference(&b) {
        push(
            ChangeKind::OperationRemoved,
            &format!("{} {}", o.0, o.1),
            true,
            format!("Removed {}", shown(o)),
        );
    }
    let (a, b) = (schemas(old), schemas(new));
    for (name, fields) in &b {
        let Some(was) = a.get(name) else {
            push(
                ChangeKind::SchemaAdded,
                name,
                false,
                format!("Added schema {name}"),
            );
            continue;
        };
        for (f, now) in fields {
            let subject = format!("{name}.{f}");
            let Some(was) = was.get(f) else {
                let r = if now.required { "required " } else { "" };
                push(
                    ChangeKind::FieldAdded,
                    &subject,
                    now.required,
                    format!("Added {r}field {subject}"),
                );
                continue;
            };
            if was.required != now.required {
                let (kind, word) = if now.required {
                    (ChangeKind::FieldRequired, "required")
                } else {
                    (ChangeKind::FieldOptional, "optional")
                };
                push(kind, &subject, now.required, format!("Made {subject} {word}"));
            }
            if was.ty == now.ty {
                continue;
            }
            let ((ob, ov), (nb, nv)) = (parts(&was.ty), parts(&now.ty));
            if ob != nb {
                let widened = nb.contains("any") || ob.is_subset(&nb);
                let (kind, word) = if widened {
                    (ChangeKind::TypeWidened, "Widened")
                } else {
                    (ChangeKind::TypeNarrowed, "Narrowed")
                };
                push(
                    kind,
                    &subject,
                    !widened,
                    format!(
                        "{word} type of {subject} from {} to {}",
                        base(&was.ty),
                        base(&now.ty)
                    ),
                );
            }
            for v in ov.difference(&nv) {
                push(
                    ChangeKind::EnumValueRemoved,
                    &subject,
                    true,
                    format!("Removed enum value {v} from {subject}"),
                );
            }
            for v in nv.difference(&ov) {
                push(
                    ChangeKind::EnumValueAdded,
                    &subject,
                    false,
                    format!("Added enum value {v} to {subject}"),
                );
            }
        }
        for f in was.keys().filter(|f| !fields.contains_key(*f)) {
            let subject = format!("{name}.{f}");
            push(
                ChangeKind::FieldRemoved,
                &subject,
                true,
                format!("Removed field {subject}"),
            );
        }
    }
    for name in a.keys().filter(|n| !b.contains_key(*n)) {
        push(
            ChangeKind::SchemaRemoved,
            name,
            true,
            format!("Removed schema {name}"),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openapi::{Operation, Schema};
    use proptest::prelude::*;

    fn op(m: &str, p: &str) -> Operation {
        Operation {
            method: m.into(),
            path: p.into(),
            operation_id: None,
            summary: None,
            line: 1,
        }
    }

    fn field(n: &str, ty: &str, required: bool) -> SchemaField {
        SchemaField {
            name: n.into(),
            ty: ty.into(),
            required,
        }
    }

    fn spec(ops: &[(&str, &str)], fields: Vec<SchemaField>) -> Contract {
        Contract {
            bases: vec![String::new()],
            operations: ops.iter().map(|(m, p)| op(m, p)).collect(),
            schemas: vec![Schema {
                name: "Order".into(),
                fields,
            }],
            ..Default::default()
        }
    }

    fn lines(a: &Contract, b: &Contract) -> Vec<String> {
        let mut v: Vec<String> = diff(a, b)
            .iter()
            .map(|c| format!("{}{}", if c.breaking { "BREAKING " } else { "" }, c.summary))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn golden_pairs() {
        let both = [("get", "/orders"), ("delete", "/orders/{id}")];
        let base = spec(
            &both,
            vec![
                field("id", "integer|null", true),
                field("status", "string{new,paid,void}", false),
            ],
        );
        let same = base.schemas[0].fields.clone();
        let cases: Vec<(Contract, Vec<&str>)> = vec![
            (
                spec(
                    &[("get", "/orders"), ("delete", "/orders/{id}"), ("post", "/orders")],
                    same.clone(),
                ),
                vec!["Added POST /orders"],
            ),
            (
                spec(&[("get", "/orders")], same),
                vec!["BREAKING Removed DELETE /orders/{id}"],
            ),
            (
                spec(&both, vec![field("id", "integer|null", true)]),
                vec!["BREAKING Removed field Order.status"],
            ),
            (
                spec(
                    &both,
                    vec![
                        field("id", "integer", true),
                        field("note", "string", true),
                        field("status", "string{new,paid}", true),
                    ],
                ),
                vec![
                    "BREAKING Added required field Order.note",
                    "BREAKING Made Order.status required",
                    "BREAKING Narrowed type of Order.id from integer|null to integer",
                    "BREAKING Removed enum value void from Order.status",
                ],
            ),
            (
                spec(
                    &both,
                    vec![
                        field("id", "integer|null|string", false),
                        field("status", "string{new,paid,void,gone}", false),
                    ],
                ),
                vec![
                    "Added enum value gone to Order.status",
                    "Made Order.id optional",
                    "Widened type of Order.id from integer|null to integer|null|string",
                ],
            ),
        ];
        for (new, want) in cases {
            let mut want: Vec<String> = want.into_iter().map(Into::into).collect();
            want.sort();
            assert_eq!(lines(&base, &new), want);
        }
    }

    #[test]
    fn titles_summaries_and_lines_are_not_changes() {
        let a = spec(&[("get", "/orders")], vec![]);
        let mut b = a.clone();
        b.title = "Renamed".into();
        b.operations[0].summary = Some("new words".into());
        b.operations[0].line = 99;
        assert!(diff(&a, &b).is_empty());
        assert_eq!(canonical(&a), canonical(&b));
    }

    proptest! {
        #[test]
        fn diff_with_itself_is_empty(
            paths in proptest::collection::btree_set("/[a-z]{1,6}", 0..8),
            names in proptest::collection::btree_set("[a-z]{1,4}", 0..6),
            required in any::<bool>(),
        ) {
            let c = spec(
                &paths.iter().map(|p| ("get", p.as_str())).collect::<Vec<_>>(),
                names.iter().map(|n| field(n, "string{a,b}", required)).collect(),
            );
            prop_assert!(diff(&c, &c).is_empty());
        }

        #[test]
        fn added_then_removed_is_breaking_exactly_once(
            paths in proptest::collection::btree_set("/[a-z]{1,6}", 0..8),
            extra in "/z[a-z]{1,6}",
        ) {
            let v1 = spec(&paths.iter().map(|p| ("get", p.as_str())).collect::<Vec<_>>(), vec![]);
            let mut v2 = v1.clone();
            v2.operations.push(op("post", &extra));
            let breaking = |a: &Contract, b: &Contract| diff(a, b).iter().filter(|c| c.breaking).count();
            prop_assert_eq!(breaking(&v1, &v2) + breaking(&v2, &v1), 1);
        }
    }
}
