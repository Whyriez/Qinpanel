"""Real SQLite upgrade/CAS checks; no CA, DNS provider, agent or host services."""
import importlib.util
from pathlib import Path
import re
import sqlite3
import sys
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
SOURCE = (ROOT / "crates/panel-api/src/storage/tls.rs").read_text(encoding="utf-8")
MIGRATION = ROOT / "crates/panel-db/migrations/20261027000001_managed_storage_tls.sql"
spec = importlib.util.spec_from_file_location("managed_fixture", Path(__file__).with_name("managed-storage-migration-test.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def sql(prefix):
    return next(value for value in re.findall(r'sqlx::query(?:_scalar)?\("([^"\n]+)"\)', SOURCE) if value.startswith(prefix))


ENROLL = sql("UPDATE managed_storage_tls SET revision=revision+1,request_id=?,job_id=?,actor_id=")
MANUAL = sql("UPDATE managed_storage_tls SET revision=revision+1,state='configuring'")
CLAIM = sql("UPDATE managed_storage_tls SET state='issuing'")
APPLY = sql("UPDATE managed_storage_tls SET state='active'")
FINISH = sql("UPDATE managed_storage_tls SET state=?,auto_renew=0")
RENEW = sql("UPDATE managed_storage_tls SET revision=revision+1,request_id=?,job_id=?,state='queued'")
FAIL = sql("UPDATE managed_storage_tls SET state='failed',auto_renew=0,updated_at=")


class StorageTlsMigration(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.addCleanup(self.db.close)
        base.initialize(self.db)
        self.before = self.db.execute("SELECT * FROM storage_connections").fetchall()
        self.users = self.db.execute("SELECT * FROM users").fetchall()
        self.db.executescript(MIGRATION.read_text())

    def enroll(self):
        return self.db.execute(ENROLL, ("request", "job", 1, "storage.example.test", "admin@example.test", "zone", "example.test", "revision", 100, 100, 1)).rowcount

    def test_upgrade_preserves_existing_users_connections_and_sealed_credentials(self):
        self.assertEqual(self.db.execute("SELECT * FROM storage_connections").fetchall(), self.before)
        self.assertEqual(self.db.execute("SELECT * FROM users").fetchall(), self.users)
        self.assertEqual(self.db.execute("SELECT revision,state,auto_renew FROM managed_storage_tls").fetchone(), (1, "idle", 0))
        columns = {row[1] for row in self.db.execute("PRAGMA table_info(managed_storage_tls)")}
        self.assertFalse(any("pem" in name or "private_key" in name or "token" in name for name in columns))

    def test_enrollment_cas_and_manual_config_are_mutually_exclusive(self):
        self.assertEqual(self.enroll(), 1)
        self.assertEqual(self.enroll(), 0)
        self.assertIsNone(self.db.execute(MANUAL, (101,)).fetchone())
        self.assertEqual(self.db.execute("SELECT revision,state FROM managed_storage_tls").fetchone(), (2, "queued"))

    def test_worker_claim_is_once_only_and_blocks_manual_changes(self):
        self.enroll()
        self.assertEqual(self.db.execute(CLAIM, (101, 2)).rowcount, 1)
        self.assertEqual(self.db.execute(CLAIM, (101, 2)).rowcount, 0)
        self.assertIsNone(self.db.execute(MANUAL, (102,)).fetchone())
        self.assertEqual(self.db.execute(APPLY, (50, 5000, 4000, 103, 2)).rowcount, 1)
        self.assertEqual(self.db.execute(APPLY, (50, 5000, 4000, 103, 2)).rowcount, 0)

    def test_manual_replacement_fences_stale_worker_and_stops_renewal(self):
        self.enroll()
        self.db.execute("UPDATE managed_storage_tls SET state='active'")
        revision = self.db.execute(MANUAL, (101,)).fetchone()[0]
        self.assertEqual(revision, 3)
        self.assertEqual(self.db.execute(APPLY, (50, 5000, 4000, 103, 2)).rowcount, 0)
        self.db.execute(FINISH, ("manual", "storage.example.test", 105, None, revision))
        self.assertEqual(self.db.execute("SELECT state,auto_renew,request_id,job_id FROM managed_storage_tls").fetchone(), ("manual", 0, None, None))
        self.assertEqual(self.db.execute(RENEW, ("next", "next-job", 200, 200, revision)).rowcount, 0)

    def test_renewal_cas_cannot_duplicate_or_replace_a_newer_generation(self):
        self.enroll()
        self.db.execute("UPDATE managed_storage_tls SET state='active'")
        self.assertEqual(self.db.execute(RENEW, ("next", "next-job", 4000, 4000, 2)).rowcount, 1)
        self.assertEqual(self.db.execute(RENEW, ("other", "other-job", 4000, 4000, 2)).rowcount, 0)
        self.assertEqual(self.db.execute("SELECT request_id,job_id,revision FROM managed_storage_tls").fetchone(), ("next", "next-job", 3))

    def test_failed_issuance_pauses_renewal_instead_of_repeating_ca_orders(self):
        self.enroll()
        self.db.execute(CLAIM,(101,2))
        self.assertEqual(self.db.execute(FAIL,(102,2)).rowcount,1)
        self.assertEqual(self.db.execute("SELECT state,auto_renew FROM managed_storage_tls").fetchone(),("failed",0))
        self.assertEqual(self.db.execute(RENEW,("retry","retry-job",4000,4000,2)).rowcount,0)

    def test_constraints_reject_unknown_states_and_invalid_flags(self):
        for statement in ["UPDATE managed_storage_tls SET auto_renew=2", "UPDATE managed_storage_tls SET revision=0", "UPDATE managed_storage_tls SET state='arbitrary'", "INSERT INTO managed_storage_tls(id) VALUES(2)"]:
            with self.subTest(statement=statement), self.assertRaises(sqlite3.IntegrityError):
                self.db.execute(statement)

    def test_dns_credential_changes_are_blocked_during_storage_issuance(self):
        dns_source=(ROOT / "crates/panel-api/src/dns01.rs").read_text(encoding="utf-8")
        query=next(value for value in re.findall(r'"(SELECT EXISTS[^"\n]+)"',dns_source) if "managed_storage_tls" in value)
        self.db.execute("CREATE TABLE IF NOT EXISTS dns01_orders(state TEXT)")
        self.assertEqual(self.db.execute(query).fetchone()[0],0)
        self.enroll()
        self.assertEqual(self.db.execute(query).fetchone()[0],1)
        self.db.execute("UPDATE managed_storage_tls SET state='active'")
        self.assertEqual(self.db.execute(query).fetchone()[0],0)

    def test_running_storage_tls_job_cannot_be_cancelled_mid_mutation(self):
        source=(ROOT / "crates/panel-db/src/repositories/job_repo.rs").read_text(encoding="utf-8")
        query=next(value for value in re.findall(r'r#"(.*?)"#',source,re.S) if "SET status = 'cancelled'" in value)
        with sqlite3.connect(":memory:") as db:
            db.execute("CREATE TABLE jobs(id TEXT,name TEXT,status TEXT,updated_at INTEGER)")
            db.execute("INSERT INTO jobs VALUES('tls','storage.tls.issue','running',0)")
            self.assertEqual(db.execute(query,(100,'tls')).rowcount,0)
            db.execute("UPDATE jobs SET status='queued'")
            self.assertEqual(db.execute(query,(101,'tls')).rowcount,1)


if __name__ == "__main__":
    unittest.main()
