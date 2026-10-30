-- Explicit opt-in only. Existing buckets, keys, identities and objects stay private.
CREATE TABLE storage_bucket_access (
    connection_id TEXT PRIMARY KEY REFERENCES storage_connections(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    visibility TEXT NOT NULL DEFAULT 'private' CHECK(visibility IN ('private','public')),
    desired_visibility TEXT NOT NULL DEFAULT 'private' CHECK(desired_visibility IN ('private','public')),
    state TEXT NOT NULL DEFAULT 'ready' CHECK(state IN ('ready','applying','unknown')),
    request_id TEXT,
    updated_at INTEGER NOT NULL DEFAULT 0
);
INSERT INTO storage_bucket_access(connection_id) SELECT id FROM storage_connections;
