-- Nusa Panel Database Migration: 20261003000003_plans.sql

-- Plans Table
CREATE TABLE IF NOT EXISTS plans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    -- Resource limits for systemd slice (cgroup v2)
    cpu_quota_percent INTEGER NOT NULL DEFAULT 100,
    memory_limit_bytes INTEGER NOT NULL DEFAULT 1073741824, -- 1 GiB
    memory_high_bytes INTEGER,
    tasks_max INTEGER NOT NULL DEFAULT 100,
    io_weight INTEGER NOT NULL DEFAULT 100,
    -- Storage limits (FS quota & scanner)
    disk_limit_bytes INTEGER NOT NULL DEFAULT 10737418240, -- 10 GiB
    inode_limit INTEGER NOT NULL DEFAULT 500000,
    -- Hosting feature quotas
    max_sites INTEGER NOT NULL DEFAULT 5,
    max_domains INTEGER NOT NULL DEFAULT 10,
    max_databases INTEGER NOT NULL DEFAULT 5,
    max_cron_jobs INTEGER NOT NULL DEFAULT 10,
    max_terminal_sessions INTEGER NOT NULL DEFAULT 2,
    max_upload_bytes INTEGER NOT NULL DEFAULT 104857600, -- 100 MiB
    shell_policy TEXT NOT NULL DEFAULT 'none' CHECK(shell_policy IN ('none', 'sftp', 'full')),
    allow_external_upstream INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_plans_name ON plans(name);

-- Insert Default Baseline Plans
INSERT OR IGNORE INTO plans (
    id, name, description,
    cpu_quota_percent, memory_limit_bytes, memory_high_bytes, tasks_max, io_weight,
    disk_limit_bytes, inode_limit,
    max_sites, max_domains, max_databases, max_cron_jobs, max_terminal_sessions,
    max_upload_bytes, shell_policy, allow_external_upstream,
    created_at, updated_at
) VALUES 
(
    1, 'Starter', 'Starter plan for small personal sites and blogs',
    100, 536870912, 429496729, 50, 100,
    5368709120, 250000,
    1, 2, 1, 3, 1,
    52428800, 'none', 0,
    1770000000, 1770000000
),
(
    2, 'Pro', 'Professional plan for commercial production applications',
    200, 2147483648, 1717986918, 200, 200,
    21474836480, 1000000,
    10, 20, 10, 20, 3,
    209715200, 'sftp', 1,
    1770000000, 1770000000
);
