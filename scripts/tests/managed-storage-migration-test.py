"""Upgrade and CAS tests using real migrations and the API's SQL statements."""
from contextlib import closing
import importlib.util
from pathlib import Path
import re
import sqlite3
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("storage_migration", Path(__file__).with_name("storage-migration-test.py"))
base = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(base)
MIGRATION = ROOT / "crates/panel-db/migrations/20261026000002_managed_object_storage.sql"
SOURCE = (ROOT / "crates/panel-api/src/storage/managed.rs").read_text(encoding="utf-8")
KEY_RESERVE = re.search(r'sqlx::query\("(UPDATE storage_connections SET application_credentials_enc=[^"\n]+)"\)', SOURCE).group(1)
KEY_FINISH = re.search(r'sqlx::query\("(UPDATE storage_connections SET application_key_state=[^"\n]+)"\)', SOURCE).group(1)
TENANTS = (ROOT / "crates/panel-api/src/tenants/handlers.rs").read_text(encoding="utf-8")
DELETE_RESERVE = re.search(r'sqlx::query\("(UPDATE users SET status=\'deleting\' WHERE id=\? AND NOT EXISTS[^"\n]+)"\)', TENANTS).group(1)


def initialize(db):
    base.initialize(db)
    base.migrate_storage(db)
    base.connection(db)
    db.executescript(MIGRATION.read_text())
    db.execute("UPDATE storage_connections SET managed=1,managed_state='ready'")
    db.commit()


class MigrationTests(unittest.TestCase):
    def test_existing_external_connection_and_identity_survive_upgrade(self):
        with closing(sqlite3.connect(":memory:")) as db:
            base.initialize(db)
            base.migrate_storage(db)
            base.connection(db)
            old = db.execute("SELECT id,owner_id,site_id,credentials_enc,revision FROM storage_connections").fetchall()
            users = db.execute("SELECT * FROM users").fetchall()
            db.executescript(MIGRATION.read_text())
            self.assertEqual(db.execute("SELECT id,owner_id,site_id,credentials_enc,revision FROM storage_connections").fetchall(), old)
            self.assertEqual(db.execute("SELECT * FROM users").fetchall(), users)
            self.assertEqual(db.execute("SELECT managed,managed_state,application_key_generation FROM storage_connections").fetchone(), (0, None, 0))
            db.execute("DELETE FROM storage_connections")

    def test_managed_identity_cannot_be_reassigned_or_cascade_deleted(self):
        with closing(sqlite3.connect(":memory:")) as db:
            initialize(db)
            db.execute("UPDATE storage_connections SET managed=1,managed_state='ready'")
            db.commit()
            for statement in ["UPDATE storage_connections SET owner_id=3", "UPDATE storage_connections SET managed=0", "UPDATE storage_connections SET bucket='other-bucket'", "DELETE FROM storage_connections", "DELETE FROM users WHERE id=2"]:
                with self.subTest(statement=statement), self.assertRaises(sqlite3.IntegrityError):
                    db.execute(statement)
                db.rollback()
            self.assertEqual(db.execute("SELECT owner_id FROM storage_connections").fetchone()[0], 2)

    def test_rotation_cas_prevents_duplicate_keys_and_has_one_successful_reveal(self):
        with closing(sqlite3.connect(":memory:")) as db:
            initialize(db)
            first = ("enc:v1:first", 1, "fixture", 1, 0)
            self.assertEqual(db.execute(KEY_RESERVE, first).rowcount, 1)
            self.assertEqual(db.execute(KEY_RESERVE, ("enc:v1:second", 1, "fixture", 1, 0)).rowcount, 0)
            self.assertEqual(db.execute(KEY_FINISH, ("ready", "fixture", 1)).rowcount, 1)
            self.assertEqual(db.execute(KEY_FINISH, ("ready", "fixture", 1)).rowcount, 0)
            self.assertEqual(db.execute("SELECT application_credentials_enc FROM storage_connections").fetchone()[0], "enc:v1:first")

    def test_unknown_outcome_requires_reconcile_before_another_rotation(self):
        with closing(sqlite3.connect(":memory:")) as db:
            initialize(db)
            db.execute(KEY_RESERVE, ("enc:v1:first", 1, "fixture", 1, 0))
            self.assertEqual(db.execute(KEY_FINISH, ("unknown", "fixture", 1)).rowcount, 1)
            self.assertEqual(db.execute(KEY_RESERVE, ("enc:v1:second", 2, "fixture", 1, 1)).rowcount, 0)
            self.assertEqual(db.execute(KEY_FINISH, ("ready", "fixture", 1)).rowcount, 1)
            self.assertEqual(db.execute(KEY_RESERVE, ("enc:v1:second", 2, "fixture", 1, 1)).rowcount, 1)

    def test_application_credentials_must_be_encrypted(self):
        with closing(sqlite3.connect(":memory:")) as db:
            initialize(db)
            with self.assertRaises(sqlite3.IntegrityError):
                db.execute("UPDATE storage_connections SET application_credentials_enc='plaintext'")

    def test_native_confirmed_removal_can_delete_metadata_after_decommission(self):
        with closing(sqlite3.connect(":memory:")) as db:
            initialize(db)
            db.execute("UPDATE storage_connections SET managed_state='deleting'")
            with self.assertRaises(sqlite3.IntegrityError):
                db.execute("DELETE FROM storage_connections")
            db.execute("UPDATE storage_connections SET managed_state='deleted'")
            self.assertEqual(db.execute("DELETE FROM storage_connections").rowcount, 1)

    def test_tenant_deletion_is_rejected_before_linux_side_effects(self):
        with closing(sqlite3.connect(":memory:")) as db:
            initialize(db)
            self.assertEqual(db.execute(DELETE_RESERVE, (2, 2)).rowcount, 0)
            self.assertEqual(db.execute("SELECT status FROM users WHERE id=2").fetchone()[0], "active")

    def test_allocation_cannot_race_an_already_reserved_tenant_deletion(self):
        with closing(sqlite3.connect(":memory:")) as db:
            base.initialize(db)
            base.migrate_storage(db)
            base.connection(db)
            db.executescript(MIGRATION.read_text())
            self.assertEqual(db.execute(DELETE_RESERVE, (2, 2)).rowcount, 1)
            with self.assertRaises(sqlite3.IntegrityError):
                db.execute("INSERT INTO storage_connections(id,owner_id,label,provider,endpoint,region,addressing,bucket,credentials_enc,created_at,updated_at,managed,managed_state) SELECT 'new-allocation',owner_id,label,provider,endpoint,region,addressing,'another-bucket',credentials_enc,created_at,updated_at,1,'pending' FROM storage_connections WHERE id='fixture'")


if __name__ == "__main__":
    unittest.main()
