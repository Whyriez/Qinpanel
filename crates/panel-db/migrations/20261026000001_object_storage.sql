-- Metadata/vault only. Removing a tenant/project never removes provider objects.
CREATE TABLE storage_connections (
    id TEXT PRIMARY KEY NOT NULL,
    owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    site_id INTEGER REFERENCES sites(id) ON DELETE SET NULL,
    label TEXT NOT NULL CHECK(length(label) BETWEEN 1 AND 80),
    provider TEXT NOT NULL CHECK(provider IN ('aws','r2','supabase','compatible')),
    endpoint TEXT NOT NULL,
    region TEXT NOT NULL,
    addressing TEXT NOT NULL CHECK(addressing IN ('path','virtual_host')),
    bucket TEXT NOT NULL,
    credentials_enc TEXT NOT NULL CHECK(credentials_enc LIKE 'enc:v1:%'),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0),
    verified_revision INTEGER,
    verified_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(endpoint,bucket)
);
CREATE INDEX storage_connections_owner ON storage_connections(owner_id,site_id);
CREATE TABLE storage_operations (
    id TEXT PRIMARY KEY NOT NULL,
    connection_id TEXT NOT NULL REFERENCES storage_connections(id) ON DELETE CASCADE,
    actor_id INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('upload','delete','create_bucket')),
    state TEXT NOT NULL CHECK(state IN ('running','succeeded','failed','unknown')),
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);
CREATE INDEX storage_operations_connection ON storage_operations(connection_id,created_at);
