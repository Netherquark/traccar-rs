// SPDX-License-Identifier: Apache-2.0
//
// Position processing: the parts of Traccar's `ProcessingHandler` that matter
// for clients — distance accumulation, motion transitions, geofence
// enter/exit, and alarm-to-event mapping. The core is pure so it can be unit
// tested without a database.

use crate::model::{event_type, position_key::*, *};
use crate::time::Timestamp;
use serde_json::Value;

const EARTH_RADIUS_M: f64 = 6_373_000.0;

/// Great-circle distance in meters.
pub fn distance_meters(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().asin()
}

/// Total distance along a polyline.
pub fn path_distance(positions: &[Position]) -> f64 {
    let mut total = 0.0;
    for w in positions.windows(2) {
        if w[0].valid && w[1].valid {
            total += distance_meters(w[0].latitude, w[0].longitude, w[1].latitude, w[1].longitude);
        }
    }
    total
}

/// Decide motion state. Prefers an explicit `motion` attribute, else falls back
/// to a speed threshold (1 knot, Traccar's default for the moving filter).
pub fn is_moving(p: &Position) -> bool {
    if let Some(m) = p.attr_bool(MOTION) {
        return m;
    }
    if let Some(ign) = p.attr_bool(IGNITION) {
        return ign;
    }
    p.speed >= 1.0
}

/// A parsed geofence area (WKT subset Traccar writes).
#[derive(Clone, Debug)]
pub enum Area {
    Circle { lat: f64, lon: f64, radius: f64 },
    Polygon(Vec<(f64, f64)>), // (lat, lon)
}

impl Area {
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        match self {
            Area::Circle { lat: clat, lon: clon, radius } => distance_meters(*clat, *clon, lat, lon) <= *radius,
            Area::Polygon(points) => point_in_polygon(lat, lon, points),
        }
    }
}

/// Parse the WKT forms Traccar stores: `CIRCLE (lat lon, radius)` and
/// `POLYGON ((lon lat, lon lat, ...))`.
pub fn parse_area(wkt: &str) -> Option<Area> {
    let s = wkt.trim();
    let upper = s.to_ascii_uppercase();
    if upper.starts_with("CIRCLE") {
        let inner = s[s.find('(')? + 1..s.rfind(')')?].trim();
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() != 2 {
            return None;
        }
        let coords: Vec<f64> = parts[0].split_whitespace().filter_map(|x| x.parse().ok()).collect();
        let radius: f64 = parts[1].trim().parse().ok()?;
        if coords.len() >= 2 {
            return Some(Area::Circle { lat: coords[0], lon: coords[1], radius });
        }
        return None;
    }
    if upper.starts_with("POLYGON") {
        let start = s.find('(')?;
        let inner = &s[start + 1..s.rfind(')')?];
        let inner = inner.trim().trim_start_matches('(').trim_end_matches(')');
        let mut points = Vec::new();
        for pair in inner.split(',') {
            let nums: Vec<f64> = pair.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if nums.len() >= 2 {
                // WKT polygons are (lon lat); store (lat, lon).
                points.push((nums[1], nums[0]));
            }
        }
        if points.len() >= 3 {
            return Some(Area::Polygon(points));
        }
    }
    None
}

fn point_in_polygon(lat: f64, lon: f64, poly: &[(f64, f64)]) -> bool {
    let (mut inside, n) = (false, poly.len());
    let mut j = n - 1;
    for i in 0..n {
        let (yi, xi) = poly[i]; // (lat, lon)
        let (yj, xj) = poly[j];
        if ((xi > lon) != (xj > lon)) && (lat < (yj - yi) * (lon - xi) / (xj - xi) + yi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Compute the `distance` attribute between two positions, when both are valid.
pub fn segment_distance(prev: &Position, cur: &Position) -> Option<f64> {
    if prev.valid && cur.valid && prev.id != 0 {
        Some(distance_meters(prev.latitude, prev.longitude, cur.latitude, cur.longitude))
    } else {
        None
    }
}

/// Result of processing one position.
#[derive(Debug, Default)]
pub struct Processed {
    pub events: Vec<Event>,
    pub distance: f64,
}

/// Process a freshly stored position against the previous one and the relevant
/// geofences, updating position attributes and emitting events.
pub fn process(
    device: &Device,
    position: &mut Position,
    previous: Option<&Position>,
    geofences: &[Geofence],
    prev_geofence_ids: &[Id],
) -> Processed {
    let mut out = Processed::default();

    // Distance accumulation.
    if let Some(prev) = previous {
        if let Some(d) = segment_distance(prev, position) {
            position.set_attr(DISTANCE, Value::from(d));
            let accumulated = prev.attr_f64(TOTAL_DISTANCE).unwrap_or(0.0) + d;
            position.set_attr(TOTAL_DISTANCE, Value::from(accumulated));
            out.distance = d;
        }
    }

    // Geofences.
    if position.valid {
        let mut current: Vec<Id> = Vec::new();
        for g in geofences {
            let Some(area) = g.area.as_deref().and_then(parse_area) else { continue };
            if area.contains(position.latitude, position.longitude) {
                current.push(g.id);
            }
        }
        // Enter / exit events.
        for id in &current {
            if !prev_geofence_ids.contains(id) {
                out.events.push(event(device.id, event_type::GEOFENCE_ENTER, position, Some(*id)));
            }
        }
        for id in prev_geofence_ids {
            if !current.contains(id) {
                out.events.push(event(device.id, event_type::GEOFENCE_EXIT, position, Some(*id)));
            }
        }
        position.geofence_ids = if current.is_empty() { None } else { Some(current) };
    }

    // Motion transitions.
    if let Some(prev) = previous {
        let was = is_moving(prev);
        let now = is_moving(position);
        if was != now {
            let t = if now { event_type::DEVICE_MOVING } else { event_type::DEVICE_STOPPED };
            out.events.push(event(device.id, t, position, None));
        }
    }

    // Alarm mapping.
    if let Some(alarm) = position.attr_str(ALARM) {
        for a in alarm.split(',') {
            let a = a.trim();
            if !a.is_empty() {
                let mut e = event(device.id, event_type::ALARM, position, None);
                e.attributes.insert("alarm".into(), Value::from(a));
                out.events.push(e);
            }
        }
    }

    // Ignition transitions.
    if let Some(prev) = previous {
        match (prev.attr_bool(IGNITION), position.attr_bool(IGNITION)) {
            (Some(false), Some(true)) => out.events.push(event(device.id, event_type::IGNITION_ON, position, None)),
            (Some(true), Some(false)) => out.events.push(event(device.id, event_type::IGNITION_OFF, position, None)),
            _ => {}
        }
    }

    out
}

fn event(device_id: Id, kind: &str, position: &Position, geofence_id: Option<Id>) -> Event {
    Event {
        id: 0,
        kind: Some(kind.to_string()),
        event_time: Some(position.fix_time.unwrap_or_else(Timestamp::now)),
        device_id,
        position_id: position.id,
        geofence_id: geofence_id.unwrap_or(0),
        maintenance_id: 0,
        attributes: Attributes::new(),
    }
}
