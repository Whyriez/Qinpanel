#!/usr/bin/python3
"""Qinpanel managed-server migration. Invoked only by a fixed root systemd unit.

No shell evaluation, API paths, OS image cloning or arbitrary command arguments.
Engine accepts an alternate filesystem/runner only for imported hermetic tests.
"""
import contextlib
import hashlib
import ipaddress
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import tarfile
import time
import uuid

STATE = "/var/lib/qonpanel-migration"
INCOMING = "/var/lib/panel/server-transfers"
UNIT = "qonpanel-server-backup.service"
MAX_BYTES = 512 * 1024**3
MAX_ENTRIES = 2_000_000
ROOTS = (
    "etc/panel", "var/lib/panel", "etc/nginx", "etc/php", "opt/panel",
    "usr/share/nginx/modules-available", "usr/lib/nginx/modules",
    "etc/mysql", "var/lib/mysql", "etc/postgresql", "var/lib/postgresql",
    "etc/redis", "var/lib/redis", "etc/postfix", "etc/dovecot", "etc/opendkim",
    "etc/opendkim.conf", "etc/letsencrypt", "var/mail", "var/vmail",
    "var/spool/cron/crontabs", "etc/apt/sources.list.d", "etc/apt/keyrings",
    "usr/local/bin/uv", "usr/local/bin/uvx", "usr/local/bin/cloudflared", "usr/bin/cloudflared",
    "etc/apt/trusted.gpg.d", "etc/apt/trusted.gpg", "usr/share/keyrings",
    "etc/ssh/sshd_config.d/50-panel.conf",
    "etc/opt/remi", "opt/remi", "var/opt/remi", "etc/my.cnf", "etc/my.cnf.d",
    "var/lib/pgsql", "etc/yum.repos.d", "etc/pki/rpm-gpg",
    "etc/dnf/plugins/versionlock.list", "etc/dnf/plugins/versionlock.conf",
    "var/spool/cron",
    "usr/lib64/nginx/modules", "usr/share/nginx/modules",
    "etc/alternatives/my.cnf", "etc/phpMyAdmin", "etc/phpmyadmin",
    "var/lib/phpmyadmin", "usr/share/adminer",
    "usr/share/panel/db-web", "var/lib/panel-db-web",
    "etc/systemd/system/panel-db-web.service", "usr/lib/tmpfiles.d/panel-db-web.conf",
)
QUEUE_ROOTS = tuple("var/spool/postfix/" + name for name in (
    "active", "bounce", "corrupt", "defer", "deferred", "flush", "hold",
    "incoming", "maildrop", "saved", "trace"))
# Nginx packages can ship this alias before any dynamic module is installed.
# Preserve that dangling symlink; do not follow it or allow arbitrary OS links.
NGINX_MODULE_ALIAS = "usr/share/nginx/modules"
NGINX_MODULE_TARGETS = {"usr/lib/nginx/modules", "usr/lib64/nginx/modules"}
SYSTEM_USERS = {"root", "panel-api", "www-data", "mysql", "postgres", "redis",
                "postfix", "postdrop", "dovecot", "dovenull", "opendkim", "nobody", "nginx", "apache", "valkey"}
UNIT_PATTERN = re.compile(r"(?:site-[A-Za-z0-9][A-Za-z0-9_.-]{0,100}\.(?:service|timer)|(?:panel-fpm@[A-Za-z0-9_.-]{1,100}|panel-u[1-5][0-9]{4}|tenant-[1-9][0-9]*)\.(?:service|slice))$")
BASE_UNITS = ["cron.service", "postfix.service", "dovecot.service", "opendkim.service",
              "crond.service", "redis.service", "valkey.service",
              "redis-server.service", "mariadb.service", "postgresql.service",
        "nginx.service", "panel-api.service", "panel-agent.service", "panel-db-web.service", "qpanel-cloudflared.service"]
PHP_UNITS = ["php" + version + "-fpm.service" for version in ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"]]
PHP_UNITS += ["php" + version + "-php-fpm.service" for version in ["74", "80", "81", "82", "83", "84", "85"]]
PACKAGE = re.compile(r"(?:php[0-9.a-z+-]*|phpMyAdmin|adminer|mariadb[a-z0-9.+-]*|mysql[a-z0-9.+-]*|postgresql[a-z0-9.+-]*|nginx[a-z0-9.+-]*|libnginx[a-z0-9.+-]*|redis[a-z0-9.+-]*|valkey[a-z0-9.+-]*|postfix[a-z0-9.+-]*|dovecot[a-z0-9.+-]*|opendkim[a-z0-9.+-]*|libsodium|cloudflared|acl|git|unzip)(?::(?:amd64|arm64|all)|\.(?:x86_64|aarch64|noarch))?$")
SUPPORTED_OS = {"ubuntu:22.04", "ubuntu:24.04", "debian:12", "almalinux:9", "almalinux:10"}
VERSION = re.compile(r"[0-9A-Za-z.+:~_-]{1,160}$")
TENANT = re.compile(r"p_[a-z0-9_]{1,28}$")


def identifier(value):
    if not isinstance(value, str) or str(uuid.UUID(value)) != value:
        raise ValueError("Identitas transfer tidak valid.")
    return value


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def version(value):
    if not isinstance(value, str) or not re.fullmatch(r"v?(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})", value):
        raise ValueError("Versi panel tidak valid.")
    return tuple(map(int, value.lstrip("v").split(".")))


def load_json(path, maximum=8 * 1024 * 1024):
    if path.is_symlink() or path.stat().st_size > maximum:
        raise ValueError("Metadata tidak valid atau terlalu besar.")
    return json.loads(path.read_text(encoding="utf-8"))


def save_json(path, value, mode=0o600):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8") as stream:
        os.chmod(temporary, mode)
        json.dump(value, stream, separators=(",", ":"))
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)
    if os.name=="posix":
        descriptor=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY)
        try:os.fsync(descriptor)
        finally:os.close(descriptor)


def safe_member(name):
    value = PurePosixPath(name)
    if not name or name.startswith("/") or "\\" in name or ".." in value.parts or len(name) > 2048:
        raise ValueError("Arsip memuat jalur tidak aman.")
    return str(value)


def unit_allowed(name):
    if not isinstance(name,str) or len(name)>255:return False
    return name in BASE_UNITS + PHP_UNITS or bool(UNIT_PATTERN.fullmatch(name)) or bool(re.fullmatch(r"postgresql@[0-9]{1,2}-[a-z0-9_-]+\.service", name)) or bool(re.fullmatch(r"qpanel-cloudflared-[a-f0-9-]{36}\.service", name))


def allowed_path(name, roots):
    return any(name == root or name.startswith(root + "/") for root in roots)


def validate_manifest(value):
    if type(value.get("managed_storage_omitted", False)) is not bool:
        raise ValueError("Metadata cakupan object storage tidak valid.")
    if value.get("format") != "qonpanel-server-1" or value.get("os") not in SUPPORTED_OS:
        raise ValueError("Format backup atau distro/versi OS tidak didukung.")
    version(value.get("panel_version"))
    if not isinstance(value.get("hostname"),str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,253}",value["hostname"]):
        raise ValueError("Hostname backup tidak valid.")
    port=value.get("public_port",17890)
    if not isinstance(port,int) or not 1024<=port<=65535 or port==7700:
        raise ValueError("Port panel backup tidak valid.")
    if value.get("arch") not in ("x86_64", "aarch64"):
        raise ValueError("Arsitektur backup tidak didukung.")
    tenants = value.get("tenants")
    if not isinstance(tenants, list) or len(tenants) > 10000:
        raise ValueError("Inventaris tenant tidak valid.")
    names, ids = set(), set()
    for tenant in tenants:
        name, uid, gid = tenant.get("name"), tenant.get("uid"), tenant.get("gid")
        if not isinstance(name, str) or not TENANT.fullmatch(name) or not isinstance(uid, int) or not 10000 <= uid <= 59999 or not isinstance(gid, int) or not 10000 <= gid <= 59999 or gid != uid:
            raise ValueError("Identitas Linux tenant tidak valid.")
        if name in names or uid in ids or tenant.get("group") != name:
            raise ValueError("Identitas tenant duplikat atau primary group tidak didukung.")
        if tenant.get("shell") not in ("/bin/bash", "/bin/sh", "/usr/sbin/nologin", "/bin/false", "/usr/lib/openssh/sftp-server"):
            raise ValueError("Shell tenant tidak didukung.")
        if not re.fullmatch(r"[!*$A-Za-z0-9./=,+_-]{0,1024}", tenant.get("password", "")):
            raise ValueError("Password hash Linux tidak valid.")
        if any(group not in ("panel-tenants", "panel-sftp") for group in tenant.get("groups", ["panel-tenants"])):
            raise ValueError("Supplementary group tenant tidak didukung.")
        names.add(name)
        ids.add(uid)
    permitted = set(ROOTS + QUEUE_ROOTS)
    permitted.update("home/" + tenant["name"] for tenant in tenants)
    roots = value.get("roots")
    if not isinstance(roots, list) or not roots or len(roots) > 15000 or len(set(roots)) != len(roots):
        raise ValueError("Daftar komponen backup tidak valid.")
    for root in roots:
        if not isinstance(root, str) or (root not in permitted and not (
            root.startswith("etc/systemd/system/") and (
                UNIT_PATTERN.fullmatch(root.removeprefix("etc/systemd/system/")) or
                root in ("etc/systemd/system/panel-fpm@.service", "etc/systemd/system/panel-fpm@.service.d", "etc/systemd/system/site-.service.d", "etc/systemd/system/qpanel-cloudflared.service") or re.fullmatch(r"etc/systemd/system/qpanel-cloudflared-[a-f0-9-]{36}\.service",root)))):
            raise ValueError("Komponen backup di luar cakupan Qinpanel.")
    if "var/lib/panel" not in roots or "etc/panel" not in roots:
        raise ValueError("Backup tidak memuat data dan kunci panel.")
    packages = value.get("packages", {})
    if not isinstance(packages, dict) or len(packages) > 1000 or any(not PACKAGE.fullmatch(k) or not isinstance(v, str) or not VERSION.fullmatch(v) for k, v in packages.items()):
        raise ValueError("Inventaris paket tidak valid.")
    bundles=value.get("package_files",{})
    if not isinstance(bundles,dict) or len(bundles)>1000:
        raise ValueError("Inventaris berkas paket tidak valid.")
    for filename,record in bundles.items():
        suffix = "rpm" if value["os"].startswith("almalinux:") else "deb"
        if not isinstance(filename,str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.+%:~_-]{0,199}\." + suffix,filename) or not isinstance(record,dict) or record.get("package") not in packages or record.get("version")!=packages[record["package"]] or not isinstance(record.get("sha256"),str) or not re.fullmatch(r"[a-f0-9]{64}",record["sha256"]):
            raise ValueError("Metadata paket tidak valid.")
    for name in value.get("active_units", []) + value.get("enabled_units", []):
        if not isinstance(name, str) or not unit_allowed(name):
            raise ValueError("Layanan backup tidak dikenali.")
    return value


class Engine:
    def __init__(self, root=Path("/"), executor=None):
        self.root = root.resolve()
        self.execute = executor or self.process
        self.base = self.path(STATE)
        self.work = self.base / "work"
        self.run = {}
        self.database_locks = []
        self.rpm_trust = None
        self.rpm_keys = []

    def release_databases(self):
        for database in self.database_locks:
            database.close()
        self.database_locks.clear()

    def drain(self):
        if self.root != Path("/"):
            return
        import urllib.request
        for _ in range(120):
            with urllib.request.urlopen("http://127.0.0.1:7700/healthz",timeout=5) as response:
                count=response.headers.get("x-qonpanel-active-writes","")
            if not count.isdecimal():
                raise ValueError("API belum mendukung drain backup; pasang paket panel yang sesuai.")
            if int(count)==0:
                break
            time.sleep(1)
        else:
            raise ValueError("Masih ada perubahan panel berjalan. Tunggu pekerjaan selesai.")
        # BEGIN IMMEDIATE serializes with a background job claim already racing
        # the maintenance flag. Hold these barriers until all writers have stopped.
        for name in ("panel.operations.db","panel.db","analytics.db"):
            path=self.path("var/lib/panel/"+name)
            if path.exists():
                connection=sqlite3.connect(path.as_uri()+"?mode=rw",uri=True,timeout=5)
                self.database_locks.append(connection)
                connection.execute("BEGIN IMMEDIATE")
        operations=self.path("var/lib/panel/panel.operations.db")
        source=operations if operations.exists() else self.path("var/lib/panel/panel.db")
        with contextlib.closing(sqlite3.connect(source.as_uri()+"?mode=ro",uri=True)) as database:
            if database.execute("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='jobs'").fetchone()[0] and database.execute("SELECT COUNT(*) FROM jobs WHERE status='running' AND name NOT IN ('server.backup.create','server.backup.inspect','server.backup.restore')").fetchone()[0]:
                raise ValueError("Ada pekerjaan panel berjalan. Tunggu instalasi/deployment selesai sebelum backup.")

    def path(self, value):
        return self.root / value.lstrip("/")

    @staticmethod
    def process(args, data=None, check=True, timeout=21600, cwd=None):
        result = subprocess.run(args, input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                check=False, timeout=timeout, cwd=cwd, env={"PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin", "HOME": "/root", "LC_ALL": "C", "DEBIAN_FRONTEND": "noninteractive"})
        if check and result.returncode:
            # Never expose provider output, database credentials, GPG diagnostics or request content.
            raise RuntimeError("Perintah " + Path(args[0]).name + " gagal; periksa prasyarat dan diagnostik layanan.")
        return result

    def command(self, *args, **kwargs):
        return self.execute(list(args), **kwargs)

    def phase(self, phase, message, progress=None):
        if phase == "succeeded":
            progress = 100
        if progress is not None:
            self.run["progress"] = max(self.run.get("progress", 0), min(100, max(0, progress)))
        self.run.update(phase=phase, message=message, updated_at=int(time.time()))
        save_json(self.base / "state.json", self.run)

    def identity(self):
        release = self.path("/etc/os-release").read_text(encoding="utf-8")
        fields = dict(line.split("=", 1) for line in release.splitlines() if "=" in line)
        os_version = fields.get("ID", "").strip('"') + ":" + fields.get("VERSION_ID", "").strip('"')
        if os_version.startswith("almalinux:"):
            os_version = os_version.split(".", 1)[0]
        if os_version not in SUPPORTED_OS:
            raise ValueError("OS backup belum didukung: Debian 12, Ubuntu 22.04/24.04, AlmaLinux 9/10.")
        value = self.command("/usr/local/bin/panelctl", "--version").stdout.decode().strip().split()[-1]
        version(value)
        return {"os": os_version, "arch": platform.machine(), "panel_version": value}

    @contextlib.contextmanager
    def database(self):
        path = self.path("/var/lib/panel/panel.db")
        if not path.is_file() or path.is_symlink():
            raise ValueError("Database panel belum tersedia.")
        connection = sqlite3.connect(path.as_uri() + "?mode=rw", uri=True)
        try:
            with connection:
                yield connection
        finally:
            connection.close()

    def fresh_target(self):
        with self.database() as db:
            for table, predicate in [("users", "role != 'admin'"), ("sites", "1=1"), ("databases", "1=1")]:
                if db.execute("SELECT COUNT(*) FROM " + table + " WHERE " + predicate).fetchone()[0]:
                    raise ValueError("Restore penuh hanya untuk instalasi baru tanpa tenant, situs, atau database aplikasi.")
        for name in ["var/lib/mysql", "var/lib/postgresql", "var/lib/pgsql", "var/lib/redis", "var/mail", "var/vmail", "var/lib/panel/mail", "opt/panel/runtimes", "var/lib/qonpanel-storage", "etc/qonpanel-storage"]:
            path = self.path(name)
            if path.exists() and any(path.iterdir()):
                raise ValueError("Server tujuan memiliki data layanan/runtime; gunakan instalasi Qinpanel yang baru.")

    def inventory(self):
        value = {**self.identity(), "format": "qonpanel-server-1", "created_at": int(time.time()),
                 "hostname": platform.node(), "tenants": [], "roots": [], "packages": {},
                 "public_port": int(self.path("etc/panel/public-port").read_text().strip()) if self.path("etc/panel/public-port").exists() else 17890}
        value["managed_storage_omitted"] = self.path("etc/qonpanel-storage/state.json").exists() or self.path("var/lib/qonpanel-storage").exists()
        if value["os"] not in SUPPORTED_OS:
            raise ValueError("Distro/versi OS backup belum didukung.")
        with self.database() as db:
            rows = db.execute("SELECT linux_username,linux_uid FROM users WHERE role='user' AND linux_username IS NOT NULL").fetchall()
        if self.root == Path("/"):
            import pwd
            import grp
            import spwd
            for name, uid in rows:
                account = pwd.getpwnam(name)
                if account.pw_uid != uid or account.pw_dir != "/home/" + name:
                    raise ValueError("Identitas tenant berbeda dari database panel.")
                value["tenants"].append({"name": name, "uid": uid, "gid": account.pw_gid,
                    "group": grp.getgrgid(account.pw_gid).gr_name, "shell": account.pw_shell,
                    "password": spwd.getspnam(name).sp_pwdp, "groups": [group.gr_name for group in grp.getgrall() if name in group.gr_mem and group.gr_name in ("panel-tenants","panel-sftp")]})
        elif rows:
            raise ValueError("Fixture tenant harus menyediakan inventaris Linux tiruan.")
        candidates = list(ROOTS + QUEUE_ROOTS) + ["home/" + tenant["name"] for tenant in value["tenants"]]
        systemd = self.path("etc/systemd/system")
        if systemd.exists():
            candidates += ["etc/systemd/system/" + p.name for p in systemd.iterdir() if UNIT_PATTERN.fullmatch(p.name) or p.name in ("panel-fpm@.service", "panel-fpm@.service.d", "site-.service.d") or re.fullmatch(r"qpanel-cloudflared-[a-f0-9-]{36}\.service",p.name)]
        value["roots"] = [p for p in candidates if self.path(p).exists() or self.path(p).is_symlink()]
        value["roots"] = [p for p in value["roots"] if not any(p.startswith(other + "/") for other in value["roots"] if other != p)]
        rpm = value["os"].startswith("almalinux:")
        installed = (self.command("rpm", "-qa", "--qf", "%{NAME}.%{ARCH}\t%{EPOCHNUM}:%{VERSION}-%{RELEASE}\n") if rpm else
                     self.command("dpkg-query", "-W", "-f=${db:Status-Status}\t${binary:Package}\t${Version}\n")).stdout.decode()
        for line in installed.splitlines():
            columns = line.split("\t")
            if rpm and len(columns) == 2:
                columns.insert(0, "installed")
            if len(columns) == 3 and columns[0] == "installed" and PACKAGE.fullmatch(columns[1]):
                value["packages"][columns[1]] = columns[2]
        units = self.units()
        value["active_units"] = [name for name in units if self.command("systemctl", "is-active", "--quiet", name, check=False).returncode == 0]
        value["enabled_units"] = [name for name in units if self.command("systemctl", "is-enabled", "--quiet", name, check=False).returncode == 0]
        return validate_manifest(value)

    def units(self):
        output = self.command("systemctl", "list-units", "--all", "--plain", "--no-legend", "--no-pager", "--type=service,slice,timer").stdout.decode()
        return sorted(set(BASE_UNITS + PHP_UNITS + [line.split()[0] for line in output.splitlines() if line.split() and unit_allowed(line.split()[0])]))

    def stop(self, names):
        # Stop ingress/writers before databases; root helper outlives API and agent.
        order = sorted(names, key=lambda name: (0 if name.endswith(".timer") else 2 if name.startswith(("mariadb", "postgresql", "redis", "valkey")) else 1, name))
        for name in order:
            if not unit_allowed(name):
                raise ValueError("Nama layanan tidak valid.")
            self.command("systemctl", "stop", name)

    def start(self, names):
        # Databases first, then agent/socket lifecycle, applications and ingress/API.
        def rank(name):
            if name.startswith(("mariadb", "postgresql", "redis", "valkey")):
                return 0
            if name == "panel-agent.service":
                return 1
            if name in ("nginx.service", "panel-api.service"):
                return 3
            return 2
        failures = []
        for name in sorted(set(names), key=lambda name: (rank(name), name)):
            if not unit_allowed(name):
                raise ValueError("Nama layanan tidak valid.")
            try:
                self.command("systemctl", "start", name)
                self.command("systemctl", "is-active", "--quiet", name)
            except (RuntimeError, OSError, subprocess.SubprocessError):
                failures.append(name)
        if failures:
            raise RuntimeError("Layanan belum aktif: " + ", ".join(failures))

    def package_info(self,path):
        if path.suffix == ".rpm":
            # Never install an archive payload with a missing/untrusted RPM signature.
            trust_args = ["--dbpath", str(self.rpm_trust)] if self.rpm_trust else []
            signature = self.command("rpmkeys", *trust_args, "--checksig", str(path)).stdout.decode()
            if not re.search(r"(?i)(?:signatures\s+OK|(?:rsa|pgp|dsa).*OK)", signature) or re.search(r"(?i)NOKEY|NOT OK|NOTTRUSTED", signature):
                raise ValueError("Tanda tangan RPM tidak dipercaya oleh server ini.")
            output = self.command("rpm", "-qp", "--qf", "%{NAME}\t%{EPOCHNUM}:%{VERSION}-%{RELEASE}\t%{ARCH}", str(path)).stdout.decode().split("\t")
            if len(output) != 3:
                raise ValueError("Metadata RPM tidak valid.")
            return tuple(output)
        name=self.command("dpkg-deb","--field",str(path),"Package").stdout.decode().strip()
        release=self.command("dpkg-deb","--field",str(path),"Version").stdout.decode().strip()
        architecture=self.command("dpkg-deb","--field",str(path),"Architecture").stdout.decode().strip()
        return name,release,architecture

    def collect_packages(self,manifest):
        directory=self.work / "qonpanel-packages"
        directory.mkdir(mode=0o700)
        files={}
        self.phase("running","Menyertakan paket aplikasi versi sumber agar restore tidak bergantung pada paket lama di repository.", 15)
        rpm = manifest["os"].startswith("almalinux:")
        for package, release in manifest["packages"].items():
            if rpm:
                name, architecture = package.rsplit(".", 1)
                # Exact NEVRA, not the repository's newest version. Failure is fatal.
                filename = name + "-" + release.split(":", 1)[-1] + "." + architecture + ".rpm"
                cache = self.path("var/cache/dnf")
                cached = next((path for path in cache.rglob(filename) if path.is_file() and not path.is_symlink() and cache.resolve() in path.resolve().parents), None) if cache.exists() else None
                if cached:
                    shutil.copy2(cached, directory / filename)
                else:
                    self.command("dnf", "download", "--destdir", str(directory), "--", name + "-" + release + "." + architecture)
            else:
                self.execute(["dpkg-repack","--tag=none",package],cwd=str(directory))
        for path in directory.glob("*.rpm" if rpm else "*.deb"):
            name,release,architecture=self.package_info(path)
            candidate = name + "." + architecture if rpm else (name+":"+architecture if name+":"+architecture in manifest["packages"] else name)
            allowed_arch = ("noarch", manifest["arch"]) if rpm else ("all", "amd64" if manifest["arch"] == "x86_64" else "arm64")
            if manifest["packages"].get(candidate)!=release or architecture not in allowed_arch:
                raise ValueError("Paket berubah ketika backup berjalan atau arsitektur paket berbeda.")
            files[path.name]={"package":candidate,"version":release,"sha256":digest(path)}
        if {entry["package"] for entry in files.values()}!=set(manifest["packages"]):
            raise ValueError("Paket aplikasi belum lengkap; backup migrasi tidak diterbitkan.")
        manifest["package_files"]=files

    def pack(self, output, manifest, portable=False):
        meta = self.work / "meta"
        meta.mkdir(parents=True, exist_ok=True, mode=0o700)
        save_json(meta / "qonpanel-manifest.json", manifest)
        args = ["tar", "--create", "--file", str(output), "--acls", "--xattrs", "--xattrs-include=user.*",
                "--exclude=etc/nginx/conf.d/qonpanel-object-storage.conf",
                "--exclude=var/lib/panel/backups", "--exclude=var/lib/panel/server-transfers",
                "--exclude=var/lib/panel/server-backups", "--exclude=var/lib/panel/*-wal", "--exclude=var/lib/panel/*-shm",
                "-C", str(meta), "qonpanel-manifest.json"]
        if portable and manifest.get("package_files"):
            args += ["-C",str(self.work),"qonpanel-packages"]
        args += ["-C",str(self.root),"--"]
        self.execute(args + manifest["roots"])
        with output.open("rb+") as stream:
            os.fsync(stream.fileno())

    def crypt(self, source, destination, passphrase, decrypt=False):
        home = self.work / "gnupg"
        home.mkdir(mode=0o700, exist_ok=True)
        args = ["gpg", "--no-options", "--homedir", str(home), "--batch", "--yes", "--no-tty",
                "--no-symkey-cache", "--pinentry-mode", "loopback", "--passphrase-fd", "0",
                "--output", str(destination)]
        maximum = min(MAX_BYTES, max(1, shutil.disk_usage(self.base).free - 512 * 1024**2))
        args += ["--max-output", str(maximum), "--decrypt"] if decrypt else ["--symmetric", "--cipher-algo", "AES256", "--force-mdc", "--compress-algo", "none"]
        self.execute(args + [str(source)], data=(passphrase + "\n").encode())

    def checkpoint(self):
        for name in ("panel.db", "panel.operations.db", "analytics.db"):
            path = self.path("/var/lib/panel/" + name)
            if path.exists():
                with contextlib.closing(sqlite3.connect(path.as_uri() + "?mode=rw", uri=True)) as db:
                    busy, _, _ = db.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
                    if busy or db.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
                        raise ValueError("Database SQLite masih dipakai atau tidak utuh.")

    def free_space(self, required):
        if shutil.disk_usage(self.base).free < required + 512 * 1024**2:
            raise ValueError("Ruang disk tidak cukup untuk staging dan snapshot keselamatan.")

    def create(self, request):
        self.phase("running", "Memeriksa inventaris server.", 5)
        manifest = self.inventory()
        self.collect_packages(manifest)
        manifest["created_at"]=int(time.time())
        estimated = sum(p.stat().st_size for root in manifest["roots"] for p in self.path(root).rglob("*") if p.is_file() and not p.is_symlink())
        self.free_space(estimated * 2)
        plain = self.work / "backup.tar"
        journal = {"active_units": manifest["active_units"], "restoring": False}
        save_json(self.base / "journal.json", journal)
        try:
            self.phase("running", "Menjeda layanan agar file dan database konsisten.", 25)
            self.stop(manifest["active_units"])
            self.release_databases()
            self.checkpoint()
            self.phase("running", "Mengarsipkan snapshot file dan database.", 40)
            self.pack(plain, manifest, portable=True)
            self.phase("running", "Memeriksa integritas arsip sebelum layanan diaktifkan kembali.", 55)
            self.validate_archive(plain)
        finally:
            self.start(manifest["active_units"])
            (self.base / "journal.json").unlink(missing_ok=True)
            (self.base / "maintenance").unlink(missing_ok=True)
        self.phase("running", "Layanan kembali aktif. Mengenkripsi arsip backup.", 75)
        output = self.base / "exports" / (request["id"] + ".qpb")
        output.parent.mkdir(mode=0o750, exist_ok=True)
        temporary = output.with_suffix(".partial")
        self.crypt(plain, temporary, request["passphrase"])
        with temporary.open("rb+") as stream:
            os.fsync(stream.fileno())
        temporary.replace(output)
        os.chmod(output, 0o640)
        if self.root == Path("/"):
            import grp
            os.chown(output.parent, 0, grp.getgrnam("panel").gr_gid)
            os.chown(output, 0, grp.getgrnam("panel").gr_gid)
        summary = {"id": request["id"], "created_at": manifest["created_at"], "size": output.stat().st_size,
                   "digest": digest(output), "hostname": manifest["hostname"], "panel_version": manifest["panel_version"]}
        catalog = load_json(self.base / "catalog.json") if (self.base / "catalog.json").exists() else []
        save_json(self.base / "catalog.json", [summary] + catalog[:99])
        self.run["archive_id"] = request["id"]

    def validate_archive(self, source, portable=True):
        manifest = None
        links, members, total, paths = {}, [], 0, set()
        with tarfile.open(source, "r|") as archive:
            for index, member in enumerate(archive):
                if index >= MAX_ENTRIES:
                    raise ValueError("Arsip memuat terlalu banyak berkas.")
                name = safe_member(member.name)
                if name in paths:
                    raise ValueError("Entri arsip duplikat.")
                paths.add(name)
                if index == 0:
                    if name != "qonpanel-manifest.json" or not member.isfile() or member.size > 8 * 1024**2:
                        raise ValueError("Manifest backup tidak tersedia.")
                    manifest = validate_manifest(json.load(archive.extractfile(member)))
                    continue
                if name=="qonpanel-packages" and member.isdir():
                    continue
                if name.startswith("qonpanel-packages/"):
                    filename=name.removeprefix("qonpanel-packages/")
                    record=manifest.get("package_files",{}).get(filename)
                    if not record or not member.isfile() or member.size>4*1024**3:
                        raise ValueError("Berkas paket backup tidak valid.")
                    hasher=hashlib.sha256()
                    stream=archive.extractfile(member)
                    for block in iter(lambda:stream.read(1024*1024),b""):
                        hasher.update(block)
                    if hasher.hexdigest()!=record["sha256"]:
                        raise ValueError("Checksum paket aplikasi tidak sesuai.")
                    total+=member.size
                    if total>MAX_BYTES:
                        raise ValueError("Isi backup melebihi batas 512 GiB.")
                    continue
                if not allowed_path(name, manifest["roots"]):
                    raise ValueError("Arsip memuat berkas di luar komponen terdaftar.")
                if not (member.isdir() or member.isfile() or member.issym() or member.islnk()):
                    raise ValueError("Arsip memuat device, socket, atau tipe berkas terlarang.")
                if name=="etc/panel/secret.key" and (not member.isfile() or member.size!=32):
                    raise ValueError("Master key dalam backup tidak valid.")
                if member.isfile() and member.mode & (stat.S_ISUID | stat.S_ISGID):
                    raise ValueError("Arsip memuat berkas dengan bit setuid/setgid.")
                total += member.size
                if total > MAX_BYTES:
                    raise ValueError("Isi backup melebihi batas 512 GiB.")
                if member.issym() or member.islnk():
                    target = member.linkname
                    if member.islnk():
                        resolved = safe_member(target)
                    else:
                        import posixpath
                        resolved = posixpath.normpath(target.lstrip("/") if target.startswith("/") else str(PurePosixPath(name).parent / target))
                    module_alias = (member.issym() and name == NGINX_MODULE_ALIAS
                                    and resolved in NGINX_MODULE_TARGETS)
                    if not allowed_path(resolved, manifest["roots"]) and not module_alias:
                        raise ValueError("Symlink/hardlink keluar dari cakupan backup: " + name)
                    links[name] = (resolved, member.issym())
                members.append(name)
        if manifest is None:
            raise ValueError("Backup kosong.")
        bundles=manifest.get("package_files",{})
        if portable and (len(bundles)!=len(manifest["packages"]) or {entry["package"] for entry in bundles.values()}!=set(manifest["packages"]) or any("qonpanel-packages/"+name not in paths for name in bundles)):
            raise ValueError("Arsip tidak memuat seluruh paket aplikasi versi sumber.")
        for name in members:
            if any(str(parent) in links for parent in PurePosixPath(name).parents):
                raise ValueError("Arsip menulis melalui link; restore ditolak.")
        required = {"var/lib/panel/panel.db", "etc/panel/secret.key"}
        if portable and not required.issubset(paths):
            raise ValueError("Database atau master key tidak tersedia; backup tidak portabel.")
        self.verify_databases(source)
        return manifest, total

    def verify_databases(self,source):
        directory=self.work / "verify-db"
        directory.mkdir(parents=True,exist_ok=True,mode=0o700)
        try:
            with tarfile.open(source,"r|") as archive:
                for member in archive:
                    if member.name not in ("var/lib/panel/panel.db","var/lib/panel/panel.operations.db","var/lib/panel/analytics.db"):
                        continue
                    if not member.isfile():
                        raise ValueError("Database backup bukan berkas biasa.")
                    path=directory / PurePosixPath(member.name).name
                    with archive.extractfile(member) as data,path.open("wb") as output:
                        shutil.copyfileobj(data,output,1024*1024)
                    with contextlib.closing(sqlite3.connect(path.as_uri()+"?mode=ro&immutable=1",uri=True)) as database:
                        if database.execute("PRAGMA integrity_check").fetchone()[0]!="ok":
                            raise ValueError("Database dalam backup tidak utuh.")
                        if path.name=="panel.db" and not database.execute("SELECT COUNT(*) FROM users WHERE role='admin'").fetchone()[0]:
                            raise ValueError("Backup tidak memiliki akun administrator.")
        finally:
            shutil.rmtree(directory)

    def input_archive(self, archive_id):
        identifier(archive_id)
        incoming = self.path(INCOMING) / (archive_id + ".qpb")
        imports = self.base / "imports"
        imports.mkdir(mode=0o700, exist_ok=True)
        cached = imports / (archive_id + ".qpb")
        if cached.is_file() and not cached.is_symlink():
            return cached
        if incoming.is_symlink() or not incoming.is_file() or not 0 < incoming.stat().st_size <= MAX_BYTES:
            raise ValueError("Unggahan backup belum lengkap atau terlalu besar.")
        # Retain one immutable root-owned import between inspect and restore.
        for previous in imports.iterdir():
            if previous.is_file():
                previous.unlink()
        self.free_space(incoming.stat().st_size)
        try:
            incoming.replace(cached)
        except OSError as error:
            import errno
            if error.errno != errno.EXDEV:
                raise
            with incoming.open("rb") as src, cached.open("xb") as dst:
                shutil.copyfileobj(src, dst, 1024 * 1024)
            incoming.unlink()
        cached.chmod(0o600)
        if self.root == Path("/"):
            os.chown(cached, 0, 0)
        return cached

    def compatibility(self, manifest):
        current = self.identity()
        if current["os"] != manifest["os"] or current["arch"] != manifest["arch"]:
            raise ValueError("Distro, versi OS, dan arsitektur harus sama dengan server sumber (AlmaLinux: versi mayor yang sama).")
        if version(current["panel_version"]) < version(manifest["panel_version"]):
            raise ValueError("Pasang Qinpanel versi yang sama atau lebih baru sebelum restore.")
        for root in manifest["roots"]:
            for parent in self.path(root).parents:
                if parent == self.root:
                    break
                if parent.is_symlink():
                    raise ValueError("Direktori induk komponen tujuan berupa symlink; gunakan layout instalasi standar.")

    def inspect(self, request):
        self.phase("running", "Mendekripsi dan memeriksa arsip. Data tujuan belum diubah.", 15)
        source = self.input_archive(request["archive_id"])
        plain = self.work / "input.tar"
        self.crypt(source, plain, request["passphrase"], decrypt=True)
        manifest, total = self.validate_archive(plain)
        self.compatibility(manifest)
        self.fresh_target()
        report = {"archive_id": request["archive_id"], "digest": digest(source), "hostname": manifest["hostname"],
                  "panel_version": manifest["panel_version"], "public_port": manifest.get("public_port",17890), "tenant_count": len(manifest["tenants"]),
                  "uncompressed_bytes": total, "packages": list(manifest["packages"]), "checked_at": int(time.time()),
                  "warnings": ["DNS/domain harus menunjuk ke IP server baru. Infrastruktur eksternal tidak disalin.",
                               "Setelah restore, masuk memakai akun dan 2FA dari server lama. Sesi login lama dihapus.",
                               "Pekerjaan queued/running dibatalkan. Antrean keluar Postfix tidak diputar ulang otomatis."]}
        if manifest.get("managed_storage_omitted"):
            report["warnings"].append("Arsip ini tidak memuat data, key, sertifikat dan engine Storage Qinpanel. Origin Nginx S3 juga tidak dipulihkan. Pulihkan backup engine terpisah sebelum memakai bucket managed; koneksi di database saja bukan pemulihan object.")
        save_json(self.base / "inspection.json", report)
        self.run["inspection"] = report
        return source, plain, manifest, total

    def make_accounts(self, tenants, journal):
        if self.root != Path("/"):
            return
        import pwd
        import grp
        for tenant in tenants:
            name, uid, gid = tenant["name"], tenant["uid"], tenant["gid"]
            for function, key in [(pwd.getpwnam, name), (pwd.getpwuid, uid), (grp.getgrnam, name), (grp.getgrgid, gid)]:
                try:
                    function(key)
                except KeyError:
                    continue
                raise ValueError("Nama/UID/GID tenant sudah digunakan pada server tujuan: " + name)
        if any("panel-sftp" in tenant.get("groups",[]) for tenant in tenants):
            self.command("groupadd","--system","--force","panel-sftp")
        for tenant in tenants:
            name = tenant["name"]
            self.command("groupadd", "--gid", str(tenant["gid"]), name)
            journal["created_groups"].append(name)
            save_json(self.base / "journal.json", journal)
            self.command("useradd", "--uid", str(tenant["uid"]), "--gid", name, "--groups", ",".join(tenant.get("groups",["panel-tenants"])) or "panel-tenants", "--home-dir", "/home/" + name, "--no-create-home", "--shell", tenant["shell"], name)
            journal["created_users"].append(name)
            save_json(self.base / "journal.json", journal)
            self.command("chpasswd", "--encrypted", data=(name + ":" + tenant["password"] + "\n").encode())

    def extract(self, archive, roots):
        # GNU tar resolves account/group names on the new host; tenant IDs are restored first.
        self.execute(["tar", "--extract", "--file", str(archive), "--acls", "--xattrs", "--xattrs-include=user.*",
                      "--same-owner", "--same-permissions", "--delay-directory-restore", "-C", str(self.root), "--"] + roots)

    def relabel(self, roots):
        if self.identity()["os"].startswith("almalinux:"):
            # Restore destination policy labels; never import security.selinux
            # xattrs from another machine or turn enforcement off.
            for name in roots:
                if self.path(name).exists():
                    self.command("restorecon", "-RF", "--", str(self.path(name)))

    def platform_aliases(self, manifest):
        if not manifest["os"].startswith("almalinux:"):
            return
        tool = self.path("usr/share/phpMyAdmin")
        alias = self.path("usr/share/phpmyadmin")
        if tool.is_dir() and not alias.exists() and not alias.is_symlink():
            alias.symlink_to("/usr/share/phpMyAdmin", target_is_directory=True)
        for version in ("7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"):
            prefix = "opt/remi/php" + version.replace(".", "") + "/root/usr/"
            for native, alias in ((prefix + "sbin/php-fpm", "usr/sbin/php-fpm" + version), (prefix + "bin/php", "usr/bin/php" + version)):
                source, destination = self.path(native), self.path(alias)
                if source.is_file():
                    if destination.is_symlink() and os.readlink(destination) == "/" + native:
                        continue
                    if destination.exists() or destination.is_symlink():
                        raise ValueError("Alias PHP custom dipertahankan: " + alias)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.symlink_to("/" + native)

    def prepare_packages(self, manifest):
        if manifest["os"].startswith("almalinux:"):
            return self.prepare_rpm_packages(manifest)
        policy = self.path("usr/sbin/policy-rc.d")
        previous = policy.read_bytes() if policy.exists() else None
        previous_mode = stat.S_IMODE(policy.stat().st_mode) if policy.exists() else None
        if policy.is_symlink():
            raise ValueError("policy-rc.d custom berupa symlink; pemeriksaan operator diperlukan.")
        policy.parent.mkdir(parents=True, exist_ok=True)
        try:
            policy.write_bytes(b"#!/bin/sh\nexit 101\n")
            policy.chmod(0o755)
            self.command("apt-get", "update", "-qq")
            packages = [str(self.work / "qonpanel-packages" / name) for name in manifest.get("package_files",{})]
            if packages:
                # Exact packages keep physical database formats/runtime versions compatible.
                self.execute(["apt-get", "-o", "APT::Sandbox::User=root", "install", "-y", "--no-install-recommends", "--allow-downgrades", "--"] + packages)
                # Repacked conffiles must not be silently replaced by an OS update.
                self.execute(["apt-mark","hold","--"]+list(manifest["packages"]))
        finally:
            if previous is None:
                policy.unlink(missing_ok=True)
            else:
                policy.write_bytes(previous)
                policy.chmod(previous_mode)

    def prepare_rpm_packages(self, manifest):
        # RPM scripts do not honor policy-rc.d. Journal only newly introduced
        # runtime masks so interruption recovery cannot unmask operator policy.
        journal = load_json(self.base / "journal.json")
        journal["package_masks"] = []
        try:
            # Imported only after the root-approved restore has a safety journal.
            # These public repository keys were also used for isolated signature
            # verification; package scripts never run during inspection/unpack.
            for key in self.rpm_keys:
                self.command("rpm", "--import", str(key))
            for unit in self.units():
                state = self.command("systemctl", "is-enabled", unit, check=False).stdout.strip()
                if state in (b"masked", b"masked-runtime"):
                    continue
                journal["package_masks"].append(unit)
                save_json(self.base / "journal.json", journal)
                self.command("systemctl", "mask", "--runtime", unit)
            # python3-rpm is supplied with DNF, not an additional panel dependency.
            import rpm
            install, downgrade, locks = [], [], []
            for filename, record in manifest["package_files"].items():
                name, arch = record["package"].rsplit(".", 1)
                evr = record["version"]
                epoch, rest = evr.split(":", 1)
                release_version, release = rest.rsplit("-", 1)
                installed = self.command("rpm", "-q", "--qf", "%{EPOCHNUM}\t%{VERSION}\t%{RELEASE}", record["package"], check=False)
                previous = installed.stdout.decode().split("\t")
                older = installed.returncode == 0 and len(previous) == 3 and rpm.labelCompare((epoch, release_version, release), tuple(previous)) < 0
                (downgrade if older else install).append(str(self.work / "qonpanel-packages" / filename))
                locks.append(name + "-" + evr + "." + arch)
            for action, paths in (("downgrade", downgrade), ("install", install)):
                if paths:
                    self.execute(["dnf", "-y", "--setopt=keepcache=True", "--setopt=localpkg_gpgcheck=True", action, "--"] + paths)
            for package, expected in manifest["packages"].items():
                actual = self.command("rpm", "-q", "--qf", "%{EPOCHNUM}:%{VERSION}-%{RELEASE}", package).stdout.decode().strip()
                if actual != expected:
                    raise ValueError("Versi RPM terpasang berbeda dari backup; data belum dipulihkan.")
            if locks:
                self.execute(["dnf", "versionlock", "add", "--"] + locks)
        finally:
            # Keep the journal until every mask has really been removed.
            self.clear_package_masks(journal)

    def clear_package_masks(self, journal):
        masks = journal.get("package_masks", [])
        if not isinstance(masks, list) or any(not unit_allowed(unit) for unit in masks):
            raise ValueError("Journal mask paket tidak valid.")
        for unit in list(masks):
            self.command("systemctl", "unmask", "--runtime", unit)
            masks.remove(unit)
            save_json(self.base / "journal.json", journal)

    def ensure_php_masks(self, journal):
        # Per-tenant FPM masters must remain the only entry point after restore
        # and reboot; disabling the package's shared service is insufficient.
        rpm = self.identity()["os"].startswith("almalinux:")
        journal["php_masks_added"] = []
        for version in ("7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"):
            native = "opt/remi/php" + version.replace(".", "") + "/root/usr/sbin/php-fpm" if rpm else "usr/sbin/php-fpm" + version
            if not self.path(native).is_file():
                continue
            unit = "php" + (version.replace(".", "") + "-php-fpm.service" if rpm else version + "-fpm.service")
            state = self.command("systemctl", "show", "--property=UnitFileState", "--value", unit).stdout.strip()
            if state == b"masked":
                continue
            journal["php_masks_added"].append(unit)
            save_json(self.base / "journal.json", journal)
            self.command("systemctl", "mask", unit)

    def clear_php_masks(self, journal):
        masks = journal.get("php_masks_added", [])
        if not isinstance(masks, list) or any(unit not in PHP_UNITS for unit in masks):
            raise ValueError("Journal mask PHP tidak valid.")
        for unit in list(masks):
            self.command("systemctl", "unmask", unit)
            masks.remove(unit)
            save_json(self.base / "journal.json", journal)

    def unpack_packages(self,archive,manifest):
        directory=self.work / "qonpanel-packages"
        directory.mkdir(mode=0o700,exist_ok=True)
        with tarfile.open(archive,"r|") as stream:
            for member in stream:
                if not member.name.startswith("qonpanel-packages/") or not member.isfile():
                    continue
                filename=member.name.removeprefix("qonpanel-packages/")
                record=manifest.get("package_files",{}).get(filename)
                if not record:
                    raise ValueError("Paket tidak terdaftar dalam manifest.")
                output=directory / filename
                with stream.extractfile(member) as source,output.open("xb") as target:
                    shutil.copyfileobj(source,target,1024*1024)
                name,release,architecture=self.package_info(output)
                rpm = manifest["os"].startswith("almalinux:")
                candidate = name + "." + architecture if rpm else (name+":"+architecture if name+":"+architecture in manifest["packages"] else name)
                allowed_arch = ("noarch", manifest["arch"]) if rpm else ("all", "amd64" if manifest["arch"] == "x86_64" else "arm64")
                if candidate!=record["package"] or release!=record["version"] or digest(output)!=record["sha256"] or architecture not in allowed_arch:
                    raise ValueError("Isi paket tidak sesuai manifest backup.")

    def prepare_rpm_trust(self, archive):
        self.rpm_trust = self.work / "rpm-trust"
        self.rpm_trust.mkdir(mode=0o700)
        self.command("rpm", "--dbpath", str(self.rpm_trust), "--initdb")
        directory = self.work / "rpm-public-keys"
        directory.mkdir(mode=0o700)
        total = 0
        # The out-of-band owner approval covers repository trust material in
        # this archive, just as it covers root service configuration and DEBs.
        with tarfile.open(archive, "r|") as stream:
            for member in stream:
                if member.name.startswith("etc/pki/rpm-gpg/") and member.isfile():
                    total += member.size
                    if member.size > 1024 * 1024 or total > 16 * 1024 * 1024:
                        raise ValueError("Kunci publik RPM terlalu besar.")
                    key = directory / (str(len(self.rpm_keys)) + ".gpg")
                    with stream.extractfile(member) as source, key.open("xb") as target:
                        shutil.copyfileobj(source, target)
                    self.command("rpm", "--dbpath", str(self.rpm_trust), "--import", str(key))
                    self.rpm_keys.append(key)
        if not self.rpm_keys:
            raise ValueError("Backup RPM tidak membawa kunci repositori sumber.")

    def selinux_policy(self, action):
        if self.identity()["os"].startswith("almalinux:"):
            self.command("/usr/bin/python3", "/usr/local/libexec/qonpanel-selinux", "--" + action)

    def reset_sessions(self, ipv4):
        with self.database() as db:
            tables = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            for name in ("sessions", "setup_tokens"):
                if name in tables:
                    db.execute("DELETE FROM " + name)
            if ipv4:
                address = ipaddress.ip_address(ipv4)
                if address.version != 4 or address.is_unspecified or address.is_multicast:
                    raise ValueError("Alamat IPv4 baru tidak valid.")
                old=db.execute("SELECT value FROM settings WHERE key='network.public_ipv4'").fetchone()
                if old and "service_endpoints" in tables:
                    db.execute("UPDATE service_endpoints SET host=?,revision=revision+1,updated_at=? WHERE host=? AND transport='direct'",(str(address),int(time.time()),old[0]))
                db.execute("INSERT INTO settings(key,value,updated_at) VALUES('network.public_ipv4',?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", (str(address), int(time.time())))
            for table in ("endpoint_evidence", "connectivity_diagnosis_leases"):
                if table in tables:
                    db.execute("DELETE FROM " + table)
            db.commit()
        operations=self.path("var/lib/panel/panel.operations.db")
        if operations.exists():
            with contextlib.closing(sqlite3.connect(operations)) as database:
                database.execute("UPDATE jobs SET status='failed',error_message='Dibatalkan oleh restore server; tidak diputar ulang otomatis',updated_at=? WHERE status IN ('queued','running')",(int(time.time()),))
                database.commit()

    def remove_root(self, name):
        safe_member(name)
        target = self.path(name)
        # Root allowlist is validated from a bounded manifest before this method is used.
        if target == self.root or self.root not in target.parents:
            raise ValueError("Target pemulihan tidak aman.")
        if target.is_symlink() or target.is_file():
            target.unlink()
        elif target.exists():
            shutil.rmtree(target)

    def rollback(self, journal):
        # A damaged journal/safety archive must never broaden deletion to OS roots.
        permitted=set(ROOTS+QUEUE_ROOTS)
        for root in journal.get("roots",[])+journal.get("original_roots",[]):
            if root not in permitted and not re.fullmatch(r"home/p_[a-z0-9][a-z0-9_-]{0,29}",root) and not (root.startswith("etc/systemd/system/") and (UNIT_PATTERN.fullmatch(root.removeprefix("etc/systemd/system/")) or root in ("etc/systemd/system/panel-fpm@.service","etc/systemd/system/panel-fpm@.service.d","etc/systemd/system/site-.service.d") or re.fullmatch(r"etc/systemd/system/qpanel-cloudflared-[a-f0-9-]{36}\.service",root))):
                raise ValueError("Journal keselamatan memuat target tidak sah.")
        if any(not unit_allowed(unit) for unit in journal.get("active_units",[])+journal.get("enabled_units",[])+journal.get("enabled_by_restore",[])) or any(not TENANT.fullmatch(name) for name in journal.get("created_users",[])+journal.get("created_groups",[])):
            raise ValueError("Identitas journal keselamatan tidak valid.")
        if not isinstance(journal.get("php_masks_added", []), list) or any(unit not in PHP_UNITS for unit in journal.get("php_masks_added", [])):
            raise ValueError("Journal mask PHP tidak valid.")
        self.clear_package_masks(journal)
        if journal.get("mutating"):
            safety=self.base / "safety.tar"
            if not safety.is_file() or safety.is_symlink() or digest(safety)!=journal.get("safety_sha256"):
                raise ValueError("Snapshot keselamatan tidak utuh. Data tidak dihapus.")
            safety_manifest,_=self.validate_archive(safety,portable=False)
            if safety_manifest["roots"]!=journal["original_roots"]:
                raise ValueError("Snapshot keselamatan tidak sesuai journal.")
            self.selinux_policy("rollback")
            self.stop([name for name in self.units() if self.command("systemctl", "is-active", "--quiet", name, check=False).returncode == 0])
            for unit in set(self.units()+journal.get("enabled_by_restore",[])):
                if unit not in journal.get("enabled_units", []):
                    self.command("systemctl", "disable", unit, check=False)
            for name in sorted(journal["roots"], key=len, reverse=True):
                self.remove_root(name)
            self.extract(self.base / "safety.tar", journal["original_roots"])
            self.relabel(journal["original_roots"])
            for name in journal.get("created_users", []):
                if self.command("userdel", name,check=False).returncode not in (0,6):
                    raise RuntimeError("Akun tenant sementara belum dapat dibersihkan.")
            for name in journal.get("created_groups", []):
                if self.command("groupdel", name,check=False).returncode not in (0,6):
                    raise RuntimeError("Grup tenant sementara belum dapat dibersihkan.")
            self.command("systemctl", "daemon-reload")
        self.clear_php_masks(journal)
        self.start(journal["active_units"])
        if self.root==Path("/"):os.sync()
        (self.base / "journal.json").unlink(missing_ok=True)

    def restore(self, request):
        self.require_approval(request.get("digest"))
        report = load_json(self.base / "inspection.json")
        if report["archive_id"] != request["archive_id"] or report["digest"] != request.get("digest") or not 0 <= int(time.time()) - report["checked_at"] <= 1800:
            raise ValueError("Periksa ulang arsip sebelum restore; pemeriksaan kedaluwarsa atau paket berubah.")
        source, plain, manifest, total = self.inspect(request)
        if digest(source) != request["digest"]:
            raise ValueError("Arsip berubah sejak pemeriksaan.")
        if request.get("new_ipv4"):
            address = ipaddress.ip_address(request["new_ipv4"])
            if address.version != 4 or address.is_unspecified or address.is_multicast:
                raise ValueError("IPv4 baru tidak valid.")
        self.free_space(total * 2)
        if manifest["os"].startswith("almalinux:"):
            self.prepare_rpm_trust(plain)
        self.unpack_packages(plain,manifest)
        installed_release = self.path("etc/panel/installed-release")
        current_release = installed_release.read_bytes() if installed_release.is_file() else None
        self.phase("running", "Membuat snapshot keselamatan server tujuan.", 35)
        target = self.inventory()
        # Queue files remain in the encrypted backup for operator recovery;
        # automatically replaying an old outgoing queue can deliver mail twice.
        effective_roots=[root for root in manifest["roots"] if root not in QUEUE_ROOTS]
        # Also remove newly created component roots during rollback, including tenant homes.
        roots = list(dict.fromkeys(target["roots"] + effective_roots))
        journal = {"restoring": True, "mutating": False, "active_units": target["active_units"],
                   "roots": roots, "original_roots": target["roots"], "enabled_units": target["enabled_units"], "enabled_by_restore":manifest["enabled_units"], "created_users": [], "created_groups": []}
        save_json(self.base / "journal.json", journal)
        self.stop(target["active_units"])
        self.release_databases()
        self.fresh_target()
        self.checkpoint()
        self.pack(self.base / "safety.tar", target)
        journal["safety_sha256"]=digest(self.base / "safety.tar")
        self.require_approval(request["digest"])
        journal["mutating"] = True
        save_json(self.base / "journal.json", journal)
        self.phase("running", "Memasang paket sesuai sumber dan memulihkan data layanan.", 65)
        repository_roots = [root for root in manifest["roots"] if root.startswith("etc/apt/") or root in ("etc/yum.repos.d", "etc/pki/rpm-gpg")]
        if repository_roots:
            self.extract(plain, repository_roots)
        self.prepare_packages(manifest)
        self.make_accounts(manifest["tenants"], journal)
        # Never merge database directories or retain fresh WAL/SHM beside restored databases.
        for root in roots:
            self.remove_root(root)
        self.extract(plain, effective_roots)
        self.platform_aliases(manifest)
        self.selinux_policy("apply")
        self.relabel(effective_roots)
        # Installed binaries/frontend belong to the destination package, never the backup.
        if current_release is None:
            installed_release.unlink(missing_ok=True)
        else:
            installed_release.write_bytes(current_release)
            installed_release.chmod(0o600)
        self.reset_sessions(request.get("new_ipv4"))
        self.command("runuser", "-u", "panel-api", "--", "/usr/local/bin/panelctl", "--db-path", "/var/lib/panel/panel.db", "admin", "list")
        self.command("systemctl", "daemon-reload")
        self.command("sshd", "-t")
        self.ensure_php_masks(journal)
        core_units=["panel-agent.service","panel-api.service","nginx.service"]
        for unit in self.units():
            if unit not in manifest["enabled_units"]+core_units:
                self.command("systemctl","disable",unit,check=False)
        for unit in set(manifest["enabled_units"]+core_units):
            self.command("systemctl", "enable", unit)
        self.command("nginx", "-t")
        # Keep the destination SSH/network policy; only expose the restored panel port.
        if shutil.which("ufw") and self.command("ufw", "status", check=False).stdout.startswith(b"Status: active"):
            self.command("ufw", "allow", str(manifest.get("public_port",17890)) + "/tcp")
        if shutil.which("firewall-cmd") and self.command("firewall-cmd", "--state", check=False).returncode == 0:
            port = "--add-port=" + str(manifest.get("public_port",17890)) + "/tcp"
            self.command("firewall-cmd", "--permanent", port)
            self.command("firewall-cmd", port)
        self.start(list(set(manifest["active_units"] + ["panel-agent.service", "panel-api.service", "nginx.service"])))
        self.command("curl", "--fail", "--silent", "--show-error", "--retry", "10", "--retry-connrefused", "--retry-delay", "2", "--max-time", "5", "http://127.0.0.1:7700/healthz")
        self.selinux_policy("commit")
        if self.root==Path("/"):os.sync()
        (self.base / "journal.json").unlink(missing_ok=True)
        (self.base / "approval.json").unlink(missing_ok=True)
        self.run["restored"] = {"hostname": manifest["hostname"], "tenant_count": len(manifest["tenants"])}

    def require_approval(self,value):
        path=self.base / "approval.json"
        if not path.is_file() or path.is_symlink():
            raise ValueError("Pemilik server perlu mengesahkan hash arsip melalui SSH atau installer sebelum restore konfigurasi root.")
        if self.root==Path("/") and (path.stat().st_uid!=0 or stat.S_IMODE(path.stat().st_mode)&0o022):
            raise ValueError("Pengesahan restore harus dimiliki root dan tidak dapat ditulis akun lain.")
        approval=load_json(path,4096)
        if not isinstance(value,str) or not re.fullmatch(r"[a-f0-9]{64}",value) or approval.get("digest")!=value or not isinstance(approval.get("expires_at"),int) or approval["expires_at"]<int(time.time()):
            raise ValueError("Pengesahan root tidak sesuai atau kedaluwarsa. Sahkan kembali hash arsip yang dipercaya.")

    def recover(self):
        journal_path = self.base / "journal.json"
        if journal_path.exists():
            self.phase("interrupted", "Memulihkan snapshot keselamatan setelah proses terputus.")
            self.rollback(load_json(journal_path))
            self.phase("interrupted", "Proses terputus; snapshot keselamatan dan layanan awal telah dipulihkan.")
        if not journal_path.exists():
            (self.base / "maintenance").unlink(missing_ok=True)
            if self.work.exists():
                shutil.rmtree(self.work)

    def main(self, finalize=False):
        if finalize:
            state = self.base / "state.json"
            if state.exists():
                self.run = load_json(state)
                if self.run.get("phase") in ("queued", "running"):
                    self.recover()
                    self.phase("interrupted", "Proses terputus. Periksa layanan sebelum mencoba kembali.")
            return
        request_path = self.base / "request.json"
        request = load_json(request_path, 4096)
        request_path.unlink()
        identifier(request["id"])
        if request.get("operation") not in ("create", "inspect", "restore") or not isinstance(request.get("passphrase"), str) or not 16 <= len(request["passphrase"].encode()) <= 256 or any(c in request["passphrase"] for c in "\r\n\0"):
            raise ValueError("Permintaan backup tidak valid.")
        self.run = {"id": request["id"], "operation": request["operation"], "phase": "running", "started_at": int(time.time()),"archive_id":request.get("archive_id"),"digest":request.get("digest"),"new_ipv4":request.get("new_ipv4")}
        if self.work.exists() and not (self.base / "journal.json").exists():
            shutil.rmtree(self.work)
        self.work.mkdir(parents=True, exist_ok=True, mode=0o700)
        try:
            rpm = self.identity()["os"].startswith("almalinux:")
            if self.root == Path("/") and rpm:
                prerequisites = ("gnupg2", "dnf-plugins-core", "python3-dnf-plugin-versionlock", "python3-rpm")
                missing = [name for name in prerequisites if self.command("rpm", "-q", name, check=False).returncode != 0]
                if missing:
                    self.phase("running", "Menyiapkan alat backup yang belum terpasang.")
                    self.command("dnf", "-y", "install", *missing)
            elif self.root == Path("/") and (shutil.which("gpg") is None or shutil.which("dpkg-repack") is None):
                self.phase("running", "Menyiapkan enkripsi dan pengarsipan paket aplikasi.")
                self.command("apt-get", "update", "-qq")
                self.command("apt-get", "install", "-y", "--no-install-recommends", "gpg","dpkg-repack")
            if (self.base / "journal.json").exists():
                raise ValueError("Masih ada transaksi terputus. Jalankan pemulihan keselamatan terlebih dahulu.")
            if request["operation"] in ("create","restore"):
                (self.base / "maintenance").write_text(request["id"],encoding="ascii")
                self.drain()
            if request["operation"] == "create":
                self.create(request)
            elif request["operation"] == "inspect":
                self.inspect(request)
            else:
                self.restore(request)
            self.phase("succeeded", "Backup siap diunduh." if request["operation"] == "create" else "Pemeriksaan selesai." if request["operation"] == "inspect" else "Restore selesai. Masuk memakai akun administrator dari server lama.")
        except Exception as error:
            message = str(error) if isinstance(error, (ValueError, RuntimeError)) else "Operasi gagal; periksa prasyarat, ruang disk dan layanan."
            try:
                self.recover()
            except Exception:
                message += " Pemulihan keselamatan belum selesai; jalankan helper --recover melalui SSH sebelum menyalakan layanan."
            self.phase("failed", message[:1500])
            raise
        finally:
            self.release_databases()
            request.clear()
            if self.work.exists():
                shutil.rmtree(self.work)
            if self.run.get("phase")=="succeeded" and self.run.get("operation")=="restore":
                for archive in (self.base / "imports").glob("*.qpb"):
                    archive.unlink()
                (self.base / "inspection.json").unlink(missing_ok=True)
            if not (self.base / "journal.json").exists():
                (self.base / "maintenance").unlink(missing_ok=True)


def main():
    if sys.platform != "linux" or os.geteuid() != 0:
        raise SystemExit("Helper ini hanya berjalan sebagai layanan root pada server Linux yang didukung; bukan pada host developer.")
    import fcntl
    os.umask(0o077)
    engine = Engine()
    engine.base.mkdir(mode=0o750, parents=True, exist_ok=True)
    import grp
    os.chown(engine.base, 0, grp.getgrnam("panel").gr_gid)
    # Out-of-band root authorization is intentionally unavailable through RPC/API.
    if len(sys.argv)==3 and sys.argv[1]=="--approve" and re.fullmatch(r"[a-f0-9]{64}",sys.argv[2]):
        save_json(engine.base / "approval.json",{"digest":sys.argv[2],"expires_at":int(time.time())+86400})
        print("Hash arsip disahkan selama 24 jam untuk satu restore. Lanjutkan melalui panel admin.")
        return
    # Same lock as install.sh prevents version updates during capture/restore.
    with engine.path("/run/lock/panel-install.lock").open("a") as deployment, (engine.base / "worker.lock").open("a") as lock:
        fcntl.flock(deployment, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if sys.argv[1:] == ["--recover"]:
            engine.run = load_json(engine.base / "state.json")
            engine.recover()
        elif sys.argv[1:] == ["--finalize"]:
            engine.main(finalize=True)
        elif len(sys.argv) == 1:
            def interrupted(_number, _frame):
                raise RuntimeError("Pekerjaan dihentikan.")
            signal.signal(signal.SIGTERM, interrupted)
            signal.signal(signal.SIGINT, interrupted)
            engine.main()
        else:
            raise ValueError("Argumen helper tidak dikenal.")


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Detailed bounded state is read over authenticated RPC; never print request/secrets.
        print("Backup/restore belum berhasil. Periksa status di panel admin atau helper --recover.", file=sys.stderr)
        sys.exit(1)
