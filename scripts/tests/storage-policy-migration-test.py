"""SQLite upgrade and interleaving checks; no daemon or provider is started."""
from contextlib import closing
import importlib.util
import json
from pathlib import Path
import re
import sqlite3
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("storage_base", Path(__file__).with_name("storage-migration-test.py"))
base = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(base)
MIGRATION = ROOT / "crates/panel-db/migrations/20261029000001_storage_privacy_policy.sql"
SOURCE = (ROOT / "crates/panel-api/src/storage/policy.rs").read_text(encoding="utf-8")
ALLOCATION = re.search(r'sqlx::query\("(UPDATE storage_policies SET revision=revision\+1,quota_bytes=[^"\n]+)"\)', SOURCE).group(1)
RULES = re.search(r'sqlx::query\("(UPDATE storage_policies SET revision=revision\+1,rules_json=[^"\n]+)"\)', SOURCE).group(1)


def previous(db):
    base.initialize(db)
    db.executescript(base.MIGRATION.read_text(encoding="utf-8"))
    base.connection(db)
    for path in sorted(MIGRATION.parent.glob("*.sql")):
        if base.MIGRATION.name < path.name < MIGRATION.name:
            db.executescript(path.read_text(encoding="utf-8"))
    db.commit()


class StoragePolicyTests(unittest.TestCase):
    def test_upgrade_preserves_existing_metadata_credentials_and_tenants(self):
        with closing(sqlite3.connect(":memory:")) as db:
            previous(db)
            old = db.execute("SELECT * FROM storage_connections").fetchall()
            users = db.execute("SELECT * FROM users").fetchall()
            db.executescript(MIGRATION.read_text(encoding="utf-8"))
            self.assertEqual(db.execute("SELECT * FROM storage_connections").fetchall(), old)
            self.assertEqual(db.execute("SELECT * FROM users").fetchall(), users)
            row = db.execute("SELECT revision,quota_bytes,used_bytes,object_count,measured_at,rules_json FROM storage_policies").fetchone()
            self.assertEqual(row[:5], (1, None, None, None, None))
            self.assertEqual(json.loads(row[5]), {"read": True, "write": True, "delete": True, "max_file_bytes": 33554432, "extensions": []})

    def test_allocation_and_owner_rules_do_not_overwrite_stale_revisions(self):
        with closing(sqlite3.connect(":memory:")) as db:
            previous(db)
            db.executescript(MIGRATION.read_text(encoding="utf-8"))
            self.assertEqual(db.execute(ALLOCATION, (1073741824, "fixture", 1, "fixture", 0)).rowcount, 1)
            new_rules = json.dumps({"read": True, "write": False, "delete": False, "max_file_bytes": 1024, "extensions": ["png"]})
            self.assertEqual(db.execute(RULES, (new_rules, "fixture", 1, "fixture", 0)).rowcount, 0)
            self.assertEqual(db.execute(RULES, (new_rules, "fixture", 2, "fixture", 0)).rowcount, 1)
            row = db.execute("SELECT revision,quota_bytes,rules_json FROM storage_policies").fetchone()
            self.assertEqual(row, (3, 1073741824, new_rules))

    def test_running_transfer_blocks_rule_and_allocation_changes(self):
        with closing(sqlite3.connect(":memory:")) as db:
            previous(db)
            db.executescript(MIGRATION.read_text(encoding="utf-8"))
            db.execute("INSERT INTO storage_operations(id,connection_id,actor_id,revision,action,state,created_at) VALUES('transfer','fixture',2,1,'upload','running',100)")
            self.assertEqual(db.execute(ALLOCATION, (1024, "fixture", 1, "fixture", 0)).rowcount, 0)
            self.assertEqual(db.execute(RULES, ('{}', "fixture", 1, "fixture", 0)).rowcount, 0)
            db.execute("UPDATE storage_operations SET state='succeeded' WHERE id='transfer'")
            self.assertEqual(db.execute(ALLOCATION, (1024, "fixture", 1, "fixture", 0)).rowcount, 1)

    def test_zero_usage_is_distinct_from_unmeasured_and_invalid_totals_are_rejected(self):
        with closing(sqlite3.connect(":memory:")) as db:
            previous(db)
            db.executescript(MIGRATION.read_text(encoding="utf-8"))
            self.assertIsNone(db.execute("SELECT used_bytes FROM storage_policies").fetchone()[0])
            db.execute("UPDATE storage_policies SET used_bytes=0,object_count=0,measured_at=100")
            self.assertEqual(db.execute("SELECT used_bytes,object_count,measured_at FROM storage_policies").fetchone(), (0, 0, 100))
            for column, value in [("used_bytes", -1), ("object_count", -1), ("quota_bytes", 0), ("quota_bytes", -1)]:
                with self.subTest(column=column, value=value), self.assertRaises(sqlite3.IntegrityError):
                    db.execute(f"UPDATE storage_policies SET {column}=?", (value,))

    def test_disconnect_cascades_policy_without_deleting_owner(self):
        with closing(sqlite3.connect(":memory:")) as db:
            previous(db)
            db.executescript(MIGRATION.read_text(encoding="utf-8"))
            db.execute("DELETE FROM storage_connections WHERE id='fixture'")
            self.assertEqual(db.execute("SELECT COUNT(*) FROM storage_policies").fetchone()[0], 0)
            self.assertEqual(db.execute("SELECT COUNT(*) FROM users WHERE id=2").fetchone()[0], 1)


if __name__ == "__main__":
    unittest.main()
