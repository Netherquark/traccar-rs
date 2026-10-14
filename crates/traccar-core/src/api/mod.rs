// SPDX-License-Identifier: Apache-2.0
//
// The API surface. `App` owns the storage handle and configuration, resolves
// the caller's identity, and dispatches to the resource handlers. All handlers
// are transport-agnostic and return a plain `Response`.

pub mod http;

pub mod events;
pub mod objects;
pub mod positions;
pub mod reports_api;
pub mod server;
pub mod session;
pub mod share;

use crate::auth::{AuthKind, Principal, TokenManager};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::{kind, Audit, Id};
use crate::repo::Repo;
use crate::store::Store;
use crate::time::Timestamp;
use http::{Method, Request, Response};
use std::sync::Arc;

/// The application. Cheap to clone via the shared store handle.
pub struct App {
    pub store: Arc<dyn Store>,
    pub config: Config,
    pub tokens: TokenManager,
}

impl App {
    pub fn new(store: Arc<dyn Store>, config: Config) -> Self {
        let tokens = TokenManager::new(&config.token_secret);
        App { store, config, tokens }
    }

    pub fn repo(&self) -> Repo<'_> {
        Repo::new(self.store.as_ref())
    }

    /// Resolve the caller identity. Malformed credentials yield an error (401);
    /// absent credentials yield an anonymous principal.
    pub async fn authenticate(&self, req: &Request) -> Result<Principal> {
        if let Some((email, password)) = req.basic_credentials() {
            let repo = self.repo();
            let user = repo
                .user_by_email(&email)
                .await?
                .ok_or_else(Error::unauthorized)?;
            if user.disabled {
                return Err(Error::unauthorized());
            }
            let (h, s) = (user.hashed_password.clone().unwrap_or_default(), user.salt.clone().unwrap_or_default());
            if !crate::auth::validate_password(&password, &h, &s) {
                return Err(Error::unauthorized());
            }
            return Ok(Principal { user, kind: AuthKind::Basic, expiration: None });
        }

        if let Some(token) = req.bearer_token() {
            return self.principal_from_token(&token, AuthKind::Token).await;
        }

        if let Some(cookie) = req.cookie("JSESSIONID") {
            if let Ok(p) = self.principal_from_token(cookie, AuthKind::Session).await {
                return Ok(p);
            }
        }

        Ok(Principal::anonymous())
    }

    async fn principal_from_token(&self, token: &str, kind: AuthKind) -> Result<Principal> {
        let data = self.tokens.decode(token)?;
        if data.expired() {
            return Err(Error::unauthorized());
        }
        if self.store.get(kind::REVOKED_TOKEN, data.i).await?.is_some() {
            return Err(Error::unauthorized());
        }
        let repo = self.repo();
        let user = repo.user(data.u).await?.ok_or_else(Error::unauthorized)?;
        if user.disabled {
            return Err(Error::unauthorized());
        }
        Ok(Principal { user, kind, expiration: Some(data.expiration()) })
    }

    /// Dispatch a request to the appropriate handler.
    pub async fn handle(&self, req: Request) -> Response {
        let mut resp = self.dispatch(req).await;
        // CORS (mirrors Traccar's CorsResponseFilter).
        resp.headers.push(("access-control-allow-origin".into(), "*".into()));
        resp.headers
            .push(("access-control-allow-headers".into(), "Content-Type, Authorization, X-Requested-With".into()));
        resp.headers.push(("access-control-allow-methods".into(), "GET, POST, PUT, DELETE, OPTIONS".into()));
        resp
    }

    async fn dispatch(&self, req: Request) -> Response {
        if req.method == Method::Options {
            return Response::no_content();
        }

        // Defensive: tolerate a caller that put the query string in `path`
        // instead of `query` (the Worker adapter separates them, but other
        // embedders may not).
        let mut req = req;
        if let Some((p, q)) = req.path.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())) {
            req.path = p;
            if req.query.is_empty() {
                req.query = crate::protocol::parse_query(&q);
            }
        }

        let path = req.path.trim_end_matches('/');
        let path = if path.is_empty() { "/" } else { path };

        // Ingestion lives outside the /api namespace (Traccar serves it on the
        // protocol port). Root and /osmand both accept OsmAnd/Traccar payloads.
        if !path.starts_with("/api") {
            return self.ingest(&req).await;
        }

        let rest = path.trim_start_matches("/api");
        let segs: Vec<&str> = rest.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();

        // Authentication errors are surfaced before routing.
        let principal = match self.authenticate(&req).await {
            Ok(p) => p,
            Err(e) => return Response::problem(&e),
        };

        let result = self.route(&req, &principal, &segs).await;
        match result {
            Ok(r) => r,
            Err(e) => Response::problem(&e),
        }
    }

    async fn route(&self, req: &Request, p: &Principal, segs: &[&str]) -> Result<Response> {
        let m = req.method;
        match (m, segs) {
            // ---- ingestion isn't here; see dispatch ----
            // ---- session ----
            (Method::Post, ["session"]) => session::login(self, req).await,
            (Method::Get, ["session"]) => session::current(self, req, p).await,
            (Method::Delete, ["session"]) => session::logout(self, req, p).await,
            (Method::Get, ["session", id]) => session::get_by_id(self, p, parse_id(id)?).await,
            (Method::Post, ["session", "token"]) => session::issue_token(self, req, p).await,
            (Method::Post, ["session", "token", "revoke"]) => session::revoke_token(self, req, p).await,
            (Method::Get, ["session", "openid", "auth"]) => Err(Error::unsupported("OpenID not enabled")),
            (Method::Get, ["session", "openid", "callback"]) => Err(Error::unsupported("OpenID not enabled")),

            // ---- server / health / statistics / audit ----
            (Method::Get, ["server"]) => server::get_server(self).await,
            (Method::Put, ["server"]) => server::put_server(self, req, p).await,
            (Method::Get, ["server", "geocode"]) => server::geocode(self, req, p).await,
            (Method::Get, ["server", "timezones"]) => Ok(server::timezones()),
            (Method::Get, ["server", "gc"]) => server::gc(self, p).await,
            (Method::Get, ["server", "cache"]) => server::cache(self, p).await,
            (Method::Post, ["server", "reboot"]) => {
                Err(Error::unsupported("reboot is not supported on a serverless runtime"))
            }
            (Method::Post, ["server", "file", _rest @ ..]) => {
                Err(Error::unsupported("static file serving is not supported"))
            }
            (Method::Get, ["health"]) => Ok(Response::text(200, "OK")),
            (Method::Get, ["statistics"]) => server::statistics(self, req, p).await,
            (Method::Get, ["audit"]) => server::audit_list(self, req, p).await,
            (Method::Get, ["password", "reset"]) => Err(Error::unsupported("password reset not configured")),
            (Method::Post, ["password", "reset"]) => Err(Error::unsupported("password reset not configured")),
            (Method::Post, ["password", "update"]) => server::password_update(self, req, p).await,

            // ---- users ----
            (Method::Get, ["users"]) => objects::users_list(self, req, p).await,
            (Method::Post, ["users"]) => objects::users_create(self, req, p).await,
            (Method::Get, ["users", id]) => objects::users_get(self, p, parse_id(id)?).await,
            (Method::Put, ["users", id]) => objects::users_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["users", id]) => objects::users_delete(self, req, p, parse_id(id)?).await,
            (Method::Post, ["users", "totp"]) => Err(Error::unsupported("TOTP not enabled")),

            // ---- devices ----
            (Method::Get, ["devices"]) => objects::devices_list(self, req, p).await,
            (Method::Post, ["devices"]) => objects::devices_create(self, req, p).await,
            (Method::Get, ["devices", id]) => objects::devices_get(self, req, p, parse_id(id)?).await,
            (Method::Put, ["devices", id]) => objects::devices_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["devices", id]) => objects::devices_delete(self, req, p, parse_id(id)?).await,
            (Method::Put, ["devices", id, "accumulators"]) => {
                objects::devices_accumulators(self, req, p, parse_id(id)?).await
            }
            (Method::Post, ["devices", id, "image"]) => {
                let _ = parse_id(id)?;
                Err(Error::unsupported("device images not supported"))
            }

            // ---- groups ----
            (Method::Get, ["groups"]) => objects::groups_list(self, req, p).await,
            (Method::Post, ["groups"]) => objects::groups_create(self, req, p).await,
            (Method::Get, ["groups", id]) => objects::groups_get(self, p, parse_id(id)?).await,
            (Method::Put, ["groups", id]) => objects::groups_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["groups", id]) => objects::groups_delete(self, req, p, parse_id(id)?).await,

            // ---- geofences / calendars / drivers / maintenance / orders / attributes ----
            (Method::Get, ["geofences"]) => objects::geofences_list(self, req, p).await,
            (Method::Post, ["geofences"]) => objects::geofences_create(self, req, p).await,
            (Method::Get, ["geofences", id]) => objects::geofences_get(self, p, parse_id(id)?).await,
            (Method::Put, ["geofences", id]) => objects::geofences_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["geofences", id]) => objects::geofences_delete(self, req, p, parse_id(id)?).await,

            (Method::Get, ["calendars"]) => objects::calendars_list(self, req, p).await,
            (Method::Post, ["calendars"]) => objects::calendars_create(self, req, p).await,
            (Method::Get, ["calendars", id]) => objects::calendars_get(self, p, parse_id(id)?).await,
            (Method::Put, ["calendars", id]) => objects::calendars_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["calendars", id]) => objects::calendars_delete(self, req, p, parse_id(id)?).await,

            (Method::Get, ["drivers"]) => objects::drivers_list(self, req, p).await,
            (Method::Post, ["drivers"]) => objects::drivers_create(self, req, p).await,
            (Method::Get, ["drivers", id]) => objects::drivers_get(self, p, parse_id(id)?).await,
            (Method::Put, ["drivers", id]) => objects::drivers_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["drivers", id]) => objects::drivers_delete(self, req, p, parse_id(id)?).await,

            (Method::Get, ["maintenance"]) => objects::maintenance_list(self, req, p).await,
            (Method::Post, ["maintenance"]) => objects::maintenance_create(self, req, p).await,
            (Method::Get, ["maintenance", id]) => objects::maintenance_get(self, p, parse_id(id)?).await,
            (Method::Put, ["maintenance", id]) => objects::maintenance_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["maintenance", id]) => objects::maintenance_delete(self, req, p, parse_id(id)?).await,

            (Method::Get, ["orders"]) => objects::orders_list(self, req, p).await,
            (Method::Post, ["orders"]) => objects::orders_create(self, req, p).await,
            (Method::Get, ["orders", id]) => objects::orders_get(self, p, parse_id(id)?).await,
            (Method::Put, ["orders", id]) => objects::orders_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["orders", id]) => objects::orders_delete(self, req, p, parse_id(id)?).await,

            (Method::Get, ["attributes", "computed"]) => objects::attributes_list(self, req, p).await,
            (Method::Post, ["attributes", "computed"]) => objects::attributes_create(self, req, p).await,
            (Method::Post, ["attributes", "computed", "test"]) => objects::attributes_test(self, req, p).await,
            (Method::Get, ["attributes", "computed", id]) => objects::attributes_get(self, p, parse_id(id)?).await,
            (Method::Put, ["attributes", "computed", id]) => objects::attributes_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["attributes", "computed", id]) => objects::attributes_delete(self, req, p, parse_id(id)?).await,

            // ---- commands ----
            (Method::Get, ["commands"]) => objects::commands_list(self, req, p).await,
            (Method::Post, ["commands"]) => objects::commands_create(self, req, p).await,
            (Method::Get, ["commands", "types"]) => Ok(objects::command_types()),
            (Method::Get, ["commands", "send"]) => objects::commands_send_list(self, req, p).await,
            (Method::Post, ["commands", "send"]) => objects::commands_send(self, req, p).await,
            (Method::Get, ["commands", id]) => objects::commands_get(self, p, parse_id(id)?).await,
            (Method::Put, ["commands", id]) => objects::commands_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["commands", id]) => objects::commands_delete(self, req, p, parse_id(id)?).await,

            // ---- notifications ----
            (Method::Get, ["notifications"]) => objects::notifications_list(self, req, p).await,
            (Method::Post, ["notifications"]) => objects::notifications_create(self, req, p).await,
            (Method::Get, ["notifications", "types"]) => Ok(objects::notification_types()),
            (Method::Get, ["notifications", "notificators"]) => Ok(objects::notificators()),
            (Method::Post, ["notifications", "test"]) => objects::notifications_test(self, req, p).await,
            (Method::Get, ["notifications", id]) => objects::notifications_get(self, p, parse_id(id)?).await,
            (Method::Put, ["notifications", id]) => objects::notifications_update(self, req, p, parse_id(id)?).await,
            (Method::Delete, ["notifications", id]) => objects::notifications_delete(self, req, p, parse_id(id)?).await,
            (Method::Post, ["notifications", "test", _n]) => objects::notifications_test(self, req, p).await,
            (Method::Post, ["notifications", "send", _n]) => objects::notifications_test(self, req, p).await,

            // ---- permissions ----
            (Method::Get, ["permissions"]) => objects::permissions_list(self, req, p).await,
            (Method::Post, ["permissions"]) => objects::permissions_link(self, req, p).await,
            (Method::Delete, ["permissions"]) => objects::permissions_unlink(self, req, p).await,
            (Method::Post, ["permissions", "bulk"]) => objects::permissions_bulk_link(self, req, p).await,
            (Method::Delete, ["permissions", "bulk"]) => objects::permissions_bulk_unlink(self, req, p).await,

            // ---- positions ----
            (Method::Get, ["positions"]) => positions::list(self, req, p).await,
            (Method::Delete, ["positions"]) => positions::delete(self, req, p).await,
            (Method::Get, ["positions", "kml"]) => positions::kml(self, req, p).await,
            (Method::Get, ["positions", "csv"]) => positions::csv(self, req, p).await,
            (Method::Get, ["positions", "gpx"]) => positions::gpx(self, req, p).await,
            (Method::Delete, ["positions", id]) => positions::delete_one(self, p, parse_id(id)?).await,

            // ---- events ----
            (Method::Get, ["events", id]) => events::get(self, p, parse_id(id)?).await,

            // ---- reports ----
            (Method::Get, ["reports", "combined"]) => reports_api::combined(self, req, p).await,
            (Method::Get, ["reports", "route"]) => reports_api::route(self, req, p).await,
            (Method::Get, ["reports", "route", kind]) => reports_api::route_typed(self, req, p, kind).await,
            (Method::Get, ["reports", "events"]) => reports_api::events_report(self, req, p, "").await,
            (Method::Get, ["reports", "events", kind]) => reports_api::events_report(self, req, p, kind).await,
            (Method::Get, ["reports", "geofences"]) => reports_api::geofences(self, req, p).await,
            (Method::Get, ["reports", "summary"]) => reports_api::summary(self, req, p, "").await,
            (Method::Get, ["reports", "summary", kind]) => reports_api::summary(self, req, p, kind).await,
            (Method::Get, ["reports", "trips"]) => reports_api::trips(self, req, p, "").await,
            (Method::Get, ["reports", "trips", kind]) => reports_api::trips(self, req, p, kind).await,
            (Method::Get, ["reports", "stops"]) => reports_api::stops(self, req, p, "").await,
            (Method::Get, ["reports", "stops", kind]) => reports_api::stops(self, req, p, kind).await,
            (Method::Get, ["reports", "devices", kind]) => reports_api::devices_report(self, req, p, kind).await,

            // ---- share ----
            (Method::Post, ["share", "device"]) => share::share_device(self, req, p).await,
            (Method::Post, ["share", "group"]) => share::share_group(self, req, p).await,
            (Method::Get, ["share", _token]) => share::view(self, req, p).await,

            // ---- websocket + streaming ----
            (Method::Get, ["socket"]) => Ok(Response::text(426, "Upgrade Required")),
            (Method::Get, ["stream", _d, _c, _f]) => Err(Error::unsupported("live streaming not supported")),

            _ => Err(Error::not_found()),
        }
    }

    // -----------------------------------------------------------------------
    // Ingestion
    // -----------------------------------------------------------------------

    async fn ingest(&self, req: &Request) -> Response {
        match crate::api::ingestion::handle(self, req).await {
            Ok(r) => r,
            Err(e) => {
                // Ingestion is device-facing and Traccar returns bare status
                // codes here, not JSON problem documents.
                Response::new(e.status)
            }
        }
    }

    /// Append an audit row. Best-effort: audit failures never break the request.
    pub async fn record_audit(
        &self,
        action_type: &str,
        object_type: &str,
        object_id: Id,
        p: &Principal,
        address: Option<&str>,
    ) {
        let mut audit = Audit {
            id: 0,
            action_time: Some(Timestamp::now()),
            address: address.map(|s| s.to_string()),
            user_id: p.id(),
            user_email: p.user.email.clone(),
            action_type: Some(action_type.into()),
            object_type: Some(object_type.into()),
            object_id,
            attributes: Default::default(),
        };
        let _ = self.store.insert(kind::AUDIT, serde_json::to_value(&audit).unwrap_or_default()).await;
        audit.id = 0;
    }
}

pub mod ingestion;

/// The implemented REST route table, as `(METHOD, "/path/{param}")` pairs in
/// Traccar OpenAPI syntax. Kept in lock-step with `route` by the
/// `api_parity` integration test, which diffs this against the vendored
/// `docs/openapi.yaml`.
pub fn routes() -> Vec<(&'static str, &'static str)> {
    vec![
        // session
        ("GET", "/session"),
        ("POST", "/session"),
        ("DELETE", "/session"),
        ("GET", "/session/{id}"),
        ("POST", "/session/token"),
        ("POST", "/session/token/revoke"),
        // server / health / statistics / audit
        ("GET", "/server"),
        ("PUT", "/server"),
        ("GET", "/server/geocode"),
        ("GET", "/server/timezones"),
        ("GET", "/server/gc"),
        ("GET", "/server/cache"),
        ("GET", "/health"),
        ("GET", "/statistics"),
        ("GET", "/audit"),
        ("POST", "/password/update"),
        // users
        ("GET", "/users"),
        ("POST", "/users"),
        ("GET", "/users/{id}"),
        ("PUT", "/users/{id}"),
        ("DELETE", "/users/{id}"),
        // devices
        ("GET", "/devices"),
        ("POST", "/devices"),
        ("GET", "/devices/{id}"),
        ("PUT", "/devices/{id}"),
        ("DELETE", "/devices/{id}"),
        ("PUT", "/devices/{id}/accumulators"),
        // groups
        ("GET", "/groups"),
        ("POST", "/groups"),
        ("GET", "/groups/{id}"),
        ("PUT", "/groups/{id}"),
        ("DELETE", "/groups/{id}"),
        // geofences
        ("GET", "/geofences"),
        ("POST", "/geofences"),
        ("GET", "/geofences/{id}"),
        ("PUT", "/geofences/{id}"),
        ("DELETE", "/geofences/{id}"),
        // calendars
        ("GET", "/calendars"),
        ("POST", "/calendars"),
        ("GET", "/calendars/{id}"),
        ("PUT", "/calendars/{id}"),
        ("DELETE", "/calendars/{id}"),
        // drivers
        ("GET", "/drivers"),
        ("POST", "/drivers"),
        ("GET", "/drivers/{id}"),
        ("PUT", "/drivers/{id}"),
        ("DELETE", "/drivers/{id}"),
        // maintenance
        ("GET", "/maintenance"),
        ("POST", "/maintenance"),
        ("GET", "/maintenance/{id}"),
        ("PUT", "/maintenance/{id}"),
        ("DELETE", "/maintenance/{id}"),
        // orders
        ("GET", "/orders"),
        ("POST", "/orders"),
        ("GET", "/orders/{id}"),
        ("PUT", "/orders/{id}"),
        ("DELETE", "/orders/{id}"),
        // computed attributes
        ("GET", "/attributes/computed"),
        ("POST", "/attributes/computed"),
        ("POST", "/attributes/computed/test"),
        ("GET", "/attributes/computed/{id}"),
        ("PUT", "/attributes/computed/{id}"),
        ("DELETE", "/attributes/computed/{id}"),
        // commands
        ("GET", "/commands"),
        ("POST", "/commands"),
        ("GET", "/commands/types"),
        ("GET", "/commands/send"),
        ("POST", "/commands/send"),
        ("GET", "/commands/{id}"),
        ("PUT", "/commands/{id}"),
        ("DELETE", "/commands/{id}"),
        // notifications
        ("GET", "/notifications"),
        ("POST", "/notifications"),
        ("GET", "/notifications/types"),
        ("GET", "/notifications/notificators"),
        ("POST", "/notifications/test"),
        ("POST", "/notifications/test/{notificator}"),
        ("POST", "/notifications/send/{notificator}"),
        ("GET", "/notifications/{id}"),
        ("PUT", "/notifications/{id}"),
        ("DELETE", "/notifications/{id}"),
        // permissions
        ("GET", "/permissions"),
        ("POST", "/permissions"),
        ("DELETE", "/permissions"),
        ("POST", "/permissions/bulk"),
        ("DELETE", "/permissions/bulk"),
        // positions
        ("GET", "/positions"),
        ("DELETE", "/positions"),
        ("DELETE", "/positions/{id}"),
        ("GET", "/positions/kml"),
        ("GET", "/positions/csv"),
        ("GET", "/positions/gpx"),
        // events
        ("GET", "/events/{id}"),
        // reports
        ("GET", "/reports/combined"),
        ("GET", "/reports/route"),
        ("GET", "/reports/route/{type}"),
        ("GET", "/reports/events"),
        ("GET", "/reports/events/{type}"),
        ("GET", "/reports/geofences"),
        ("GET", "/reports/summary"),
        ("GET", "/reports/summary/{type}"),
        ("GET", "/reports/trips"),
        ("GET", "/reports/trips/{type}"),
        ("GET", "/reports/stops"),
        ("GET", "/reports/stops/{type}"),
        ("GET", "/reports/devices/{type}"),
        // share
        ("POST", "/share/device"),
        ("POST", "/share/group"),
    ]
}

/// Contract endpoints intentionally not implemented, with a reason. The parity
/// test requires every OpenAPI path/method to be either implemented or listed
/// here, so nothing is silently dropped.
pub fn deferred_routes() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("POST", "/devices/{id}/image", "Media storage (R2) not wired in the core crate"),
        ("POST", "/password/reset", "Requires an outbound mail provider"),
        ("POST", "/server/file/{path}", "No writable filesystem on Workers"),
        ("POST", "/server/reboot", "No host process to reboot on Workers"),
        ("GET", "/session/openid/auth", "OpenID Connect not configured"),
        ("GET", "/session/openid/callback", "OpenID Connect not configured"),
        ("POST", "/users/totp", "TOTP enrolment not implemented"),
        ("GET", "/stream/{deviceId}/{channel}/live.m3u8", "Media streaming deferred to R2/Stream"),
        ("GET", "/stream/{deviceId}/{channel}/{index}.ts", "Media streaming deferred to R2/Stream"),
    ]
}

fn parse_id(s: &str) -> Result<Id> {
    s.parse::<Id>().map_err(|_| Error::bad_request("invalid id"))
}
