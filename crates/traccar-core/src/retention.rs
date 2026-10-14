// SPDX-License-Identifier: Apache-2.0
//
// Retention enforcement. Called by the Worker's scheduled handler. Deliberately
// conservative: only positions and events older than the cutoff are removed,
// and only when a retention window is configured.

use crate::error::Result;
use crate::model::kind;
use crate::store::{Filter, Store};
use crate::time::Timestamp;

/// Delete positions and events older than `cutoff_millis`. Returns the number of
/// positions removed. Never touches users, devices, or the audit log.
pub async fn enforce(store: &dyn Store, cutoff_millis: i64) -> Result<usize> {
    let (lo, hi) = bounds(cutoff_millis);
    let positions = store
        .delete_many(kind::POSITION, &Filter::new().range_iso("fixTime", lo.clone(), hi.clone()))
        .await?;
    let _ = store
        .delete_many(kind::EVENT, &Filter::new().range_iso("eventTime", lo, hi))
        .await?;
    Ok(positions)
}

fn bounds(cutoff_millis: i64) -> (String, String) {
    use crate::time::format_iso;
    let lo = format_iso(Timestamp::from_millis(0).0);
    let hi = format_iso(Timestamp::from_millis(cutoff_millis).0);
    (lo, hi)
}
