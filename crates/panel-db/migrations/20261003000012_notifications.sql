-- 20261003000012_notifications.sql
-- Schema for system notifications, in-app notification center, and alert configuration.

CREATE TABLE IF NOT EXISTS notifications (
    id TEXT PRIMARY KEY,
    user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    site_id INTEGER REFERENCES sites(id) ON DELETE CASCADE,
    category TEXT NOT NULL CHECK(category IN ('ssl_expiring', 'quota_warning', 'job_failed', 'deploy_failed', 'backup_failed', 'security', 'system', 'analytics_insight')),
    severity TEXT NOT NULL CHECK(severity IN ('info', 'warning', 'error', 'critical')),
    title TEXT NOT NULL,
    message TEXT NOT NULL,
    metadata_json TEXT,
    action_url TEXT,
    is_read INTEGER NOT NULL DEFAULT 0,
    read_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notifications_user_id ON notifications(user_id);
CREATE INDEX IF NOT EXISTS idx_notifications_site_id ON notifications(site_id);
CREATE INDEX IF NOT EXISTS idx_notifications_category ON notifications(category);
CREATE INDEX IF NOT EXISTS idx_notifications_is_read ON notifications(is_read);
CREATE INDEX IF NOT EXISTS idx_notifications_created_at ON notifications(created_at);

CREATE TABLE IF NOT EXISTS notification_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    smtp_enabled INTEGER NOT NULL DEFAULT 0,
    smtp_host TEXT,
    smtp_port INTEGER NOT NULL DEFAULT 587,
    smtp_username TEXT,
    smtp_password_enc TEXT,
    smtp_encryption TEXT NOT NULL DEFAULT 'starttls' CHECK(smtp_encryption IN ('none', 'starttls', 'tls')),
    from_address TEXT NOT NULL DEFAULT 'noreply@panel.local',
    from_name TEXT NOT NULL DEFAULT 'Nusa Panel',
    admin_email TEXT,
    ssl_warning_days INTEGER NOT NULL DEFAULT 30,
    quota_warning_percent INTEGER NOT NULL DEFAULT 90,
    notify_ssl_expiring INTEGER NOT NULL DEFAULT 1,
    notify_quota_warning INTEGER NOT NULL DEFAULT 1,
    notify_job_failed INTEGER NOT NULL DEFAULT 1,
    notify_deploy_failed INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO notification_settings (
    id,
    smtp_enabled,
    smtp_host,
    smtp_port,
    smtp_username,
    smtp_password_enc,
    smtp_encryption,
    from_address,
    from_name,
    admin_email,
    ssl_warning_days,
    quota_warning_percent,
    notify_ssl_expiring,
    notify_quota_warning,
    notify_job_failed,
    notify_deploy_failed,
    updated_at
) VALUES (
    1,
    0,
    '',
    587,
    '',
    '',
    'starttls',
    'noreply@panel.local',
    'Nusa Panel',
    '',
    30,
    90,
    1,
    1,
    1,
    1,
    1770000000
);
