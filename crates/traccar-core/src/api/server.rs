// SPDX-License-Identifier: Apache-2.0
//
// Server information, statistics, audit log and password management.

use super::http::{Request, Response};
use super::App;
use crate::auth::Principal;
use crate::error::{Error, Result};
use crate::model::{kind, Audit, Server, Statistics, User};
use crate::store::{ensure_server, Filter};
use crate::time::{parse_iso, Timestamp};

pub async fn get_server(app: &App) -> Result<Response> {
    let mut v = ensure_server(app.store.as_ref()).await?;
    // Advertise the live version from config.
    if let Some(o) = v.as_object_mut() {
        o.insert("version".into(), serde_json::Value::from(crate::VERSION));
    }
    Ok(Response::json_value(200, v))
}

pub async fn put_server(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    if !p.is_admin() {
        return Err(Error::forbidden("not an administrator"));
    }
    let incoming: Server = req.json()?;
    let mut server = Server { id: 1, ..incoming };
    server.version = Some(crate::VERSION.to_string());
    app.store
        .update(kind::SERVER, 1, serde_json::to_value(&server)?)
        .await?;
    app.record_audit("edit", "server", 1, p, req.remote.as_deref()).await;
    Ok(Response::json(200, &server))
}

pub async fn statistics(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    if !p.authenticated() {
        return Err(Error::unauthorized());
    }
    let from = req.query_first("from").and_then(parse_iso).map(Timestamp);
    let to = req.query_first("to").and_then(parse_iso).map(Timestamp);
    let repo = app.repo();
    let active_users = repo.users_all().await?.into_iter().filter(|u| !u.disabled).count() as i64;
    let devices = repo.list::<crate::model::Device>(kind::DEVICE, &Filter::new()).await?;
    let active_devices = devices.len() as i64;
    let requests = app.store.count(kind::AUDIT, &Filter::new()).await.unwrap_or(0);
    let messages_received = app.store.count(kind::POSITION, &Filter::new()).await.unwrap_or(0);
    let stat = Statistics {
        capture_time: Some(to.or(from).unwrap_or_else(Timestamp::now)),
        active_users,
        active_devices,
        requests,
        messages_received,
        messages_stored: messages_received,
    };
    Ok(Response::json(200, &vec![stat]))
}

pub async fn audit_list(app: &App, _req: &Request, p: &Principal) -> Result<Response> {
    if !p.authenticated() {
        return Err(Error::unauthorized());
    }
    let mut filter = Filter::new().order("actionTime", true);
    if !p.is_admin() {
        filter = filter.eq("userId", p.id());
    }
    let rows: Vec<Audit> = app.repo().list(kind::AUDIT, &filter).await?;
    Ok(Response::json(200, &rows))
}

pub async fn password_update(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    if !p.authenticated() {
        return Err(Error::unauthorized());
    }
    let password = req.form_first("password").ok_or_else(|| Error::bad_request("missing password"))?;
    let mut user: User = p.user.clone();
    let hashed = crate::auth::create_hash(&password)?;
    user.hashed_password = Some(hashed.hash);
    user.salt = Some(hashed.salt);
    app.repo().update(kind::USER, user.id, &user).await?;
    app.record_audit("edit", "user", user.id, p, req.remote.as_deref()).await;
    Ok(Response::no_content())
}

/// GET /api/server/geocode — reverse geocoding. No provider is configured in
/// the portable core, so this returns an empty result set (the shape Traccar
/// returns when the geocoder finds nothing).
pub async fn geocode(_app: &App, _req: &Request, p: &Principal) -> Result<Response> {
    if !p.authenticated() {
        return Err(Error::unauthorized());
    }
    Ok(Response::json(200, &Vec::<serde_json::Value>::new()))
}

/// GET /api/server/timezones — the IANA zone list Traccar exposes for the UI.
pub fn timezones() -> Response {
    let zones = [
        "UTC", "Asia/Kolkata", "Asia/Dubai", "Asia/Singapore", "Asia/Tokyo", "Europe/London",
        "Europe/Paris", "Europe/Berlin", "Europe/Moscow", "America/New_York", "America/Chicago",
        "America/Denver", "America/Los_Angeles", "America/Sao_Paulo", "Australia/Sydney",
        "Africa/Johannesburg", "Africa/Lagos", "Pacific/Auckland",
    ];
    let body: Vec<serde_json::Value> = zones.iter().map(|z| serde_json::json!({ "id": z })).collect();
    Response::json(200, &body)
}

/// GET /api/server/gc — Traccar triggers a storage compaction. A managed D1
/// instance handles vacuuming itself, so this is an acknowledged no-op.
pub async fn gc(_app: &App, p: &Principal) -> Result<Response> {
    if !p.is_admin() {
        return Err(Error::forbidden("not an administrator"));
    }
    Ok(Response::ok())
}

/// GET /api/server/cache — clears the in-memory cache. There is no shared
/// mutable cache in the Worker model, so this is an acknowledged no-op.
pub async fn cache(_app: &App, p: &Principal) -> Result<Response> {
    if !p.is_admin() {
        return Err(Error::forbidden("not an administrator"));
    }
    Ok(Response::ok())
}
