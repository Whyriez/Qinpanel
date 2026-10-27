-- Nusa Panel Database Migration: 20261003000009_git_deploy.sql
-- P8-T4: Git deploy with deploy key, releases structure, atomic symlink swap, rollback, and webhooks.

-- 1. Site Git Configurations Table
CREATE TABLE IF NOT EXISTS site_git_configs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id INTEGER NOT NULL UNIQUE REFERENCES sites(id) ON DELETE CASCADE,
    repo_url TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT 'main',
    deploy_key_pub TEXT,
    deploy_key_priv_path TEXT,
    build_command TEXT,
    post_deploy_hook TEXT,
    docroot_subpath TEXT NOT NULL DEFAULT '',
    releases_to_keep INTEGER NOT NULL DEFAULT 5,
    webhook_secret TEXT,
    auto_deploy_enabled INTEGER NOT NULL DEFAULT 0 CHECK(auto_deploy_enabled IN (0, 1)),
    current_release_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_site_git_configs_site_id ON site_git_configs(site_id);

-- 2. Site Deployments Table (Execution history, logs, and analytics markers)
CREATE TABLE IF NOT EXISTS site_deployments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    release_id TEXT NOT NULL,
    commit_hash TEXT,
    commit_message TEXT,
    commit_author TEXT,
    branch TEXT NOT NULL,
    trigger_type TEXT NOT NULL CHECK(trigger_type IN ('manual', 'webhook_github', 'webhook_gitlab', 'rollback')),
    status TEXT NOT NULL CHECK(status IN ('pending', 'running', 'success', 'failed', 'rolled_back')),
    log_output TEXT,
    duration_ms INTEGER,
    created_at INTEGER NOT NULL,
    finished_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_site_deployments_site_id ON site_deployments(site_id);
CREATE INDEX IF NOT EXISTS idx_site_deployments_created_at ON site_deployments(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_site_deployments_status ON site_deployments(status);
