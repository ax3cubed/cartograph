//! Paging (F2, SRV-07): `limit` rows from `cursor` on. A cursor is an opaque offset into an answer
//! whose order is fixed for one generation, so it stays valid while the generation does.

use cartograph_api::types::page_limit;

use crate::QueryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub offset: usize,
    pub limit: usize,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: cartograph_api::types::DEFAULT_LIMIT as usize,
        }
    }
}

impl Page {
    /// Checks `limit` (1 to 1,000, 50 when absent) and `cursor` (one this layer handed out).
    pub fn parse(limit: Option<u32>, cursor: Option<&str>) -> Result<Self, QueryError> {
        let limit = page_limit(limit).map_err(|e| QueryError::bad(e.what, e.why, e.action))?;
        let offset = match cursor.filter(|c| !c.is_empty()) {
            None => 0,
            Some(c) => c
                .strip_prefix('o')
                .and_then(|d| d.parse::<usize>().ok())
                .ok_or_else(|| {
                    QueryError::bad(
                        "the cursor is malformed",
                        "a cursor is the next_cursor of an earlier answer",
                        "start again without a cursor",
                    )
                })?,
        };
        Ok(Self {
            offset,
            limit: limit as usize,
        })
    }

    /// The page of `rows`, and the cursor of the next one when there is more.
    pub fn take<T>(self, rows: Vec<T>) -> Paged<T> {
        let total = rows.len();
        let rows: Vec<T> = rows
            .into_iter()
            .skip(self.offset)
            .take(self.limit)
            .collect();
        let end = self.offset + rows.len();
        Paged {
            rows,
            next_cursor: (end < total).then(|| format!("o{end}")),
        }
    }
}

/// One page of an answer.
#[derive(Debug, Clone)]
pub struct Paged<T> {
    pub rows: Vec<T>,
    pub next_cursor: Option<String>,
}
