// SPDX-License-Identifier: Apache-2.0
//
// OsmAnd / Traccar-client ingestion decoder tests. These pin the two input
// shapes the ASK-Nano Traccar bridge and the Traccar Client SDK use: an
// OsmAnd-style query string, and the Traccar client's local JSON payload.

use traccar_core::model::position_key;
use traccar_core::protocol::{decode, RawIngest};

fn q(pairs: &[(&str, &str)]) -> RawIngest {
    RawIngest {
        content_type: None,
        query: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        body: Vec::new(),
    }
}

#[test]
fn decodes_osmand_query_string() {
    let ingest = decode(
        "osmand",
        &q(&[
            ("id", "123"),
            ("lat", "10.5"),
            ("lon", "20.5"),
            ("timestamp", "1700000000000"),
            ("speed", "12.3"),
            ("bearing", "45"),
            ("altitude", "100"),
            ("accuracy", "5"),
            ("batt", "80"),
            ("charge", "true"),
            ("hdop", "1.1"),
            ("driverUniqueId", "driver-9"),
        ]),
    )
    .unwrap();

    assert_eq!(ingest.unique_id, "123");
    assert!(!ingest.json);
    let pos = &ingest.position;
    assert_eq!(pos.latitude, 10.5);
    assert_eq!(pos.longitude, 20.5);
    assert_eq!(pos.fix_time.unwrap().millis(), 1_700_000_000_000);
    assert_eq!(pos.speed, 12.3);
    assert_eq!(pos.course, 45.0);
    assert_eq!(pos.altitude, 100.0);
    assert_eq!(pos.accuracy, 5.0);
    assert_eq!(pos.attr_f64(position_key::HDOP), Some(1.1));
    assert_eq!(pos.attributes.get(position_key::BATTERY_LEVEL).and_then(|v| v.as_f64()), Some(80.0));
    assert_eq!(pos.attributes.get(position_key::CHARGE).and_then(|v| v.as_bool()), Some(true));
    assert_eq!(pos.attr_str(position_key::DRIVER_UNIQUE_ID).as_deref(), Some("driver-9"));
}

#[test]
fn location_param_splits_into_coordinates() {
    let ingest = decode("osmand", &q(&[("id", "abc"), ("location", "1.25,2.5")])).unwrap();
    assert_eq!(ingest.position.latitude, 1.25);
    assert_eq!(ingest.position.longitude, 2.5);
}

#[test]
fn missing_id_is_rejected() {
    let err = decode("osmand", &q(&[("lat", "1.0"), ("lon", "2.0")])).unwrap_err();
    assert_eq!(err.status, 400);
}

#[test]
fn decodes_traccar_client_json() {
    let body = br#"{
        "device_id": "dev-1",
        "location": {
            "coords": {"latitude": 1.0, "longitude": 2.0, "speed": 3.0, "heading": 90.0, "accuracy": 4.0, "altitude": 5.0},
            "timestamp": "2024-01-01T00:00:00.000+00:00",
            "is_moving": true,
            "battery": {"level": 0.5, "is_charging": true}
        }
    }"#;
    let raw = RawIngest {
        content_type: Some("application/json".into()),
        query: Vec::new(),
        body: body.to_vec(),
    };
    let ingest = decode("osmand", &raw).unwrap();
    assert!(ingest.json);
    assert_eq!(ingest.unique_id, "dev-1");
    let pos = &ingest.position;
    assert_eq!(pos.latitude, 1.0);
    assert_eq!(pos.longitude, 2.0);
    // m/s -> knots
    assert!((pos.speed - 3.0 * 1.943_844_492).abs() < 1e-6);
    assert_eq!(pos.course, 90.0);
    assert_eq!(pos.accuracy, 4.0);
    assert_eq!(pos.altitude, 5.0);
    assert_eq!(pos.attributes.get(position_key::BATTERY_LEVEL).and_then(|v| v.as_i64()), Some(50));
    assert_eq!(pos.attributes.get(position_key::CHARGE).and_then(|v| v.as_bool()), Some(true));
    assert_eq!(pos.attributes.get(position_key::MOTION).and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn unknown_protocol_is_unsupported() {
    let err = decode("nope", &q(&[("id", "1")])).unwrap_err();
    assert_eq!(err.status, 501);
}
