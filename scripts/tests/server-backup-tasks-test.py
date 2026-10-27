"""Exercise checked-in observation/claim/cancel/recovery SQL in temporary SQLite.

This does not substitute for Rust/SQLx tests or native systemd acceptance.
"""
import ast
import json
from pathlib import Path
import re
import sqlite3
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = (ROOT / "crates/panel-db/src/repositories/job_repo.rs").read_text()


def sql(method):
    section = SOURCE.split("pub async fn " + method + "(", 1)[1].split("pub async fn ", 1)[0]
    raw = re.search(r'r#"(.*?)"#', section, re.S)
    return raw.group(1)


class ObservationContracts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.db = sqlite3.connect(Path(self.temp.name) / "operations.db")
        for path in sorted((ROOT / "crates/panel-db/migrations").glob("*.sql")):
            self.db.executescript(path.read_text())
        self.addCleanup(self.temp.cleanup)
        self.addCleanup(self.db.close)

    def observe(self, identity="fixture", status="queued", progress=0, time=100, operation="create"):
        return self.db.execute(sql("observe_server_backup"), (
            identity, None, "server.backup." + operation, status, progress,
            json.dumps({"external": True, "operation": operation}),
            json.dumps({"message": status, "progress_known": True}),
            "Failure" if status == "failed" else None, 100, time,
        )).fetchone()

    def test_observations_are_monotonic_idempotent_and_reject_operation_collision(self):
        self.assertIsNotNone(self.observe())
        self.assertIsNone(self.observe())
        self.assertIsNotNone(self.observe(status="running", progress=40, time=110))
        self.assertIsNone(self.observe(progress=40, time=120))
        self.assertIsNone(self.observe(status="running", progress=10, time=120))
        self.assertIsNone(self.observe(status="running", progress=50, time=105))
        self.assertIsNotNone(self.observe(status="succeeded", progress=100, time=90))
        self.assertIsNone(self.observe(status="running", progress=50, time=130))
        self.assertIsNone(self.observe(status="failed", progress=100, time=140))
        self.assertIsNone(self.observe(operation="restore", status="succeeded", progress=100))
        self.assertEqual(self.db.execute("SELECT status,progress,updated_at FROM jobs").fetchone(), ("succeeded", 100, 110))

    def test_root_work_is_not_claimed_cancelled_or_marked_interrupted_by_api_restart(self):
        for operation in ("create", "inspect", "restore"):
            self.observe(identity=operation, operation=operation)
        self.assertIsNone(self.db.execute(sql("claim_next_queued"), (200,)).fetchone())
        for identity in ("create", "inspect", "restore"):
            self.assertEqual(self.db.execute(sql("cancel"), (200, identity)).rowcount, 0)
        self.db.execute("UPDATE jobs SET status='running'")
        self.assertEqual(self.db.execute(sql("recover_interrupted_jobs"), (200,)).rowcount, 0)
        self.db.execute("INSERT INTO jobs(id,name,status,progress,payload_json,retry_count,max_retries,created_at,updated_at) VALUES('ordinary','system.echo','queued',0,'{}',0,0,1,1)")
        self.assertEqual(self.db.execute(sql("claim_next_queued"), (200,)).fetchone()[0], "ordinary")
        self.assertEqual(self.db.execute(sql("recover_interrupted_jobs"), (200,)).rowcount, 1)

    def test_root_drain_does_not_wait_for_its_own_observed_task(self):
        tree = ast.parse((ROOT / "scripts/server-backup-worker.py").read_text(encoding="utf-8"))
        query = next(node.value for node in ast.walk(tree) if isinstance(node, ast.Constant) and isinstance(node.value, str) and node.value.startswith("SELECT COUNT(*) FROM jobs WHERE status='running'"))
        self.observe(status="running", progress=25)
        self.assertEqual(self.db.execute(query).fetchone()[0], 0)
        self.db.execute("INSERT INTO jobs(id,name,status,progress,payload_json,retry_count,max_retries,created_at,updated_at) VALUES('ordinary','email.install','running',0,'{}',0,0,1,1)")
        self.assertEqual(self.db.execute(query).fetchone()[0], 1)

    def test_read_only_live_status_filters_the_whole_dataset_before_pagination(self):
        self.observe()
        section = SOURCE.split("fn observed_status_filter", 1)[1]
        pieces = re.findall(r'\.push\("([^"\n]*)"\)', section)[:4]
        self.assertEqual(len(pieces), 4)
        where = "?".join(pieces) + "?"
        params = ("fixture", "server.backup.create", "running", "running")
        query = "SELECT id FROM jobs WHERE 1=1" + where
        self.assertEqual(self.db.execute(query + " LIMIT 1 OFFSET 0", params).fetchall(), [("fixture",)])
        self.assertEqual(self.db.execute(query + " LIMIT 1 OFFSET 1", params).fetchall(), [])
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM jobs WHERE 1=1" + where, params).fetchone()[0], 1)
        self.assertEqual(self.db.execute(query, params[:-1] + ("queued",)).fetchall(), [])
        self.assertEqual(self.db.execute(query.replace("WHERE 1=1", "WHERE user_id=999"), params).fetchall(), [])
        self.assertEqual(self.db.execute("SELECT status,progress FROM jobs").fetchone(), ("queued", 0))

    def test_agent_netlink_only_does_not_broaden_api_privileges(self):
        agent = (ROOT / "templates/systemd/panel-agent.service").read_text()
        api = (ROOT / "templates/systemd/panel-api.service").read_text()
        self.assertIn("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK", agent)
        self.assertIn("CAP_NET_ADMIN", agent)
        self.assertIn("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6\n", api)
        self.assertNotIn("AF_NETLINK", api)
        self.assertIn("User=panel-api", api)


if __name__ == "__main__":
    unittest.main()
