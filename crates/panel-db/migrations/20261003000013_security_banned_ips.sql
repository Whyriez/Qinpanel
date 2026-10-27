-- 20261003000013_security_banned_ips.sql
-- Table for temporary and permanent IP bans with TTL per site.

CREATE TABLE IF NOT EXISTS site_banned_ips (
    id TEXT PRIMARY KEY,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    ip_address TEXT NOT NULL,
    reason TEXT NOT NULL,
    banned_at INTEGER NOT NULL,
    expires_at INTEGER,
    banned_by TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_banned_ips_site_active ON site_banned_ips(site_id, is_active);
CREATE INDEX IF NOT EXISTS idx_banned_ips_ip ON site_banned_ips(ip_address);
CREATE INDEX IF NOT EXISTS idx_banned_ips_expires ON site_banned_ips(expires_at);
