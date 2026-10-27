CREATE TABLE dns01_orders (
 id TEXT PRIMARY KEY,
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 owner_id INTEGER NOT NULL,
 actor_id INTEGER,
 domains_json TEXT NOT NULL,
 settings_json TEXT NOT NULL,
 zone_id TEXT NOT NULL,
 zone_name TEXT NOT NULL,
 credential_revision TEXT NOT NULL,
 email TEXT NOT NULL,
 staging INTEGER NOT NULL CHECK(staging IN (0,1)),
 source_id INTEGER,
 certificate_id INTEGER,
 state TEXT NOT NULL CHECK(state IN ('queued','issuing','candidate','active','staging','failed')),
 fullchain_pem TEXT NOT NULL DEFAULT '',
 private_key_pem TEXT NOT NULL DEFAULT '',
 job_id TEXT NOT NULL,
 error TEXT,
 created_at INTEGER NOT NULL,
 last_attempt INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX dns01_pending ON dns01_orders((1)) WHERE state IN ('queued','issuing','candidate');
CREATE TABLE dns01_bindings (
 site_id INTEGER PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
 certificate_id INTEGER NOT NULL REFERENCES certificates(id) ON DELETE CASCADE,
 zone_id TEXT NOT NULL,
 zone_name TEXT NOT NULL,
 email TEXT NOT NULL,
 renew_at INTEGER NOT NULL,
 last_attempt INTEGER NOT NULL DEFAULT 0,
 error TEXT
);
CREATE TABLE dns01_lock(id INTEGER PRIMARY KEY CHECK(id=1), token TEXT, expires_at INTEGER NOT NULL DEFAULT 0);
INSERT INTO dns01_lock(id) VALUES(1);
