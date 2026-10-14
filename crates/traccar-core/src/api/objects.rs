// SPDX-License-Identifier: Apache-2.0
//
// CRUD resources for the object types Traccar exposes: users, devices, groups,
// geofences, calendars, drivers, maintenance, orders, computed attributes,
// commands, notifications, and permissions.

use super::http::{Request, Response};
use super::share;
use super::App;
use crate::auth::{AuthKind, Principal};
use crate::error::{Error, Result};
use crate::model::*;
use crate::store::Filter;
use serde::de::DeserializeOwned;
use serde::Serialize;

fn require_auth(p: &crate::auth::Principal) -> Result<()> {
    if p.authenticated() {
        Ok(())
    } else {
        Err(Error::unauthorized())
    }
}

/// Extract an object id from a JSON body field, a form field, or a query param.
pub fn extract_id(req: &Request, key: &str) -> Result<Id> {
    if !req.body.is_empty() {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&req.body) {
            if let Some(n) = v.get(key).and_then(|x| x.as_i64()) {
                if n != 0 {
                    return Ok(n);
                }
            }
        }
    }
    if let Some(n) = req.query_i64(key) {
        if n != 0 {
            return Ok(n);
        }
    }
    if let Some(s) = req.form_first(key) {
        if let Ok(n) = s.parse::<Id>() {
            if n != 0 {
                return Ok(n);
            }
        }
    }
    Err(Error::bad_request(format!("{key} is required")))
}

fn json_vec<T: Serialize>(rows: &[T]) -> Response {
    Response::json(200, &rows)
}

// ---------------------------------------------------------------------------
// Simple object CRUD (geofences, calendars, drivers, maintenance, orders,
// computed attributes)
// ---------------------------------------------------------------------------

macro_rules! simple_crud {
    ($list:ident, $create:ident, $get:ident, $update:ident, $delete:ident, $ty:ty, $k:expr, $name:expr) => {
        pub async fn $list(app: &App, req: &Request, p: &Principal) -> Result<Response> {
            require_auth(p)?;
            let all = req.query_bool("all") || req.query_first("all") == Some("true");
            let mut rows: Vec<$ty> = app.repo().accessible_objects(p, $k, all).await?;
            if let Some(kw) = req.query_first("keyword") {
                let kw = kw.to_lowercase();
                rows.retain(|r| {
                    let s = serde_json::to_string(r).unwrap_or_default().to_lowercase();
                    s.contains(&kw)
                });
            }
            apply_window::<$ty>(&mut rows, req);
            Ok(json_vec(&rows))
        }

        pub async fn $create(app: &App, req: &Request, p: &Principal) -> Result<Response> {
            require_auth(p)?;
            let mut entity: $ty = req.json()?;
            set_id(&mut entity, 0);
            let id = app.repo().insert($k, &entity).await?;
            set_id(&mut entity, id);
            app.repo().link_user_object(p.id(), $k, id).await?;
            app.record_audit("create", $name, id, p, req.remote.as_deref()).await;
            Ok(Response::json(200, &entity))
        }

        pub async fn $get(app: &App, p: &Principal, id: Id) -> Result<Response> {
            require_auth(p)?;
            app.repo().require_object(p, $k, id).await?;
            let entity: $ty = app.repo().get($k, id).await?.ok_or_else(Error::not_found)?;
            Ok(Response::json(200, &entity))
        }

        pub async fn $update(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
            require_auth(p)?;
            app.repo().require_object(p, $k, id).await?;
            let mut entity: $ty = req.json()?;
            set_id(&mut entity, id);
            app.repo().update($k, id, &entity).await?;
            app.record_audit("edit", $name, id, p, req.remote.as_deref()).await;
            Ok(Response::json(200, &entity))
        }

        pub async fn $delete(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
            require_auth(p)?;
            app.repo().require_object(p, $k, id).await?;
            app.repo().delete($k, id).await?;
            // Remove links to the deleted object.
            let field = permission_field($k);
            if let Some(field) = field {
                let links: Vec<Permission> = app.repo().permissions(&Filter::new()).await?;
                for l in links {
                    if l.get(field) == id {
                        app.repo().delete(kind::PERMISSION, l.id).await?;
                    }
                }
            }
            app.record_audit("delete", $name, id, p, req.remote.as_deref()).await;
            Ok(Response::no_content())
        }
    };
}

fn apply_window<T>(rows: &mut Vec<T>, req: &Request) {
    if let Some(off) = req.query_i64("offset") {
        if off > 0 {
            *rows = std::mem::take(rows).into_iter().skip(off as usize).collect();
        }
    }
    if let Some(lim) = req.query_i64("limit") {
        rows.truncate(lim.max(0) as usize);
    }
}

fn set_id<T: Serialize + DeserializeOwned>(entity: &mut T, id: Id) {
    // All entities have an `id` field; serialise, set, deserialise.
    if let Ok(mut v) = serde_json::to_value(&*entity) {
        if let Some(o) = v.as_object_mut() {
            o.insert("id".into(), serde_json::Value::from(id));
        }
        if let Ok(t) = serde_json::from_value::<T>(v) {
            *entity = t;
        }
    }
}

simple_crud!(geofences_list, geofences_create, geofences_get, geofences_update, geofences_delete, Geofence, kind::GEOFENCE, "geofence");
simple_crud!(calendars_list, calendars_create, calendars_get, calendars_update, calendars_delete, Calendar, kind::CALENDAR, "calendar");
simple_crud!(drivers_list, drivers_create, drivers_get, drivers_update, drivers_delete, Driver, kind::DRIVER, "driver");
simple_crud!(maintenance_list, maintenance_create, maintenance_get, maintenance_update, maintenance_delete, Maintenance, kind::MAINTENANCE, "maintenance");
simple_crud!(orders_list, orders_create, orders_get, orders_update, orders_delete, Order, kind::ORDER, "order");
simple_crud!(attributes_list, attributes_create, attributes_get, attributes_update, attributes_delete, Attribute, kind::ATTRIBUTE, "attribute");
simple_crud!(commands_list, commands_create, commands_get, commands_update, commands_delete, Command, kind::COMMAND, "command");
simple_crud!(notifications_list, notifications_create, notifications_get, notifications_update, notifications_delete, Notification, kind::NOTIFICATION, "notification");

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

pub async fn users_list(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let repo = app.repo();
    let mut users: Vec<User> = if p.is_admin() {
        repo.users_all().await?
    } else {
        let mut out = vec![p.user.clone()];
        for u in repo.users_all().await? {
            if u.id != p.id() && repo.can_manage_user(p, u.id).await {
                out.push(u);
            }
        }
        out
    };
    if let Some(kw) = req.query_first("keyword") {
        let kw = kw.to_lowercase();
        users.retain(|u| {
            u.name.as_deref().unwrap_or("").to_lowercase().contains(&kw)
                || u.email.as_deref().unwrap_or("").to_lowercase().contains(&kw)
        });
    }
    apply_window(&mut users, req);
    let public: Vec<serde_json::Value> = users.iter().map(|u| u.to_public()).collect();
    Ok(Response::json(200, &public))
}

pub async fn users_create(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    let mut user: User = req.json()?;
    user.id = 0;
    if !p.is_admin() {
        // Self-service registration must be enabled and requested for self.
        if !app.config.registration {
            return Err(Error::forbidden("registration disabled"));
        }
        if user.administrator {
            return Err(Error::forbidden("cannot self-grant administrator"));
        }
    }
    if user.email.as_deref().unwrap_or("").is_empty() {
        return Err(Error::bad_request("email is required"));
    }
    if let Some(_existing) = app.repo().user_by_email(user.email.as_deref().unwrap()).await? {
        return Err(Error::conflict("email already registered"));
    }
    if let Some(pw) = user.password.clone() {
        let hashed = crate::auth::create_hash(&pw)?;
        user.hashed_password = Some(hashed.hash);
        user.salt = Some(hashed.salt);
    }
    let id = app.repo().insert(kind::USER, &user).await?;
    user.id = id;
    if p.is_admin() {
        app.repo().link_user_object(p.id(), kind::USER, id).await.ok();
    }
    app.record_audit("create", "user", id, p, req.remote.as_deref()).await;
    Ok(Response::json(200, &user.to_public()))
}

pub async fn users_get(app: &App, p: &Principal, id: Id) -> Result<Response> {
    app.repo().require_manage_user(p, id).await?;
    let user = app.repo().user(id).await?.ok_or_else(Error::not_found)?;
    Ok(Response::json(200, &user.to_public()))
}

pub async fn users_update(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    app.repo().require_manage_user(p, id).await?;
    let incoming: User = req.json()?;
    let existing = app.repo().user(id).await?.ok_or_else(Error::not_found)?;
    let mut user = existing.clone();
    merge_user(&mut user, &incoming);
    if !p.is_admin() {
        // Non-admins cannot escalate privileges.
        user.administrator = existing.administrator;
        user.readonly = existing.readonly;
        user.device_limit = existing.device_limit;
        user.user_limit = existing.user_limit;
        user.disabled = existing.disabled;
    }
    if let Some(pw) = incoming.password.clone() {
        let hashed = crate::auth::create_hash(&pw)?;
        user.hashed_password = Some(hashed.hash);
        user.salt = Some(hashed.salt);
    }
    app.repo().update(kind::USER, id, &user).await?;
    app.record_audit("edit", "user", id, p, req.remote.as_deref()).await;
    Ok(Response::json(200, &user.to_public()))
}

/// Merge only the fields the caller explicitly supplied. Because `User` uses
/// defaults, we re-serialise both and copy keys present in the request body.
fn merge_user(target: &mut User, incoming: &User) {
    let inc = serde_json::to_value(incoming).unwrap_or_default();
    let inc = inc.as_object().cloned().unwrap_or_default();
    let mut cur = serde_json::to_value(&*target).unwrap_or_default();
    if let Some(cur) = cur.as_object_mut() {
        for (k, v) in inc {
            // Preserve credential material handled separately.
            if k == "password" || k == "hashedPassword" || k == "salt" || k == "totpKey" {
                continue;
            }
            cur.insert(k, v);
        }
    }
    if let Ok(u) = serde_json::from_value(cur) {
        *target = u;
    }
}

pub async fn users_delete(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    if !p.is_admin() {
        return Err(Error::forbidden("only administrators can delete users"));
    }
    if id == p.id() {
        return Err(Error::bad_request("cannot delete yourself"));
    }
    app.repo().delete(kind::USER, id).await?;
    app.repo().delete_many_permissions_for("userId", id).await;
    app.record_audit("delete", "user", id, p, req.remote.as_deref()).await;
    Ok(Response::no_content())
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

pub async fn devices_list(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let all = req.query_bool("all");
    let repo = app.repo();
    let mut devices: Vec<Device> = if let Some(uid) = req.query_first("userId") {
        let uid: Id = uid.parse().map_err(|_| Error::bad_request("invalid userId"))?;
        if uid != p.id() && !p.is_admin() {
            return Err(Error::forbidden("no permission"));
        }
        let ids = repo.user_device_ids(uid).await?;
        repo.devices_by_ids(&ids).await?
    } else {
        repo.accessible_devices(p, all).await?
    };

    if let Some(group) = req.query_i64("groupId") {
        devices.retain(|d| d.group_id == Some(group));
    }
    if !req.query_all("id").is_empty() {
        let wanted: Vec<Id> = req
            .query_all("id")
            .into_iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        devices.retain(|d| wanted.contains(&d.id));
    }
    if !req.query_all("uniqueId").is_empty() {
        let wanted: Vec<&str> = req.query_all("uniqueId");
        devices.retain(|d| d.unique_id.as_deref().map(|u| wanted.contains(&u)).unwrap_or(false));
    }
    if let Some(kw) = req.query_first("keyword") {
        let kw = kw.to_lowercase();
        devices.retain(|d| {
            [d.name.as_deref(), d.unique_id.as_deref(), d.phone.as_deref(), d.model.as_deref(), d.contact.as_deref()]
                .into_iter()
                .flatten()
                .any(|s| s.to_lowercase().contains(&kw))
        });
    }
    apply_window(&mut devices, req);

    // Fill dynamic status + last position id.
    let now = crate::time::Timestamp::now().millis();
    for d in &mut devices {
        d.status = Some(crate::repo::device_status(d, now, app.config.device_timeout_secs).to_string());
        if d.position_id.is_none() {
            if let Some(pos) = repo.last_position(d.id).await? {
                d.position_id = Some(pos.id);
            }
        }
    }

    let exclude_attrs = req.query_bool("excludeAttributes");
    let values: Vec<serde_json::Value> = devices
        .iter()
        .map(|d| {
            let mut v = serde_json::to_value(d).unwrap_or_default();
            if exclude_attrs {
                if let Some(o) = v.as_object_mut() {
                    o.remove("attributes");
                }
            }
            v
        })
        .collect();
    Ok(Response::json(200, &values))
}

pub async fn devices_create(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    if p.user.device_readonly {
        return Err(Error::forbidden("device is read-only"));
    }
    let mut device: Device = req.json()?;
    device.id = 0;
    if device.unique_id.as_deref().unwrap_or("").is_empty() {
        return Err(Error::bad_request("uniqueId is required"));
    }
    if app.repo().device_by_unique_id(device.unique_id.as_deref().unwrap()).await?.is_some() {
        return Err(Error::conflict("uniqueId already exists"));
    }
    // Enforce device limit.
    if !p.is_admin() && p.user.device_limit > 0 {
        let count = app.repo().user_device_ids(p.id()).await?.len() as i32;
        if count >= p.user.device_limit {
            return Err(Error::forbidden("device limit reached"));
        }
    }
    let id = app.repo().insert(kind::DEVICE, &device).await?;
    device.id = id;
    app.repo().link_user_object(p.id(), kind::DEVICE, id).await?;
    app.record_audit("create", "device", id, p, req.remote.as_deref()).await;
    Ok(Response::json(200, &device))
}

async fn fill_device_dynamic(app: &App, mut device: Device) -> Result<Device> {
    let now = crate::time::Timestamp::now().millis();
    device.status = Some(crate::repo::device_status(&device, now, app.config.device_timeout_secs).to_string());
    if let Some(pos) = app.repo().last_position(device.id).await? {
        device.position_id = Some(pos.id);
    }
    Ok(device)
}

pub async fn devices_get(app: &App, _req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    let device = app.repo().require_device(p, id).await?;
    let device = fill_device_dynamic(app, device).await?;
    Ok(Response::json(200, &device))
}

pub async fn devices_update(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    let existing = app.repo().require_device(p, id).await?;
    if p.user.device_readonly && !p.is_admin() {
        return Err(Error::forbidden("device is read-only"));
    }
    let mut device: Device = req.json()?;
    device.id = id;
    // Preserve server-owned fields.
    device.status = existing.status;
    device.last_update = existing.last_update;
    device.position_id = existing.position_id;
    app.repo().update(kind::DEVICE, id, &device).await?;
    app.record_audit("edit", "device", id, p, req.remote.as_deref()).await;
    let device = fill_device_dynamic(app, device).await?;
    Ok(Response::json(200, &device))
}

pub async fn devices_delete(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    app.repo().require_device(p, id).await?;
    app.repo().delete(kind::DEVICE, id).await?;
    app.repo().delete_many_permissions_for("deviceId", id).await;
    app.record_audit("delete", "device", id, p, req.remote.as_deref()).await;
    Ok(Response::no_content())
}

pub async fn devices_accumulators(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    let device = app.repo().require_device(p, id).await?;
    let acc: DeviceAccumulators = req.json()?;
    let positions = app
        .repo()
        .list::<Position>(kind::POSITION, &Filter::new().eq("deviceId", id))
        .await?;
    let total = positions
        .last()
        .and_then(|pos| pos.attr_f64(position_key::TOTAL_DISTANCE))
        .unwrap_or(0.0);
    let _ = device;
    Ok(Response::json(200, &DeviceAccumulators { device_id: id, total_distance: acc.total_distance.max(total), hours: acc.hours }))
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

pub async fn groups_list(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let all = req.query_bool("all");
    let mut groups: Vec<Group> = app.repo().accessible_objects(p, kind::GROUP, all).await?;
    apply_window(&mut groups, req);
    Ok(json_vec(&groups))
}

pub async fn groups_create(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let mut group: Group = req.json()?;
    group.id = 0;
    if group.name.as_deref().unwrap_or("").is_empty() {
        return Err(Error::bad_request("name is required"));
    }
    let id = app.repo().insert(kind::GROUP, &group).await?;
    group.id = id;
    app.repo().link_user_object(p.id(), kind::GROUP, id).await?;
    app.record_audit("create", "group", id, p, req.remote.as_deref()).await;
    Ok(Response::json(200, &group))
}

pub async fn groups_get(app: &App, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    app.repo().require_object(p, kind::GROUP, id).await?;
    let group: Group = app.repo().get(kind::GROUP, id).await?.ok_or_else(Error::not_found)?;
    Ok(Response::json(200, &group))
}

pub async fn groups_update(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    app.repo().require_object(p, kind::GROUP, id).await?;
    let mut group: Group = req.json()?;
    group.id = id;
    app.repo().update(kind::GROUP, id, &group).await?;
    app.record_audit("edit", "group", id, p, req.remote.as_deref()).await;
    Ok(Response::json(200, &group))
}

pub async fn groups_delete(app: &App, req: &Request, p: &Principal, id: Id) -> Result<Response> {
    require_auth(p)?;
    app.repo().require_object(p, kind::GROUP, id).await?;
    app.repo().delete(kind::GROUP, id).await?;
    app.repo().delete_many_permissions_for("groupId", id).await;
    app.record_audit("delete", "group", id, p, req.remote.as_deref()).await;
    Ok(Response::no_content())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub fn command_types() -> Response {
    let types = [
        "custom", "rebootDevice", "engineStop", "engineResume", "positionSingle", "positionPeriodic",
        "positionStop", "setTimezone", "setOdometer", "alarmArm", "alarmDisarm", "alarmSound",
        "sos", "firmwareUpdate", "outputControl", "sendSms", "message",
    ];
    let body: Vec<serde_json::Value> =
        types.iter().map(|t| serde_json::json!({ "type": t })).collect();
    Response::json(200, &body)
}

pub async fn commands_send_list(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let device_id = req.query_i64("deviceId").ok_or_else(|| Error::bad_request("missing deviceId"))?;
    app.repo().require_device(p, device_id).await?;
    let commands: Vec<Command> = app
        .repo()
        .list(kind::COMMAND, &Filter::new().eq("deviceId", device_id))
        .await?;
    Ok(json_vec(&commands))
}

pub async fn commands_send(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    if p.user.limit_commands {
        return Err(Error::forbidden("commands are limited for this user"));
    }
    let command: Command = req.json()?;
    let device_ids: Vec<Id> = if let Some(gid) = req.query_i64("groupId") {
        let devices: Vec<Device> = app
            .repo()
            .accessible_objects(p, kind::DEVICE, true)
            .await?;
        devices.into_iter().filter(|d| d.group_id == Some(gid)).map(|d| d.id).collect()
    } else if command.device_id != 0 {
        app.repo().require_device(p, command.device_id).await?;
        vec![command.device_id]
    } else {
        return Err(Error::bad_request("missing deviceId"));
    };

    let mut queued = Vec::new();
    for device_id in &device_ids {
        let q = QueuedCommand {
            id: 0,
            device_id: *device_id,
            kind: command.kind.clone(),
            text_channel: command.text_channel,
            attributes: command.attributes.clone(),
        };
        let id = app.repo().insert(kind::QUEUED_COMMAND, &q).await?;
        queued.push(QueuedCommand { id, ..q });
    }
    app.record_audit("command", "device", command.device_id, p, req.remote.as_deref()).await;
    if queued.len() == 1 {
        Ok(Response::json(202, &queued[0]))
    } else {
        Ok(Response::json(202, &queued))
    }
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

pub fn notification_types() -> Response {
    let types = [
        event_type::ALARM, event_type::GEOFENCE_ENTER, event_type::GEOFENCE_EXIT,
        event_type::DEVICE_MOVING, event_type::DEVICE_STOPPED, event_type::DEVICE_ONLINE,
        event_type::DEVICE_OFFLINE, event_type::DEVICE_UNKNOWN, event_type::IGNITION_ON,
        event_type::IGNITION_OFF, event_type::MAINTENANCE, event_type::TEXT_MESSAGE,
        event_type::DRIVER_CHANGED, event_type::MEDIA,
    ];
    let body: Vec<serde_json::Value> = types.iter().map(|t| serde_json::json!({ "type": t })).collect();
    Response::json(200, &body)
}

pub fn notificators() -> Response {
    let body = vec![
        serde_json::json!({ "type": "web" }),
        serde_json::json!({ "type": "mail" }),
        serde_json::json!({ "type": "sms" }),
    ];
    Response::json(200, &body)
}

pub async fn notifications_test(_app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    // Validate the body shape, then acknowledge. Delivery is a queue concern in
    // ASK-Server; this endpoint only confirms the message is well-formed.
    let _message: NotificationMessage = req.json()?;
    Ok(Response::no_content())
}

// ---------------------------------------------------------------------------
// Computed attributes
// ---------------------------------------------------------------------------

pub async fn attributes_test(_app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let attribute: Attribute = req.json()?;
    if attribute.attribute.as_deref().unwrap_or("").is_empty() {
        return Err(Error::bad_request("attribute is required"));
    }
    if let Some(expr) = attribute.expression.as_deref() {
        crate::expression::validate(expr)?;
    }
    Ok(Response::json(200, &attribute))
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

pub async fn permissions_list(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    // Build a filter from every `<name>Id` query param.
    let known = [
        "userId", "deviceId", "groupId", "geofenceId", "notificationId", "calendarId",
        "attributeId", "driverId", "managedUserId", "commandId", "maintenanceId", "orderId",
    ];
    let mut filter = Filter::new();
    let mut any = false;
    for (k, v) in &req.query {
        if known.contains(&k.as_str()) {
            if let Ok(n) = v.parse::<i64>() {
                if n != 0 {
                    filter = filter.eq(k, n);
                    any = true;
                } else {
                    // value 0 means "any", so we cannot filter on it
                }
            }
        }
    }
    if !any {
        return Err(Error::bad_request("at least one non-zero id is required"));
    }
    let mut rows: Vec<Permission> = app.repo().permissions(&filter).await?;
    // Non-admins may only see their own links.
    if !p.is_admin() {
        rows.retain(|r| r.user_id == p.id() || r.user_id == 0);
    }
    Ok(json_vec(&rows))
}

pub async fn permissions_link(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let perm: Permission = req.json()?;
    share::require_can_link(app, p, &perm).await?;
    app.repo().insert_permission(&perm).await?;
    app.record_audit("link", "permission", 0, p, req.remote.as_deref()).await;
    Ok(Response::no_content())
}

pub async fn permissions_unlink(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let perm: Permission = req.json()?;
    share::require_can_link(app, p, &perm).await?;
    app.repo().delete_permission(&perm).await?;
    app.record_audit("unlink", "permission", 0, p, req.remote.as_deref()).await;
    Ok(Response::no_content())
}

pub async fn permissions_bulk_link(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let body: serde_json::Value = req.json()?;
    let (rows, _) = crate::api::share::parse_permission_array(&body)?;
    for perm in &rows {
        share::require_can_link(app, p, perm).await?;
        app.repo().insert_permission(perm).await?;
    }
    Ok(Response::no_content())
}

pub async fn permissions_bulk_unlink(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    require_auth(p)?;
    let body: serde_json::Value = req.json()?;
    let (rows, _) = crate::api::share::parse_permission_array(&body)?;
    for perm in &rows {
        share::require_can_link(app, p, perm).await?;
        app.repo().delete_permission(perm).await?;
    }
    Ok(Response::no_content())
}

// keep imports used
#[allow(dead_code)]
fn _assert<T: DeserializeOwned>() {}
#[allow(dead_code)]
fn _uses(_k: AuthKind) {}
