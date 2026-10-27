-- Panel scope is independent of tenant certificates and never references sites.
CREATE TABLE panel_tls_certificates (
 id TEXT PRIMARY KEY NOT NULL, endpoint_id TEXT NOT NULL, endpoint_revision INTEGER NOT NULL,
 ip TEXT NOT NULL, port INTEGER NOT NULL CHECK(port BETWEEN 1024 AND 65535),
 staging INTEGER NOT NULL CHECK(staging IN (0,1)), email TEXT NOT NULL,
 fullchain_pem TEXT NOT NULL DEFAULT '', private_key_pem TEXT NOT NULL DEFAULT '',
 fingerprint TEXT, not_before INTEGER, not_after INTEGER, renew_at INTEGER,
 state TEXT NOT NULL CHECK(state IN ('queued','issuing','candidate','staging','active','retired','failed')),
 source_id TEXT,
 error TEXT, created_at INTEGER NOT NULL, last_attempt INTEGER, job_id TEXT, actor_id INTEGER, ari_checked_at INTEGER
);
CREATE UNIQUE INDEX one_active_panel_certificate ON panel_tls_certificates(state) WHERE state='active';
CREATE UNIQUE INDEX one_pending_panel_certificate ON panel_tls_certificates((1)) WHERE state IN ('queued','issuing','candidate');
CREATE TABLE panel_tls_lock (id INTEGER PRIMARY KEY CHECK(id=1), token TEXT, expires_at INTEGER NOT NULL DEFAULT 0);
INSERT INTO panel_tls_lock(id) VALUES(1);
