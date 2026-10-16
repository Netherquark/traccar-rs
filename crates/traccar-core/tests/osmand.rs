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

#[test]
fn decodes_osmand_json_java_case_basic() {
    let body = br#"{"location":{"timestamp":"2025-06-15T13:45:12.862Z","coords":{"latitude":37.4219983,"longitude":-122.084,"accuracy":5,"speed":0,"heading":-1,"altitude":5},"is_moving":false,"odometer":0,"event":"motionchange","battery":{"level":1,"is_charging":false},"activity":{"type":"still","confidence":100},"extras":{},"_":"&id=48241179&"},"device_id":"48241179"}"#;
    let raw = RawIngest { content_type: Some("application/json".into()), query: Vec::new(), body: body.to_vec() };
    let ingest = decode("osmand", &raw).unwrap();
    assert!(ingest.json);
    assert_eq!(ingest.unique_id, "48241179");
    let pos = &ingest.position;
    assert!((pos.latitude - 37.4219983).abs() < 1e-5);
    assert!((pos.longitude + 122.084).abs() < 1e-5);
    assert_eq!(pos.altitude, 5.0);
    assert_eq!(pos.speed, 0.0);
    assert_eq!(pos.course, 0.0); // heading -1 skipped
    assert_eq!(pos.attributes.get(position_key::EVENT).and_then(|v| v.as_str()), Some("motionchange"));
    assert_eq!(pos.attributes.get(position_key::MOTION).and_then(|v| v.as_bool()), Some(false));
    assert_eq!(pos.attributes.get(position_key::ODOMETER).and_then(|v| v.as_i64()), Some(0));
    assert_eq!(pos.attributes.get(position_key::BATTERY_LEVEL).and_then(|v| v.as_i64()), Some(100));
    assert_eq!(pos.attributes.get("charge"), None); // is_charging false => not set
    assert_eq!(pos.attributes.get("activity").and_then(|v| v.as_str()), Some("still"));
}

#[test]
fn decodes_osmand_json_java_case_extras_promoted() {
    let body = br#"{"location":{"timestamp":"2025-06-15T13:45:12.862Z","coords":{"latitude":37.4219983,"longitude":-122.084,"accuracy":5,"speed":0,"heading":-1,"altitude":5},"is_moving":false,"odometer":0,"event":"motionchange","battery":{"level":1,"is_charging":false},"activity":{"type":"still"},"extras":{"device_integer_twelve_test":12,"device_bool_test":true,"device_bool_false_test":false,"device_double_test":1.345}},"device_id":"48241179"}"#;
    let raw = RawIngest { content_type: Some("application/json".into()), query: Vec::new(), body: body.to_vec() };
    let ingest = decode("osmand", &raw).unwrap();
    assert_eq!(ingest.position.attributes.get("device_integer_twelve_test").and_then(|v| v.as_i64()), Some(12));
    assert_eq!(ingest.position.attributes.get("device_bool_test").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(ingest.position.attributes.get("device_bool_false_test").and_then(|v| v.as_bool()), Some(false));
}

#[test]
fn decodes_osmand_json_java_case_mock() {
    let body = br#"{"location":{"extras":{},"mock":true,"coords":{"speed_accuracy":-1,"speed":-1,"longitude":-122.406417,"ellipsoidal_altitude":0,"floor":null,"heading_accuracy":-1,"latitude":37.785834,"accuracy":5,"altitude_accuracy":-1,"altitude":0,"heading":-1},"is_moving":false,"age":188,"odometer":0,"uuid":"2FB04C65-99CF-42AB-8DD3-EBCB4B108BF8","event":"motionchange","battery":{"level":-1,"is_charging":false},"activity":{"type":"unknown","confidence":100},"timestamp":"2025-05-09T04:11:30.579Z"},"device_id":"658765"}"#;
    let raw = RawIngest { content_type: Some("application/json".into()), query: Vec::new(), body: body.to_vec() };
    let ingest = decode("osmand", &raw).unwrap();
    assert_eq!(ingest.unique_id, "658765");
    assert!((ingest.position.latitude - 37.785834).abs() < 1e-5);
    assert_eq!(ingest.position.attributes.get("mock").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(ingest.position.attributes.get(position_key::BATTERY_LEVEL), None); // level -1 skipped
    assert_eq!(ingest.position.attributes.get("activity").and_then(|v| v.as_str()), Some("unknown"));
    assert_eq!(ingest.position.speed, 0.0);
}

#[test]
fn decodes_osmand_json_java_case_speed() {
    let body = br#"{"location":{"event":"motionchange","is_moving":false,"uuid":"0e9a2473-a9a7-4c00-997b-fb97d2154e75","timestamp":"2021-07-21T08:06:34.444Z","odometer":0,"coords":{"latitude":-6.1148096,"longitude":106.6837015,"accuracy":3.8,"speed":18.67,"speed_accuracy":0.26,"heading":63,"heading_accuracy":0.28,"altitude":35.7,"altitude_accuracy":3.8},"activity":{"type":"still","confidence":100},"battery":{"is_charging":false,"level":0.79},"extras":{}},"device_id":"8737767034"}"#;
    let raw = RawIngest { content_type: Some("application/json".into()), query: Vec::new(), body: body.to_vec() };
    let ingest = decode("osmand", &raw).unwrap();
    assert_eq!(ingest.unique_id, "8737767034");
    assert!((ingest.position.altitude - 35.7).abs() < 1e-6);
    assert!((ingest.position.speed - 18.67 * 1.943_844_492).abs() < 1e-3);
    assert_eq!(ingest.position.course, 63.0);
    assert_eq!(ingest.position.accuracy, 3.8);
    assert_eq!(ingest.position.attributes.get(position_key::BATTERY_LEVEL).and_then(|v| v.as_i64()), Some(79));
}

#[test]
fn osmand_cell_wifi_network_parsing() {
    let ingest = decode(
        "osmand",
        &q(&[
            ("id", "123456"),
            ("cell", "257,02,16,2224"),
            ("cell", "257,02,16,2223,-90"),
            ("wifi", "00-14-22-01-23-45,-80"),
            ("wifi", "00-1C-B3-09-85-15,-70"),
        ]),
    )
    .unwrap();
    let net = ingest.position.network.clone().unwrap();
    assert_eq!(net.cell_towers.as_ref().unwrap().len(), 2);
    assert_eq!(net.cell_towers.as_ref().unwrap()[0].mobile_country_code, Some(257));
    assert_eq!(net.cell_towers.as_ref().unwrap()[1].signal_strength, Some(-90));
    assert_eq!(net.wifi_access_points.as_ref().unwrap()[0].mac_address, Some("00:14:22:01:23:45".into()));
    assert_eq!(net.wifi_access_points.as_ref().unwrap()[0].signal_strength, Some(-80));
}

#[test]
fn osmand_location_string_splits() {
    let ingest = decode("osmand", &q(&[("id", "123456"), ("location", "1.25,2.5")])).unwrap();
    assert_eq!(ingest.position.latitude, 1.25);
    assert_eq!(ingest.position.longitude, 2.5);
}

#[test]
fn osmand_deviceid_alias() {
    let ingest = decode("osmand", &q(&[("deviceid", "123456"), ("lat", "60"), ("lon", "30")])).unwrap();
    assert_eq!(ingest.unique_id, "123456");
}

#[test]
fn osmand_hdop_altitude() {
    let ingest = decode(
        "osmand",
        &q(&[
            ("id", "902064"),
            ("lat", "42.06288"),
            ("lon", "-88.23412"),
            ("timestamp", "2016-01-27T18:55:47Z"),
            ("hdop", "6.0"),
            ("altitude", "224.0"),
            ("speed", "0.0"),
        ]),
    )
    .unwrap();
    assert_eq!(ingest.position.attributes.get(position_key::HDOP).and_then(|v| v.as_f64()), Some(6.0));
    assert_eq!(ingest.position.altitude, 224.0);
}

#[test]
fn osmand_space_date() {
    let ingest = decode(
        "osmand",
        &q(&[("id", "x"), ("lat", "49.60688"), ("lon", "6.15788"), ("timestamp", "2014-06-04 09:10:11"), ("altitude", "384.7")]),
    )
    .unwrap();
    // chrono parses naive as UTC
    assert_eq!(ingest.position.fix_time.unwrap().millis(), 1401873011000);
}

#[test]
fn osmand_batt_attribute() {
    let ingest = decode(
        "osmand",
        &q(&[("id", "861001000719969"), ("lat", "41.666667"), ("lon", "-0.883333"), ("altitude", "350.059479"), ("speed", "0.0"), ("batt", "87")]),
    )
    .unwrap();
    assert_eq!(ingest.position.attributes.get(position_key::BATTERY_LEVEL).and_then(|v| v.as_f64()), Some(87.0));
}
