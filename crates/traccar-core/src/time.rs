// SPDX-License-Identifier: Apache-2.0
//
// Time handling. Traccar serializes instants as ISO-8601 in UTC with millisecond
// precision and a numeric offset, e.g. `2024-01-01T00:00:00.000+00:00`. Devices
// (and the Traccar client) send timestamps as ISO strings, seconds-since-epoch,
// or `yyyy-MM-dd HH:mm:ss`. This module normalises all of those.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A UTC instant that (de)serializes the way Traccar does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub DateTime<Utc>);

impl Timestamp {
    pub fn now() -> Self {
        Timestamp(Utc::now())
    }
    pub fn from_millis(ms: i64) -> Self {
        Timestamp(Utc.timestamp_millis_opt(ms).single().unwrap_or_else(Utc::now))
    }
    pub fn millis(&self) -> i64 {
        self.0.timestamp_millis()
    }
    pub fn seconds(&self) -> i64 {
        self.0.timestamp()
    }
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format_iso(self.0))
    }
}

impl From<DateTime<Utc>> for Timestamp {
    fn from(dt: DateTime<Utc>) -> Self {
        Timestamp(dt)
    }
}

/// Format an instant the way Jackson does for Traccar: millisecond precision,
/// numeric UTC offset.
pub fn format_iso(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
}

/// Parse an ISO-8601 instant, tolerating both `Z` and numeric offsets, and
/// missing fractional seconds.
pub fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Full RFC 3339 first (handles Z and offsets and fractional seconds).
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Numeric epoch (seconds or millis) as a fallback — some clients send this
    // even on JSON endpoints.
    if let Ok(n) = s.parse::<i64>() {
        return Some(from_epoch(n));
    }
    // Naive ISO, no offset.
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(Utc.from_utc_datetime(&ndt));
        }
    }
    // Date only.
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()));
    }
    None
}

/// Interpret a device-supplied epoch value, applying Traccar's rule: values
/// below `i32::MAX` are seconds, larger values are milliseconds.
pub fn from_epoch(n: i64) -> DateTime<Utc> {
    let ms = if n < i32::MAX as i64 { n * 1000 } else { n };
    Utc.timestamp_millis_opt(ms).single().unwrap_or_else(Utc::now)
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format_iso(self.0))
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        // Accept string form (ISO) or an integer epoch.
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::String(s) => parse_iso(&s)
                .map(Timestamp)
                .ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp: {s}"))),
            serde_json::Value::Number(n) => n
                .as_i64()
                .map(|n| Timestamp(from_epoch(n)))
                .ok_or_else(|| serde::de::Error::custom("invalid epoch timestamp")),
            _ => Err(serde::de::Error::custom("invalid timestamp type")),
        }
    }
}

/// Serde helper for `Option<Timestamp>`.
pub mod option {
    use super::*;

    pub fn serialize<S: Serializer>(
        v: &Option<Timestamp>,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match v {
            Some(t) => t.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Option<Timestamp>, D::Error> {
        Ok(Option::<Timestamp>::deserialize(d)?)
    }
}
