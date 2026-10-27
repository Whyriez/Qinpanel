"""Actual additive branding migration, temporary SQLite, no services/network."""
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import unittest

ROOT = Path(__file__).resolve().parents[2]
MIGRATION = ROOT / "crates/panel-db/migrations/20261026000003_qinpanel_branding.sql"


def database():
    db = sqlite3.connect(":memory:")
    for path in sorted(MIGRATION.parent.glob("*.sql")):
        if path.name < MIGRATION.name:
            db.executescript(path.read_text(encoding="utf-8"))
    return db


class BrandingMigrationTests(unittest.TestCase):
    def test_historical_migration_targets_remain_distinct_from_current_brand(self):
        with closing(database()) as db:
            db.execute("UPDATE notification_settings SET from_name='Nusa Panel'")
            db.execute("INSERT OR REPLACE INTO settings VALUES('panel.title','Nusa Panel',123)")
            db.execute("INSERT OR REPLACE INTO settings VALUES('custom.title','Nusa Panel',123)")
            for filename, expected in [
                ("20261016000001_qpanel_branding.sql", "Qpanel"),
                ("20261024000001_qonpanel_branding.sql", "QonPanel"),
                (MIGRATION.name, "Qinpanel"),
            ]:
                with self.subTest(migration=filename):
                    db.executescript((MIGRATION.parent / filename).read_text(encoding="utf-8"))
                    self.assertEqual(db.execute("SELECT from_name FROM notification_settings").fetchone()[0], expected)
                    self.assertEqual(db.execute("SELECT value FROM settings WHERE key='panel.title'").fetchone()[0], expected)
                    self.assertEqual(db.execute("SELECT value FROM settings WHERE key='custom.title'").fetchone()[0], "Nusa Panel")

    def test_upgrade_changes_only_recognized_defaults_and_is_idempotent(self):
        with closing(database()) as db:
            db.execute("INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES('panel.title','QonPanel',123),('site.title','QonPanel',123),('custom.brand','QonPanel',123)")
            db.execute("UPDATE notification_settings SET from_name='QonPanel',smtp_host='smtp.customer.test',smtp_username='customer',smtp_password_enc='fixture-cipher',updated_at=123")
            before = db.execute("SELECT * FROM notification_settings").fetchone()
            sql = MIGRATION.read_text()
            db.executescript(sql)
            after = db.execute("SELECT * FROM notification_settings").fetchone()
            columns = [row[1] for row in db.execute("PRAGMA table_info(notification_settings)")]
            for index, name in enumerate(columns):
                self.assertEqual(after[index], "Qinpanel" if name == "from_name" else before[index])
            self.assertEqual(dict(db.execute("SELECT key,value FROM settings WHERE key IN ('panel.title','site.title','custom.brand')")), {"panel.title": "Qinpanel", "site.title": "Qinpanel", "custom.brand": "QonPanel"})
            db.executescript(sql)
            self.assertEqual(db.execute("SELECT * FROM notification_settings").fetchone(), after)

    def test_customer_names_and_template_contents_are_preserved(self):
        with closing(database()) as db:
            template = {"panel_name": "Acme Hosting", "subject": "Customer subject", "body": "Custom QonPanel reference", "extra": "keep"}
            db.execute("UPDATE notification_settings SET from_name='Acme Hosting'")
            db.execute("INSERT OR REPLACE INTO settings VALUES('panel.title','Acme Hosting',123)")
            db.execute("INSERT OR REPLACE INTO settings VALUES('email.template',?,123)", (json.dumps(template),))
            db.executescript(MIGRATION.read_text())
            self.assertEqual(db.execute("SELECT from_name FROM notification_settings").fetchone()[0], "Acme Hosting")
            self.assertEqual(db.execute("SELECT value FROM settings WHERE key='panel.title'").fetchone()[0], "Acme Hosting")
            self.assertEqual(json.loads(db.execute("SELECT value FROM settings WHERE key='email.template'").fetchone()[0]), template)

    def test_saved_default_template_changes_only_panel_name(self):
        with closing(database()) as db:
            template = {"panel_name": "QonPanel", "subject": "Custom subject", "body": "Keep message", "extra": [1, 2]}
            db.execute("INSERT OR REPLACE INTO settings VALUES('email.template',?,123)", (json.dumps(template),))
            db.executescript(MIGRATION.read_text())
            actual = json.loads(db.execute("SELECT value FROM settings WHERE key='email.template'").fetchone()[0])
            self.assertEqual(actual, {**template, "panel_name": "Qinpanel"})

    def test_invalid_legacy_template_does_not_block_upgrade(self):
        with closing(database()) as db:
            db.execute("INSERT OR REPLACE INTO settings VALUES('email.template','not-json',123)")
            db.execute("UPDATE notification_settings SET from_name='QonPanel Alerts'")
            db.executescript(MIGRATION.read_text())
            self.assertEqual(db.execute("SELECT value FROM settings WHERE key='email.template'").fetchone()[0], "not-json")
            self.assertEqual(db.execute("SELECT from_name FROM notification_settings").fetchone()[0], "Qinpanel Alerts")


if __name__ == "__main__":
    unittest.main()
