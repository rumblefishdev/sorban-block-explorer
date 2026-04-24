//! Helpers that turn a `limit+1` row fetch into the canonical
//! [`Paginated`](crate::openapi::schemas::Paginated) envelope.
//!
//! The helpers live outside any specific ORM layer on purpose: every
//! list endpoint in this crate builds its SQL with a hand-written
//! `sqlx::QueryBuilder`, so the pagination contract is "fetch one extra
//! row, hand it here, receive a ready envelope back". This keeps the
//! SQL plans per-endpoint explicit (no generic WHERE clause injection)
//! while centralising the wire-shape choreography.

use crate::openapi::schemas::{PageInfo, Paginated};

use super::cursor;

/// Trim a `limit+1` row slice to `limit` rows and derive the
/// [`PageInfo`] for the page.
///
/// `rows` comes in with up to `limit + 1` items — the extra row, when
/// present, is the "peek" that signals another page exists. This
/// function drops that peek in place, returns the `PageInfo` with
/// `has_more` set accordingly, and builds `cursor` from the last kept
/// row via `cursor_of`.
///
/// `cursor_of` is called at most once (for the final row on the page)
/// and returns the opaque cursor string the client will send back to
/// fetch the next page. Resources use [`cursor::encode`] plus a
/// payload struct (e.g. [`cursor::TsIdCursor`]) for this.
pub fn finalize_page<Row>(
    rows: &mut Vec<Row>,
    limit: u32,
    cursor_of: impl FnOnce(&Row) -> String,
) -> PageInfo {
    let limit_usize = limit as usize;
    let has_more = rows.len() > limit_usize;
    if has_more {
        rows.truncate(limit_usize);
    }

    let cursor = if has_more {
        rows.last().map(cursor_of)
    } else {
        None
    };

    PageInfo {
        cursor,
        limit,
        has_more,
    }
}

/// Convenience over [`finalize_page`] for the overwhelmingly common case
/// of a `(created_at, id)` cursor: pass the accessor closures for those
/// two fields and this helper assembles the [`cursor::TsIdCursor`]
/// payload and encodes it.
pub fn finalize_ts_id_page<Row>(
    rows: &mut Vec<Row>,
    limit: u32,
    ts_of: impl Fn(&Row) -> chrono::DateTime<chrono::Utc>,
    id_of: impl Fn(&Row) -> i64,
) -> PageInfo {
    finalize_page(rows, limit, |r| {
        cursor::encode(&cursor::TsIdCursor::new(ts_of(r), id_of(r)))
    })
}

/// Assemble rows + [`PageInfo`] into the canonical [`Paginated`] envelope.
///
/// Separate from [`finalize_page`] so handlers can map DB rows to
/// response DTOs between the two calls — the common shape is:
///
/// ```ignore
/// let page = finalize_ts_id_page(&mut db_rows, limit, |r| r.created_at, |r| r.id);
/// let data: Vec<ResponseItem> = db_rows.into_iter().map(into_response_item).collect();
/// Json(into_envelope(data, page))
/// ```
pub fn into_envelope<T>(data: Vec<T>, page: PageInfo) -> Paginated<T>
where
    T: utoipa::ToSchema,
{
    Paginated { data, page }
}

// ---------------------------------------------------------------------------
// SQL helper: cursor predicate for `(ts, id)` pagination
// ---------------------------------------------------------------------------

/// Append a cursor predicate to a `sqlx::QueryBuilder` for the common
/// `(created_at DESC, id DESC)` ordering.
///
/// Generates `(<ts_col>, <id_col>) < ($ts, $id)` with the cursor values
/// properly bound. The caller is responsible for preceding the predicate
/// with the correct `WHERE` / `AND` glue — consistent with how the
/// existing `fetch_list` query in the transactions module tracks whether
/// a `WHERE` clause has already been emitted.
pub fn push_ts_id_cursor_predicate(
    qb: &mut sqlx::QueryBuilder<'_, sqlx::Postgres>,
    ts_col: &str,
    id_col: &str,
    payload: &cursor::TsIdCursor,
) {
    qb.push(" (");
    qb.push(ts_col);
    qb.push(", ");
    qb.push(id_col);
    qb.push(") < (");
    qb.push_bind(payload.ts);
    qb.push(", ");
    qb.push_bind(payload.id);
    qb.push(")");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Timelike, Utc};

    #[derive(Debug, Clone)]
    struct Row {
        id: i64,
        ts: DateTime<Utc>,
    }

    fn row(id: i64, sec: u32) -> Row {
        Row {
            id,
            ts: Utc.with_ymd_and_hms(2026, 4, 24, 12, 0, sec).unwrap(),
        }
    }

    #[test]
    fn no_extra_row_means_last_page() {
        let mut rows = vec![row(1, 0), row(2, 1), row(3, 2)];
        let page = finalize_page(&mut rows, 5, |_| String::from("cursor"));
        assert_eq!(rows.len(), 3);
        assert!(page.cursor.is_none());
        assert!(!page.has_more);
        assert_eq!(page.limit, 5);
    }

    #[test]
    fn extra_row_gets_truncated_and_produces_cursor() {
        let mut rows = vec![row(1, 0), row(2, 1), row(3, 2), row(4, 3)];
        let page = finalize_page(&mut rows, 3, |r| format!("id-{}", r.id));
        assert_eq!(rows.len(), 3);
        assert_eq!(page.cursor.as_deref(), Some("id-3"));
        assert!(page.has_more);
        assert_eq!(page.limit, 3);
    }

    #[test]
    fn exact_limit_means_last_page() {
        let mut rows = vec![row(1, 0), row(2, 1), row(3, 2)];
        let page = finalize_page(&mut rows, 3, |_| String::from("cursor"));
        assert_eq!(rows.len(), 3);
        assert!(!page.has_more);
        assert!(page.cursor.is_none());
    }

    #[test]
    fn ts_id_helper_encodes_last_rows_cursor() {
        let mut rows = vec![row(1, 0), row(2, 1), row(3, 2), row(4, 3)];
        let page = finalize_ts_id_page(&mut rows, 3, |r| r.ts, |r| r.id);
        assert_eq!(rows.len(), 3);
        assert!(page.has_more);

        let encoded = page.cursor.clone().unwrap();
        let decoded: cursor::TsIdCursor = cursor::decode(&encoded).unwrap();
        assert_eq!(decoded.id, 3);
        assert_eq!(decoded.ts.second(), 2);
    }
}
