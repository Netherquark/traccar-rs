// SPDX-License-Identifier: Apache-2.0
//!
//! # traccar-core
//!
//! A portable, API-compatible reimplementation of the [Traccar](https://github.com/traccar/traccar)
//! GPS tracking server (Apache-2.0). The wire contract follows Traccar's own
//! OpenAPI 3.1 document; see `docs/openapi.yaml` and the `api_parity` test.
//!
//! The crate is transport-agnostic and has no Cloudflare dependencies, so it
//! compiles and tests natively. The `traccar-worker` crate adapts it to
//! Cloudflare Workers (D1 + Durable Objects) for the `wasm32` target.
//!
//! ## Layout
//! - [`model`] — wire models (Position, Device, User, …).
//! - [`protocol`] — ingestion decoders (OsmAnd / Traccar client).
//! - [`store`] + [`repo`] — storage abstraction and typed access + permissions.
//! - [`processing`] — distance, motion, geofence and alarm processing.
//! - [`reports`] — summary/trips/stops/route generation.
//! - [`api`] — HTTP router and resource handlers.
//! - [`auth`] — PBKDF2 password hashing and signed tokens.

pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod expression;
pub mod model;
pub mod processing;
pub mod protocol;
pub mod reports;
pub mod repo;
pub mod retention;
pub mod seed;
pub mod store;
pub mod time;

/// Advertised server version. The Traccar release this contract tracks,
/// suffixed with the implementation revision.
pub const VERSION: &str = "6.16.0-traccar-rs.0";

pub use config::Config;
pub use error::{Error, Result};
pub use model::Id;
pub use store::{Filter, MemoryStore, Store};
