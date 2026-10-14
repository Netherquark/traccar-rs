// SPDX-License-Identifier: Apache-2.0
//
// Typed repository over the document `Store`, plus Traccar-compatible
// permission resolution (admin sees everything; otherwise access is granted
// through user->device and user->group links, following the group hierarchy).

use crate::auth::Principal;
use crate::error::{Error, Result};
use crate::model::*;
use crate::store::{Filter, Store};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

pub struct Repo<'a> {
    pub store: &'a dyn Store,
}

fn from_value<T: DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| Error::internal(format!("decode: {e}")))
}

fn to_value<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| Error::internal(format!("encode: {e}")))
}

impl<'a> Repo<'a> {
    pub fn new(store: &'a dyn Store) -> Self {
        Repo { store }
    }

    // --- generic ---
    pub async fn get<T: DeserializeOwned>(&self, kind: &str, id: Id) -> Result<Option<T>> {
        match self.store.get(kind, id).await? {
            Some(v) => Ok(Some(from_value(v)?)),
            None => Ok(None),
        }
    }

    pub async fn list<T: DeserializeOwned>(&self, kind: &str, f: &Filter) -> Result<Vec<T>> {
        self.store.list(kind, f).await?.into_iter().map(from_value).collect()
    }

    pub async fn insert<T: Serialize>(&self, kind: &str, v: &T) -> Result<Id> {
        self.store.insert(kind, to_value(v)?).await
    }

    pub async fn update<T: Serialize>(&self, kind: &str, id: Id, v: &T) -> Result<()> {
        self.store.update(kind, id, to_value(v)?).await
    }

    pub async fn delete(&self, kind: &str, id: Id) -> Result<()> {
        self.store.delete(kind, id).await
    }

    // --- users ---
    pub async fn user(&self, id: Id) -> Result<Option<User>> {
        self.get(kind::USER, id).await
    }
    pub async fn user_by_email(&self, email: &str) -> Result<Option<User>> {
        Ok(self
            .list::<User>(kind::USER, &Filter::new().eq("email", email))
            .await?
            .into_iter()
            .next())
    }
    pub async fn users_all(&self) -> Result<Vec<User>> {
        self.list(kind::USER, &Filter::new()).await
    }

    // --- devices ---
    pub async fn device(&self, id: Id) -> Result<Option<Device>> {
        self.get(kind::DEVICE, id).await
    }
    pub async fn device_by_unique_id(&self, unique_id: &str) -> Result<Option<Device>> {
        Ok(self
            .list::<Device>(kind::DEVICE, &Filter::new().eq("uniqueId", unique_id))
            .await?
            .into_iter()
            .next())
    }
    pub async fn devices_by_ids(&self, ids: &[Id]) -> Result<Vec<Device>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let vals: Vec<Value> = ids.iter().map(|i| Value::from(*i)).collect();
        self.list(kind::DEVICE, &Filter::new().in_("id", vals)).await
    }

    // --- positions ---
    pub async fn insert_position(&self, p: &mut Position) -> Result<()> {
        let id = self.store.insert(kind::POSITION, to_value(p)?).await?;
        p.id = id;
        Ok(())
    }

    /// Positions for one device within [from, to] ordered by fixTime ascending.
    pub async fn positions_range(&self, device_id: Id, from: i64, to: i64) -> Result<Vec<Position>> {
        let (lo, hi) = iso_bounds(from, to);
        self.list(
            kind::POSITION,
            &Filter::new().eq("deviceId", device_id).range_iso("fixTime", lo, hi).order("fixTime", false),
        )
        .await
    }

    pub async fn position(&self, id: Id) -> Result<Option<Position>> {
        self.get(kind::POSITION, id).await
    }

    pub async fn last_position(&self, device_id: Id) -> Result<Option<Position>> {
        Ok(self
            .list::<Position>(kind::POSITION, &Filter::new().eq("deviceId", device_id).order("fixTime", true).limit(1))
            .await?
            .into_iter()
            .next())
    }

    pub async fn last_positions(&self, device_ids: &[Id]) -> Result<Vec<Position>> {
        let mut out = Vec::new();
        for id in device_ids {
            if let Some(p) = self.last_position(*id).await? {
                out.push(p);
            }
        }
        Ok(out)
    }

    pub async fn delete_positions_range(&self, device_id: Id, from: i64, to: i64) -> Result<usize> {
        let (lo, hi) = iso_bounds(from, to);
        Ok(self
            .store
            .delete_many(
                kind::POSITION,
                &Filter::new().eq("deviceId", device_id).range_iso("fixTime", lo, hi),
            )
            .await?)
    }

    // --- permissions ---
    pub async fn permissions(&self, f: &Filter) -> Result<Vec<Permission>> {
        self.list(kind::PERMISSION, f).await
    }

    pub async fn insert_permission(&self, p: &Permission) -> Result<()> {
        if p.pair().is_none() {
            return Err(Error::bad_request("invalid permission pair"));
        }
        // Deduplicate identical links.
        let existing = self
            .list::<Permission>(kind::PERMISSION, &Filter::new())
            .await?
            .into_iter()
            .any(|x| &x == p);
        if !existing {
            self.insert(kind::PERMISSION, p).await?;
        }
        Ok(())
    }

    pub async fn delete_permission(&self, p: &Permission) -> Result<bool> {
        let all = self.permissions(&Filter::new()).await?;
        for row in all {
            if &row == p {
                self.delete(kind::PERMISSION, row.id).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Direct + inherited group links for a user.
    pub async fn user_group_ids(&self, user_id: Id) -> Result<HashSet<Id>> {
        let links = self
            .list::<Permission>(kind::PERMISSION, &Filter::new().eq("userId", user_id))
            .await?;
        let mut seed: HashSet<Id> = links.into_iter().filter(|p| p.group_id != 0).map(|p| p.group_id).collect();
        // Expand to descendants.
        let all_groups: Vec<Group> = self.list(kind::GROUP, &Filter::new()).await?;
        let mut changed = true;
        while changed {
            changed = false;
            for g in &all_groups {
                if seed.contains(&g.group_id) && !seed.contains(&g.id) {
                    seed.insert(g.id);
                    changed = true;
                }
            }
        }
        Ok(seed)
    }

    pub async fn user_device_ids(&self, user_id: Id) -> Result<Vec<Id>> {
        let links = self
            .list::<Permission>(kind::PERMISSION, &Filter::new().eq("userId", user_id))
            .await?;
        let mut ids: HashSet<Id> = links.into_iter().filter(|p| p.device_id != 0).map(|p| p.device_id).collect();
        let groups = self.user_group_ids(user_id).await?;
        if !groups.is_empty() {
            let devices: Vec<Device> = self.list(kind::DEVICE, &Filter::new()).await?;
            for d in devices {
                if let Some(g) = d.group_id {
                    if g != 0 && groups.contains(&g) {
                        ids.insert(d.id);
                    }
                }
            }
        }
        Ok(ids.into_iter().collect())
    }

    /// Devices (not just ids) accessible to a principal.
    pub async fn accessible_devices(&self, p: &Principal, all: bool) -> Result<Vec<Device>> {
        if all && (p.is_admin() || p.user.readonly) {
            return self.list(kind::DEVICE, &Filter::new()).await;
        }
        if p.is_admin() {
            return self.list(kind::DEVICE, &Filter::new()).await;
        }
        let ids = self.user_device_ids(p.id()).await?;
        self.devices_by_ids(&ids).await
    }

    pub async fn can_access_device(&self, p: &Principal, device: &Device) -> bool {
        if p.is_admin() {
            return true;
        }
        let ids = self.user_device_ids(p.id()).await.unwrap_or_default();
        ids.contains(&device.id)
    }

    pub async fn require_device(&self, p: &Principal, id: Id) -> Result<Device> {
        let d = self.device(id).await?.ok_or_else(Error::not_found)?;
        if !self.can_access_device(p, &d).await {
            return Err(Error::forbidden("no access to device"));
        }
        Ok(d)
    }

    /// Check basic user-management permission: admins manage everyone; a user
    /// may manage itself and users linked via `managedUserId`.
    pub async fn can_manage_user(&self, p: &Principal, target: Id) -> bool {
        if p.is_admin() {
            return true;
        }
        if p.id() == target {
            return true;
        }
        let links = self
            .permissions(&Filter::new().eq("userId", p.id()))
            .await
            .unwrap_or_default();
        links.iter().any(|x| x.managed_user_id == target)
    }

    pub async fn require_manage_user(&self, p: &Principal, target: Id) -> Result<()> {
        if self.can_manage_user(p, target).await {
            Ok(())
        } else {
            Err(Error::forbidden("no permission"))
        }
    }

    // --- generic object permissions -------------------------------------

    /// Objects of `kind` directly linked to `user_id` via `userId -> <kind>Id`.
    pub async fn user_object_ids(&self, user_id: Id, object_kind: &str) -> Result<Vec<Id>> {
        let Some(field) = permission_field(object_kind) else { return Ok(Vec::new()) };
        let links = self
            .list::<Permission>(kind::PERMISSION, &Filter::new().eq("userId", user_id))
            .await?;
        Ok(links.into_iter().map(|p| p.get(field)).filter(|v| *v != 0).collect())
    }

    /// Link a user to an object of `kind` (Traccar `Permission(user, kind, id)`).
    pub async fn link_user_object(&self, user_id: Id, object_kind: &str, object_id: Id) -> Result<()> {
        let Some(field) = permission_field(object_kind) else {
            return Err(Error::bad_request("unlinkable object kind"));
        };
        let mut perm = Permission { user_id, ..Default::default() };
        perm.set(field, object_id);
        self.insert_permission(&perm).await
    }

    pub async fn unlink_user_object(&self, user_id: Id, object_kind: &str, object_id: Id) -> Result<bool> {
        let Some(field) = permission_field(object_kind) else {
            return Err(Error::bad_request("unlinkable object kind"));
        };
        let mut perm = Permission { user_id, ..Default::default() };
        perm.set(field, object_id);
        self.delete_permission(&perm).await
    }

    /// Remove every permission row whose `field` equals `id`. Used when an
    /// object is deleted so no dangling links remain.
    pub async fn delete_many_permissions_for(&self, field: &str, id: Id) {
        let _ = self.delete_many_permissions(field, id).await;
    }

    async fn delete_many_permissions(&self, field: &str, id: Id) -> Result<()> {
        let links = self.permissions(&Filter::new()).await?;
        for l in links {
            if l.get(field) == id {
                self.delete(kind::PERMISSION, l.id).await?;
            }
        }
        Ok(())
    }

    pub async fn can_access_object(&self, p: &Principal, object_kind: &str, object_id: Id) -> bool {
        if p.is_admin() {
            return true;
        }
        self.user_object_ids(p.id(), object_kind)
            .await
            .unwrap_or_default()
            .contains(&object_id)
    }

    pub async fn require_object(&self, p: &Principal, object_kind: &str, object_id: Id) -> Result<()> {
        if self.can_access_object(p, object_kind, object_id).await {
            Ok(())
        } else {
            Err(Error::forbidden("no permission"))
        }
    }

    /// Objects of `kind` visible to a principal (all for admins when `all`;
    /// linked otherwise).
    pub async fn accessible_objects<T: DeserializeOwned>(
        &self,
        p: &Principal,
        object_kind: &str,
        all: bool,
    ) -> Result<Vec<T>> {
        if p.is_admin() && all {
            return self.list(object_kind, &Filter::new()).await;
        }
        let ids = self.user_object_ids(p.id(), object_kind).await?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let vals: Vec<Value> = ids.iter().map(|i| Value::from(*i)).collect();
        self.list(object_kind, &Filter::new().in_("id", vals)).await
    }
}

/// Convert a millisecond range to inclusive ISO-8601 string bounds. Uses a
/// wide-but-valid default when the caller passes open bounds.
pub fn iso_bounds(from: i64, to: i64) -> (String, String) {
    use crate::time::{format_iso, Timestamp};
    let lo = if from <= 0 { 0 } else { from };
    let hi = if to >= i64::MAX / 2 {
        crate::time::Timestamp::now().millis() + 100 * 366 * 24 * 3600 * 1000
    } else {
        to
    };
    (format_iso(Timestamp::from_millis(lo).0), format_iso(Timestamp::from_millis(hi).0))
}

/// Whether a device should show as online given its last update and the
/// configured timeout (Traccar default 10 minutes).
pub fn device_status(device: &Device, now_millis: i64, timeout_secs: i64) -> &'static str {
    match device.last_update {
        None => device_status::UNKNOWN,
        Some(t) => {
            if now_millis - t.millis() <= timeout_secs * 1000 {
                device_status::ONLINE
            } else {
                device_status::OFFLINE
            }
        }
    }
}
