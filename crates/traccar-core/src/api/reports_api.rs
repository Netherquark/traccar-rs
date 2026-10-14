// SPDX-License-Identifier: Apache-2.0
//
// Report resources (`/api/reports/*`). Shapes follow Traccar's OpenAPI.

use super::http::{Request, Response};
use super::positions::export;
use super::App;
use crate::auth::Principal;
use crate::error::{Error, Result};
use crate::model::{kind, CombinedReportItem, Event, Geofence, Id, Position};
use crate::time::parse_iso;

fn require_auth(p: &Principal) -> Result<()> {
    if p.authenticated() {
        Ok(())
    } else {
        Err(Error::unauthorized())
    }
}

fn window(req: &Request) -> Result<(i64, i64)> {
    let from = req
        .query_first("from")
        .and_then(parse_iso)
        .ok_or_else(|| Error::bad_request("from is required"))?
        .timestamp_millis();
    let to = req
        .query_first("to")
        .and_then(parse_iso)
        .ok_or_else(|| Error::bad_request("to is required"))?
        .timestamp_millis();
    Ok((from, to))
}

/// Resolve the set of devices addressed by `deviceId`/`groupId` params.
async fn target_devices(app: &App, req: &Request, p: &Principal) -> Result<Vec<Id>> {
    let repo = app.repo();
    let mut ids: Vec<Id> = req
        .query_all("deviceId")
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    for group in req.query_all("groupId").into_iter().filter_map(|s| s.parse::<Id>().ok()) {
        let groups = repo.user_group_ids(p.id()).await?;
        if !p.is_admin() && !groups.contains(&group) {
            return Err(Error::forbidden("no permission for group"));
        }
        let devices = repo.accessible_devices(p, p.is_admin()).await?;
        ids.extend(devices.into_iter().filter(|d| d.group_id == Some(group)).map(|d| d.id));
    }
    if ids.is_empty() {
        return Err(Error::bad_request("deviceId or groupId is required"));
    }
    // Validate access.
    for id in &ids {
        repo.require_device(p, *id).await?;
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

async fn positions_for(app: &App, device: Id, from: i64, to: i64) -> Result<Vec<Position>> {
    app.repo().positions_range(device, from, to).await
}

async fn events_for(app: &App, device: Id, from: i64, to: i64) -> Result<Vec<Event>> {
    let (lo, hi) = crate::repo::iso_bounds(from, to);
    let mut events: Vec<Event> = app
        .repo()
        .list(
            kind::EVENT,
            &crate::store::Filter::new().eq("deviceId", device).range_iso("eventTime", lo, hi),
        )
        .await?;
    events.sort_by_key(|e| e.event_time.map(|t| t.millis()).unwrap_or(0));
    Ok(events)
}

/// GET /api/reports/route → array of positions.
pub async fn route(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let mut out = Vec::new();
    for d in devices {
        out.extend(positions_for(app, d, from, to).await?);
    }
    Ok(Response::json(200, &out))
}

/// GET /api/reports/route/{type} → KML / CSV / GPX / GeoJSON.
pub async fn route_typed(app: &App, req: &Request, p: &Principal, kind: &str) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let mut positions = Vec::new();
    for d in devices {
        positions.extend(positions_for(app, d, from, to).await?);
    }
    Ok(export_response(kind, &positions))
}

pub async fn events_report(app: &App, req: &Request, p: &Principal, _kind: &str) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let mut out = Vec::new();
    for d in devices {
        out.extend(events_for(app, d, from, to).await?);
    }
    Ok(Response::json(200, &out))
}

pub async fn geofences(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let repo = app.repo();
    let mut out = Vec::new();
    for d in devices {
        let device = repo.device(d).await?.ok_or_else(Error::not_found)?;
        let events = events_for(app, d, from, to).await?;
        out.extend(crate::reports::geofences(&device, &events));
    }
    Ok(Response::json(200, &out))
}

pub async fn summary(app: &App, req: &Request, p: &Principal, _kind: &str) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let repo = app.repo();
    let mut out = Vec::new();
    for d in devices {
        let device = repo.device(d).await?.ok_or_else(Error::not_found)?;
        let positions = positions_for(app, d, from, to).await?;
        out.push(crate::reports::summary(&device, &positions));
    }
    Ok(Response::json(200, &out))
}

pub async fn trips(app: &App, req: &Request, p: &Principal, _kind: &str) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let repo = app.repo();
    let mut out = Vec::new();
    for d in devices {
        let device = repo.device(d).await?.ok_or_else(Error::not_found)?;
        let positions = positions_for(app, d, from, to).await?;
        out.extend(crate::reports::trips(&device, &positions));
    }
    Ok(Response::json(200, &out))
}

pub async fn stops(app: &App, req: &Request, p: &Principal, _kind: &str) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let repo = app.repo();
    let mut out = Vec::new();
    for d in devices {
        let device = repo.device(d).await?.ok_or_else(Error::not_found)?;
        let positions = positions_for(app, d, from, to).await?;
        out.extend(crate::reports::stops(&device, &positions));
    }
    Ok(Response::json(200, &out))
}

pub async fn combined(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let (from, to) = window(req)?;
    let devices = target_devices(app, req, p).await?;
    let repo = app.repo();
    let mut out = Vec::new();
    for d in devices {
        let device = repo.device(d).await?.ok_or_else(Error::not_found)?;
        let positions = positions_for(app, d, from, to).await?;
        let events = events_for(app, d, from, to).await?;
        out.push(crate::reports::combined(&device, &positions, &events, 1));
    }
    Ok(Response::json(200, &out))
}

/// GET /api/reports/devices/{type} → device list or export of last positions.
pub async fn devices_report(app: &App, _req: &Request, p: &Principal, kind: &str) -> Result<Response> {
    require_auth(p)?;
    let repo = app.repo();
    let devices = repo.accessible_devices(p, true).await?;
    if matches!(kind, "csv" | "kml" | "gpx" | "geojson") {
        let mut positions = Vec::new();
        for d in &devices {
            if let Some(pos) = repo.last_position(d.id).await? {
                positions.push(pos);
            }
        }
        return Ok(export_response(kind, &positions));
    }
    Ok(Response::json(200, &devices))
}

/// Also expose the raw geofence list for combined-report consumers.
#[allow(dead_code)]
pub async fn geofence_defs(app: &App, p: &Principal) -> Result<Vec<Geofence>> {
    app.repo().accessible_objects(p, kind::GEOFENCE, true).await
}

fn export_response(kind: &str, positions: &[Position]) -> Response {
    match kind {
        "kml" => Response::text(200, export::kml(positions)).content_type("application/vnd.google-earth.kml+xml"),
        "csv" => Response::text(200, export::csv(positions)).content_type("text/csv"),
        "gpx" => Response::text(200, export::gpx(positions)).content_type("application/gpx+xml"),
        "geojson" => Response::json_value(200, export::geojson(positions)),
        _ => Response::json(200, &Vec::<Position>::new()),
    }
}

/// Helper so report consumers can build a combined item themselves.
#[allow(dead_code)]
pub fn combined_item(device_id: Id, positions: Vec<Position>, events: Vec<Event>) -> CombinedReportItem {
    CombinedReportItem { device_id, route: crate::reports::route(&positions), events, positions }
}
