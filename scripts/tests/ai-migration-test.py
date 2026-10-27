"""Additive AI schema/upgrade and actual permit SQL, using temporary SQLite only."""
import concurrent.futures
from contextlib import closing
from pathlib import Path
import re
import sqlite3
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]
MIGRATION = ROOT / "crates/panel-db/migrations/20261025000001_ai_assistant.sql"
STORE = (ROOT / "crates/panel-api/src/ai/store.rs").read_text(encoding="utf-8")
PERMIT = re.search(r'sqlx::query\("(INSERT INTO ai_requests[^"\n]+)"\)', STORE).group(1)


def initialize(connection):
    for path in sorted(MIGRATION.parent.glob("*.sql")):
        if path.name < MIGRATION.name:
            connection.executescript(path.read_text(encoding="utf-8"))


def permit(connection, identifier, now=10000, limit=2, provider="gemini"):
    return connection.execute(PERMIT, (identifier, 1, provider, "fixture-model", 1,
                                      now, 1, now // 86400 * 86400, limit, now - 90, now - 5)).rowcount


class AiUpgradeTests(unittest.TestCase):
    def test_existing_data_is_unchanged_and_feature_starts_disabled(self):
        with closing(sqlite3.connect(":memory:")) as db:
            initialize(db)
            db.execute("INSERT INTO users(username,password_hash,role,created_at,updated_at) VALUES('legacyadmin','fixture-hash','admin',1,1)")
            db.execute("INSERT INTO settings(key,value,updated_at) VALUES('legacy.runtime.identifier','preserve-me',1)")
            users = db.execute("SELECT * FROM users").fetchall()
            plans = db.execute("SELECT * FROM plans").fetchall()
            settings = db.execute("SELECT * FROM settings ORDER BY key").fetchall()
            db.commit()
            db.executescript(MIGRATION.read_text())
            self.assertEqual(db.execute("SELECT * FROM users").fetchall(), users)
            self.assertEqual(db.execute("SELECT * FROM plans").fetchall(), plans)
            self.assertEqual(db.execute("SELECT * FROM settings ORDER BY key").fetchall(), settings)
            self.assertEqual(db.execute("SELECT enabled,revision,daily_request_limit FROM ai_settings").fetchone(), (0, 1, 20))
            self.assertEqual(db.execute("SELECT COUNT(*) FROM ai_credentials").fetchone()[0], 0)

    def test_constraints_reject_invalid_modes_plaintext_and_unbounded_limits(self):
        with closing(sqlite3.connect(":memory:")) as db:
            db.executescript(MIGRATION.read_text())
            for statement in ["UPDATE ai_settings SET enabled=2", "UPDATE ai_settings SET provider='custom'",
                              "UPDATE ai_settings SET daily_request_limit=0", "UPDATE ai_settings SET daily_request_limit=1001",
                              "INSERT INTO ai_credentials VALUES('gemini','plaintext-fixture',1)"]:
                with self.assertRaises(sqlite3.IntegrityError):
                    db.execute(statement)
                db.rollback()

    def test_actual_permit_sql_enforces_cooldown_daily_budget_and_rotation(self):
        with closing(sqlite3.connect(":memory:")) as db:
            db.executescript(MIGRATION.read_text())
            self.assertEqual(permit(db, "first"), 1)
            self.assertEqual(permit(db, "parallel"), 0)
            db.execute("UPDATE ai_requests SET state='failed'")
            self.assertEqual(permit(db, "too-soon", 10001), 0)
            self.assertEqual(permit(db, "rotated-provider", 10006, provider="openai"), 1)
            self.assertEqual(permit(db, "daily-exhausted", 10100), 0)
            self.assertEqual(permit(db, "tomorrow", 96400), 1)

    def test_separate_connections_cannot_race_into_two_active_permits(self):
        with tempfile.TemporaryDirectory(prefix="qonpanel-ai-sql-") as temp:
            filename = str(Path(temp) / "fixture.db")
            with closing(sqlite3.connect(filename)) as db:
                db.executescript(MIGRATION.read_text())
            barrier = threading.Barrier(2)

            def attempt(identifier):
                with closing(sqlite3.connect(filename, timeout=5, isolation_level=None)) as db:
                    barrier.wait(timeout=5)
                    return permit(db, identifier)

            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
                results = list(executor.map(attempt, ["one", "two"]))
            self.assertEqual(sorted(results), [0, 1])


if __name__ == "__main__":
    unittest.main()
