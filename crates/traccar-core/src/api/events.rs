// SPDX-License-Identifier: Apache-2.0
//
// Event resource (`/api/events/{id}`).

use super::http::Response;
use super::App;
use crate::auth::Principal;
use crate::error::{Error, Result};
use crate::model::{kind, Event, Id};

pub async fn get(app: &App, p: &Principal, id: Id) -> Result<Response> {
    if !p.authenticated() {
        return Err(Error::unauthorized());
    }
    let repo = app.repo();
    let event: Event = repo.get(kind::EVENT, id).await?.ok_or_else(Error::not_found)?;
    repo.require_device(p, event.device_id).await?;
    Ok(Response::json(200, &event))
}
