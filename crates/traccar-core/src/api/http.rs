// SPDX-License-Identifier: Apache-2.0
//
// Transport-agnostic HTTP types. The Cloudflare worker adapter converts
// `worker::Request`/`worker::Response` to and from these.

use crate::error::{Error, ProblemDetails};
use base64::Engine;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Options,
    Head,
}

impl Method {
    pub fn parse(s: &str) -> Option<Method> {
        Some(match s.to_ascii_uppercase().as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "DELETE" => Method::Delete,
            "PATCH" => Method::Patch,
            "OPTIONS" => Method::Options,
            "HEAD" => Method::Head,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Patch => "PATCH",
            Method::Options => "OPTIONS",
            Method::Head => "HEAD",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub remote: Option<String>,
}

impl Request {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Request {
            method,
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            body: Vec::new(),
            remote: None,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn query_first(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn query_all(&self, key: &str) -> Vec<&str> {
        self.query.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
    }

    pub fn query_i64(&self, key: &str) -> Option<i64> {
        self.query_first(key).and_then(|v| v.parse().ok())
    }

    pub fn query_bool(&self, key: &str) -> bool {
        matches!(self.query_first(key), Some("true") | Some("1"))
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Parse the body as JSON. Traccar tolerates empty bodies on some PUTs, so
    /// callers may treat an empty body as "no change".
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        if self.body.is_empty() {
            return Err(Error::bad_request("empty body"));
        }
        serde_json::from_slice(&self.body).map_err(|e| Error::bad_request(format!("invalid JSON: {e}")))
    }

    /// Parse an `application/x-www-form-urlencoded` body.
    pub fn form(&self) -> Vec<(String, String)> {
        crate::protocol::parse_query(&String::from_utf8_lossy(&self.body))
    }

    pub fn form_first(&self, key: &str) -> Option<String> {
        self.form().into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Decode HTTP Basic credentials as `(email, password)`.
    pub fn basic_credentials(&self) -> Option<(String, String)> {
        let auth = self.header("authorization")?;
        let encoded = auth.strip_prefix("Basic ").or_else(|| auth.strip_prefix("basic "))?;
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
        let text = String::from_utf8(decoded).ok()?;
        let (user, pass) = text.split_once(':')?;
        Some((user.to_string(), pass.to_string()))
    }

    pub fn bearer_token(&self) -> Option<String> {
        let auth = self.header("authorization")?;
        let t = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("bearer "))?;
        Some(t.trim().to_string())
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Response { status, headers: Vec::new(), body: Vec::new() }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Response {
            status,
            headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
            body: body.into().into_bytes(),
        }
    }

    pub fn json<T: Serialize>(status: u16, value: &T) -> Self {
        let body = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
        Response {
            status,
            headers: vec![("content-type".into(), "application/json; charset=utf-8".into())],
            body,
        }
    }

    pub fn json_value(status: u16, value: serde_json::Value) -> Self {
        let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"null".to_vec());
        Response {
            status,
            headers: vec![("content-type".into(), "application/json; charset=utf-8".into())],
            body,
        }
    }

    pub fn no_content() -> Self {
        Response::new(204)
    }

    pub fn ok() -> Self {
        Response::new(200)
    }

    pub fn problem(e: &Error) -> Self {
        let pd = ProblemDetails::from(e);
        Response::json(e.status, &pd)
    }

    pub fn header(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.to_string(), v.into()));
        self
    }

    pub fn content_type(mut self, ct: &str) -> Self {
        self.headers.push(("content-type".into(), ct.to_string()));
        self
    }
}
