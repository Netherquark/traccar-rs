// SPDX-License-Identifier: Apache-2.0
//
// Traccar-compatible ingestion protocols.
//
// The primary decoder is OsmAnd (Traccar `OsmAndProtocolDecoder`), which is
// also what the Traccar Client SDK and the ASK-Nano Traccar bridge speak. It
// accepts either a query string (GET or form-encoded POST) or a JSON body that
// matches the Traccar client's local HTTP payload.
//
// Reference: src/main/java/org/traccar/protocol/OsmAndProtocolDecoder.java

use crate::error::{Error, Result};
use crate::model::{position_key::*, *};
use crate::time::{from_epoch, parse_iso, Timestamp};
use serde_json::Value;

/// Raw ingestion input handed to the decoder.
#[derive(Clone, Debug, Default)]
pub struct RawIngest {
    pub content_type: Option<String>,
    pub query: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Decoded result: the device unique id plus the parsed position. Resolving the
/// unique id to a device (and registration of unknown devices) is the caller's
/// responsibility.
#[derive(Clone, Debug)]
pub struct Ingest {
    pub unique_id: String,
    pub position: Position,
    pub notification_token: Option<String>,
    /// JSON ingestion path (registration semantics differ: 404 vs 400).
    pub json: bool,
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse an `a=1&b=2&b=3` string into ordered key/value pairs.
pub fn parse_query(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (url_decode(k), url_decode(v)),
            None => (url_decode(pair), String::new()),
        })
        .collect()
}

fn is_json(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .map(|c| c.trim_start().to_ascii_lowercase().starts_with("application/json"))
        .unwrap_or(false)
}

/// Decode an ingestion request using the requested protocol.
pub fn decode(protocol: &str, raw: &RawIngest) -> Result<Ingest> {
    match protocol {
        "osmand" | "traccar" | "traccarclient" => {
            if is_json(&raw.content_type) {
                decode_json(raw)
            } else {
                decode_query(raw)
            }
        }
        other => Err(Error::unsupported(format!("unknown protocol: {other}"))),
    }
}

fn parse_timestamp(value: &str) -> Option<Timestamp> {
    if let Ok(n) = value.parse::<i64>() {
        return Some(Timestamp(from_epoch(n)));
    }
    parse_iso(value).map(Timestamp)
}

fn decode_query(raw: &RawIngest) -> Result<Ingest> {
    let mut params = raw.query.clone();
    if params.is_empty() {
        // Traccar falls back to parsing the body as a query string.
        params = parse_query(&String::from_utf8_lossy(&raw.body));
    }
    if params.is_empty() {
        return Err(Error::bad_request("empty request"));
    }

    let mut position = Position::with_protocol("osmand");
    position.valid = true;
    let mut network = Network::default();
    let mut latitude: Option<f64> = None;
    let mut longitude: Option<f64> = None;
    let mut unique_id: Option<String> = None;
    let mut notification_token: Option<String> = None;

    for (key, value) in &params {
        match key.as_str() {
            "id" | "deviceid" => unique_id = Some(value.clone()),
            "notificationToken" => notification_token = Some(value.clone()),
            "valid" => position.valid = value.eq_ignore_ascii_case("true") || value == "1",
            "timestamp" => {
                if let Some(ts) = parse_timestamp(value) {
                    position.set_time(ts);
                }
            }
            "lat" => latitude = value.parse::<f64>().ok(),
            "lon" => longitude = value.parse::<f64>().ok(),
            "location" => {
                let parts: Vec<&str> = value.split(',').collect();
                if parts.len() >= 2 {
                    latitude = parts[0].parse::<f64>().ok();
                    longitude = parts[1].parse::<f64>().ok();
                }
            }
            "cell" => {
                let parts: Vec<&str> = value.split(',').collect();
                if parts.len() > 4 {
                    if let (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e)) = (
                        parts[0].parse::<i32>(),
                        parts[1].parse::<i32>(),
                        parts[2].parse::<i32>(),
                        parts[3].parse::<i64>(),
                        parts[4].parse::<i32>(),
                    ) {
                        network.add_cell_tower(CellTower::from(a, b, c, d, Some(e)));
                    }
                } else if parts.len() == 4 {
                    if let (Ok(a), Ok(b), Ok(c), Ok(d)) = (
                        parts[0].parse::<i32>(),
                        parts[1].parse::<i32>(),
                        parts[2].parse::<i32>(),
                        parts[3].parse::<i64>(),
                    ) {
                        network.add_cell_tower(CellTower::from(a, b, c, d, None));
                    }
                }
            }
            "wifi" => {
                let parts: Vec<&str> = value.split(',').collect();
                if parts.len() >= 2 {
                    if let Ok(rssi) = parts[1].parse::<i32>() {
                        let mac = parts[0].replace('-', ":");
                        network.add_wifi(WifiAccessPoint::from(mac, rssi));
                    }
                }
            }
            // OsmAnd/Traccar client speed is already in knots.
            "speed" => position.speed = value.parse::<f64>().unwrap_or(0.0),
            "bearing" | "heading" => position.course = value.parse::<f64>().unwrap_or(0.0),
            "altitude" => position.altitude = value.parse::<f64>().unwrap_or(0.0),
            "accuracy" => position.accuracy = value.parse::<f64>().unwrap_or(0.0),
            "hdop" => {
                if let Ok(v) = value.parse::<f64>() {
                    position.set_attr(HDOP, Value::from(v));
                }
            }
            "batt" => {
                if let Ok(v) = value.parse::<f64>() {
                    position.set_attr(BATTERY_LEVEL, Value::from(v));
                }
            }
            "driverUniqueId" => position.set_attr(DRIVER_UNIQUE_ID, Value::from(value.clone())),
            "charge" => position.set_attr(CHARGE, Value::from(value.eq_ignore_ascii_case("true"))),
            other => {
                if let Ok(n) = value.parse::<f64>() {
                    position.set_attr(other, Value::from(n));
                } else {
                    match value.as_str() {
                        "true" => position.set_attr(other, Value::from(true)),
                        "false" => position.set_attr(other, Value::from(false)),
                        _ => position.set_attr(other, Value::from(value.clone())),
                    }
                }
            }
        }
    }

    if position.fix_time.is_none() {
        position.set_time(Timestamp::now());
    }
    if network.cell_towers.is_some() || network.wifi_access_points.is_some() {
        position.network = Some(network);
    }
    match (latitude, longitude) {
        (Some(la), Some(lo)) => {
            position.latitude = la;
            position.longitude = lo;
        }
        _ => {
            // No fix in this message; caller may fill from the last position.
            position.valid = position.valid && false;
        }
    }

    let unique_id = unique_id.ok_or_else(|| Error::bad_request("missing id"))?;
    Ok(Ingest { unique_id, position, notification_token, json: false })
}

fn decode_json(raw: &RawIngest) -> Result<Ingest> {
    let root: Value = serde_json::from_slice(&raw.body).map_err(|e| Error::bad_request(format!("invalid JSON: {e}")))?;
    let unique_id = root
        .get("device_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::bad_request("missing device_id"))?
        .to_string();
    let location = root.get("location").ok_or_else(|| Error::bad_request("missing location"))?;

    let mut position = Position::with_protocol("osmand");
    position.device_time = location
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| parse_iso(s).map(Timestamp));
    position.fix_time = position.device_time;
    if position.fix_time.is_none() {
        position.set_time(Timestamp::now());
    }

    if let Some(coords) = location.get("coords") {
        position.valid = true;
        position.latitude = coords.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
        position.longitude = coords.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if let Some(speed) = coords.get("speed").and_then(|v| v.as_f64()) {
            if speed >= 0.0 {
                position.speed = speed * 1.94384; // m/s -> knots
            }
        }
        if let Some(heading) = coords.get("heading").and_then(|v| v.as_f64()) {
            if heading >= 0.0 {
                position.course = heading;
            }
        }
        if let Some(accuracy) = coords.get("accuracy").and_then(|v| v.as_f64()) {
            if accuracy >= 10.0 {
                position.accuracy = accuracy;
            }
        }
        if let Some(altitude) = coords.get("altitude").and_then(|v| v.as_f64()) {
            position.altitude = altitude;
        }
    }

    if let Some(event) = location.get("event").and_then(|v| v.as_str()) {
        position.set_attr(EVENT, Value::from(event));
    }
    if let Some(moving) = location.get("is_moving").and_then(|v| v.as_bool()) {
        position.set_attr(MOTION, Value::from(moving));
    }
    if let Some(odometer) = location.get("odometer").and_then(|v| v.as_i64()) {
        position.set_attr(ODOMETER, Value::from(odometer));
    }
    if let Some(mock) = location.get("mock").and_then(|v| v.as_bool()) {
        position.set_attr("mock", Value::from(mock));
    }
    if let Some(activity) = location.get("activity").and_then(|v| v.get("type")).and_then(|v| v.as_str()) {
        position.set_attr("activity", Value::from(activity));
    }
    if let Some(battery) = location.get("battery") {
        if let Some(level) = battery.get("level").and_then(|v| v.as_f64()) {
            if level >= 0.0 {
                position.set_attr(BATTERY_LEVEL, Value::from((level * 100.0) as i64));
            }
        }
        if battery.get("is_charging").and_then(|v| v.as_bool()).unwrap_or(false) {
            position.set_attr(CHARGE, Value::from(true));
        }
    }
    if let Some(alarm) = location.get("alarm").and_then(|v| v.as_str()) {
        position.set_attr(ALARM, Value::from(alarm));
    }
    if let Some(extras) = location.get("extras").and_then(|v| v.as_object()) {
        let has_alarm = location.get("alarm").is_some();
        for (k, v) in extras {
            if k == "alarm" && has_alarm {
                continue;
            }
            position.set_attr(k, v.clone());
        }
    }

    Ok(Ingest { unique_id, position, notification_token: None, json: true })
}

/// The OsmAnd/Traccar client response body when a queued command is pending.
/// Traccar returns the command data as the plain-text 200 body.
pub fn command_response(command: &Command) -> Option<String> {
    command
        .attributes
        .get("data")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
