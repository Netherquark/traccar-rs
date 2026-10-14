// SPDX-License-Identifier: Apache-2.0
//
// Sharing resource (`/api/share/*`). Issues scoped, expiring tokens for a
// device or group and renders a minimal public viewer page.

use super::http::{Request, Response};
use super::App;
use crate::auth::Principal;
use crate::error::{Error, Result};
use crate::model::{kind, Id, Permission};
use crate::time::{parse_iso, Timestamp};
use serde_json::Value;

fn require_auth(p: &Principal) -> Result<()> {
    if p.authenticated() {
        Ok(())
    } else {
        Err(Error::unauthorized())
    }
}

/// POST /api/share/device — body/query: `deviceId`, `expiration`.
pub async fn share_device(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let device_id = crate::api::objects::extract_id(req, "deviceId")?;
    app.repo().require_device(p, device_id).await?;
    let expiration = extract_expiration(req);
    let token = app
        .tokens
        .generate_with_expiry(0, Some(&format!("share:device:{device_id}")), expiration)?;
    app.record_audit("share", "device", device_id, p, req.remote.as_deref()).await;
    Ok(Response::text(200, token))
}

/// POST /api/share/group — body/query: `groupId`, `expiration`.
pub async fn share_group(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let group_id = crate::api::objects::extract_id(req, "groupId")?;
    app.repo().require_object(p, kind::GROUP, group_id).await?;
    let expiration = extract_expiration(req);
    let token = app
        .tokens
        .generate_with_expiry(0, Some(&format!("share:group:{group_id}")), expiration)?;
    app.record_audit("share", "group", group_id, p, req.remote.as_deref()).await;
    Ok(Response::text(200, token))
}

fn extract_expiration(req: &Request) -> Option<Timestamp> {
    let raw = req
        .query_first("expiration")
        .map(|s| s.to_string())
        .or_else(|| req.form_first("expiration"));
    raw.and_then(|s| parse_iso(&s)).map(Timestamp)
}

/// GET /api/share/{token} — public viewer page.
pub async fn view(app: &App, req: &Request, _p: &Principal) -> Result<Response> {
    let token = req
        .path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let data = app.tokens.decode(&token).map_err(|_| Error::unauthorized())?;
    if data.expired() {
        return Err(Error::unauthorized());
    }
    let scope = data.s.clone().unwrap_or_default();
    let repo = app.repo();

    let (title, body) = if let Some(id) = scope.strip_prefix("share:device:") {
        let device_id: Id = id.parse().map_err(|_| Error::bad_request("bad scope"))?;
        let device = repo.device(device_id).await?.ok_or_else(Error::not_found)?;
        let last = repo.last_position(device_id).await?;
        (format!("Device {}", device.name.clone().unwrap_or_default()), render_last(last))
    } else if let Some(id) = scope.strip_prefix("share:group:") {
        let group_id: Id = id.parse().map_err(|_| Error::bad_request("bad scope"))?;
        let devices = repo
            .list::<crate::model::Device>(kind::DEVICE, &crate::store::Filter::new().eq("groupId", group_id))
            .await?;
        let mut rows = String::new();
        for d in devices {
            let last = repo.last_position(d.id).await?;
            rows.push_str(&format!(
                "<h3>{}</h3>{}",
                html_escape(d.name.as_deref().unwrap_or("device")),
                render_last(last)
            ));
        }
        (format!("Group {group_id}"), rows)
    } else {
        return Err(Error::unauthorized());
    };

    let page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title></head>\
         <body><h1>{}</h1>{}</body></html>",
        html_escape(&title),
        html_escape(&title),
        body
    );
    Ok(Response::text(200, page).content_type("text/html; charset=utf-8"))
}

fn render_last(last: Option<crate::model::Position>) -> String {
    match last {
        None => "<p>No position reported.</p>".to_string(),
        Some(pos) => format!(
            "<p>Last position: {:.5}, {:.5} at {}</p>",
            pos.latitude,
            pos.longitude,
            pos.fix_time.map(|t| t.to_string()).unwrap_or_default()
        ),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Check that a principal may create/delete the given permission link.
pub async fn require_can_link(app: &App, p: &Principal, perm: &Permission) -> Result<()> {
    let Some((a_field, a_id, b_field, b_id)) = perm.pair() else {
        return Err(Error::bad_request("invalid permission pair"));
    };
    if p.is_admin() {
        return Ok(());
    }
    // The userId side must be the caller (or a user they manage).
    for (field, id) in [(&a_field, a_id), (&b_field, b_id)] {
        if field == "userId" {
            if id != p.id() && !app.repo().can_manage_user(p, id).await {
                return Err(Error::forbidden("cannot link another user"));
            }
        } else if field == "managedUserId" {
            return Err(Error::forbidden("only administrators can manage users"));
        } else if let Some(object_kind) = field.strip_suffix("Id") {
            // camelCase field -> kind name (deviceId -> device).
            let kind_name = object_kind.to_lowercase();
            if !app.repo().can_access_object(p, &kind_name, id).await {
                return Err(Error::forbidden("no permission for linked object"));
            }
        }
    }
    Ok(())
}

/// Parse a bulk-permission body. Accepts either an array of permission objects
/// or an object mapping one or two `<name>Id` keys to arrays of ids (the
/// Traccar cross-product form).
pub fn parse_permission_array(value: &Value) -> Result<(Vec<Permission>, bool)> {
    if let Some(arr) = value.as_array() {
        let mut rows = Vec::new();
        for item in arr {
            let perm: Permission = serde_json::from_value(item.clone())
                .map_err(|e| Error::bad_request(format!("invalid permission: {e}")))?;
            rows.push(perm);
        }
        return Ok((rows, false));
    }
    let Some(obj) = value.as_object() else {
        return Err(Error::bad_request("expected object or array"));
    };
    // Collect (field, ids) pairs.
    let mut pairs: Vec<(String, Vec<Id>)> = Vec::new();
    for (k, v) in obj {
        if permission_field_for_key(k).is_none() {
            continue;
        }
        let ids: Vec<Id> = match v {
            Value::Array(a) => a.iter().filter_map(|x| x.as_i64()).collect(),
            Value::Number(n) => n.as_i64().into_iter().collect(),
            _ => continue,
        };
        if !ids.is_empty() {
            pairs.push((k.clone(), ids));
        }
    }
    if pairs.len() != 2 {
        return Err(Error::bad_request("bulk permissions need exactly two id fields"));
    }
    let mut rows = Vec::new();
    for a in &pairs[0].1 {
        for b in &pairs[1].1 {
            let mut perm = Permission::default();
            perm.set(&pairs[0].0, *a);
            perm.set(&pairs[1].0, *b);
            rows.push(perm);
        }
    }
    Ok((rows, true))
}

fn permission_field_for_key(key: &str) -> Option<&'static str> {
    const KEYS: [&str; 12] = [
        "userId", "deviceId", "groupId", "geofenceId", "notificationId", "calendarId", "attributeId",
        "driverId", "managedUserId", "commandId", "maintenanceId", "orderId",
    ];
    KEYS.iter().find(|k| **k == key).copied()
}
