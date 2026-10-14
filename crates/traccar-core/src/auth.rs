// SPDX-License-Identifier: Apache-2.0
//
// Credential handling: PBKDF2 password hashing (byte-compatible with Traccar's
// `Hashing` helper) and HMAC-signed opaque tokens (functionally equivalent to
// Traccar's RSA-signed tokens; the token is opaque to clients, so the signing
// primitive is an internal choice).

use crate::error::{Error, Result};
use crate::model::User;
use crate::time::Timestamp;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const PBKDF2_SHA256: &str = "PBKDF2WithHmacSHA256";
const PBKDF2_SHA256_ITERATIONS: u32 = 200_000;
const PBKDF2_SHA1: &str = "PBKDF2WithHmacSHA1";
const PBKDF2_SHA1_ITERATIONS: u32 = 1_000;
const SALT_SIZE: usize = 24;
const HASH_SIZE: usize = 24;

const TOKEN_DEFAULT_DAYS: i64 = 7;

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

/// Result of hashing a password: the encoded hash and the hex salt.
pub struct Hashed {
    pub hash: String,
    pub salt: String,
}

/// Create a new PBKDF2-HMAC-SHA256 hash exactly like Traccar does.
pub fn create_hash(password: &str) -> Result<Hashed> {
    let mut salt = [0u8; SALT_SIZE];
    getrandom::getrandom(&mut salt).map_err(|e| Error::internal(format!("rng: {e}")))?;
    let mut out = [0u8; HASH_SIZE];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_SHA256_ITERATIONS, &mut out);
    Ok(Hashed {
        hash: format!("{PBKDF2_SHA256}${PBKDF2_SHA256_ITERATIONS}${}", hex::encode(out)),
        salt: hex::encode(salt),
    })
}

/// Verify a password against a stored hash/salt. Supports both the current
/// PBKDF2-HMAC-SHA256 format and the legacy PBKDF2-HMAC-SHA1 format.
pub fn validate_password(password: &str, hashed: &str, salt_hex: &str) -> bool {
    let salt = match hex::decode(salt_hex) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let parts: Vec<&str> = hashed.split('$').collect();
    let (algo, iterations, hash_hex) = if parts.len() > 1 {
        (parts[0], parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(PBKDF2_SHA256_ITERATIONS), parts.get(2).copied().unwrap_or(""))
    } else {
        (PBKDF2_SHA1, PBKDF2_SHA1_ITERATIONS, hashed)
    };
    let expected = match hex::decode(hash_hex) {
        Ok(h) => h,
        Err(_) => return false,
    };
    let mut out = [0u8; HASH_SIZE];
    match algo {
        PBKDF2_SHA256 => {
            pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations, &mut out);
        }
        PBKDF2_SHA1 => {
            // PBKDF2 output length is fixed at HASH_SIZE (24 bytes).
            pbkdf2_hmac::<Sha1>(password.as_bytes(), &salt, iterations, &mut out);
        }
        _ => return false,
    }
    constant_time_eq(&out, &expected)
}

/// Length-constant-time byte comparison.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().min(b.len()) {
        diff |= (a[i] ^ b[i]) as usize;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// Signed tokens
// ---------------------------------------------------------------------------

/// Token payload. Field names follow Traccar's `TokenManager.TokenData`.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TokenData {
    /// Unique token id (used for revocation).
    pub i: i64,
    /// Owning user id.
    pub u: i64,
    /// Expiration, epoch millis.
    pub e: i64,
    /// Optional scope tag, e.g. `share:<deviceId>` or `session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s: Option<String>,
}

impl TokenData {
    pub fn expiration(&self) -> Timestamp {
        Timestamp::from_millis(self.e)
    }
    pub fn expired(&self) -> bool {
        self.e < Utc::now().timestamp_millis()
    }
}

/// Signs and verifies opaque tokens.
pub struct TokenManager {
    secret: Vec<u8>,
}

impl TokenManager {
    pub fn new(secret: impl AsRef<[u8]>) -> Self {
        TokenManager { secret: secret.as_ref().to_vec() }
    }

    fn new_id() -> i64 {
        let mut b = [0u8; 8];
        let _ = getrandom::getrandom(&mut b);
        (u64::from_le_bytes(b) & i64::MAX as u64) as i64
    }

    fn sign(&self, payload: &[u8]) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("hmac key");
        mac.update(payload);
        mac.finalize().into_bytes().to_vec()
    }

    /// Generate a token for `user_id`, defaulting to 7-day expiry.
    pub fn generate(&self, user_id: i64, scope: Option<&str>) -> Result<String> {
        self.generate_with_expiry(user_id, scope, None)
    }

    pub fn generate_with_expiry(
        &self,
        user_id: i64,
        scope: Option<&str>,
        expiration: Option<Timestamp>,
    ) -> Result<String> {
        let exp = expiration
            .map(|t| t.millis())
            .unwrap_or_else(|| Utc::now().timestamp_millis() + TOKEN_DEFAULT_DAYS * 86_400_000);
        let data = TokenData { i: Self::new_id(), u: user_id, e: exp, s: scope.map(|s| s.to_string()) };
        let payload = serde_json::to_vec(&data).map_err(|e| Error::internal(e.to_string()))?;
        let sig = self.sign(&payload);
        let engine = b64();
        Ok(format!("{}.{}", engine.encode(payload), engine.encode(sig)))
    }

    /// Decode and verify a token's signature. Does not check expiry/revocation.
    pub fn decode(&self, token: &str) -> Result<TokenData> {
        let (p, s) = token.split_once('.').ok_or_else(Error::unauthorized)?;
        let engine = b64();
        let payload = engine.decode(p).map_err(|_| Error::unauthorized())?;
        let sig = engine.decode(s).map_err(|_| Error::unauthorized())?;
        if !constant_time_eq(&self.sign(&payload), &sig) {
            return Err(Error::unauthorized());
        }
        serde_json::from_slice(&payload).map_err(|_| Error::unauthorized())
    }
}

// ---------------------------------------------------------------------------
// Principal
// ---------------------------------------------------------------------------

/// How the caller authenticated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthKind {
    /// HTTP Basic (`email:password`).
    Basic,
    /// Bearer API/session token.
    Token,
    /// Session cookie.
    Session,
    /// Anonymous (only allowed on `@PermitAll` endpoints).
    Anonymous,
}

/// The authenticated caller.
#[derive(Clone, Debug)]
pub struct Principal {
    pub user: User,
    pub kind: AuthKind,
    /// Expiration carried by the token, if any (caps issued tokens).
    pub expiration: Option<Timestamp>,
}

impl Principal {
    pub fn anonymous() -> Self {
        Principal { user: User::default(), kind: AuthKind::Anonymous, expiration: None }
    }
    pub fn is_admin(&self) -> bool {
        self.user.administrator
    }
    pub fn id(&self) -> i64 {
        self.user.id
    }
    pub fn authenticated(&self) -> bool {
        self.kind != AuthKind::Anonymous
    }
}
