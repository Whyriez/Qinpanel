-- Additive upgrade: existing provider connections retain their external target.
ALTER TABLE storage_connections ADD COLUMN managed INTEGER NOT NULL DEFAULT 0 CHECK(managed IN (0,1));
ALTER TABLE storage_connections ADD COLUMN managed_state TEXT CHECK(managed_state IN ('pending','ready','unknown','deleting','deleted'));
ALTER TABLE storage_connections ADD COLUMN application_key_generation INTEGER NOT NULL DEFAULT 0 CHECK(application_key_generation >= 0);
ALTER TABLE storage_connections ADD COLUMN application_key_state TEXT CHECK(application_key_state IN ('pending','ready','unknown'));
ALTER TABLE storage_connections ADD COLUMN application_credentials_enc TEXT CHECK(application_credentials_enc IS NULL OR application_credentials_enc LIKE 'enc:v1:%');

-- Serialize allocation against tenant deletion's status reservation. The API
-- validates the full tenant context; this closes the check/insert race.
CREATE TRIGGER storage_managed_owner_guard BEFORE INSERT ON storage_connections
WHEN NEW.managed = 1 AND NOT EXISTS (
    SELECT 1 FROM users WHERE id=NEW.owner_id AND role='user' AND status='active'
        AND linux_uid BETWEEN 10000 AND 59999
)
BEGIN
    SELECT RAISE(ABORT, 'Managed bucket requires an active provisioned tenant');
END;

-- Native buckets/data outlive metadata and tenant deletion. Never orphan a live
-- bucket by cascading a user/site deletion through storage_connections.
CREATE TRIGGER storage_managed_delete_guard BEFORE DELETE ON storage_connections
WHEN OLD.managed = 1 AND COALESCE(OLD.managed_state,'') != 'deleted'
BEGIN
    SELECT RAISE(ABORT, 'Managed bucket must be retained until explicit storage decommission');
END;
CREATE TRIGGER storage_managed_identity_guard BEFORE UPDATE OF managed,owner_id,endpoint,bucket,provider,addressing ON storage_connections
WHEN OLD.managed = 1 AND (NEW.managed != 1 OR NEW.owner_id != OLD.owner_id OR NEW.endpoint != OLD.endpoint OR NEW.bucket != OLD.bucket OR NEW.provider != OLD.provider OR NEW.addressing != OLD.addressing)
BEGIN
    SELECT RAISE(ABORT, 'Managed bucket identity is immutable');
END;
