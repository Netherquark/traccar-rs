-- SPDX-License-Identifier: Apache-2.0
--
-- traccar-rs — initial D1 schema.
--
-- The portable core treats persistence as a document store keyed by
-- (kind, id). This keeps the schema tiny and stable while the object contract
-- (the wire models) evolves independently of physical tables.

CREATE TABLE IF NOT EXISTS counters (
    kind  TEXT PRIMARY KEY,
    value INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS objects (
    kind  TEXT    NOT NULL,
    id    INTEGER NOT NULL,
    data  TEXT    NOT NULL,
    PRIMARY KEY (kind, id)
);

-- Common access paths: list a kind, and position/event history by device+time.
CREATE INDEX IF NOT EXISTS idx_objects_kind ON objects (kind);
CREATE INDEX IF NOT EXISTS idx_positions_device_time
    ON objects (kind, json_extract(data, '$.deviceId'), json_extract(data, '$.fixTime'));
CREATE INDEX IF NOT EXISTS idx_events_device_time
    ON objects (kind, json_extract(data, '$.deviceId'), json_extract(data, '$.eventTime'));
CREATE INDEX IF NOT EXISTS idx_users_email
    ON objects (kind, json_extract(data, '$.email'));
CREATE INDEX IF NOT EXISTS idx_devices_unique_id
    ON objects (kind, json_extract(data, '$.uniqueId'));
