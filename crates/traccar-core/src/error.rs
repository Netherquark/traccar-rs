// SPDX-License-Identifier: Apache-2.0
//
// Error model. Mirrors the Traccar HTTP error semantics closely enough for
// clients: 400 bad request, 401 unauthorized, 403 forbidden, 404 not found,
// 409 conflict, 429 rate limited, 500 internal.

use serde::Serialize;

/// Library-wide result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Typed error carrying an HTTP status and optional message.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{status}: {message}")]
pub struct Error {
    pub status: u16,
    pub message: String,
}

impl Error {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(400, msg)
    }
    pub fn unauthorized() -> Self {
        Self::new(401, "Unauthorized")
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(403, msg)
    }
    pub fn not_found() -> Self {
        Self::new(404, "Not found")
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(409, msg)
    }
    pub fn too_many() -> Self {
        Self::new(429, "Too many requests")
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(500, msg)
    }
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::new(501, msg)
    }

    pub fn is_auth_failure(&self) -> bool {
        self.status == 401
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::bad_request(format!("invalid JSON: {e}"))
    }
}

/// RFC 7807 Problem Details body used for error responses where a JSON body
/// is appropriate. Traccar itself returns mostly empty bodies, but clients
/// tolerate a structured body, and it is far more debuggable.
#[derive(Debug, Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl From<&Error> for ProblemDetails {
    fn from(e: &Error) -> Self {
        ProblemDetails {
            type_: "about:blank".to_string(),
            title: e.message.clone(),
            status: e.status,
            detail: None,
        }
    }
}
