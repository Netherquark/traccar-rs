// SPDX-License-Identifier: Apache-2.0
//
// Domain models. Field names and JSON shapes mirror the Traccar OpenAPI
// document (`components/schemas`) so existing Traccar clients keep working.

use crate::time::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Traccar object identifiers are 64-bit integers.
pub type Id = i64;

/// Free-form attribute bag. Traccar stores numbers, strings and booleans here.
pub type Attributes = serde_json::Map<String, Value>;

fn is_false(b: &bool) -> bool {
    !*b
}

/// Serialize a value to JSON, stripping the given top-level keys. Used to keep
/// secrets (password hashes) out of API responses.
pub fn strip_keys(mut v: Value, keys: &[&str]) -> Value {
    if let Some(obj) = v.as_object_mut() {
        for k in keys {
            obj.remove(*k);
        }
    }
    v
}

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CellTower {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radio_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile_country_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile_network_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_area_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_strength: Option<i32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WifiAccessPoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_strength: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<i32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_mobile_country_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_mobile_network_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radio_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consider_ip: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_towers: Option<Vec<CellTower>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_access_points: Option<Vec<WifiAccessPoint>>,
}

/// Position attribute keys used across the codebase (Traccar `Position.KEY_*`).
pub mod position_key {
    pub const HDOP: &str = "hdop";
    pub const PDOP: &str = "pdop";
    pub const SATELLITES: &str = "sat";
    pub const RSSI: &str = "rssi";
    pub const EVENT: &str = "event";
    pub const ALARM: &str = "alarm";
    pub const STATUS: &str = "status";
    pub const ODOMETER: &str = "odometer";
    pub const HOURS: &str = "hours";
    pub const POWER: &str = "power";
    pub const BATTERY_LEVEL: &str = "batteryLevel";
    pub const FUEL: &str = "fuel";
    pub const IGNITION: &str = "ignition";
    pub const CHARGE: &str = "charge";
    pub const DISTANCE: &str = "distance";
    pub const TOTAL_DISTANCE: &str = "totalDistance";
    pub const MOTION: &str = "motion";
    pub const DRIVER_UNIQUE_ID: &str = "driverUniqueId";
    pub const INDEX: &str = "index";
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub device_time: Option<Timestamp>,
    #[serde(default)]
    pub fix_time: Option<Timestamp>,
    #[serde(default)]
    pub server_time: Option<Timestamp>,
    #[serde(default)]
    pub outdated: bool,
    #[serde(default)]
    pub valid: bool,
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
    #[serde(default)]
    pub altitude: f64,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub course: f64,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub accuracy: f64,
    #[serde(default)]
    pub network: Option<Network>,
    #[serde(default)]
    pub geofence_ids: Option<Vec<Id>>,
    #[serde(default)]
    pub attributes: Attributes,
}

impl Default for Position {
    fn default() -> Self {
        Position {
            id: 0,
            device_id: 0,
            protocol: None,
            device_time: None,
            fix_time: None,
            server_time: Some(Timestamp::now()),
            outdated: false,
            valid: false,
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
            speed: 0.0,
            course: 0.0,
            address: None,
            accuracy: 0.0,
            network: None,
            geofence_ids: None,
            attributes: Attributes::new(),
        }
    }
}

impl Position {
    pub fn with_protocol(protocol: &str) -> Self {
        Position { protocol: Some(protocol.to_string()), ..Default::default() }
    }

    /// Traccar's `setTime`: sets both device and fix time.
    pub fn set_time(&mut self, t: Timestamp) {
        self.device_time = Some(t);
        self.fix_time = Some(t);
    }

    pub fn device_time_or_now(&self) -> Timestamp {
        self.device_time.or(self.fix_time).unwrap_or_else(Timestamp::now)
    }

    pub fn set_attr(&mut self, key: &str, value: Value) {
        self.attributes.insert(key.to_string(), value);
    }

    pub fn attr_str(&self, key: &str) -> Option<String> {
        self.attributes.get(key).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
    }

    pub fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attributes.get(key).and_then(|v| v.as_f64())
    }

    pub fn attr_bool(&self, key: &str) -> Option<bool> {
        self.attributes.get(key).and_then(|v| v.as_bool())
    }
}

impl Default for CellTower {
    fn default() -> Self {
        CellTower {
            radio_type: Some("gsm".into()),
            mobile_country_code: None,
            mobile_network_code: None,
            location_area_code: None,
            cell_id: None,
            signal_strength: None,
        }
    }
}

impl CellTower {
    pub fn from(mcc: i32, mnc: i32, lac: i32, cid: i64, rssi: Option<i32>) -> Self {
        CellTower {
            mobile_country_code: Some(mcc),
            mobile_network_code: Some(mnc),
            location_area_code: Some(lac),
            cell_id: Some(cid),
            signal_strength: rssi,
            ..Default::default()
        }
    }
}

impl WifiAccessPoint {
    pub fn from(mac: String, rssi: i32) -> Self {
        WifiAccessPoint {
            mac_address: Some(mac),
            signal_strength: Some(rssi),
            channel: None,
        }
    }
}

impl Network {
    pub fn add_cell_tower(&mut self, t: CellTower) {
        self.cell_towers.get_or_insert_with(Vec::new).push(t);
    }
    pub fn add_wifi(&mut self, w: WifiAccessPoint) {
        self.wifi_access_points.get_or_insert_with(Vec::new).push(w);
    }
}

// ---------------------------------------------------------------------------
// Device / Group
// ---------------------------------------------------------------------------

pub mod device_status {
    pub const ONLINE: &str = "online";
    pub const OFFLINE: &str = "offline";
    pub const UNKNOWN: &str = "unknown";
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub unique_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub last_update: Option<Timestamp>,
    #[serde(default)]
    pub position_id: Option<Id>,
    #[serde(default)]
    pub group_id: Option<Id>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub contact: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub group_id: Id,
    #[serde(default)]
    pub attributes: Attributes,
}

// ---------------------------------------------------------------------------
// User
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct User {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub administrator: bool,
    #[serde(default)]
    pub map: Option<String>,
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
    #[serde(default)]
    pub zoom: i32,
    /// Plaintext password — write-only, never serialized back to a client.
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    #[serde(default)]
    pub coordinate_format: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub expiration_time: Option<Timestamp>,
    #[serde(default)]
    pub device_limit: i32,
    #[serde(default)]
    pub user_limit: i32,
    #[serde(default)]
    pub device_readonly: bool,
    #[serde(default)]
    pub limit_commands: bool,
    #[serde(default)]
    pub fixed_email: bool,
    #[serde(default)]
    pub poi_layer: Option<String>,
    #[serde(default)]
    pub attributes: Attributes,
    // --- credential material (persisted, never exposed) ---
    #[serde(default)]
    pub hashed_password: Option<String>,
    #[serde(default)]
    pub salt: Option<String>,
    #[serde(default)]
    pub totp_key: Option<String>,
}

impl Default for User {
    fn default() -> Self {
        User {
            id: 0,
            name: None,
            email: None,
            phone: None,
            readonly: false,
            administrator: false,
            map: None,
            latitude: 0.0,
            longitude: 0.0,
            zoom: 0,
            password: None,
            coordinate_format: None,
            disabled: false,
            expiration_time: None,
            device_limit: 0,
            user_limit: 0,
            device_readonly: false,
            limit_commands: false,
            fixed_email: false,
            poi_layer: None,
            attributes: Attributes::new(),
            hashed_password: None,
            salt: None,
            totp_key: None,
        }
    }
}

impl User {
    /// API-safe representation (drops hashedPassword / salt / totpKey).
    pub fn to_public(&self) -> Value {
        let v = serde_json::to_value(self).unwrap_or(Value::Object(Default::default()));
        strip_keys(v, &["hashedPassword", "salt", "totpKey", "password"])
    }
}

// ---------------------------------------------------------------------------
// Geofence / Calendar / Driver / Maintenance / Order / Attribute
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Geofence {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub area: Option<String>,
    #[serde(default)]
    pub calendar_id: Id,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Driver {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub unique_id: Option<String>,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Maintenance {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub period: f64,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub unique_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub to_address: Option<String>,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Attribute {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attribute: Option<String>,
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

// ---------------------------------------------------------------------------
// Command / Notification
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub text_channel: bool,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueuedCommand {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub text_channel: bool,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub command_id: Id,
    #[serde(default)]
    pub notificators: Option<String>,
    #[serde(default)]
    pub calendar_id: Id,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NotificationMessage {
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
    pub body: String,
    #[serde(default)]
    pub priority: bool,
}

// ---------------------------------------------------------------------------
// Event
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub event_time: Option<Timestamp>,
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub position_id: Id,
    #[serde(default)]
    pub geofence_id: Id,
    #[serde(default)]
    pub maintenance_id: Id,
    #[serde(default)]
    pub attributes: Attributes,
}

/// Common Traccar event types (`org.traccar.model.EventType`).
pub mod event_type {
    pub const DEVICE_MOVING: &str = "deviceMoving";
    pub const DEVICE_STOPPED: &str = "deviceStopped";
    pub const DEVICE_ONLINE: &str = "deviceOnline";
    pub const DEVICE_UNKNOWN: &str = "deviceUnknown";
    pub const DEVICE_OFFLINE: &str = "deviceOffline";
    pub const DEVICE_INACTIVE: &str = "deviceInactive";
    pub const DEVICE_FUEL_DROP: &str = "deviceFuelDrop";
    pub const COMMAND_RESULT: &str = "commandResult";
    pub const DEVICE_FUEL_INCREASE: &str = "deviceFuelIncrease";
    pub const GEOFENCE_ENTER: &str = "geofenceEnter";
    pub const GEOFENCE_EXIT: &str = "geofenceExit";
    pub const ALARM: &str = "alarm";
    pub const IGNITION_ON: &str = "ignitionOn";
    pub const IGNITION_OFF: &str = "ignitionOff";
    pub const MAINTENANCE: &str = "maintenance";
    pub const TEXT_MESSAGE: &str = "textMessage";
    pub const DRIVER_CHANGED: &str = "driverChanged";
    pub const MEDIA: &str = "media";
}

/// Traccar alarm names worth recognising when turning device alarms into events.
pub mod alarm {
    pub const SOS: &str = "sos";
    pub const FALL_DOWN: &str = "fallDown";
    pub const CRASH: &str = "crash";
    pub const VIBRATION: &str = "vibration";
    pub const POWER_OFF: &str = "powerOff";
    pub const POWER_ON: &str = "powerOn";
    pub const LOW_BATTERY: &str = "lowBattery";
    pub const TAMPERING: &str = "tampering";
}

// ---------------------------------------------------------------------------
// Permission
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Permission {
    /// Storage row id. Assigned by the store on insert; never sent on the wire
    /// (Traccar's `Permission` model has no `id`).
    #[serde(default, skip_serializing)]
    pub id: Id,
    #[serde(default)]
    pub user_id: Id,
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub group_id: Id,
    #[serde(default)]
    pub geofence_id: Id,
    #[serde(default)]
    pub notification_id: Id,
    #[serde(default)]
    pub calendar_id: Id,
    #[serde(default)]
    pub attribute_id: Id,
    #[serde(default)]
    pub driver_id: Id,
    #[serde(default)]
    pub managed_user_id: Id,
    #[serde(default)]
    pub command_id: Id,
    #[serde(default)]
    pub maintenance_id: Id,
    #[serde(default)]
    pub order_id: Id,
}

/// The `<name>Id` key Traccar uses for each object kind, in the canonical
/// order used by `Permission::pair`.
pub fn permission_field(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "user" => "userId",
        "device" => "deviceId",
        "group" => "groupId",
        "geofence" => "geofenceId",
        "notification" => "notificationId",
        "calendar" => "calendarId",
        "attribute" => "attributeId",
        "driver" => "driverId",
        "command" => "commandId",
        "maintenance" => "maintenanceId",
        "order" => "orderId",
        _ => return None,
    })
}

impl PartialEq for Permission {
    /// Compare the object references only; the storage `id` is not part of the
    /// Traccar permission identity, so two links with the same references are
    /// equal regardless of row id.
    fn eq(&self, other: &Self) -> bool {
        self.fields() == other.fields()
    }
}

impl Permission {
    /// Every `<name>Id` field, in canonical order.
    pub fn fields(&self) -> [(&'static str, Id); 12] {
        [
            ("userId", self.user_id),
            ("deviceId", self.device_id),
            ("groupId", self.group_id),
            ("geofenceId", self.geofence_id),
            ("notificationId", self.notification_id),
            ("calendarId", self.calendar_id),
            ("attributeId", self.attribute_id),
            ("driverId", self.driver_id),
            ("managedUserId", self.managed_user_id),
            ("commandId", self.command_id),
            ("maintenanceId", self.maintenance_id),
            ("orderId", self.order_id),
        ]
    }

    pub fn get(&self, field: &str) -> Id {
        self.fields().into_iter().find(|(f, _)| *f == field).map(|(_, v)| v).unwrap_or(0)
    }

    pub fn set(&mut self, field: &str, id: Id) {
        match field {
            "userId" => self.user_id = id,
            "deviceId" => self.device_id = id,
            "groupId" => self.group_id = id,
            "geofenceId" => self.geofence_id = id,
            "notificationId" => self.notification_id = id,
            "calendarId" => self.calendar_id = id,
            "attributeId" => self.attribute_id = id,
            "driverId" => self.driver_id = id,
            "managedUserId" => self.managed_user_id = id,
            "commandId" => self.command_id = id,
            "maintenanceId" => self.maintenance_id = id,
            "orderId" => self.order_id = id,
            _ => {}
        }
    }

    /// The two non-zero object references this link connects, in canonical
    /// order. Traccar permissions are always a pair.
    pub fn pair(&self) -> Option<(String, Id, String, Id)> {
        let nz: Vec<(&str, Id)> = self.fields().into_iter().filter(|(_, v)| *v != 0).collect();
        if nz.len() != 2 {
            return None;
        }
        Some((nz[0].0.into(), nz[0].1, nz[1].0.into(), nz[1].1))
    }
}

// ---------------------------------------------------------------------------
// Server / Statistics / accumulators
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    #[serde(default = "default_one")]
    pub id: Id,
    #[serde(default)]
    pub registration: bool,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub device_readonly: bool,
    #[serde(default)]
    pub limit_commands: bool,
    #[serde(default)]
    pub map: Option<String>,
    #[serde(default)]
    pub bing_key: Option<String>,
    #[serde(default)]
    pub map_url: Option<String>,
    #[serde(default)]
    pub poi_layer: Option<String>,
    #[serde(default)]
    pub announcement: Option<String>,
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
    #[serde(default)]
    pub zoom: i32,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub force_settings: bool,
    #[serde(default)]
    pub coordinate_format: Option<String>,
    #[serde(default)]
    pub open_id_enabled: bool,
    #[serde(default)]
    pub open_id_force: bool,
    #[serde(default)]
    pub attributes: Attributes,
}

fn default_one() -> Id {
    1
}

impl Default for Server {
    fn default() -> Self {
        Server {
            id: 1,
            registration: false,
            readonly: false,
            device_readonly: false,
            limit_commands: false,
            map: Some("osm".into()),
            bing_key: None,
            map_url: None,
            poi_layer: None,
            announcement: None,
            latitude: 0.0,
            longitude: 0.0,
            zoom: 0,
            version: Some(crate::VERSION.into()),
            force_settings: false,
            coordinate_format: None,
            open_id_enabled: false,
            open_id_force: false,
            attributes: Attributes::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAccumulators {
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub total_distance: f64,
    #[serde(default)]
    pub hours: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Statistics {
    #[serde(default)]
    pub capture_time: Option<Timestamp>,
    #[serde(default)]
    pub active_users: i64,
    #[serde(default)]
    pub active_devices: i64,
    #[serde(default)]
    pub requests: i64,
    #[serde(default)]
    pub messages_received: i64,
    #[serde(default)]
    pub messages_stored: i64,
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub max_speed: f64,
    #[serde(default)]
    pub average_speed: f64,
    #[serde(default)]
    pub distance: f64,
    #[serde(default)]
    pub spent_fuel: f64,
    #[serde(default)]
    pub engine_hours: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportGeofences {
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub geofence_id: Id,
    #[serde(default)]
    pub start_time: Option<Timestamp>,
    #[serde(default)]
    pub end_time: Option<Timestamp>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportTrips {
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub max_speed: f64,
    #[serde(default)]
    pub average_speed: f64,
    #[serde(default)]
    pub distance: f64,
    #[serde(default)]
    pub spent_fuel: f64,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub start_time: Option<Timestamp>,
    #[serde(default)]
    pub start_address: Option<String>,
    #[serde(default)]
    pub start_lat: f64,
    #[serde(default)]
    pub start_lon: f64,
    #[serde(default)]
    pub end_time: Option<Timestamp>,
    #[serde(default)]
    pub end_address: Option<String>,
    #[serde(default)]
    pub end_lat: f64,
    #[serde(default)]
    pub end_lon: f64,
    #[serde(default)]
    pub driver_unique_id: Option<String>,
    #[serde(default)]
    pub driver_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportStops {
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub start_time: Option<Timestamp>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub lat: f64,
    #[serde(default)]
    pub lon: f64,
    #[serde(default)]
    pub end_time: Option<Timestamp>,
    #[serde(default)]
    pub spent_fuel: f64,
    #[serde(default)]
    pub engine_hours: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CombinedReportItem {
    #[serde(default)]
    pub device_id: Id,
    #[serde(default)]
    pub route: Vec<Vec<f64>>,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub positions: Vec<Position>,
}

/// A generic object type tag used by the document store.
pub mod kind {
    pub const USER: &str = "user";
    pub const DEVICE: &str = "device";
    pub const GROUP: &str = "group";
    pub const GEOFENCE: &str = "geofence";
    pub const CALENDAR: &str = "calendar";
    pub const DRIVER: &str = "driver";
    pub const MAINTENANCE: &str = "maintenance";
    pub const ORDER: &str = "order";
    pub const ATTRIBUTE: &str = "attribute";
    pub const COMMAND: &str = "command";
    pub const QUEUED_COMMAND: &str = "queuedCommand";
    pub const NOTIFICATION: &str = "notification";
    pub const EVENT: &str = "event";
    pub const PERMISSION: &str = "permission";
    pub const POSITION: &str = "position";
    pub const SERVER: &str = "server";
    pub const REVOKED_TOKEN: &str = "revokedToken";
    pub const AUDIT: &str = "audit";
}

/// Actions recorded in the audit log (subset of Traccar's action types).
pub mod action_type {
    pub const LOGIN: &str = "login";
    pub const LOGOUT: &str = "logout";
    pub const DEVICE_UPDATE: &str = "deviceUpdate";
    pub const DEVICE_DELETE: &str = "deviceDelete";
    pub const EDIT: &str = "edit";
    pub const DELETE: &str = "delete";
    pub const LINK: &str = "link";
    pub const UNLINK: &str = "unlink";
    pub const COMMAND: &str = "command";
    pub const SHARE: &str = "share";
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Audit {
    #[serde(default)]
    pub id: Id,
    #[serde(default)]
    pub action_time: Option<Timestamp>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub user_id: Id,
    #[serde(default)]
    pub user_email: Option<String>,
    #[serde(default)]
    pub action_type: Option<String>,
    #[serde(default)]
    pub object_type: Option<String>,
    #[serde(default)]
    pub object_id: Id,
    #[serde(default)]
    pub attributes: Attributes,
}

#[allow(dead_code)]
fn _silence_unused(b: &bool) -> bool {
    is_false(b)
}
