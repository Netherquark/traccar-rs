// SPDX-License-Identifier: Apache-2.0
//
// Bootstrap helpers: create the server row and an initial administrator when a
// store is empty. Idempotent, so it is safe to call on every cold start.

use crate::error::Result;
use crate::model::{kind, Server, User};
use crate::store::{ensure_server, Store};

/// Ensure the singleton server row exists.
pub async fn bootstrap_server(store: &dyn Store) -> Result<Server> {
    let v = ensure_server(store).await?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// Create an administrator if no user with `email` exists. Returns the user id
/// that now owns that email.
pub async fn bootstrap_admin(store: &dyn Store, email: &str, name: &str, password: &str) -> Result<i64> {
    let existing = store
        .list(kind::USER, &crate::store::Filter::new().eq("email", email))
        .await?;
    if let Some(row) = existing.into_iter().next() {
        if let Some(id) = row.get("id").and_then(|v| v.as_i64()) {
            return Ok(id);
        }
    }
    let hashed = crate::auth::create_hash(password)?;
    let user = User {
        id: 0,
        name: Some(name.to_string()),
        email: Some(email.to_string()),
        administrator: true,
        hashed_password: Some(hashed.hash),
        salt: Some(hashed.salt),
        ..Default::default()
    };
    let id = store.insert(kind::USER, serde_json::to_value(&user)?).await?;
    Ok(id)
}
