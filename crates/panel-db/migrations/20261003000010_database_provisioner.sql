-- Nusa Panel Database Migration: 20261003000010_database_provisioner.sql
-- P8-T5: Database provisioner (MariaDB / PostgreSQL on-demand, user per tenant with prefix, plan limits, password rotation, remote access, import/export jobs).

-- 1. Database Servers Engine Status Cache Table
CREATE TABLE IF NOT EXISTS database_servers (
    engine TEXT PRIMARY KEY CHECK(engine IN ('mariadb', 'postgresql')),
    is_installed INTEGER NOT NULL DEFAULT 0 CHECK(is_installed IN (0, 1)),
    is_running INTEGER NOT NULL DEFAULT 0 CHECK(is_running IN (0, 1)),
    version TEXT,
    port INTEGER NOT NULL,
    socket_path TEXT,
    updated_at INTEGER NOT NULL
);

-- Seed initial records for both engines
INSERT OR IGNORE INTO database_servers (engine, is_installed, is_running, version, port, socket_path, updated_at)
VALUES 
    ('mariadb', 0, 0, NULL, 3306, '/run/mysqld/mysqld.sock', unixepoch()),
    ('postgresql', 0, 0, NULL, 5432, '/var/run/postgresql/.s.PGSQL.5432', unixepoch());

-- 2. Databases Table
CREATE TABLE IF NOT EXISTS databases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    site_id INTEGER REFERENCES sites(id) ON DELETE SET NULL,
    engine TEXT NOT NULL CHECK(engine IN ('mariadb', 'postgresql')),
    name TEXT NOT NULL UNIQUE,
    db_user TEXT NOT NULL,
    remote_access INTEGER NOT NULL DEFAULT 0 CHECK(remote_access IN (0, 1)),
    allowed_hosts TEXT NOT NULL DEFAULT 'localhost',
    collation TEXT NOT NULL DEFAULT 'utf8mb4_unicode_ci',
    character_set TEXT NOT NULL DEFAULT 'utf8mb4',
    size_bytes INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_databases_user_id ON databases(user_id);
CREATE INDEX IF NOT EXISTS idx_databases_site_id ON databases(site_id);
CREATE INDEX IF NOT EXISTS idx_databases_engine ON databases(engine);
CREATE INDEX IF NOT EXISTS idx_databases_name ON databases(name);
