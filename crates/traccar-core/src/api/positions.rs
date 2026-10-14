// SPDX-License-Identifier: Apache-2.0
//
// Position history resource (`/api/positions`) and export formats
// (KML / CSV / GPX), matching Traccar's shapes.

use super::http::{Request, Response};
use super::App;
use crate::auth::Principal;
use crate::error::{Error, Result};
use crate::model::{kind, Id, Position};
use crate::time::parse_iso;

fn require_auth(p: &Principal) -> Result<()> {
    if p.authenticated() {
        Ok(())
    } else {
        Err(Error::unauthorized())
    }
}

/// Devices requested via repeated `deviceId` params, validated against access.
async fn requested_devices(app: &App, req: &Request, p: &Principal) -> Result<Vec<Id>> {
    let ids: Vec<Id> = req
        .query_all("deviceId")
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let repo = app.repo();
    let mut out = Vec::new();
    for id in ids {
        repo.require_device(p, id).await?;
        out.push(id);
    }
    Ok(out)
}

fn window(req: &Request) -> (i64, i64) {
    let from = req.query_first("from").and_then(parse_iso).map(|t| t.timestamp_millis()).unwrap_or(i64::MIN);
    let to = req.query_first("to").and_then(parse_iso).map(|t| t.timestamp_millis()).unwrap_or(i64::MAX);
    (from, to)
}

pub async fn list(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let repo = app.repo();
    let (from, to) = window(req);

    // By explicit position id(s).
    let id_params: Vec<Id> = req
        .query_all("id")
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    if !id_params.is_empty() {
        let mut out = Vec::new();
        for id in id_params {
            if let Some(pos) = repo.position(id).await? {
                if repo.require_device(p, pos.device_id).await.is_ok() {
                    out.push(pos);
                }
            }
        }
        return Ok(Response::json(200, &out));
    }

    let device_ids = requested_devices(app, req, p).await?;
    if device_ids.is_empty() {
        return Err(Error::bad_request("deviceId is required"));
    }
    let mut positions = Vec::new();
    for device_id in device_ids {
        positions.extend(repo.positions_range(device_id, from, to).await?);
    }
    positions.sort_by_key(|p: &Position| p.fix_time.map(|t| t.millis()).unwrap_or(0));
    Ok(Response::json(200, &positions))
}

pub async fn delete(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let device_ids = requested_devices(app, req, p).await?;
    if device_ids.is_empty() {
        return Err(Error::bad_request("deviceId is required"));
    }
    let (from, to) = window(req);
    for device_id in device_ids {
        app.repo().delete_positions_range(device_id, from, to).await?;
    }
    Ok(Response::no_content())
}

pub async fn delete_one(app: &App, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    let repo = app.repo();
    let pos = repo.position(id).await?.ok_or_else(Error::not_found)?;
    repo.require_device(p, pos.device_id).await?;
    repo.delete(kind::POSITION, id).await?;
    Ok(Response::no_content())
}

pub async fn kml(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    let positions = collect(app, req, p).await?;
    Ok(Response::text(200, export::kml(&positions))
        .content_type("application/vnd.google-earth.kml+xml"))
}

pub async fn csv(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    let positions = collect(app, req, p).await?;
    Ok(Response::text(200, export::csv(&positions)).content_type("text/csv"))
}

pub async fn gpx(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    let positions = collect(app, req, p).await?;
    Ok(Response::text(200, export::gpx(&positions)).content_type("application/gpx+xml"))
}

async fn collect(app: &App, req: &Request, p: &Principal) -> Result<Vec<Position>> {
    require_auth(p)?;
    let repo = app.repo();
    let (from, to) = window(req);
    let device_ids = requested_devices(app, req, p).await?;
    let mut out = Vec::new();
    for device_id in device_ids {
        out.extend(repo.positions_range(device_id, from, to).await?);
    }
    out.sort_by_key(|x: &Position| x.fix_time.map(|t| t.millis()).unwrap_or(0));
    Ok(out)
}

/// Export serializers shared by position and report endpoints.
pub mod export {
    use crate::model::Position;
    use crate::time::format_iso;

    pub fn csv(positions: &[Position]) -> String {
        let mut s = String::from("id,deviceId,protocol,deviceTime,fixTime,serverTime,valid,latitude,longitude,altitude,speed,course,accuracy\n");
        for p in positions {
            let t = |t: Option<crate::time::Timestamp>| t.map(|x| format_iso(x.0)).unwrap_or_default();
            s.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                p.id,
                p.device_id,
                p.protocol.clone().unwrap_or_default(),
                t(p.device_time),
                t(p.fix_time),
                t(p.server_time),
                p.valid,
                p.latitude,
                p.longitude,
                p.altitude,
                p.speed,
                p.course,
                p.accuracy,
            ));
        }
        s
    }

    pub fn kml(positions: &[Position]) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>\n");
        for p in positions.iter().filter(|p| p.valid) {
            s.push_str(&format!(
                "<Placemark><TimeStamp><when>{}</when></TimeStamp><Point><coordinates>{},{}</coordinates></Point></Placemark>\n",
                p.fix_time.map(|t| format_iso(t.0)).unwrap_or_default(),
                p.longitude,
                p.latitude,
            ));
        }
        s.push_str("</Document></kml>\n");
        s
    }

    pub fn gpx(positions: &[Position]) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"traccar-rs\" xmlns=\"http://www.topografix.com/GPX/1/1\"><trk><trkseg>\n");
        for p in positions.iter().filter(|p| p.valid) {
            s.push_str(&format!(
                "<trkpt lat=\"{}\" lon=\"{}\"><time>{}</time></trkpt>\n",
                p.latitude,
                p.longitude,
                p.fix_time.map(|t| format_iso(t.0)).unwrap_or_default(),
            ));
        }
        s.push_str("</trkseg></trk></gpx>\n");
        s
    }

    /// GeoJSON FeatureCollection of the route as a LineString per device.
    pub fn geojson(positions: &[Position]) -> serde_json::Value {
        let coords: Vec<Vec<f64>> = positions
            .iter()
            .filter(|p| p.valid)
            .map(|p| vec![p.longitude, p.latitude])
            .collect();
        serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "geometry": { "type": "LineString", "coordinates": coords },
                "properties": {}
            }]
        })
    }
}
