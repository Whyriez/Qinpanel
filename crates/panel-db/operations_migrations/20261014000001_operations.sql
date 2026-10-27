-- No cross-file foreign keys: ownership is checked against the core database by
-- the API and again immediately before executing typed tenant jobs.
CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    user_id INTEGER,
    name TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    progress INTEGER NOT NULL DEFAULT 0 CHECK(progress >= 0 AND progress <= 100),
    payload_json TEXT NOT NULL,
    result_json TEXT,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX idx_jobs_user_id ON jobs(user_id);
CREATE INDEX idx_jobs_status_created ON jobs(status, created_at, id);
CREATE INDEX idx_jobs_created_at ON jobs(created_at);

CREATE TABLE audit_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER,
    actor_username TEXT NOT NULL,
    action TEXT NOT NULL,
    target_resource TEXT NOT NULL,
    details_json TEXT,
    ip_address TEXT,
    status TEXT NOT NULL CHECK(status IN ('success', 'failure')),
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_audit_logs_user_id ON audit_logs(user_id);
CREATE INDEX idx_audit_logs_action ON audit_logs(action);
CREATE INDEX idx_audit_logs_created_at ON audit_logs(created_at);

CREATE TABLE legacy_import (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    source_path TEXT NOT NULL
);
