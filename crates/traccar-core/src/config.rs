// SPDX-License-Identifier: Apache-2.0
//
// Runtime configuration. Defaults mirror Traccar's `traccar.xml` defaults so a
// client configured against a stock Traccar behaves the same here.

use crate::model::Id;

#[derive(Clone, Debug)]
pub struct Config {
    /// HMAC secret for signing session/share tokens. Must be set in production.
    pub token_secret: Vec<u8>,
    /// Advertised contract/protocol version.
    pub contract_version: String,
    /// Device considered offline after this many seconds without an update.
    pub device_timeout_secs: i64,
    /// Auto-create devices for previously unseen unique ids (Traccar
    /// `database.registerUnknown`).
    pub register_unknown: bool,
    pub register_unknown_category: Option<String>,
    pub register_unknown_group_id: Id,
    /// Drop positions whose accuracy is worse than this (OsmAnd only; metres).
    pub osmand_min_accuracy: f64,
    /// Whether self-service registration is allowed.
    pub registration: bool,
    /// Retention window in days for position history. `None` disables deletion
    /// (fail-safe: an unset retention policy never destroys data).
    pub retention_days: Option<i64>,
    /// Base URL used when building share links.
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            token_secret: b"traccar-rs-development-secret-change-me".to_vec(),
            contract_version: "v4".into(),
            device_timeout_secs: 600,
            register_unknown: true,
            register_unknown_category: Some("default".into()),
            register_unknown_group_id: 0,
            osmand_min_accuracy: 0.0,
            registration: false,
            retention_days: None,
            base_url: "http://localhost".into(),
        }
    }
}

impl Config {
    pub fn with_secret(mut self, secret: impl Into<Vec<u8>>) -> Self {
        self.token_secret = secret.into();
        self
    }
}
