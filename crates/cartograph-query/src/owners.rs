//! Owners from CODEOWNERS (XS-23, D14): who owns what now, and who owned one file at a step.
//!
//! Owner strings are untrusted text (F8). The indexer keeps only `@user`, `@org/team` and
//! `user@host` shapes, so nothing returned here can carry markup; clients still render it as text.

use cartograph_api::types::{Owner, OwnerKind};

use crate::{QueryError, Snapshot};

impl Snapshot<'_> {
    /// `GET …/owners`: each owner with the patterns that name them and the files they own now.
    pub fn owners(&self) -> Result<Vec<Owner>, QueryError> {
        let mut st = self.conn().prepare_cached(
            "SELECT owner, kind, pattern, count(*) FROM ownership WHERE to_seq IS NULL
             GROUP BY owner, kind, pattern ORDER BY owner, pattern",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out: Vec<Owner> = Vec::new();
        for row in rows {
            let (owner, kind, pattern, n) = row?;
            let n = n.max(0) as u32;
            match out.last_mut() {
                Some(o) if o.id == owner => {
                    o.patterns.push(pattern);
                    o.nodes += n;
                }
                _ => out.push(Owner {
                    id: owner.clone(),
                    name: owner,
                    kind: if kind == "team" {
                        OwnerKind::Team
                    } else {
                        OwnerKind::Person
                    },
                    patterns: vec![pattern],
                    nodes: n,
                }),
            }
        }
        Ok(out)
    }

    /// The owners of node `row` at step `seq`, by name: a file's, none for a scope.
    pub(crate) fn owners_of_node(&self, row: i64, seq: u32) -> Result<Vec<String>, QueryError> {
        Ok(self
            .conn()
            .prepare_cached(
                "SELECT DISTINCT o.owner FROM node n JOIN ownership o ON o.file = n.file
                 WHERE n.id = ?1 AND o.from_seq <= ?2 AND (o.to_seq IS NULL OR o.to_seq > ?2)
                 ORDER BY o.owner",
            )?
            .query_map([row, i64::from(seq)], |r| r.get(0))?
            .collect::<Result<_, _>>()?)
    }
}
