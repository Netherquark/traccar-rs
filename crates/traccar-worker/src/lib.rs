// SPDX-License-Identifier: Apache-2.0
//!
//! Cloudflare Workers adapter for `traccar-core`.
//!
//! Provides a D1-backed [`Store`] and a `fetch` entry point that translates
//! `worker::Request`/`worker::Response` to and from the portable core types.
//! The document model lives in one table (`objects`) plus a counter table, so
//! the schema is small and the object contract is identical to the native core.

mod d1;

use traccar_core::api::http::{Method, Request as CoreRequest, Response as CoreResponse};
use traccar_core::api::App;
use traccar_core::config::Config;
use traccar_core::store::Store;
use worker::*;

use std::sync::Arc;

/// Build the application from the Worker environment.
async fn build_app(env: &Env) -> Result<App> {
    let db = env.d1("DB")?;
    let store: Arc<dyn Store> = Arc::new(d1::D1Store::new(db));
    let mut config = Config::default();
    // Secret comes from `wrangler secret put TOKEN_SECRET`.
    if let Ok(secret) = env.var("TOKEN_SECRET") {
        config.token_secret = secret.to_string().into_bytes();
    }
    if let Ok(base) = env.var("BASE_URL") {
        config.base_url = base.to_string();
    }
    if let Ok(reg) = env.var("REGISTRATION") {
        config.registration = reg.to_string() == "true";
    }
    if let Ok(rd) = env.var("RETENTION_DAYS") {
        config.retention_days = rd.to_string().parse().ok();
    }
    Ok(App::new(store, config))
}

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let app = build_app(&env).await?;
    let core_req = to_core_request(req).await?;
    let core_res = app.handle(core_req).await;
    to_worker_response(core_res)
}

async fn to_core_request(req: Request) -> Result<CoreRequest> {
    let method = Method::parse(req.method().as_ref())
        .ok_or_else(|| Error::RustError("unsupported method".into()))?;
    let url = req.url()?;
    let path = url.path().to_string();
    let query: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let mut headers: Vec<(String, String)> = Vec::new();
    for (k, v) in req.headers().entries() {
        headers.push((k, v));
    }

    // Parse the Cookie header into individual cookies.
    let mut cookies = Vec::new();
    if let Some(cookie) = req.headers().get("Cookie")? {
        for part in cookie.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                cookies.push((k.to_string(), v.to_string()));
            }
        }
    }

    let body = req.bytes().await.unwrap_or_default();
    let remote = req.headers().get("CF-Connecting-IP").ok().flatten();

    let mut out = CoreRequest::new(method, path);
    out.query = query;
    out.headers = headers;
    out.cookies = cookies;
    out.body = body;
    out.remote = remote;
    Ok(out)
}

fn to_worker_response(res: CoreResponse) -> Result<Response> {
    let mut builder = ResponseBuilder::new().with_status(res.status);
    for (k, v) in &res.headers {
        builder = builder.with_header(k, v)?;
    }
    builder.from_bytes(res.body)
}

/// Scheduled maintenance: enforce the retention window if one is configured.
/// Failure-safety: an unset retention policy never deletes anything.
#[event(scheduled)]
pub async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let Ok(app) = build_app(&env).await else { return };
    let Some(days) = app.config.retention_days else { return };
    let cutoff = traccar_core::time::Timestamp::now().millis() - days * 86_400_000;
    let _ = traccar_core::retention::enforce(app.store.as_ref(), cutoff).await;
}
