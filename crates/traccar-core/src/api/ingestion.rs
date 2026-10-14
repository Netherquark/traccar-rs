// SPDX-License-Identifier: Apache-2.0
//
// Device ingestion endpoint. Handles the OsmAnd / Traccar-Client HTTP payload
// on `/` (and `/osmand`). Mirrors `OsmAndProtocolDecoder`: unknown devices get
// 400 (query) or 404 (JSON), positions are persisted, the device's last state
// is updated, queued commands are returned as the 200 body.

use super::http::{Method, Request, Response};
use super::App;
use crate::error::{Error, Result};
use crate::model::{kind, Device, QueuedCommand};
use crate::store::Filter;

/// Handle a device ingestion request. Returns a bare status response, matching
/// Traccar's device-facing behaviour.
pub async fn handle(app: &App, req: &Request) -> Result<Response> {
    if !matches!(req.method, Method::Get | Method::Post) {
        return Ok(Response::new(405));
    }

    let raw = crate::protocol::RawIngest {
        content_type: req.header("content-type").map(|s| s.to_string()),
        query: req.query.clone(),
        body: req.body.clone(),
    };
    let decoded = crate::protocol::decode("osmand", &raw)?;

    let repo = app.repo();
    let existing = repo.device_by_unique_id(&decoded.unique_id).await?;
    let device = match existing {
        Some(d) => d,
        None => {
            if decoded.json {
                return Ok(Response::new(404));
            }
            if !app.config.register_unknown {
                return Ok(Response::new(400));
            }
            let mut d = Device {
                id: 0,
                name: Some(decoded.unique_id.clone()),
                unique_id: Some(decoded.unique_id.clone()),
                category: app.config.register_unknown_category.clone(),
                group_id: if app.config.register_unknown_group_id != 0 {
                    Some(app.config.register_unknown_group_id)
                } else {
                    None
                },
                ..Default::default()
            };
            let id = repo.insert(kind::DEVICE, &d).await?;
            d.id = id;
            d
        }
    };

    if device.disabled {
        return Ok(Response::new(403));
    }

    let mut position = decoded.position;
    position.device_id = device.id;
    position.protocol = Some("osmand".into());

    // If the message carried no fix, reuse the last known location (Traccar's
    // `getLastLocation`).
    if !position.valid && position.latitude == 0.0 && position.longitude == 0.0 {
        if let Some(last) = repo.last_position(device.id).await? {
            position.latitude = last.latitude;
            position.longitude = last.longitude;
        }
    }

    let previous = repo.last_position(device.id).await?;
    if let Some(ot) = previous.as_ref().and_then(|p| p.fix_time) {
        if position.fix_time.map(|t| t.millis() < ot.millis()).unwrap_or(false) {
            position.outdated = true;
        }
    }

    repo.insert_position(&mut position).await?;

    // Update device last-seen state.
    let mut updated = device.clone();
    updated.last_update = position.fix_time.or(position.device_time);
    updated.position_id = Some(position.id);
    if let Some(token) = &decoded.notification_token {
        updated
            .attributes
            .insert("notificationToken".into(), serde_json::Value::from(token.clone()));
    }
    repo.update(kind::DEVICE, updated.id, &updated).await?;

    // Return a queued command payload, if the device is waiting for one.
    if let Some(cmd) = pop_queued_command(app, device.id).await? {
        if let Some(data) = cmd
            .attributes
            .get("data")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
        {
            return Ok(Response::text(200, data));
        }
    }
    Ok(Response::new(200))
}

async fn pop_queued_command(app: &App, device_id: i64) -> Result<Option<QueuedCommand>> {
    let repo = app.repo();
    let rows = repo
        .list::<QueuedCommand>(kind::QUEUED_COMMAND, &Filter::new().eq("deviceId", device_id))
        .await?;
    if let Some(cmd) = rows.into_iter().next() {
        repo.delete(kind::QUEUED_COMMAND, cmd.id).await?;
        return Ok(Some(cmd));
    }
    Ok(None)
}

/// Helper used by command dispatch to remember an outbound command.
pub fn err_bad_device() -> Error {
    Error::bad_request("unknown device")
}
