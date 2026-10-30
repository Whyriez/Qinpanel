"""Upgrade and actual access/transfer CAS SQL, temporary SQLite only."""
from contextlib import closing
import importlib.util
from pathlib import Path
import re
import sqlite3
import sys
import unittest

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("base", Path(__file__).with_name("storage-migration-test.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
MIGRATION = ROOT / "crates/panel-db/migrations/20261030000002_storage_bucket_access.sql"
ACCESS = (ROOT / "crates/panel-api/src/storage/access.rs").read_text(encoding="utf-8")
CAS = re.search(r'sqlx::query\("(UPDATE storage_bucket_access SET revision=revision\+1[^"\n]+)"\)', ACCESS).group(1)


class AccessUpgradeTests(unittest.TestCase):
    def database(self):
        db = sqlite3.connect(":memory:")
        base.initialize(db)
        db.executescript(base.MIGRATION.read_text())
        base.connection(db)
        return db

    def test_old_bucket_and_keys_preserved_and_private_after_upgrade(self):
        with closing(self.database()) as db:
            before = db.execute("SELECT * FROM storage_connections").fetchall()
            db.executescript(MIGRATION.read_text())
            self.assertEqual(db.execute("SELECT * FROM storage_connections").fetchall(), before)
            self.assertEqual(db.execute("SELECT revision,visibility,desired_visibility,state,request_id FROM storage_bucket_access").fetchone(), (1, "private", "private", "ready", None))

    def test_running_transfer_and_pending_access_cannot_overlap(self):
        with closing(self.database()) as db:
            db.executescript(MIGRATION.read_text())
            params = ("public", "request-one", 1001, "fixture", 1, "fixture", 881)
            self.assertEqual(base.reserve(db, "upload-one"), 1)
            self.assertEqual(db.execute(CAS, params).rowcount, 0)
            db.execute("UPDATE storage_operations SET state='succeeded'")
            self.assertEqual(db.execute(CAS, params).rowcount, 1)
            self.assertEqual(base.reserve(db, "upload-two"), 0)
            self.assertEqual(db.execute(CAS, params).rowcount, 0)
            db.execute("UPDATE storage_bucket_access SET state='unknown'")
            self.assertEqual(base.reserve(db, "upload-three"), 0)
            db.execute("UPDATE storage_bucket_access SET state='ready',visibility=desired_visibility")
            self.assertEqual(base.reserve(db, "upload-four"), 1)

    def test_constraints_and_connection_deletion_only_cascade_access_metadata(self):
        with closing(self.database()) as db:
            db.executescript(MIGRATION.read_text())
            for field, value in [("visibility", "all"), ("state", "done"), ("revision", 0)]:
                with self.assertRaises(sqlite3.IntegrityError):
                    db.execute(f"UPDATE storage_bucket_access SET {field}=?", (value,))
            db.execute("DELETE FROM storage_connections WHERE id='fixture'")
            self.assertEqual(db.execute("SELECT COUNT(*) FROM storage_bucket_access").fetchone()[0], 0)
            self.assertEqual(db.execute("SELECT COUNT(*) FROM users WHERE id IN (2,3)").fetchone()[0], 2)


if __name__ == "__main__":
    unittest.main()
