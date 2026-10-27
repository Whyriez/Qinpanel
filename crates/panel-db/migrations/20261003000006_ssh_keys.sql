-- Nusa Panel Database Migration: 20261003000006_ssh_keys.sql

CREATE TABLE IF NOT EXISTS ssh_keys (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    public_key TEXT NOT NULL,
    key_type TEXT NOT NULL,
    fingerprint_sha256 TEXT NOT NULL,
    fingerprint_md5 TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(user_id, fingerprint_sha256)
);

CREATE INDEX IF NOT EXISTS idx_ssh_keys_user_id ON ssh_keys(user_id);
CREATE INDEX IF NOT EXISTS idx_ssh_keys_fingerprint ON ssh_keys(fingerprint_sha256);
