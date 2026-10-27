"""SQL contract checks using temporary SQLite files, without API/agent services.

This exercises the checked-in SQL, not the Rust/SQLx runtime or VM acceptance.
"""

from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from pathlib import Path
import re
import shutil
import sqlite3
import tempfile


REPO = Path(__file__).resolve().parents[2]
IMPORT = REPO / "crates/panel-db/src/operations.rs"
MIGRATIONS = REPO / "crates/panel-db/migrations"
OPERATIONS = REPO / "crates/panel-db/operations_migrations"
checks = 0


def check(condition, message):
    global checks
    assert condition, message
    checks += 1
    print(f"PASS {message}")


def connect(path, query_only=False):
    connection = sqlite3.connect(path, timeout=15, isolation_level=None)
    if not query_only:
        connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("PRAGMA foreign_keys=ON")
    if query_only:
        connection.execute("PRAGMA query_only=ON")
    return connection


def migrate(connection, directory):
    for migration in sorted(directory.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))


def import_legacy(connection, source, interrupt=False):
    if connection.execute("SELECT COUNT(*) FROM legacy_import").fetchone()[0]:
        return
    source_code = IMPORT.read_text(encoding="utf-8")
    statements = re.findall(r'sqlx::query\("(INSERT INTO [^"\n]+)"\)', source_code)
    assert len(statements) == 3
    connection.execute("ATTACH DATABASE ? AS legacy", (str(source),))
    try:
        connection.execute("BEGIN IMMEDIATE")
        for index, sql in enumerate(statements):
            connection.execute(sql, (str(source),) if "legacy_import" in sql else ())
            if interrupt and index == 0:
                raise RuntimeError("interrupted import")
        connection.execute("COMMIT")
    except Exception:
        connection.execute("ROLLBACK")
        raise
    finally:
        connection.execute("DETACH DATABASE legacy")


def main():
    with tempfile.TemporaryDirectory(prefix="vpanel-db-contract-") as tmp:
        directory = Path(tmp)
        core_path = directory / "panel.db"
        ops_path = directory / "panel.operations.db"
        core = connect(core_path)
        migrate(core, MIGRATIONS)
        core.execute("INSERT INTO jobs VALUES ('legacy',NULL,'test.sleep','queued',0,'{}',NULL,NULL,0,3,1,1)")
        core.execute("INSERT INTO audit_logs VALUES (17,NULL,'admin','test','test',NULL,NULL,'success',1)")
        writer = connect(ops_path)
        migrate(writer, OPERATIONS)
        try:
            import_legacy(writer, core_path, interrupt=True)
        except RuntimeError:
            pass
        check(writer.execute("SELECT COUNT(*) FROM jobs").fetchone()[0] == 0,
              "interrupted import rolls back job rows")
        check(writer.execute("SELECT COUNT(*) FROM legacy_import").fetchone()[0] == 0,
              "interrupted import does not commit its marker")
        import_legacy(writer, core_path)
        check(writer.execute("SELECT id FROM audit_logs").fetchone()[0] == 17,
              "audit IDs survive import")
        writer.execute("UPDATE jobs SET status='succeeded' WHERE id='legacy'")
        import_legacy(writer, core_path)
        check(writer.execute("SELECT status FROM jobs WHERE id='legacy'").fetchone()[0] == "succeeded",
              "restart does not overwrite operational state with stale source data")
        core.execute("BEGIN IMMEDIATE")
        core.execute("INSERT INTO settings VALUES ('held','1',0)")

        def write(index):
            connection = connect(ops_path)
            try:
                connection.execute("INSERT INTO jobs VALUES (?,NULL,'test.sleep','queued',0,'{}',NULL,NULL,0,3,1,1)",
                                   (f"job-{index}",))
                connection.execute("INSERT INTO audit_logs(user_id,actor_username,action,target_resource,status,created_at) VALUES (NULL,'admin','test','test','success',1)")
            finally:
                connection.close()

        # Mirrors the operational single-connection writer queue under many callers.
        with ThreadPoolExecutor(max_workers=1) as queue:
            list(queue.map(write, range(128)))
        check(writer.execute("SELECT COUNT(*) FROM jobs").fetchone()[0] == 129,
              "128 queued writes finish while the core database is write-locked")
        reader = connect(ops_path, query_only=True)
        writer.execute("BEGIN IMMEDIATE")
        writer.execute("UPDATE jobs SET progress=10")
        check(reader.execute("SELECT COUNT(*) FROM jobs WHERE progress=0").fetchone()[0] == 129,
              "WAL readers see committed data while an operational write is in progress")
        try:
            reader.execute("DELETE FROM jobs")
        except sqlite3.OperationalError:
            check(True, "reader pool rejects writes")
        else:
            raise AssertionError("reader unexpectedly accepted a write")
        writer.execute("ROLLBACK")
        core.execute("ROLLBACK")
        snapshot = directory / "operations.snapshot.db"
        source = sqlite3.connect(f"{ops_path.as_uri()}?mode=ro", uri=True, isolation_level=None)
        source.execute("VACUUM main INTO ?", (str(snapshot),))
        source.close()
        with closing(sqlite3.connect(snapshot)) as restored:
            check(restored.execute("SELECT COUNT(*) FROM jobs").fetchone()[0] == 129,
                  "read-only SQLite snapshot includes committed WAL writes")
            check(restored.execute("PRAGMA quick_check").fetchone()[0] == "ok",
                  "snapshot passes integrity check")
        check(core.execute("SELECT COUNT(*) FROM jobs").fetchone()[0] == 1,
              "new job writes do not return to the core database")

        # A VACUUM snapshot has DELETE journaling. Restore both files offline,
        # then let the writer establish WAL before readers inherit that mode.
        with closing(sqlite3.connect(snapshot)) as copy:
            check(copy.execute("PRAGMA journal_mode").fetchone()[0] == "delete",
                  "VACUUM snapshot starts in standalone rollback-journal mode")
        core.execute("INSERT INTO settings VALUES ('restore.fixture','saved',0)")
        core_snapshot = directory / "core.snapshot.db"
        core.execute("VACUUM main INTO ?", (str(core_snapshot),))
        core.execute("UPDATE settings SET value='changed' WHERE key='restore.fixture'")
        writer.execute("UPDATE jobs SET progress=99")
        reader.close()
        writer.close()
        core.close()
        for source, target in [(core_snapshot, core_path), (snapshot, ops_path)]:
            for suffix in ("-wal", "-shm"):
                Path(str(target) + suffix).unlink(missing_ok=True)
            shutil.copyfile(source, target)
        core = connect(core_path)
        writer = connect(ops_path)
        reader = connect(ops_path, query_only=True)
        try:
            check(core.execute("SELECT value FROM settings WHERE key='restore.fixture'").fetchone()[0] == "saved",
                  "offline restore returns core data from its snapshot")
            check(reader.execute("SELECT COUNT(*) FROM jobs WHERE progress=0").fetchone()[0] == 129,
                  "offline restore returns operational WAL contents from its snapshot")
            check(all(connection.execute("PRAGMA journal_mode").fetchone()[0] == "wal" for connection in (writer, reader)),
                  "restored writer establishes WAL and reader inherits it")
            try:
                reader.execute("DELETE FROM jobs")
            except sqlite3.OperationalError:
                check(True, "restored reader remains query-only")
            else:
                raise AssertionError("restored reader unexpectedly accepted a write")
            writer.execute("UPDATE jobs SET progress=67 WHERE id='job-0'")
            check(reader.execute("SELECT progress FROM jobs WHERE id='job-0'").fetchone()[0] == 67,
                  "restored writer and reader continue serving new committed changes")
            check(all(connection.execute("PRAGMA quick_check").fetchone()[0] == "ok" for connection in (core, writer)),
                  "restored core and operations pass integrity checks")
        finally:
            reader.close()
            writer.close()
            core.close()
    print(f"{checks} SQLite SQL contract checks passed; Rust/VM acceptance was not run.")


if __name__ == "__main__":
    main()
