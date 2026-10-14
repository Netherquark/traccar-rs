// SPDX-License-Identifier: Apache-2.0
//
// Report generation. Faithful in shape to Traccar's report resources; the
// numeric thresholds (trip/stop minimum duration) follow Traccar's defaults.

use crate::model::*;
use crate::processing::{distance_meters, is_moving};

const KM: f64 = 1000.0;

/// A summary of one device over the queried window.
pub fn summary(device: &Device, positions: &[Position]) -> ReportSummary {
    let mut max_speed = 0.0f64;
    let mut speed_sum = 0.0f64;
    let mut count = 0u32;
    for p in positions {
        if p.valid {
            max_speed = max_speed.max(p.speed);
            speed_sum += p.speed;
            count += 1;
        }
    }
    let distance = crate::processing::path_distance(positions);
    let engine_hours = total_hours(positions);
    ReportSummary {
        device_id: device.id,
        device_name: device.name.clone(),
        max_speed,
        average_speed: if count > 0 { speed_sum / count as f64 } else { 0.0 },
        distance,
        spent_fuel: fuel(positions),
        engine_hours,
    }
}

fn total_hours(positions: &[Position]) -> i64 {
    match (positions.first(), positions.last()) {
        (Some(a), Some(b)) => {
            let a = a.fix_time.map(|t| t.millis()).unwrap_or(0);
            let b = b.fix_time.map(|t| t.millis()).unwrap_or(0);
            ((b - a).max(0)) / 3_600_000
        }
        _ => 0,
    }
}

fn fuel(positions: &[Position]) -> f64 {
    // Real Traccar computes consumption from a calibrated fuel factor. Until a
    // device reports `fuel`, this stays zero, which is the honest result.
    let first = positions.first().and_then(|p| p.attr_f64(position_key::FUEL));
    let last = positions.last().and_then(|p| p.attr_f64(position_key::FUEL));
    match (first, last) {
        (Some(a), Some(b)) if b >= a => b - a,
        _ => 0.0,
    }
}

/// Split a position series into trips (runs where the device is moving).
pub fn trips(device: &Device, positions: &[Position]) -> Vec<ReportTrips> {
    let mut out = Vec::new();
    let mut current: Vec<&Position> = Vec::new();
    for p in positions {
        if is_moving(p) {
            current.push(p);
        } else if !current.is_empty() {
            if let Some(t) = build_trip(device, &current) {
                out.push(t);
            }
            current.clear();
        }
    }
    if let Some(t) = build_trip(device, &current) {
        out.push(t);
    }
    out
}

fn build_trip(device: &Device, points: &[&Position]) -> Option<ReportTrips> {
    if points.len() < 2 {
        return None;
    }
    let first = points.first()?;
    let last = points.last()?;
    let duration = last.fix_time.map(|t| t.millis()).unwrap_or(0) - first.fix_time.map(|t| t.millis()).unwrap_or(0);
    if duration <= 0 {
        return None;
    }
    let owned: Vec<Position> = points.iter().map(|p| (*p).clone()).collect();
    let distance = crate::processing::path_distance(&owned);
    let max_speed = points.iter().map(|p| p.speed).fold(0.0, f64::max);
    let avg = points.iter().map(|p| p.speed).sum::<f64>() / points.len() as f64;
    Some(ReportTrips {
        device_id: device.id,
        device_name: device.name.clone(),
        max_speed,
        average_speed: avg,
        distance,
        spent_fuel: 0.0,
        duration: duration / 1000,
        start_time: first.fix_time,
        start_address: first.address.clone(),
        start_lat: first.latitude,
        start_lon: first.longitude,
        end_time: last.fix_time,
        end_address: last.address.clone(),
        end_lat: last.latitude,
        end_lon: last.longitude,
        driver_unique_id: first.attr_str(position_key::DRIVER_UNIQUE_ID),
        driver_name: None,
    })
}

/// Split a position series into stops (stationary runs of at least one minute).
pub fn stops(device: &Device, positions: &[Position]) -> Vec<ReportStops> {
    const MIN_STOP_SECONDS: i64 = 60;
    let mut out = Vec::new();
    let mut current: Vec<&Position> = Vec::new();
    let flush = |current: &mut Vec<&Position>, out: &mut Vec<ReportStops>| {
        if current.len() >= 2 {
            let first = current.first().unwrap();
            let last = current.last().unwrap();
            let start = first.fix_time.map(|t| t.millis()).unwrap_or(0);
            let end = last.fix_time.map(|t| t.millis()).unwrap_or(0);
            let duration = (end - start) / 1000;
            if duration >= MIN_STOP_SECONDS {
                out.push(ReportStops {
                    device_id: device.id,
                    device_name: device.name.clone(),
                    duration,
                    start_time: first.fix_time,
                    address: first.address.clone(),
                    lat: first.latitude,
                    lon: first.longitude,
                    end_time: last.fix_time,
                    spent_fuel: 0.0,
                    engine_hours: duration / 3600,
                });
            }
        }
        current.clear();
    };
    for p in positions {
        if is_moving(p) {
            flush(&mut current, &mut out);
        } else {
            current.push(p);
        }
    }
    flush(&mut current, &mut out);
    out
}

/// `[longitude, latitude]` pairs, one per valid position.
pub fn route(positions: &[Position]) -> Vec<Vec<f64>> {
    positions
        .iter()
        .filter(|p| p.valid)
        .map(|p| vec![p.longitude, p.latitude])
        .collect()
}

/// Geofence intervals derived from enter/exit events.
pub fn geofences(device: &Device, events: &[Event]) -> Vec<ReportGeofences> {
    use std::collections::HashMap;
    let mut open: HashMap<Id, Event> = HashMap::new();
    let mut out = Vec::new();
    for e in events {
        match e.kind.as_deref() {
            Some(event_type::GEOFENCE_ENTER) => {
                open.insert(e.geofence_id, e.clone());
            }
            Some(event_type::GEOFENCE_EXIT) => {
                if let Some(start) = open.remove(&e.geofence_id) {
                    out.push(ReportGeofences {
                        device_id: device.id,
                        device_name: device.name.clone(),
                        geofence_id: e.geofence_id,
                        start_time: start.event_time,
                        end_time: e.event_time,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// A combined report item: simplified route + events + positions.
pub fn combined(device: &Device, positions: &[Position], events: &[Event], simplify: usize) -> CombinedReportItem {
    let full = route(positions);
    let step = simplify.max(1);
    let mut simplified: Vec<Vec<f64>> = full.iter().step_by(step).cloned().collect();
    if let (Some(first), Some(last)) = (full.first(), full.last()) {
        if simplified.first() != Some(first) {
            simplified.insert(0, first.clone());
        }
        if simplified.last() != Some(last) {
            simplified.push(last.clone());
        }
    }
    CombinedReportItem {
        device_id: device.id,
        route: simplified,
        events: events.to_vec(),
        positions: positions.to_vec(),
    }
}

/// Metres helper for clients that think in km.
pub fn meters_to_km(m: f64) -> f64 {
    m / KM
}

/// Distance between two devices' last positions is not meaningful; this is a
/// thin wrapper kept for symmetry with Traccar's report helpers.
pub fn haversine(a: (f64, f64), b: (f64, f64)) -> f64 {
    distance_meters(a.0, a.1, b.0, b.1)
}
