-- Additive metadata only: TLS private keys remain in agent-owned files.
CREATE TABLE managed_storage_tls (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    request_id TEXT,
    job_id TEXT,
    actor_id INTEGER,
    hostname TEXT,
    email TEXT,
    zone_id TEXT,
    zone_name TEXT,
    credential_revision TEXT,
    state TEXT NOT NULL DEFAULT 'idle' CHECK (state IN ('idle','queued','issuing','configuring','active','failed','manual')),
    auto_renew INTEGER NOT NULL DEFAULT 0 CHECK (auto_renew IN (0,1)),
    not_before INTEGER,
    not_after INTEGER,
    renew_at INTEGER,
    last_attempt INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    error TEXT
);
INSERT INTO managed_storage_tls(id) VALUES (1);
