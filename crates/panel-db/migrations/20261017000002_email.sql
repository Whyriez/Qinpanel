-- Mailbox credentials are hashes; SMTP secrets remain in notification_settings as ciphertext.
CREATE TABLE email_mailboxes (
 id TEXT PRIMARY KEY,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 domain_id INTEGER NOT NULL REFERENCES domains(id),
 address TEXT NOT NULL UNIQUE COLLATE NOCASE,
 password_hash TEXT NOT NULL,
 quota_mb INTEGER NOT NULL CHECK(quota_mb BETWEEN 10 AND 102400),
 enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN(0,1)),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN('pending','active','disabled','failed')),
 operation TEXT NOT NULL DEFAULT 'apply' CHECK(operation IN('apply','delete')),
 pending_job_id TEXT,
 error TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE INDEX email_mailboxes_user ON email_mailboxes(user_id);
CREATE TABLE email_deliveries (
 id TEXT PRIMARY KEY,
 user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
 mailbox_id TEXT REFERENCES email_mailboxes(id) ON DELETE CASCADE,
 to_address TEXT NOT NULL,
 content_hash TEXT NOT NULL DEFAULT '',
 subject TEXT NOT NULL,
 body TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN('queued','sending','accepted','uncertain')),
 created_at INTEGER NOT NULL
);
