-- Nusa Panel Database Migration: 20261003000002_recovery_codes.sql

CREATE TABLE IF NOT EXISTS user_recovery_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_user_recovery_codes_user_id ON user_recovery_codes(user_id);
CREATE INDEX IF NOT EXISTS idx_user_recovery_codes_code_hash ON user_recovery_codes(code_hash);
