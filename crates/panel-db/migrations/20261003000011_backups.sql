-- 20261003000011_backups.sql
-- Schema for site backups, snapshots, and automatic backup schedules.

CREATE TABLE IF NOT EXISTS site_backups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    filepath TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    label TEXT,
    includes_files INTEGER NOT NULL DEFAULT 1,
    includes_databases INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'completed',
    error_message TEXT,
    storage_type TEXT NOT NULL DEFAULT 'local',
    s3_key TEXT,
    checksum_sha256 TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_site_backups_site_id ON site_backups(site_id);
CREATE INDEX IF NOT EXISTS idx_site_backups_user_id ON site_backups(user_id);
CREATE INDEX IF NOT EXISTS idx_site_backups_created_at ON site_backups(created_at);

CREATE TABLE IF NOT EXISTS site_backup_schedules (
    site_id INTEGER PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    is_enabled INTEGER NOT NULL DEFAULT 0,
    frequency TEXT NOT NULL DEFAULT 'daily',
    time_hour INTEGER NOT NULL DEFAULT 2,
    time_minute INTEGER NOT NULL DEFAULT 0,
    retention_count INTEGER NOT NULL DEFAULT 7,
    include_files INTEGER NOT NULL DEFAULT 1,
    include_databases INTEGER NOT NULL DEFAULT 1,
    last_run_at INTEGER,
    next_run_at INTEGER,
    storage_destination TEXT NOT NULL DEFAULT 'local',
    s3_endpoint TEXT,
    s3_bucket TEXT,
    s3_region TEXT,
    s3_access_key TEXT,
    s3_secret_key_enc TEXT,
    s3_prefix TEXT,
    updated_at INTEGER NOT NULL
);
