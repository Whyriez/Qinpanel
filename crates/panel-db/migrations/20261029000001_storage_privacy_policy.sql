-- Additive allocation and owner policy metadata. Provider objects are not changed.
CREATE TABLE storage_policies (
    connection_id TEXT PRIMARY KEY REFERENCES storage_connections(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    quota_bytes INTEGER CHECK(quota_bytes IS NULL OR quota_bytes > 0),
    used_bytes INTEGER CHECK(used_bytes IS NULL OR used_bytes >= 0),
    object_count INTEGER CHECK(object_count IS NULL OR object_count >= 0),
    measured_at INTEGER,
    rules_json TEXT NOT NULL DEFAULT '{"read":true,"write":true,"delete":true,"max_file_bytes":33554432,"extensions":[]}'
);
INSERT INTO storage_policies(connection_id) SELECT id FROM storage_connections;
