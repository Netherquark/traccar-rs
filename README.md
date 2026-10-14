# traccar-rs

An API-compatible reimplementation of the [Traccar](https://github.com/traccar/traccar) GPS tracking server (Apache 2.0), written in Rust. The portable `traccar-core` crate runs natively and on Cloudflare Workers (via `traccar-worker`).

- API contract matches Traccar 6.16.0's OpenAPI (`docs/openapi.yaml`). The `api_parity` test enforces this.
- Ingestion: OsmAnd protocol decoder (GET/POST/query + JSON). Unknown devices behave as Traccar does (400/404, registration gated by config).
- Auth: PBKDF2-HMAC-SHA256 (200000 iterations) matching Traccar; legacy PBKDF2-HMAC-SHA1 supported. HMAC-signed opaque tokens (replaces Traccar's RSA tokens, opaque to clients).
- Storage: document store (`kind,id -> JSON`). `MemoryStore` for tests; D1-backed implementation in `traccar-worker`.
- Processing: distance (haversine), motion detection, geofence (circle/polygon), event generation.
- Reports: summary/trips/stops/route/geofences/combined.

## Tests

```sh
cargo test -p traccar-core
```

## Contracts

- `docs/openapi.yaml` — canonical Traccar contract (Apache-2.0).
- `docs/TRACCAR-LICENSE.txt` — upstream license notice.

## Deploy (Cloudflare)

- `crates/traccar-worker/wrangler.toml` for basic config. Set `TOKEN_SECRET` via `wrangler secret put`.
- D1 schema in `migrations/0001_init.sql`.
- Scheduled job `0 3 * * *` enforces retention (only positions/events; unset retention disables deletion).
