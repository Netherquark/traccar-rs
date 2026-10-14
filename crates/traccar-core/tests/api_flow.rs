// SPDX-License-Identifier: Apache-2.0
//
// End-to-end HTTP flow through `App::handle`: login, session, device CRUD,
// device ingestion, position history, permissions, and error semantics. These
// exercise the same code path the Worker adapter uses (transport-agnostic
// `Request` -> `Response`), so a passing run means the core contract works.

use std::sync::Arc;

use serde_json::{json, Value};
use traccar_core::api::http::{Method, Request, Response};
use traccar_core::api::App;
use traccar_core::config::Config;
use traccar_core::store::MemoryStore;

fn make_app() -> App {
    make_app_with(Config::default())
}

fn make_app_with(config: Config) -> App {
    let store = Arc::new(MemoryStore::new());
    let app = App::new(store, config);
    pollster::block_on(traccar_core::seed::bootstrap_admin(
        app.store.as_ref(),
        "admin@example.com",
        "Admin",
        "secret",
    ))
    .unwrap();
    app
}

fn call(app: &App, req: Request) -> Response {
    pollster::block_on(app.handle(req))
}

fn header(res: &Response, name: &str) -> Option<String> {
    res.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn body_json<T: serde::de::DeserializeOwned>(res: &Response) -> T {
    serde_json::from_slice(&res.body).unwrap_or_else(|e| {
        panic!(
            "response body was not valid JSON ({e}): {}",
            String::from_utf8_lossy(&res.body)
        )
    })
}

fn form(method: Method, path: &str, body: &str) -> Request {
    let mut r = Request::new(method, path);
    r.headers
        .push(("content-type".into(), "application/x-www-form-urlencoded".into()));
    r.body = body.as_bytes().to_vec();
    r
}

fn json_req(method: Method, path: &str, v: Value) -> Request {
    let mut r = Request::new(method, path);
    r.headers.push(("content-type".into(), "application/json".into()));
    r.body = serde_json::to_vec(&v).unwrap();
    r
}

fn authed(mut r: Request, token: &str) -> Request {
    r.cookies.push(("JSESSIONID".into(), token.to_string()));
    r
}

fn login(app: &App) -> String {
    let res = call(app, form(Method::Post, "/api/session", "email=admin@example.com&password=secret"));
    assert_eq!(res.status, 200, "login failed");
    let set_cookie = header(&res, "set-cookie").unwrap();
    set_cookie
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("JSESSIONID=")
        .to_string()
}

#[test]
fn login_returns_session_cookie_and_public_user() {
    let app = make_app();
    let res = call(&app, form(Method::Post, "/api/session", "email=admin@example.com&password=secret"));
    assert_eq!(res.status, 200);
    assert!(header(&res, "set-cookie").unwrap().contains("JSESSIONID="));
    let user: Value = body_json(&res);
    assert_eq!(user["email"], "admin@example.com");
    assert!(user.get("hashedPassword").is_none(), "credential material must not leak");
    assert!(user.get("salt").is_none());
}

#[test]
fn wrong_password_is_401() {
    let app = make_app();
    let res = call(&app, form(Method::Post, "/api/session", "email=admin@example.com&password=nope"));
    assert_eq!(res.status, 401);
}

#[test]
fn protected_endpoint_requires_auth() {
    let app = make_app();
    let res = call(&app, Request::new(Method::Get, "/api/devices"));
    assert_eq!(res.status, 401);
}

#[test]
fn session_current_needs_cookie() {
    let app = make_app();
    let token = login(&app);
    let res = call(&app, authed(Request::new(Method::Get, "/api/session"), &token));
    assert_eq!(res.status, 200);
    let user: Value = body_json(&res);
    assert_eq!(user["email"], "admin@example.com");
}

#[test]
fn bearer_token_grants_access() {
    let app = make_app();
    let token = login(&app);
    let res = call(&app, authed(form(Method::Post, "/api/session/token", ""), &token));
    assert_eq!(res.status, 200);
    let api_token = String::from_utf8(res.body.clone()).unwrap();
    assert!(!api_token.is_empty());

    let mut req = Request::new(Method::Get, "/api/devices");
    req.headers.push(("authorization".into(), format!("Bearer {api_token}")));
    assert_eq!(call(&app, req).status, 200);
}

#[test]
fn device_crud_and_duplicate_unique_id() {
    let app = make_app();
    let token = login(&app);

    let res = call(
        &app,
        authed(json_req(Method::Post, "/api/devices", json!({"name": "Car", "uniqueId": "TEST123"})), &token),
    );
    assert_eq!(res.status, 200);
    let device: Value = body_json(&res);
    let device_id = device["id"].as_i64().unwrap();
    assert_eq!(device["uniqueId"], "TEST123");
    assert_eq!(device["name"], "Car");

    // Duplicate uniqueId is a conflict.
    let dup = call(
        &app,
        authed(json_req(Method::Post, "/api/devices", json!({"name": "Other", "uniqueId": "TEST123"})), &token),
    );
    assert_eq!(dup.status, 409);

    // List shows exactly the one device.
    let list = call(&app, authed(Request::new(Method::Get, "/api/devices"), &token));
    let devices: Vec<Value> = body_json(&list);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["status"], "unknown");

    // Get by id.
    let got = call(&app, authed(Request::new(Method::Get, &format!("/api/devices/{device_id}")), &token));
    assert_eq!(got.status, 200);

    // Update.
    let upd = call(
        &app,
        authed(json_req(Method::Put, &format!("/api/devices/{device_id}"), json!({"name": "Van", "uniqueId": "TEST123"})), &token),
    );
    assert_eq!(upd.status, 200);
    let updated: Value = body_json(&upd);
    assert_eq!(updated["name"], "Van");

    // Delete.
    let del = call(&app, authed(Request::new(Method::Delete, &format!("/api/devices/{device_id}")), &token));
    assert_eq!(del.status, 204);
    let list = call(&app, authed(Request::new(Method::Get, "/api/devices"), &token));
    let devices: Vec<Value> = body_json(&list);
    assert!(devices.is_empty());
}

#[test]
fn ingestion_stores_position_and_updates_device() {
    let app = make_app();
    let token = login(&app);
    let created = call(
        &app,
        authed(json_req(Method::Post, "/api/devices", json!({"name": "Car", "uniqueId": "TEST123"})), &token),
    );
    let device_id = body_json::<Value>(&created)["id"].as_i64().unwrap();

    let mut ingest = Request::new(Method::Get, "/");
    ingest.query = vec![
        ("id".into(), "TEST123".into()),
        ("lat".into(), "10.5".into()),
        ("lon".into(), "20.5".into()),
        ("timestamp".into(), "2024-01-01T00:00:00.000+00:00".into()),
        ("speed".into(), "12.5".into()),
        ("bearing".into(), "90".into()),
    ];
    assert_eq!(call(&app, ingest).status, 200);

    let positions = call(
        &app,
        authed(Request::new(Method::Get, &format!("/api/positions?deviceId={device_id}")), &token),
    );
    assert_eq!(positions.status, 200);
    let positions: Vec<Value> = body_json(&positions);
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0]["latitude"], 10.5);
    assert_eq!(positions[0]["longitude"], 20.5);
    assert_eq!(positions[0]["deviceId"], device_id);
    assert_eq!(positions[0]["valid"], true);

    // The device now reports a position id and a last update.
    let dev = call(&app, authed(Request::new(Method::Get, &format!("/api/devices/{device_id}")), &token));
    let dev: Value = body_json(&dev);
    assert!(dev["positionId"].as_i64().unwrap() > 0);
    assert!(dev["lastUpdate"].is_string());
}

#[test]
fn ingestion_unknown_device_query_is_400_when_registration_disabled() {
    let mut config = Config::default();
    config.register_unknown = false;
    let app = make_app_with(config);

    let mut ingest = Request::new(Method::Get, "/");
    ingest.query = vec![("id".into(), "GHOST".into()), ("lat".into(), "1".into()), ("lon".into(), "2".into())];
    assert_eq!(call(&app, ingest).status, 400);
}

#[test]
fn ingestion_unknown_device_json_is_404() {
    let app = make_app();
    let mut ingest = Request::new(Method::Post, "/");
    ingest.headers.push(("content-type".into(), "application/json".into()));
    ingest.body = serde_json::to_vec(&json!({
        "device_id": "GHOST",
        "location": {"coords": {"latitude": 1.0, "longitude": 2.0}, "timestamp": "2024-01-01T00:00:00.000+00:00"}
    }))
    .unwrap();
    assert_eq!(call(&app, ingest).status, 404);
}

#[test]
fn disabled_device_ingestion_is_403() {
    let app = make_app();
    let token = login(&app);
    let created = call(
        &app,
        authed(json_req(Method::Post, "/api/devices", json!({"name": "Car", "uniqueId": "D1", "disabled": true})), &token),
    );
    let device: Value = body_json(&created);
    assert_eq!(device["disabled"], true);

    let mut ingest = Request::new(Method::Get, "/");
    ingest.query = vec![("id".into(), "D1".into()), ("lat".into(), "1".into()), ("lon".into(), "2".into())];
    assert_eq!(call(&app, ingest).status, 403);
}

#[test]
fn permissions_link_and_unlink() {
    let app = make_app();
    let token = login(&app);
    let user: Value = body_json(&call(
        &app,
        authed(json_req(Method::Post, "/api/users", json!({"name": "Bob", "email": "bob@example.com", "password": "pw"})), &token),
    ));
    let user_id = user["id"].as_i64().unwrap();
    let device: Value = body_json(&call(
        &app,
        authed(json_req(Method::Post, "/api/devices", json!({"name": "Car", "uniqueId": "LINK1"})), &token),
    ));
    let device_id = device["id"].as_i64().unwrap();

    let link = call(
        &app,
        authed(json_req(Method::Post, "/api/permissions", json!({"userId": user_id, "deviceId": device_id})), &token),
    );
    assert_eq!(link.status, 204);

    let list = call(
        &app,
        authed(Request::new(Method::Get, &format!("/api/permissions?userId={user_id}")), &token),
    );
    assert_eq!(list.status, 200);
    let perms: Vec<Value> = body_json(&list);
    assert_eq!(perms.len(), 1);
    assert_eq!(perms[0]["userId"], user_id);
    assert_eq!(perms[0]["deviceId"], device_id);
    assert!(perms[0].get("id").is_none(), "permission id must not be on the wire");

    let unlink = call(
        &app,
        authed(json_req(Method::Delete, "/api/permissions", json!({"userId": user_id, "deviceId": device_id})), &token),
    );
    assert_eq!(unlink.status, 204);
    let perms: Vec<Value> = body_json(&call(
        &app,
        authed(Request::new(Method::Get, &format!("/api/permissions?userId={user_id}")), &token),
    ));
    assert!(perms.is_empty());
}

#[test]
fn socket_endpoint_requests_upgrade() {
    let app = make_app();
    let token = login(&app);
    let res = call(&app, authed(Request::new(Method::Get, "/api/socket"), &token));
    assert_eq!(res.status, 426);
}

#[test]
fn health_is_public() {
    let app = make_app();
    assert_eq!(call(&app, Request::new(Method::Get, "/api/health")).status, 200);
}

#[test]
fn unknown_api_path_is_404() {
    let app = make_app();
    let token = login(&app);
    let res = call(&app, authed(Request::new(Method::Get, "/api/does-not-exist"), &token));
    assert_eq!(res.status, 404);
}

#[test]
fn cors_headers_present() {
    let app = make_app();
    let res = call(&app, Request::new(Method::Get, "/api/health"));
    assert_eq!(header(&res, "access-control-allow-origin").as_deref(), Some("*"));
}
