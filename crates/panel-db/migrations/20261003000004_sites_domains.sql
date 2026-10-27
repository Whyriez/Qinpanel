-- Nusa Panel Database Migration: 20261003000004_sites_domains.sql

-- 1. Sites Table
CREATE TABLE IF NOT EXISTS sites (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('static', 'php', 'nodejs', 'python', 'proxy')),
    docroot TEXT NOT NULL,
    runtime TEXT,
    version TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'disabled', 'suspended')),
    settings_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sites_user_id ON sites(user_id);
CREATE INDEX IF NOT EXISTS idx_sites_kind ON sites(kind);
CREATE INDEX IF NOT EXISTS idx_sites_status ON sites(status);

-- 2. Domains Table
CREATE TABLE IF NOT EXISTS domains (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    name TEXT NOT NULL UNIQUE,
    unicode_name TEXT NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK(is_primary IN (0, 1)),
    redirect_mode TEXT NOT NULL DEFAULT 'none' CHECK(redirect_mode IN ('none', 'to_primary', 'to_www', 'to_non_www', 'custom')),
    redirect_target TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'pending_ssl', 'disabled')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_domains_name ON domains(name);
CREATE INDEX IF NOT EXISTS idx_domains_site_id ON domains(site_id);
CREATE INDEX IF NOT EXISTS idx_domains_is_primary ON domains(is_primary);
CREATE INDEX IF NOT EXISTS idx_domains_status ON domains(status);
