-- Existing pending jobs and renewals keep their historical DNS-01 method.
ALTER TABLE managed_storage_tls ADD COLUMN method TEXT NOT NULL DEFAULT 'dns01'
    CHECK (method IN ('dns01', 'http01', 'tunnel'));
ALTER TABLE managed_storage_tls ADD COLUMN tunnel_id TEXT;
