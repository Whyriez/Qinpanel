"""OS/backup/SELinux fixtures. All commands and privileged paths are replaced."""
import copy
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import tarfile
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


backup = module("backup_platform", "server-backup-worker.py")
selinux = module("selinux_platform", "platform-selinux.py")


def manifest(os="ubuntu:24.04"):
    return {"format": "qonpanel-server-1", "os": os, "panel_version": "0.1.0",
            "hostname": "fixture", "arch": "x86_64", "tenants": [],
            "roots": ["etc/panel", "var/lib/panel"], "packages": {},
            "active_units": [], "enabled_units": []}


class PlatformTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "etc").mkdir()
        self.commands = []

    def execute(self, args, **kwargs):
        self.commands.append(args)
        return types.SimpleNamespace(returncode=0, stdout=b"panelctl 0.1.0\n", stderr=b"")

    def test_matrix_normalizes_only_alma_minor_release(self):
        for identifier, release, expected in (("debian", "12", "debian:12"), ("ubuntu", "22.04", "ubuntu:22.04"),
            ("ubuntu", "24.04", "ubuntu:24.04"), ("almalinux", "9.7", "almalinux:9"), ("almalinux", "10.2", "almalinux:10")):
            (self.root / "etc/os-release").write_text(f'ID={identifier}\nVERSION_ID="{release}"\n')
            engine = backup.Engine(self.root, self.execute)
            self.assertEqual(engine.identity()["os"], expected)
            backup.validate_manifest(manifest(expected))
        for identifier, release in (("debian", "13"), ("almalinux", "8.10"), ("rocky", "9"), ("ubuntu", "26.04")):
            (self.root / "etc/os-release").write_text(f"ID={identifier}\nVERSION_ID={release}\n")
            before = len(self.commands)
            with self.assertRaises(ValueError):
                backup.Engine(self.root, self.execute).identity()
            self.assertEqual(len(self.commands), before)

    def test_legacy_format_and_cross_distribution_guard(self):
        engine = backup.Engine(self.root, self.execute)
        engine.identity = lambda: {"os": "debian:12", "arch": "x86_64", "panel_version": "0.1.0"}
        with self.assertRaises(ValueError):
            engine.compatibility(manifest("ubuntu:24.04"))
        self.assertFalse(self.commands)
        engine.compatibility(manifest("debian:12"))
        for os in backup.SUPPORTED_OS:
            backup.validate_manifest(manifest(os))

    def test_package_format_cannot_be_mixed_and_rpm_architecture_is_explicit(self):
        for os, name, filename in (("ubuntu:24.04", "mariadb-server", "mariadb.deb"), ("debian:12", "mariadb-server", "mariadb.deb"), ("almalinux:9", "mariadb-server.x86_64", "mariadb.rpm"), ("almalinux:10", "php83-php-common.aarch64", "php.rpm")):
            value = manifest(os)
            value["packages"] = {name: "0:10.5.29-1"}
            value["package_files"] = {filename: {"package": name, "version": "0:10.5.29-1", "sha256": "a" * 64}}
            backup.validate_manifest(value)
            value["package_files"][filename.rsplit(".", 1)[0] + (".deb" if filename.endswith("rpm") else ".rpm")] = value["package_files"].pop(filename)
            with self.assertRaises(ValueError):
                backup.validate_manifest(value)

    def test_unsigned_or_untrusted_rpm_never_reaches_metadata_query(self):
        for signature in (b"fixture.rpm: digests OK\n", b"fixture.rpm: digests SIGNATURES NOT OK\n", b"fixture.rpm: RSA NOKEY\n"):
            calls = []
            def execute(args, **kwargs):
                calls.append(args)
                return types.SimpleNamespace(returncode=0, stdout=signature)
            with self.assertRaises(ValueError):
                backup.Engine(self.root, execute).package_info(Path("fixture.rpm"))
            self.assertEqual(len(calls), 1)

    def test_restore_checks_source_signatures_in_isolated_database_before_global_import(self):
        engine = backup.Engine(self.root, self.execute)
        engine.work.mkdir(parents=True)
        archive = engine.work / "keys.tar"
        with tarfile.open(archive, "w") as stream:
            key = b"fixture public repository key only"
            member = tarfile.TarInfo("etc/pki/rpm-gpg/QONPANEL-REMI")
            member.size = len(key)
            stream.addfile(member, io.BytesIO(key))
        engine.prepare_rpm_trust(archive)
        self.assertEqual(len(engine.rpm_keys), 1)
        self.assertTrue(all("--dbpath" in args for args in self.commands if "--import" in args))
        self.assertTrue(engine.rpm_trust.is_dir())
        self.assertEqual(engine.rpm_keys[0].read_bytes(), key)

    def test_cached_exact_rpm_is_reused_and_timer_stops_before_writers(self):
        engine = backup.Engine(self.root, self.execute)
        engine.work.mkdir(parents=True)
        cached = self.root / "var/cache/dnf/fixture/packages/mariadb-server-10.5-1.x86_64.rpm"
        cached.parent.mkdir(parents=True)
        cached.write_bytes(b"fixture signed RPM")
        engine.package_info = lambda path: ("mariadb-server", "0:10.5-1", "x86_64")
        value = manifest("almalinux:9")
        value["packages"] = {"mariadb-server.x86_64": "0:10.5-1"}
        engine.collect_packages(value)
        self.assertEqual(len(value["package_files"]), 1)
        self.assertFalse(any(args[0] == "dnf" for args in self.commands))
        self.commands.clear()
        engine.stop(["mariadb.service", "site-1-cron-2.service", "site-1-cron-2.timer"])
        self.assertEqual(self.commands[0], ["systemctl", "stop", "site-1-cron-2.timer"])
        self.assertFalse(backup.unit_allowed("unmanaged.timer"))

    def test_restored_parallel_php_stays_masked_across_reboot_and_masks_roll_back(self):
        engine = backup.Engine(self.root, self.execute)
        engine.identity = lambda: {"os": "almalinux:10", "arch": "x86_64", "panel_version": "0.1.0"}
        binary = self.root / "opt/remi/php83/root/usr/sbin/php-fpm"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"fixture executable only")
        journal = {}
        engine.ensure_php_masks(journal)
        self.assertEqual(journal["php_masks_added"], ["php83-php-fpm.service"])
        self.assertIn(["systemctl", "mask", "php83-php-fpm.service"], self.commands)
        engine.clear_php_masks(journal)
        self.assertIn(["systemctl", "unmask", "php83-php-fpm.service"], self.commands)
        self.assertEqual(backup.load_json(engine.base / "journal.json")["php_masks_added"], [])
        before = len(self.commands)
        with self.assertRaises(ValueError):
            engine.clear_php_masks({"php_masks_added": ["sshd.service"]})
        self.assertEqual(len(self.commands), before)

    def test_rpm_transaction_preserves_operator_masks_and_cleans_failure_masks(self):
        calls = []
        def execute(args, **kwargs):
            calls.append(args)
            if args[:3] == ["systemctl", "is-enabled", "nginx.service"]:
                return types.SimpleNamespace(returncode=1, stdout=b"masked\n")
            if args[0] == "dnf" and "install" in args:
                raise RuntimeError("fixture package failure")
            return types.SimpleNamespace(returncode=1 if args[0] == "rpm" else 0, stdout=b"")
        engine = backup.Engine(self.root, execute)
        engine.units = lambda: ["nginx.service", "mariadb.service"]
        backup.save_json(engine.base / "journal.json", {"package_masks": []})
        value = manifest("almalinux:9")
        value["packages"] = {"mariadb-server.x86_64": "0:10.5-1"}
        value["package_files"] = {"mariadb.rpm": {"package": "mariadb-server.x86_64", "version": "0:10.5-1", "sha256": "a" * 64}}
        with patch.dict(sys.modules, {"rpm": types.SimpleNamespace(labelCompare=lambda a, b: 0)}):
            with self.assertRaises(RuntimeError):
                engine.prepare_packages(value)
        self.assertIn(["systemctl", "mask", "--runtime", "mariadb.service"], calls)
        self.assertIn(["systemctl", "unmask", "--runtime", "mariadb.service"], calls)
        self.assertNotIn(["systemctl", "unmask", "--runtime", "nginx.service"], calls)
        self.assertEqual(backup.load_json(engine.base / "journal.json")["package_masks"], [])


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "etc/panel").mkdir(parents=True)
        (self.root / "etc/os-release").write_text('ID="almalinux"\nVERSION_ID="9.7"\n')
        (self.root / "etc/panel/public-port").write_text("17890\n")
        (self.root / "sys/fs/selinux").mkdir(parents=True)
        (self.root / "sys/fs/selinux/enforce").write_text("1")
        self.contexts = {}
        self.booleans = {name: "off" for name in selinux.BOOLEANS}
        self.persistent_booleans = {}
        self.ports = set()
        self.commands = []
        store = types.SimpleNamespace(read=lambda: copy.deepcopy(self.persistent_booleans), remove=self.remove_boolean)
        self.policy = selinux.Policy(self.root, self.execute, store)

    def remove_boolean(self, name):
        self.persistent_booleans.pop(name)
        self.booleans[name] = "off"  # Native delete returns to policy default.

    def execute(self, args):
        self.commands.append(args)
        if args == ["semanage", "export"]:
            return "\n".join([f"fcontext -a -t {context} '{pattern}'" for pattern, context in self.contexts.items()] + [f"port -a -t http_port_t -p tcp {port}" for port in self.ports])
        if args[:2] == ["semanage", "fcontext"]:
            if args[2] == "-a":
                self.contexts[args[-1]] = args[4]
            else:
                self.contexts.pop(args[-1])
        if args[0] == "getsebool":
            return args[1] + " --> " + self.booleans[args[1]]
        if args[0] == "setsebool":
            if args[1] == "-P":
                self.booleans[args[2]] = args[3]
                self.persistent_booleans[args[2]] = args[3]
            else:
                self.booleans[args[1]] = args[2]
        if args == ["semanage", "port", "-l"]:
            return "http_port_t tcp 80, 443" + (", 17890" if self.ports else "")
        if args[:3] == ["semanage", "port", "-a"]:
            self.ports.add(args[-1])
        if args[:3] == ["semanage", "port", "-d"]:
            self.ports.remove(args[-1])
        return ""

    def test_apply_rollback_restores_existing_policy_without_disabling_enforcement(self):
        preserved = next(iter(selinux.CONTEXTS))
        self.contexts[preserved] = selinux.CONTEXTS[preserved]
        self.booleans[selinux.BOOLEANS[0]] = "on"
        self.booleans["unrelated_operator_boolean"] = "on"
        self.persistent_booleans["unrelated_operator_boolean"] = "off"
        original = copy.deepcopy((self.contexts, self.booleans, self.ports, self.persistent_booleans))
        self.policy.apply()
        self.assertTrue(self.policy.file.exists())
        self.assertEqual(set(self.contexts), set(selinux.CONTEXTS))
        self.policy.rollback()
        self.assertEqual((self.contexts, self.booleans, self.ports, self.persistent_booleans), original)
        self.assertEqual((self.root / "sys/fs/selinux/enforce").read_text(), "1")
        self.assertFalse(any(args[0] == "setenforce" for args in self.commands))
        self.assertFalse(self.policy.file.exists())

    def test_runtime_and_persistent_boolean_states_are_restored_separately(self):
        first, second = selinux.BOOLEANS
        self.booleans[first] = "on"
        self.persistent_booleans[first] = "off"
        self.booleans[second] = "off"
        self.persistent_booleans[second] = "on"
        original = copy.deepcopy((self.booleans, self.persistent_booleans))
        self.policy.apply()
        self.assertTrue(all(value == "on" for value in self.booleans.values()))
        self.assertTrue(all(value == "on" for value in self.persistent_booleans.values()))
        self.policy.rollback()
        self.assertEqual((self.booleans, self.persistent_booleans), original)

    def test_custom_rule_is_preserved_and_forged_journal_cannot_execute(self):
        preserved = next(iter(selinux.CONTEXTS))
        self.contexts[preserved] = "custom_t"
        with self.assertRaises(ValueError):
            self.policy.apply()
        self.policy.rollback()
        self.assertEqual(self.contexts[preserved], "custom_t")
        self.policy.save([["context", "/etc(/.*)?"]])
        before = len(self.commands)
        with self.assertRaises(ValueError):
            self.policy.rollback()
        self.assertEqual(len(self.commands), before)


if __name__ == "__main__":
    unittest.main()
