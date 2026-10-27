-- Nusa Panel Database Migration: 20261003000008_cron_jobs.sql

-- 1. Cron Jobs Table
CREATE TABLE IF NOT EXISTS cron_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    command TEXT NOT NULL,
    schedule TEXT NOT NULL,
    systemd_calendar TEXT NOT NULL,
    description TEXT,
    work_dir TEXT,
    timeout_secs INTEGER NOT NULL DEFAULT 3600,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0, 1)),
    status TEXT NOT NULL DEFAULT 'idle' CHECK(status IN ('idle', 'running', 'success', 'failed')),
    last_run_at INTEGER,
    last_duration_ms INTEGER,
    last_exit_code INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cron_jobs_site_id ON cron_jobs(site_id);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_enabled ON cron_jobs(enabled);

-- 2. Cron Runs Execution History Table
CREATE TABLE IF NOT EXISTS cron_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_job_id INTEGER NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK(status IN ('running', 'success', 'failed')),
    exit_code INTEGER,
    stdout TEXT,
    stderr TEXT,
    duration_ms INTEGER,
    started_at INTEGER NOT NULL,
    finished_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_cron_runs_cron_job_id ON cron_runs(cron_job_id);
CREATE INDEX IF NOT EXISTS idx_cron_runs_started_at ON cron_runs(started_at DESC);
