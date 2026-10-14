// SPDX-License-Identifier: Apache-2.0
//
// Processing and report unit tests: distance, motion, geofence areas, and the
// summary/trips report shapes. These are pure functions, so they are checked
// without a store.

use serde_json::Value;
use traccar_core::model::{position_key, Device, Position};
use traccar_core::processing::{distance_meters, is_moving, parse_area, path_distance, segment_distance};
use traccar_core::reports;
use traccar_core::time::Timestamp;

fn pos(lat: f64, lon: f64, speed: f64, at_secs: i64) -> Position {
    let mut p = Position::default();
    p.valid = true;
    p.latitude = lat;
    p.longitude = lon;
    p.speed = speed;
    p.device_id = 1;
    let t = Timestamp::from_millis(at_secs * 1000);
    p.set_time(t);
    p
}

#[test]
fn equator_degree_of_longitude_is_about_111km() {
    let d = distance_meters(0.0, 0.0, 0.0, 1.0);
    assert!((d - 111_194.9).abs() < 50.0, "got {d}");
}

#[test]
fn path_distance_sums_valid_segments_only() {
    let mut a = pos(0.0, 0.0, 0.0, 0);
    a.valid = true;
    let b = pos(0.0, 1.0, 0.0, 10);
    let mut bad = pos(0.0, 2.0, 0.0, 20);
    bad.valid = false;
    let total = path_distance(&[a, b, bad]);
    assert!((total - 111_194.9).abs() < 50.0, "got {total}");
}

#[test]
fn motion_prefers_attribute_over_speed() {
    let mut slow = pos(0.0, 0.0, 0.2, 0);
    assert!(!is_moving(&slow));
    slow.set_attr(position_key::MOTION, Value::from(true));
    assert!(is_moving(&slow));

    let mut fast = pos(0.0, 0.0, 5.0, 0);
    assert!(is_moving(&fast));
    fast.set_attr(position_key::MOTION, Value::from(false));
    assert!(!is_moving(&fast));
}

#[test]
fn circle_area_contains_points_within_radius() {
    let area = parse_area("CIRCLE (10.0 20.0, 1000.0)").expect("circle parses");
    assert!(area.contains(10.0, 20.0));
    // ~500 m north-east is inside 1 km; ~2 km away is outside.
    assert!(area.contains(10.003, 20.0));
    assert!(!area.contains(10.05, 20.0));
}

#[test]
fn polygon_area_parses_geojson_wkt_order() {
    // WKT polygons list (lon lat); a square around (1,1).
    let area = parse_area("POLYGON ((0 0, 2 0, 2 2, 0 2, 0 0))").expect("polygon parses");
    assert!(area.contains(1.0, 1.0));
    assert!(!area.contains(3.0, 3.0));
}

#[test]
fn segment_distance_requires_previous_stored_position() {
    let prev = pos(0.0, 0.0, 0.0, 0); // id 0 -> not a stored previous position
    let cur = pos(0.0, 1.0, 0.0, 10);
    assert!(segment_distance(&prev, &cur).is_none());

    let mut prev = prev;
    prev.id = 7;
    assert!(segment_distance(&prev, &cur).is_some());
}

#[test]
fn summary_reports_distance_speeds_and_hours() {
    let device = Device { id: 1, name: Some("Car".into()), ..Default::default() };
    let positions = vec![
        pos(0.0, 0.0, 0.0, 0),
        pos(0.0, 1.0, 10.0, 3600),
        pos(0.0, 2.0, 20.0, 7200),
    ];
    let s = reports::summary(&device, &positions);
    assert_eq!(s.device_id, 1);
    assert_eq!(s.device_name.as_deref(), Some("Car"));
    assert_eq!(s.max_speed, 20.0);
    assert_eq!(s.average_speed, 10.0);
    assert_eq!(s.engine_hours, 2);
    assert!((s.distance - 222_389.8).abs() < 200.0, "got {}", s.distance);
}

#[test]
fn trips_are_runs_of_moving_positions() {
    let device = Device { id: 1, name: Some("Car".into()), ..Default::default() };
    let positions = vec![
        pos(0.0, 0.0, 0.0, 0),
        pos(0.0, 0.1, 10.0, 60),
        pos(0.0, 0.2, 10.0, 120),
        pos(0.0, 0.2, 0.0, 180),
    ];
    let trips = reports::trips(&device, &positions);
    assert_eq!(trips.len(), 1);
    assert_eq!(trips[0].device_id, 1);
}

#[test]
fn route_is_a_list_of_lon_lat_pairs() {
    let positions = vec![pos(10.0, 20.0, 0.0, 0), pos(11.0, 21.0, 0.0, 10)];
    let r = reports::route(&positions);
    assert_eq!(r, vec![vec![20.0, 10.0], vec![21.0, 11.0]]);
}

#[test]
fn meters_to_km_converts() {
    assert_eq!(reports::meters_to_km(1500.0), 1.5);
}
