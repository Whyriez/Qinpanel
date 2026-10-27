-- Persist create retries independently of endpoint lifetime. Deleting an endpoint
-- leaves a tombstone so a delayed create retry cannot recreate a deleted address.
CREATE TABLE IF NOT EXISTS connectivity_create_requests (
    actor_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    request_key TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    endpoint_id TEXT NOT NULL,
    PRIMARY KEY(actor_id, request_key)
);
