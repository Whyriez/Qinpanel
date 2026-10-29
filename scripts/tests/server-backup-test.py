"""Hermetic full-server migration fixtures. Never invoke host services/agent.

The Engine's filesystem and all privileged commands/encryption are replaced.
A separate optional GPG test uses only temporary files and a private homedir.
"""
import copy
import hashlib
import importlib.util
import io
import json
import shutil
from pathlib import Path
import sqlite3
import tarfile
import tempfile
import types
import unittest
import uuid
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("server_backup", REPO / "scripts/server-backup-worker.py")
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
PASSWORD = "fixture backup passphrase only"


class FixtureEngine(worker.Engine):
    def __init__(self, root, manifest=None):
        self.commands = []
        self.failure = None
        self.archive_links = {}
        super().__init__(root, self.fake_command)
        self.base.mkdir(parents=True)
        self.work.mkdir()
        self.manifest = manifest
        self.run = {"id": str(uuid.uuid4()), "operation": "create"}

    def fake_command(self, args, **kwargs):
        self.commands.append((args, kwargs))
        if self.failure and args[:len(self.failure)] == self.failure:
            self.failure = None
            raise RuntimeError("Fixture command failed.")
        return types.SimpleNamespace(returncode=0, stdout=b"", stderr=b"")

    def identity(self):
        return {"os": "ubuntu:24.04", "arch": "x86_64", "panel_version": "0.1.0"}

    def inventory(self):
        if self.manifest:
            return copy.deepcopy(self.manifest)
        return manifest([name for name in ["etc/panel", "var/lib/panel", "etc/nginx"] if self.path(name).exists()], [])

    def units(self):
        return ["nginx.service", "panel-api.service", "panel-agent.service"]

    def collect_packages(self,value):
        directory=self.work / "qonpanel-packages"
        directory.mkdir()
        files={}
        for name,release in value["packages"].items():
            path=directory / (name.replace(":","-")+".deb")
            path.write_text(json.dumps({"package":name,"version":release,"arch":"amd64"}))
            files[path.name]={"package":name,"version":release,"sha256":worker.digest(path)}
        value["package_files"]=files

    def package_info(self,path):
        value=json.loads(path.read_text())
        return value["package"],value["version"],value["arch"]

    def pack(self, output, value,portable=False):
        with tarfile.open(output, "w") as archive:
            raw = json.dumps(value).encode()
            header = tarfile.TarInfo("qonpanel-manifest.json")
            header.size = len(raw)
            archive.addfile(header, io.BytesIO(raw))
            if portable and value.get("package_files"):
                archive.add(self.work / "qonpanel-packages",arcname="qonpanel-packages")
            for root in value["roots"]:
                if root in self.archive_links:
                    member = tarfile.TarInfo(root)
                    member.type = tarfile.SYMTYPE
                    member.linkname = self.archive_links[root]
                    archive.addfile(member)
                else:
                    archive.add(self.path(root), arcname=root)

    def extract(self, source, roots):
        # Fixtures contain only regular files/directories. Copy their bytes
        # explicitly: older distro Python builds have no tar extraction filter.
        # Production still validates archives and extracts with GNU tar.
        root = self.root.resolve()
        with tarfile.open(source) as archive:
            for member in archive:
                name = worker.safe_member(member.name)
                if not worker.allowed_path(name, roots):
                    continue
                destination = (root / name).resolve()
                if not destination.is_relative_to(root):
                    raise ValueError("Fixture extraction escapes its temporary root.")
                if member.isdir():
                    destination.mkdir(parents=True, exist_ok=True)
                elif member.isfile():
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with archive.extractfile(member) as data, destination.open("wb") as output:
                        shutil.copyfileobj(data, output)
                else:
                    raise ValueError("Fixture extraction accepts only files/directories.")

    def crypt(self, source, destination, passphrase, decrypt=False):
        # Deliberately a fixture codec; not evidence of production encryption.
        key = hashlib.sha256(passphrase.encode()).digest()
        data = source.read_bytes()
        if decrypt:
            if not data.startswith(b"FIXTURE" + key):
                raise RuntimeError("Fixture passphrase mismatch.")
            destination.write_bytes(data[39:])
        else:
            destination.write_bytes(b"FIXTURE" + key + data)


def manifest(roots, tenants):
    return {"format": "qonpanel-server-1", "os": "ubuntu:24.04", "arch": "x86_64",
            "panel_version": "0.1.0", "created_at": 1, "hostname": "old-server",
            "roots": roots, "tenants": tenants, "packages": {"mariadb-server": "1:10.11.8-1"},
            "active_units": ["panel-api.service", "panel-agent.service", "nginx.service"],
            "enabled_units": ["panel-api.service", "panel-agent.service", "nginx.service"]}


def seed(root, source=False):
    (root / "etc/panel").mkdir(parents=True)
    (root / "etc/panel/secret.key").write_bytes(b"s" * 32 if source else b"t" * 32)
    (root / "etc/panel/credentials.json").write_text("source-encrypted-settings" if source else "fresh-settings")
    (root / "etc/nginx").mkdir()
    (root / "etc/nginx/panel.conf").write_text("source-vhost" if source else "fresh-vhost")
    (root / "var/lib/panel").mkdir(parents=True)
    db = sqlite3.connect(root / "var/lib/panel/panel.db")
    db.executescript("CREATE TABLE users(id INTEGER PRIMARY KEY,username TEXT,role TEXT,linux_username TEXT,linux_uid INTEGER,password_hash TEXT,totp_secret TEXT); CREATE TABLE sites(id INTEGER); CREATE TABLE databases(id INTEGER); CREATE TABLE sessions(id TEXT); CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT,updated_at INTEGER); CREATE TABLE endpoint_evidence(id INTEGER);")
    db.execute("INSERT INTO users VALUES(1,?,?,NULL,NULL,?,?)", ("source-admin" if source else "new-admin", "admin", "password-hash", "totp-secret"))
    db.execute("INSERT INTO sessions VALUES('stale-source-session')")
    db.execute("INSERT INTO endpoint_evidence VALUES(1)")
    if source:
        db.execute("INSERT INTO users VALUES(2,'alice','user','p_alice',10000,'tenant-password','tenant-totp')")
        db.execute("INSERT INTO sites VALUES(1)")
        db.execute("INSERT INTO databases VALUES(1)")
        (root / "home/p_alice/site").mkdir(parents=True)
        (root / "home/p_alice/site/index.php").write_text("customer application")
        (root / "var/lib/mysql").mkdir()
        (root / "var/lib/mysql/tenant-data").write_bytes(b"database contents and hashed SQL accounts")
        (root / "opt/panel/runtimes/node/22").mkdir(parents=True)
        (root / "opt/panel/runtimes/node/22/node").write_bytes(b"runtime fixture")
        (root / "var/lib/panel/mail").mkdir()
        (root / "var/lib/panel/mail/message").write_text("mailbox content")
    db.commit()
    db.close()
    with sqlite3.connect(root / "var/lib/panel/panel.operations.db") as operations:
        operations.execute("CREATE TABLE jobs(id TEXT,status TEXT,error_message TEXT,updated_at INTEGER)")
        if source:
            operations.executemany("INSERT INTO jobs VALUES(?,?,NULL,1)",[("queued-old","queued"),("running-old","running")])
    operations.close()


class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.target = self.root / "target"
        self.source.mkdir(); self.target.mkdir()
        seed(self.source, True); seed(self.target)
        self.tenant = {"name": "p_alice", "uid": 10000, "gid": 10000, "group": "p_alice", "shell": "/bin/bash", "password": "!"}
        self.metadata = manifest(["etc/panel", "etc/nginx", "var/lib/panel", "home/p_alice", "var/lib/mysql", "opt/panel"], [self.tenant])

    def tearDown(self):
        self.temporary.cleanup()

    def exported(self):
        engine = FixtureEngine(self.source, self.metadata)
        request = {"id": str(uuid.uuid4()), "passphrase": PASSWORD}
        engine.create(request)
        return engine.base / "exports" / (request["id"] + ".qpb")

    def uploaded(self):
        exported = self.exported()
        engine = FixtureEngine(self.target)
        incoming = engine.path(worker.INCOMING)
        incoming.mkdir()
        archive_id = str(uuid.uuid4())
        (incoming / (archive_id + ".qpb")).write_bytes(exported.read_bytes())
        return engine, {"id": str(uuid.uuid4()), "archive_id": archive_id, "passphrase": PASSWORD}

    def test_restore_warns_when_managed_object_data_is_not_included(self):
        self.metadata["managed_storage_omitted"] = True
        engine, request = self.uploaded()
        engine.inspect(request)
        report = worker.load_json(engine.base / "inspection.json")
        self.assertTrue(any("Storage Qinpanel" in warning for warning in report["warnings"]))

    def test_restore_target_with_managed_storage_is_not_a_fresh_server(self):
        engine = FixtureEngine(self.target)
        directory = engine.path("var/lib/qonpanel-storage")
        directory.mkdir(parents=True)
        (directory / "volume.dat").write_bytes(b"keep native objects")
        with self.assertRaises(ValueError):
            engine.fresh_target()
        self.assertEqual((directory / "volume.dat").read_bytes(), b"keep native objects")

    def test_real_backup_pack_omits_s3_origin_without_its_engine_certificate(self):
        engine = FixtureEngine(self.source, self.metadata)
        calls = []
        output = engine.work / "pack-fixture.tar"
        def execute(args, **options):
            calls.append(args)
            output.write_bytes(b"tar fixture")
        engine.execute = execute
        worker.Engine.pack(engine, output, self.metadata)
        self.assertIn("--exclude=etc/nginx/conf.d/qonpanel-object-storage.conf", calls[0])
        self.assertIn("etc/nginx", calls[0])

    def test_full_roundtrip_keeps_accounts_secrets_applications_mail_and_databases(self):
        engine, request = self.uploaded()
        _, _, value, _ = engine.inspect(request)
        self.assertEqual(value["tenants"][0]["uid"], 10000)
        proof = worker.load_json(engine.base / "inspection.json")
        self.assertNotIn("password", json.dumps(proof))
        request.update(digest=proof["digest"], new_ipv4="203.0.113.10")
        # Start a separate worker lifetime: previous inspect staging is cleaned.
        import shutil
        shutil.rmtree(engine.work); engine.work.mkdir()
        worker.save_json(engine.base / "approval.json",{"digest":request["digest"],"expires_at":int(worker.time.time())+86400})
        with patch.object(tarfile.TarFile, "extract", side_effect=AssertionError("Do not use the version-dependent extraction API")):
            engine.restore(request)
        self.assertEqual((self.target / "home/p_alice/site/index.php").read_text(), "customer application")
        self.assertEqual((self.target / "etc/panel/secret.key").read_bytes(), b"s" * 32)
        self.assertEqual((self.target / "var/lib/panel/mail/message").read_text(), "mailbox content")
        self.assertTrue((self.target / "var/lib/mysql/tenant-data").is_file())
        self.assertTrue((self.target / "opt/panel/runtimes/node/22/node").is_file())
        with engine.database() as db:
            self.assertEqual(db.execute("SELECT username,password_hash,totp_secret FROM users WHERE id=1").fetchone(), ("source-admin", "password-hash", "totp-secret"))
            self.assertEqual(db.execute("SELECT COUNT(*) FROM sessions").fetchone()[0], 0)
            self.assertEqual(db.execute("SELECT COUNT(*) FROM endpoint_evidence").fetchone()[0], 0)
            self.assertEqual(db.execute("SELECT value FROM settings WHERE key='network.public_ipv4'").fetchone()[0], "203.0.113.10")
        commands = [call[0] for call in engine.commands]
        self.assertIn(["nginx", "-t"], commands)
        self.assertFalse((engine.base / "journal.json").exists())
        with sqlite3.connect(self.target / "var/lib/panel/panel.operations.db") as operations:
            self.assertEqual(operations.execute("SELECT DISTINCT status FROM jobs").fetchall(),[("failed",)])
        operations.close()
        self.assertTrue(any(command[:2]==["apt-mark","hold"] for command in commands))
        install=next(command for command in commands if command[0]=="apt-get" and "install" in command)
        self.assertTrue(any(argument.endswith(".deb") for argument in install))
        self.assertFalse(any(argument.startswith("mariadb-server=") for argument in install))

    def test_recovery_does_not_delete_data_when_safety_snapshot_is_corrupt(self):
        engine,request=self.uploaded();engine.inspect(request)
        report=worker.load_json(engine.base / "inspection.json");request["digest"]=report["digest"]
        import shutil
        shutil.rmtree(engine.work);engine.work.mkdir()
        engine.failure=["nginx","-t"]
        worker.save_json(engine.base / "approval.json",{"digest":request["digest"],"expires_at":int(worker.time.time())+86400})
        with self.assertRaises(RuntimeError):engine.restore(request)
        before=(self.target / "etc/nginx/panel.conf").read_bytes()
        (engine.base / "safety.tar").write_bytes(b"corrupt fixture")
        with self.assertRaisesRegex(ValueError,"tidak utuh"):engine.recover()
        self.assertEqual((self.target / "etc/nginx/panel.conf").read_bytes(),before)
        self.assertTrue((engine.base / "journal.json").exists())

    def test_runtime_units_and_tenant_slices_use_actual_panel_names(self):
        for name in ["site-12.service","site-site_1.service","panel-fpm@8.4-p_alice.service","panel-u10000.slice"]:
            self.assertTrue(worker.unit_allowed(name),name)
        for name in ["panel-u0.slice","ssh.service","site-../../root.service"]:
            self.assertFalse(worker.unit_allowed(name),name)

    def test_sftp_identity_is_preserved_and_nonpanel_groups_are_rejected(self):
        value=copy.deepcopy(self.metadata)
        value["tenants"][0].update(shell="/usr/lib/openssh/sftp-server",groups=["panel-tenants","panel-sftp"])
        self.assertEqual(worker.validate_manifest(value)["tenants"][0]["shell"],"/usr/lib/openssh/sftp-server")
        value["tenants"][0]["groups"].append("sudo")
        with self.assertRaises(ValueError):worker.validate_manifest(value)

    def test_privileged_restore_needs_out_of_band_root_hash_approval(self):
        engine,request=self.uploaded();engine.inspect(request)
        request["digest"]=worker.load_json(engine.base / "inspection.json")["digest"]
        with self.assertRaisesRegex(ValueError,"mengesahkan"):
            engine.restore(request)
        worker.save_json(engine.base / "approval.json",{"digest":"a"*64,"expires_at":int(worker.time.time())+86400})
        with self.assertRaisesRegex(ValueError,"tidak sesuai"):
            engine.restore(request)
        self.assertFalse(any(call[0][0] in ("dpkg-deb","apt-get","useradd") for call in engine.commands))
        self.assertEqual((self.target / "etc/nginx/panel.conf").read_text(),"fresh-vhost")

    def test_restore_refuses_nonempty_server_before_changes(self):
        engine, request = self.uploaded()
        with engine.database() as db:
            db.execute("INSERT INTO sites VALUES(99)")
        with self.assertRaisesRegex(ValueError, "instalasi baru"):
            engine.inspect(request)
        self.assertEqual((self.target / "etc/nginx/panel.conf").read_text(), "fresh-vhost")
        self.assertFalse(any(call[0][:2] == ["systemctl", "stop"] for call in engine.commands))

    def test_wrong_passphrase_never_changes_target(self):
        engine, request = self.uploaded()
        request["passphrase"] = "wrong backup passphrase"
        with self.assertRaises(RuntimeError):
            engine.inspect(request)
        self.assertEqual((self.target / "etc/panel/secret.key").read_bytes(), b"t" * 32)

    def test_preflight_rejects_newer_panel_different_os_arch_and_stale_digest(self):
        engine = FixtureEngine(self.target)
        for field, value in [("panel_version", "99.0.0"), ("arch", "aarch64"), ("os", "ubuntu:22.04")]:
            metadata = copy.deepcopy(self.metadata); metadata[field] = value
            with self.assertRaises(ValueError): engine.compatibility(metadata)
        worker.save_json(engine.base / "inspection.json", {"archive_id":str(uuid.uuid4()),"digest":"a"*64,"checked_at":0})
        worker.save_json(engine.base / "approval.json",{"digest":"a"*64,"expires_at":int(worker.time.time())+86400})
        with self.assertRaisesRegex(ValueError, "Periksa ulang"):
            engine.restore({"archive_id":str(uuid.uuid4()),"digest":"a"*64})

    def test_tar_traversal_symlink_devices_duplicates_and_missing_key_are_rejected(self):
        engine = FixtureEngine(self.target)
        for case in ["traversal", "symlink", "device", "duplicate", "missing-key"]:
            archive_path = self.root / (case + ".tar")
            with tarfile.open(archive_path, "w") as archive:
                data = json.dumps(self.metadata).encode(); member = tarfile.TarInfo("qonpanel-manifest.json"); member.size=len(data); archive.addfile(member,io.BytesIO(data))
                name={"traversal":"../../etc/shadow","symlink":"etc/panel/link","device":"etc/panel/device","duplicate":"etc/panel/file","missing-key":"etc/panel/file"}[case]
                member=tarfile.TarInfo(name)
                if case=="symlink":member.type=tarfile.SYMTYPE;member.linkname="/etc/shadow"
                if case=="device":member.type=tarfile.CHRTYPE
                archive.addfile(member)
                if case=="duplicate":archive.addfile(member)
            with self.assertRaises(ValueError, msg=case):engine.validate_archive(archive_path)

    def test_fixture_extraction_rejects_traversal_links_and_special_files(self):
        engine = FixtureEngine(self.target)
        for name, kind in [("etc/panel/../../escape", tarfile.REGTYPE),
                           ("etc/panel/link", tarfile.SYMTYPE),
                           ("etc/panel/hardlink", tarfile.LNKTYPE),
                           ("etc/panel/device", tarfile.CHRTYPE)]:
            archive_path = self.root / "unsafe-fixture.tar"
            with tarfile.open(archive_path, "w") as archive:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.linkname = "etc/panel/secret.key"
                archive.addfile(member)
            with self.assertRaises(ValueError):
                engine.extract(archive_path, ["etc/panel"])
        self.assertFalse((self.target / "escape").exists())
        self.assertEqual((self.target / "etc/panel/secret.key").read_bytes(), b"t" * 32)

    def test_invalid_identity_packages_and_host_roots_fail_closed(self):
        for mutation in [lambda m:m["roots"].append("etc/shadow"),lambda m:m["tenants"][0].update(uid=0),lambda m:m["packages"].update({"mariadb;id":"1"}),lambda m:m["enabled_units"].append("ssh.service")]:
            metadata=copy.deepcopy(self.metadata);mutation(metadata)
            with self.assertRaises(ValueError):worker.validate_manifest(metadata)
        # Username::from_slug also permits a leading underscore after p_.
        metadata=copy.deepcopy(self.metadata)
        metadata["tenants"][0].update(name="p__alice",group="p__alice")
        metadata["roots"]=[root.replace("home/p_alice","home/p__alice") for root in metadata["roots"]]
        worker.validate_manifest(metadata)

    def test_capture_failure_restarts_original_services_without_publishing_backup(self):
        engine = FixtureEngine(self.source, self.metadata)
        engine.pack = lambda *_,**_kwargs: (_ for _ in ()).throw(RuntimeError("fixture disk full"))
        with self.assertRaises(RuntimeError):engine.create({"id":str(uuid.uuid4()),"passphrase":PASSWORD})
        self.assertTrue(any(call[0]==["systemctl","start","panel-api.service"] for call in engine.commands))
        self.assertFalse((engine.base / "catalog.json").exists())

    def test_backup_with_dangling_nginx_module_alias_is_published_and_inspectable(self):
        alias = "usr/share/nginx/modules"
        self.metadata["roots"].append(alias)
        engine = FixtureEngine(self.source, self.metadata)
        engine.archive_links[alias] = "../../lib/nginx/modules"
        request = {"id": str(uuid.uuid4()), "passphrase": PASSWORD}
        self.assertFalse(engine.path("usr/lib/nginx/modules").exists())
        engine.create(request)
        output = engine.base / "exports" / (request["id"] + ".qpb")
        self.assertTrue(output.is_file())
        self.assertEqual(engine.run["progress"], 75)
        self.assertTrue(any(call[0] == ["systemctl", "start", "nginx.service"] for call in engine.commands))
        self.assertFalse((engine.base / "maintenance").exists())
        self.assertFalse((engine.base / "journal.json").exists())
        # Inspection on a fresh target must apply the same archive policy.
        target = FixtureEngine(self.target)
        incoming = target.path(worker.INCOMING)
        incoming.mkdir()
        archive_id = str(uuid.uuid4())
        (incoming / (archive_id + ".qpb")).write_bytes(output.read_bytes())
        _, plain, value, _ = target.inspect({"archive_id": archive_id, "passphrase": PASSWORD})
        self.assertIn(alias, value["roots"])
        self.assertNotIn("usr/lib/nginx/modules", value["roots"])
        with tarfile.open(plain) as archive:
            link = archive.getmember(alias)
            self.assertTrue(link.issym())
            self.assertEqual(link.linkname, "../../lib/nginx/modules")
        self.assertEqual((self.target / "etc/panel/secret.key").read_bytes(), b"t" * 32)

    def nginx_link_archive(self, target, kind=tarfile.SYMTYPE, child=False, name="usr/share/nginx/modules"):
        value = copy.deepcopy(self.metadata)
        value["packages"] = {}
        value["roots"].append("usr/share/nginx/modules")
        archive_path = self.root / "nginx-link.tar"
        with tarfile.open(archive_path, "w") as archive:
            raw = json.dumps(value).encode()
            header = tarfile.TarInfo("qonpanel-manifest.json")
            header.size = len(raw)
            archive.addfile(header, io.BytesIO(raw))
            for root in self.metadata["roots"]:
                archive.add(self.source / root, arcname=root)
            link = tarfile.TarInfo(name)
            link.type = kind
            link.linkname = target
            archive.addfile(link)
            if child:
                archive.addfile(tarfile.TarInfo("usr/share/nginx/modules/injected.so"))
        return archive_path

    def test_invalid_nginx_alias_restarts_services_without_publishing_backup(self):
        alias = "usr/share/nginx/modules"
        self.metadata["roots"].append(alias)
        engine = FixtureEngine(self.source, self.metadata)
        engine.archive_links[alias] = "/etc/shadow"
        with self.assertRaisesRegex(ValueError, "Symlink/hardlink keluar"):
            engine.create({"id": str(uuid.uuid4()), "passphrase": PASSWORD})
        self.assertEqual(engine.run["progress"], 55)
        self.assertTrue(any(call[0] == ["systemctl", "start", "nginx.service"] for call in engine.commands))
        self.assertTrue(any(call[0] == ["systemctl", "start", "panel-api.service"] for call in engine.commands))
        self.assertFalse((engine.base / "catalog.json").exists())
        self.assertFalse((engine.base / "exports").exists())
        self.assertFalse((engine.base / "maintenance").exists())
        self.assertFalse((engine.base / "journal.json").exists())

    def test_standard_nginx_module_symlinks_allow_absent_targets_without_dereferencing(self):
        engine = FixtureEngine(self.target)
        for target in ("../../lib/nginx/modules", "/usr/lib/nginx/modules",
                       "../../lib64/nginx/modules", "/usr/lib64/nginx/modules"):
            with self.subTest(target=target):
                engine.validate_archive(self.nginx_link_archive(target))
        self.assertFalse(engine.path("usr/lib/nginx/modules").exists())
        self.assertFalse(engine.path("usr/lib64/nginx/modules").exists())

    def test_nginx_module_exception_rejects_other_links_hardlinks_and_child_writes(self):
        engine = FixtureEngine(self.target)
        for target, kind, child, name in [
            ("/etc/shadow", tarfile.SYMTYPE, False, "usr/share/nginx/modules"),
            ("/usr/lib/nginx/modules-extra", tarfile.SYMTYPE, False, "usr/share/nginx/modules"),
            ("../../../../usr/lib/nginx/modules", tarfile.SYMTYPE, False, "usr/share/nginx/modules"),
            ("usr/lib/nginx/modules", tarfile.LNKTYPE, False, "usr/share/nginx/modules"),
            ("/usr/lib/nginx/modules", tarfile.SYMTYPE, False, "etc/nginx/custom-link"),
            ("/usr/lib/nginx/modules", tarfile.SYMTYPE, True, "usr/share/nginx/modules"),
        ]:
            with self.subTest(target=target, kind=kind, child=child, name=name):
                with self.assertRaises(ValueError):
                    engine.validate_archive(self.nginx_link_archive(target, kind, child, name))
        self.assertEqual((self.target / "etc/panel/secret.key").read_bytes(), b"t" * 32)

    def test_health_failure_can_restore_target_snapshot_without_merging_databases(self):
        engine, request = self.uploaded();engine.inspect(request)
        report=worker.load_json(engine.base / "inspection.json");request["digest"]=report["digest"]
        import shutil
        shutil.rmtree(engine.work);engine.work.mkdir()
        engine.failure=["nginx","-t"]
        worker.save_json(engine.base / "approval.json",{"digest":request["digest"],"expires_at":int(worker.time.time())+86400})
        with self.assertRaises(RuntimeError):engine.restore(request)
        engine.recover()
        self.assertEqual((self.target / "etc/nginx/panel.conf").read_text(), "fresh-vhost")
        self.assertEqual((self.target / "etc/panel/secret.key").read_bytes(), b"t"*32)
        self.assertFalse((self.target / "var/lib/mysql").exists())
        self.assertFalse((self.target / "home/p_alice").exists())
        self.assertFalse((engine.base / "journal.json").exists())


if __name__ == "__main__":
    unittest.main()
