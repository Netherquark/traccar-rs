// SPDX-License-Identifier: Apache-2.0
//
// Session resource (`/api/session`). Mirrors Traccar: form login, cookie-backed
// session, opaque bearer tokens, and token revocation.

use super::http::{Request, Response};
use super::App;
use crate::auth::Principal;
use crate::error::{Error, Result};
use crate::model::{kind, Id};
use crate::time::{parse_iso, Timestamp};

pub async fn login(app: &App, req: &Request) -> Result<Response> {
    let email = req.form_first("email").ok_or_else(|| Error::bad_request("missing email"))?;
    let password = req.form_first("password").ok_or_else(|| Error::bad_request("missing password"))?;

    let repo = app.repo();
    let user = repo.user_by_email(&email).await?;
    let user = match user {
        Some(u) if !u.disabled => {
            let (h, s) = (u.hashed_password.clone().unwrap_or_default(), u.salt.clone().unwrap_or_default());
            if crate::auth::validate_password(&password, &h, &s) {
                u
            } else {
                return Ok(Response::new(401));
            }
        }
        _ => return Ok(Response::new(401)),
    };

    let token = app.tokens.generate(user.id, Some("session"))?;
    app.record_audit("login", "user", user.id, &Principal { user: user.clone(), kind: crate::auth::AuthKind::Basic, expiration: None }, req.remote.as_deref())
        .await;
    let cookie = format!("JSESSIONID={token}; Path=/; HttpOnly; SameSite=Lax");
    Ok(Response::json(200, &user.to_public()).header("Set-Cookie", cookie))
}

pub async fn current(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    if let Some(token) = req.query_first("token") {
        let data = app.tokens.decode(token)?;
        if data.expired() {
            return Ok(Response::new(401));
        }
        let user = app.repo().user(data.u).await?.ok_or_else(Error::unauthorized)?;
        return Ok(Response::json(200, &user.to_public()));
    }
    if p.authenticated() {
        return Ok(Response::json(200, &p.user.to_public()));
    }
    Ok(Response::new(404))
}

pub async fn get_by_id(app: &App, p: &Principal, id: Id) -> Result<Response> {
    app.repo().require_manage_user(p, id).await?;
    let user = app.repo().user(id).await?.ok_or_else(Error::not_found)?;
    Ok(Response::json(200, &user.to_public()))
}

pub async fn logout(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    if p.authenticated() {
        app.record_audit("logout", "user", p.id(), p, req.remote.as_deref()).await;
    }
    Ok(Response::no_content().header("Set-Cookie", "JSESSIONID=; Path=/; Max-Age=0"))
}

pub async fn issue_token(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    if !p.authenticated() {
        return Err(Error::unauthorized());
    }
    let expiration = req
        .form_first("expiration")
        .and_then(|s| parse_iso(&s))
        .map(Timestamp);
    let token = app.tokens.generate_with_expiry(p.id(), Some("api"), expiration)?;
    app.record_audit("token", "user", p.id(), p, req.remote.as_deref()).await;
    Ok(Response::text(200, token))
}

pub async fn revoke_token(app: &App, req: &Request, p: &Principal) -> Result<Response> {
    let token = req.form_first("token").ok_or_else(|| Error::bad_request("missing token"))?;
    let data = app.tokens.decode(&token)?;
    let mut row = serde_json::Map::new();
    row.insert("id".into(), serde_json::Value::from(data.i));
    row.insert("userId".into(), serde_json::Value::from(data.u));
    app.store
        .insert(kind::REVOKED_TOKEN, serde_json::Value::Object(row))
        .await?;
    app.record_audit("revoke", "token", data.i, p, req.remote.as_deref()).await;
    Ok(Response::no_content())
}
