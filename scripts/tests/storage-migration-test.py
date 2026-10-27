"""Actual storage SQL and upgrade fixtures, temporary SQLite, no provider/daemon."""
from contextlib import closing
from pathlib import Path
import concurrent.futures
import re
import sqlite3
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]
MIGRATION = ROOT / 'crates/panel-db/migrations/20261026000001_object_storage.sql'
STORE = (ROOT / 'crates/panel-api/src/storage/store.rs').read_text(encoding='utf-8')
HANDLERS = (ROOT / 'crates/panel-api/src/storage/handlers.rs').read_text(encoding='utf-8')
PERMIT = re.search(r'sqlx::query\("(INSERT INTO storage_operations[^"\n]+)"\)', STORE).group(1)
SITE_CONTEXT = re.search(r'sqlx::query_as\("(SELECT s.id[^"\n]+)"\)', HANDLERS).group(1)
BIND = re.search(r'sqlx::query\("(UPDATE storage_connections SET site_id=\?,revision[^"\n]+)"\)', HANDLERS).group(1)


def initialize(db):
    db.execute('PRAGMA foreign_keys=ON')
    for path in sorted(MIGRATION.parent.glob('*.sql')):
        if path.name < MIGRATION.name:
            db.executescript(path.read_text(encoding='utf-8'))
    for identifier, name in [(2, 'alice'), (3, 'bob')]:
        db.execute("INSERT INTO users(id,username,password_hash,role,status,linux_username,linux_uid,created_at,updated_at) VALUES(?,?,'fixture','user','active',?,?,1,1)", (identifier, name, 'p_' + name, identifier + 10000))
        db.execute("INSERT INTO sites(id,user_id,kind,docroot,status,created_at,updated_at) VALUES(?,?,'static','fixture','active',1,1)", (identifier, identifier))
        db.execute("INSERT INTO domains(site_id,name,unicode_name,is_primary,created_at,updated_at) VALUES(?,?,?,1,1,1)", (identifier, name + '.example.test', name + '.example.test'))
    db.commit()


def connection(db):
    db.execute("INSERT INTO storage_connections(id,owner_id,site_id,label,provider,endpoint,region,addressing,bucket,credentials_enc,created_at,updated_at) VALUES('fixture',2,2,'Project media','compatible','https://objects.example.test','us-east-1','path','project-media','enc:v1:fixture',1,1)")
    db.execute("UPDATE storage_connections SET verified_revision=1,verified_at=1000")
    db.commit()


def reserve(db, identifier):
    return db.execute(PERMIT, (identifier, 'fixture', 2, 1, 'upload', 1000, 'fixture', 1, 'upload', 400, 'fixture', 880)).rowcount


class StorageMigrationTests(unittest.TestCase):
    def test_upgrade_preserves_legacy_ids_data_and_has_no_connections_by_default(self):
        with closing(sqlite3.connect(':memory:')) as db:
            initialize(db)
            old = {name: db.execute('SELECT * FROM ' + name).fetchall() for name in ['users', 'sites', 'domains', 'plans', 'settings', 'ai_settings']}
            db.executescript(MIGRATION.read_text())
            for name, rows in old.items():
                self.assertEqual(db.execute('SELECT * FROM ' + name).fetchall(), rows)
            self.assertEqual(db.execute('SELECT COUNT(*) FROM storage_connections').fetchone()[0], 0)

    def test_actual_context_query_uses_domain_relation_and_filters_owners(self):
        with closing(sqlite3.connect(':memory:')) as db:
            initialize(db)
            self.assertEqual(db.execute(SITE_CONTEXT, (0, 2)).fetchall(), [(2, 2, 'alice.example.test')])
            self.assertEqual(len(db.execute(SITE_CONTEXT, (1, 1)).fetchall()), 2)

    def test_cipher_constraint_and_endpoint_bucket_uniqueness(self):
        with closing(sqlite3.connect(':memory:')) as db:
            initialize(db)
            db.executescript(MIGRATION.read_text())
            connection(db)
            with self.assertRaises(sqlite3.IntegrityError):
                db.execute("UPDATE storage_connections SET credentials_enc='plaintext secret'")
            db.rollback()
            with self.assertRaises(sqlite3.IntegrityError):
                db.execute("INSERT INTO storage_connections SELECT 'other',owner_id,site_id,label,provider,endpoint,region,addressing,bucket,credentials_enc,revision,verified_revision,verified_at,created_at,updated_at FROM storage_connections")

    def test_active_operation_blocks_binding_and_stale_revision(self):
        with closing(sqlite3.connect(':memory:')) as db:
            initialize(db)
            db.executescript(MIGRATION.read_text())
            connection(db)
            self.assertEqual(reserve(db, 'one'), 1)
            self.assertEqual(db.execute(BIND, (None, 1001, 'fixture', 1, 'fixture', 881)).rowcount, 0)
            db.execute("UPDATE storage_operations SET state='succeeded'")
            self.assertEqual(db.execute(BIND, (None, 1001, 'fixture', 9, 'fixture', 881)).rowcount, 0)
            self.assertEqual(db.execute(BIND, (None, 1001, 'fixture', 1, 'fixture', 881)).rowcount, 1)

    def test_project_deletion_retains_bucket_metadata_and_tenant_deletion_only_removes_metadata(self):
        with closing(sqlite3.connect(':memory:')) as db:
            initialize(db)
            db.executescript(MIGRATION.read_text())
            connection(db)
            db.execute('DELETE FROM sites WHERE id=2')
            self.assertIsNone(db.execute('SELECT site_id FROM storage_connections').fetchone()[0])
            self.assertEqual(db.execute('SELECT bucket FROM storage_connections').fetchone()[0], 'project-media')
            db.execute('DELETE FROM users WHERE id=2')
            self.assertEqual(db.execute('SELECT COUNT(*) FROM storage_connections').fetchone()[0], 0)
            self.assertEqual(db.execute('SELECT id FROM users WHERE id=3').fetchone()[0], 3)

    def test_independent_connections_cannot_reserve_the_same_remote_mutation_concurrently(self):
        with tempfile.TemporaryDirectory(prefix='qonpanel-storage-sql-') as directory:
            filename = str(Path(directory) / 'fixture.db')
            with closing(sqlite3.connect(filename)) as db:
                initialize(db)
                db.executescript(MIGRATION.read_text())
                connection(db)
            barrier = threading.Barrier(2)

            def attempt(identifier):
                with closing(sqlite3.connect(filename, timeout=5, isolation_level=None)) as db:
                    barrier.wait(timeout=5)
                    return reserve(db, identifier)

            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
                self.assertEqual(sorted(executor.map(attempt, ['one', 'two'])), [0, 1])


if __name__ == '__main__':
    unittest.main()
