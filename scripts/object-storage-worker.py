#!/usr/bin/env python3
"""Typed local control plane for Qinpanel's single-node S3 service.

Only the agent invokes this helper. Object bytes never traverse its stdin/stdout.
The engine's network namespace is private; only its authenticated S3 Unix socket
is shared with Nginx and panel-api. No client-supplied paths or commands exist.
"""
import copy
import datetime
import hashlib
import hmac
import http.client
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shutil
import socket
import ssl
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import uuid

VERSION = "4.50"
ASSETS = {
    "x86_64": ("amd64", "8fc88061421a74d1cd486d1cd9dc5874d6495f9ed17fed8fa677bb16bd195b07"),
    "aarch64": ("arm64", "8bbd74337e233dbcbdc65e765f937a7334acc588ebb09dc0136b8692d9380311"),
}
UNIT = "qonpanel-object-storage.service"
SERVICE_USER = "qonpanel-storage"
SOCKET = "/run/qonpanel-storage/s3.sock"
HELPER = "/usr/local/libexec/qonpanel-object-storage"
DATA = "/var/lib/qonpanel-storage"
CONFIG = "/etc/qonpanel-storage"
ENGINE = "/usr/local/lib/qonpanel-storage/weed"
MARKER = "# Managed by QonPanel object storage\n"


class StorageError(Exception):
    """Only safe labels and machine state may be returned through the agent."""


class StorageSetupError(StorageError):
    """A failed setup carries a fixed stage independently of its diagnostic text."""

    def __init__(self, message, stage):
        super().__init__(message)
        self.stage = stage


def error_response(error):
    response = {"error": str(error)}
    if isinstance(error, StorageSetupError) and error.stage in {
        "certificate validation", "verified engine installation",
        "Nginx configuration validation", "storage service startup",
        "storage readiness", "HTTPS activation",
    }:
        response["stage"] = error.stage
    return response


def command(args, timeout=60):
    # Never return argv, stdout/stderr, PEM or credentials in a control-plane error.
    program = Path(args[0]).name if args else ""
    label = program if program in {"systemctl", "nginx", "openssl", "setfacl", "restorecon", "useradd"} else "storage command"
    if program == "systemctl" and len(args) > 1 and args[1] in {"is-active", "show", "enable", "disable", "daemon-reload", "restart", "reload"}:
        label += " " + args[1]
    elif program == "nginx" and len(args) > 1 and args[1] in {"-t", "-T"}:
        label += " " + args[1]
    try:
        result = subprocess.run(args, stdin=subprocess.DEVNULL, capture_output=True,
                                timeout=timeout, check=False, env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C"})
    except subprocess.TimeoutExpired:
        raise StorageError(f"Storage command timed out ({label}). Inspect the service journal on the server.") from None
    except OSError:
        raise StorageError(f"Storage command could not start ({label}). Check the installed dependencies.") from None
    if result.returncode:
        raise StorageError(f"Storage command failed ({label}, exit {result.returncode}). Inspect the service journal on the server.")
    return result.stdout


def service_status_summary(unit):
    """Only bounded machine-state fields; journal/config dumps may contain S3 keys."""
    if unit not in (UNIT, "nginx"):
        return "unavailable"
    fields = {"LoadState", "ActiveState", "SubState", "Result", "ExecMainCode", "ExecMainStatus"}
    try:
        raw = command(["systemctl", "show", unit, "--no-pager", "--property=" + ",".join(sorted(fields))], 10)
    except Exception:
        return "unavailable"
    values = []
    for line in raw.decode(errors="replace").splitlines():
        key, separator, value = line.partition("=")
        if separator and key in fields and re.fullmatch(r"[a-z0-9-]{1,40}", value):
            values.append(f"{key}={value}")
    return ", ".join(values[:len(fields)]) or "unavailable"


def relabel(paths):
    enforce = Path("/sys/fs/selinux/enforce")
    if enforce.exists() and enforce.read_text().strip() == "1":
        command(["restorecon", "-RF", "--"] + [str(path) for path in paths], 30)


def hostname(value):
    if not isinstance(value, str) or value != value.lower() or len(value) > 253:
        raise StorageError("Use a lowercase DNS hostname for the separate S3 HTTPS endpoint.")
    labels = value.split(".")
    if len(labels) < 2 or any(not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", p) for p in labels):
        raise StorageError("Use a valid DNS hostname for storage.")
    if all(p.isdecimal() for p in labels):
        raise StorageError("IP endpoints are not supported for managed storage.")
    return value


def bucket_name(value):
    if not isinstance(value, str) or not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,61}[a-z0-9]", value):
        raise StorageError("Bucket names require 3-63 lowercase letters, numbers or hyphens.")
    return value


def credentials(value):
    # The wire secret is intentionally a redacted String in panel-proto.
    if not isinstance(value, str) or len(value) > 8192:
        raise StorageError("Invalid bucket credentials.")
    try:
        result = json.loads(value)
    except (ValueError, TypeError):
        raise StorageError("Invalid bucket credentials.") from None
    if not isinstance(result, dict) or set(result) != {"access_key", "secret_key", "session_token"}:
        raise StorageError("Invalid bucket credentials.")
    if not isinstance(result["access_key"], str) or not re.fullmatch(r"[A-Za-z0-9_-]{20,128}", result["access_key"]):
        raise StorageError("Invalid bucket credentials.")
    if not isinstance(result["secret_key"], str) or not re.fullmatch(r"[A-Za-z0-9_-]{32,128}", result["secret_key"]) or result["session_token"] is not None:
        raise StorageError("Invalid bucket credentials.")
    return result


def validate_request(value):
    if not isinstance(value, dict):
        raise StorageError("Invalid managed storage operation.")
    action = value.get("action")
    fields = {
        "status": {"action"},
        "configure": {"action", "hostname", "fullchain_pem", "private_key_pem"},
        "provision": {"action", "connection_id", "owner_id", "bucket", "credentials"},
        "application_key": {"action", "connection_id", "owner_id", "bucket", "generation", "credentials"},
        "decommission": {"action", "connection_id", "owner_id", "bucket"},
    }
    if action not in fields or set(value) != fields[action]:
        raise StorageError("Invalid managed storage fields.")
    if action == "configure":
        hostname(value["hostname"])
        for key in ("fullchain_pem", "private_key_pem"):
            if not isinstance(value[key], str) or not 64 <= len(value[key]) <= 65536:
                raise StorageError("Supply a valid HTTPS certificate and private key.")
    elif action in ("provision", "application_key", "decommission"):
        try:
            parsed = uuid.UUID(value["connection_id"])
        except (ValueError, TypeError, AttributeError):
            raise StorageError("Invalid bucket identity.") from None
        if str(parsed) != value["connection_id"] or parsed.int == 0 or type(value["owner_id"]) is not int or value["owner_id"] <= 0:
            raise StorageError("Invalid bucket ownership.")
        bucket_name(value["bucket"])
        if action != "decommission":
            credentials(value["credentials"])
        if action == "application_key" and (type(value["generation"]) is not int or not 1 <= value["generation"] <= 2147483647):
            raise StorageError("Invalid application key generation.")
    return value


def atomic_write(path, content, mode=0o600, gid=None):
    if path.is_symlink() or (path.exists() and not path.is_file()):
        raise StorageError("Unsafe managed configuration file.")
    fd, temporary = tempfile.mkstemp(prefix=".qonpanel-", dir=path.parent)
    try:
        os.fchmod(fd, mode)
        if gid is not None:
            os.fchown(fd, 0, gid)
        with os.fdopen(fd, "wb") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def safe_directory(path, mode=0o700, uid=0, gid=0):
    # Inspect every ancestor before writing; never follow tenant-created links.
    for ancestor in reversed([path] + list(path.parents)[:-1]):
        if ancestor.is_symlink():
            raise StorageError("Unsafe managed storage directory.")
        if ancestor.exists():
            info = ancestor.stat()
            if not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0, uid) or info.st_mode & 0o022:
                raise StorageError("Managed storage directories must not be writable by other users.")
        else:
            ancestor.mkdir(mode=mode)
    os.chown(path, uid, gid)
    os.chmod(path, mode)


def service_template(endpoint):
    return f"""{MARKER}[Unit]
Description=Qinpanel Object Storage (SeaweedFS {VERSION})
After=local-fs.target

[Service]
Type=simple
User={SERVICE_USER}
Group={SERVICE_USER}
WorkingDirectory={DATA}
Environment=HOME={DATA}
RuntimeDirectory=qonpanel-storage
RuntimeDirectoryMode=0700
UMask=0077
ExecStart={ENGINE} mini -dir={DATA} -ip=127.0.0.1 -ip.bind=127.0.0.1 -master.peers= -master.telemetry=false -master.volumeSizeLimitMB=128 -volume.max=0 -s3.config={CONFIG}/s3.json -s3.externalUrl={endpoint} -s3.localSocket={SOCKET} -filer.localSocket=/run/qonpanel-storage/filer.sock -s3.autoCreateBucket=false -s3.iam=false -s3.port.iceberg=0 -s3.port.lance=0 -webdav=false -admin.ui=false
ExecStartPost=+/usr/bin/python3 {HELPER} --socket-ready
Restart=on-failure
RestartSec=3
TimeoutStartSec=120
TimeoutStopSec=60
PrivateNetwork=true
PrivateTmp=true
PrivateDevices=true
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths={DATA}
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictRealtime=true
RestrictNamespaces=true
LockPersonality=true
CapabilityBoundingSet=
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
"""


def nginx_template(host, revision):
    if not re.fullmatch(r"[0-9a-f]{64}", revision):
        raise StorageError("Invalid internal TLS revision.")
    return f"""{MARKER}server {{
    listen 443 ssl;
    server_name {host};
    ssl_certificate {CONFIG}/certificates/{revision}/fullchain.pem;
    ssl_certificate_key {CONFIG}/certificates/{revision}/private-key.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    client_max_body_size 5g;
    # Presigned URLs are short-lived bearer credentials; omit raw request logs.
    access_log off;
    # Signature paths and Host must reach S3 unchanged. No URI suffix/rewrite.
    location / {{
        proxy_pass http://unix:{SOCKET};
        proxy_http_version 1.1;
        proxy_set_header Host $http_host;
        proxy_set_header Connection "";
        proxy_set_header X-Forwarded-Proto https;
        proxy_set_header Authorization $http_authorization;
        proxy_request_buffering off;
        proxy_buffering off;
        proxy_read_timeout 300s;
        proxy_send_timeout 300s;
    }}
}}
"""


def auth_document(state):
    def identity(name, keys, actions):
        return {"name": name, "credentials": [{"accessKey": keys["access_key"], "secretKey": keys["secret_key"]}], "actions": actions}
    identities = [identity("qonpanel-control", state["root"], ["Admin", "Read", "List", "Write"])]
    for key, record in sorted(state["buckets"].items()):
        if record.get("deleted"):
            continue
        actions = [f"{action}:{record['bucket']}" for action in ("Read", "List", "Write")]
        identities.append(identity(f"panel-{key}", record["panel"], actions))
        if record.get("application"):
            identities.append(identity(f"app-{key}", record["application"], actions))
    return {"identities": identities}


class UnixHTTP(http.client.HTTPConnection):
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(SOCKET)


class LocalHTTPS(http.client.HTTPSConnection):
    def connect(self):
        tcp = socket.create_connection(("127.0.0.1", 443), timeout=self.timeout)
        self.sock = self._context.wrap_socket(tcp, server_hostname=self.host)


def s3_request(state, method, path, keys=None, public=False, timeout=10):
    """Small control-plane requests only; never reads object contents."""
    keys = keys or state["root"]
    now = datetime.datetime.now(datetime.timezone.utc)
    timestamp, day = now.strftime("%Y%m%dT%H%M%SZ"), now.strftime("%Y%m%d")
    digest = hashlib.sha256(b"").hexdigest()
    headers = {"host": state["hostname"], "x-amz-content-sha256": digest, "x-amz-date": timestamp}
    signed = ";".join(sorted(headers))
    canonical = "\n".join([method, path, "", "".join(f"{k}:{headers[k]}\n" for k in sorted(headers)), signed, digest])
    scope = f"{day}/us-east-1/s3/aws4_request"
    string = f"AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{hashlib.sha256(canonical.encode()).hexdigest()}"
    signing = ("AWS4" + keys["secret_key"]).encode()
    for part in (day, "us-east-1", "s3", "aws4_request"):
        signing = hmac.new(signing, part.encode(), hashlib.sha256).digest()
    signature = hmac.new(signing, string.encode(), hashlib.sha256).hexdigest()
    headers["authorization"] = f"AWS4-HMAC-SHA256 Credential={keys['access_key']}/{scope}, SignedHeaders={signed}, Signature={signature}"
    connection = LocalHTTPS(state["hostname"], timeout=timeout) if public else UnixHTTP("localhost", timeout=timeout)
    try:
        connection.request(method, path, body=b"", headers=headers)
        response = connection.getresponse()
        code = response.status
        response.read(65537)
        return code
    finally:
        connection.close()


def wait_for_https(state, fullchain, timeout=30):
    """Reload sends a signal; require the new trusted listener and S3, not just the signal."""
    leaf = re.search(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----", fullchain, re.S)
    if leaf is None:
        raise StorageError("The storage HTTPS certificate is missing.")
    expected = ssl.PEM_cert_to_DER_cert(leaf.group())
    context = ssl.create_default_context()
    deadline = time.monotonic() + timeout
    reason = "The storage HTTPS listener did not become ready."
    while (remaining := deadline - time.monotonic()) > 0:
        try:
            # Always verify system trust, SNI/hostname and the installed leaf before
            # sending any signed request. A renewal may briefly serve the old leaf.
            with socket.create_connection(("127.0.0.1", 443), timeout=min(3, remaining)) as tcp:
                with context.wrap_socket(tcp, server_hostname=state["hostname"]) as tls:
                    matched = tls.getpeercert(binary_form=True) == expected
            if not matched:
                reason = "The storage HTTPS listener did not present the installed certificate."
            else:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                code = s3_request(state, "GET", "/", public=True, timeout=min(3, remaining))
                if code == 200:
                    return
                reason = f"Authenticated S3 through the HTTPS proxy returned HTTP {code}."
        except ssl.SSLCertVerificationError:
            reason = "Storage HTTPS certificate trust or hostname verification failed."
        except ssl.SSLError:
            reason = "Storage HTTPS TLS handshake failed."
        except http.client.HTTPException:
            reason = "Storage HTTPS proxy returned an invalid or interrupted HTTP response."
        except OSError:
            reason = "Storage HTTPS listener connection failed or timed out."
        remaining = deadline - time.monotonic()
        if remaining > 0:
            time.sleep(min(0.5, remaining))
    raise StorageError("Storage HTTPS readiness timed out. " + reason)


class ReleaseRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, url):
        from urllib.parse import urlsplit
        target = urlsplit(url)
        if target.scheme != "https" or target.hostname not in ("github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com") or target.username or target.password:
            raise StorageError("Unexpected engine download redirect.")
        return super().redirect_request(request, fp, code, msg, headers, url)


def install_engine():
    asset = ASSETS.get(platform.machine())
    if asset is None:
        raise StorageError("Managed storage supports x86_64 and aarch64 Linux releases.")
    target = Path(ENGINE)
    safe_directory(target.parent, 0o755)
    marker = target.parent / "release.json"
    if target.exists():
        if target.is_symlink() or marker.is_symlink() or not marker.is_file():
            raise StorageError("An unmanaged engine binary already exists; it was preserved.")
        installed = json.loads(marker.read_text())
        info = target.stat()
        if installed.get("version") != VERSION or installed.get("archive_sha256") != asset[1] or info.st_uid != 0 or info.st_mode & 0o022:
            raise StorageError("The installed storage engine does not match the pinned release.")
        if hashlib.sha256(target.read_bytes()).hexdigest() != installed.get("binary_sha256"):
            raise StorageError("The installed engine checksum changed; it was preserved for inspection.")
        return
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), ReleaseRedirect())
    url = f"https://github.com/seaweedfs/seaweedfs/releases/download/{VERSION}/linux_{asset[0]}.tar.gz"
    with tempfile.TemporaryFile() as archive:
        digest, size, started = hashlib.sha256(), 0, time.monotonic()
        with opener.open(url, timeout=30) as response:
            while True:
                chunk = response.read(1024 * 1024)
                if not chunk:
                    break
                size += len(chunk)
                if size > 100 * 1024 * 1024 or time.monotonic() - started > 180:
                    raise StorageError("Engine download exceeded its size or time limit.")
                digest.update(chunk)
                archive.write(chunk)
        if digest.hexdigest() != asset[1]:
            raise StorageError("Engine checksum mismatch; installation stopped.")
        archive.seek(0)
        with tarfile.open(fileobj=archive, mode="r:gz") as package:
            candidates = [entry for entry in package.getmembers() if entry.name in ("weed", "./weed")]
            if len(candidates) != 1 or not candidates[0].isfile() or candidates[0].size > 300 * 1024 * 1024:
                raise StorageError("Unexpected engine archive.")
            source = package.extractfile(candidates[0])
            if source is None:
                raise StorageError("Engine binary missing from archive.")
            binary = source.read()
            # A marker without its binary can be retried after a crash. An
            # unknown existing executable is never replaced automatically.
            atomic_write(marker, json.dumps({"version": VERSION, "archive_sha256": asset[1], "binary_sha256": hashlib.sha256(binary).hexdigest()}).encode())
            atomic_write(target, binary, 0o755)


class Controller:
    def __init__(self, config=Path(CONFIG), data=Path(DATA), unit=Path("/etc/systemd/system") / UNIT,
                 nginx=Path("/etc/nginx/conf.d/qonpanel-object-storage.conf")):
        self.config, self.data, self.unit, self.nginx = config, data, unit, nginx
        self.state_path = config / "state.json"

    def state(self):
        if not self.state_path.exists():
            return None
        if self.state_path.is_symlink() or self.state_path.stat().st_mode & 0o077:
            raise StorageError("Unsafe managed storage state file.")
        value = json.loads(self.state_path.read_text())
        if value.get("schema") != 1:
            raise StorageError("Unsupported managed storage state. Existing data was preserved.")
        return value

    def status(self):
        state = self.state()
        running = False
        if state:
            try:
                command(["systemctl", "is-active", "--quiet", UNIT], 10)
                running = s3_request(state, "GET", "/") == 200
            except (StorageError, OSError, TimeoutError, http.client.HTTPException):
                pass
        usage = shutil.disk_usage(self.data) if self.data.exists() else None
        return {"configured": bool(state), "running": running, "endpoint": f"https://{state['hostname']}" if state else None,
                "engine_version": VERSION, "data_directory": DATA, "capacity_bytes": usage.total if usage else None,
                "available_bytes": usage.free if usage else None, "bucket_count": sum(not record.get("deleted", False) for record in state["buckets"].values()) if state else 0,
                "quota_enforcement": "none"}

    def identity(self):
        import pwd
        import grp
        try:
            user = pwd.getpwnam(SERVICE_USER)
        except KeyError:
            command(["useradd", "--system", "--user-group", "--home-dir", DATA, "--no-create-home", "--shell", "/usr/sbin/nologin", SERVICE_USER])
            user = pwd.getpwnam(SERVICE_USER)
        if user.pw_uid == 0 or user.pw_uid >= 10000 or user.pw_dir != DATA or user.pw_shell not in ("/usr/sbin/nologin", "/sbin/nologin"):
            raise StorageError("The storage service identity conflicts with an existing user.")
        group = grp.getgrgid(user.pw_gid)
        shared = any(other.pw_name != SERVICE_USER and (other.pw_uid == user.pw_uid or other.pw_gid == user.pw_gid) for other in pwd.getpwall())
        if group.gr_name != SERVICE_USER or any(name != SERVICE_USER for name in group.gr_mem) or shared:
            raise StorageError("Storage requires its own unshared system user and group.")
        return user.pw_uid, user.pw_gid

    def write_state(self, state, gid):
        atomic_write(self.state_path, json.dumps(state, sort_keys=True).encode())
        atomic_write(self.config / "s3.json", json.dumps(auth_document(state)).encode(), 0o640, gid)

    def healthy(self, state):
        for _ in range(45):
            try:
                if s3_request(state, "GET", "/") == 200:
                    return
            except (OSError, TimeoutError, http.client.HTTPException):
                pass
            time.sleep(1)
        raise StorageError("Storage did not become ready. Existing object data was preserved.")

    def configure(self, request):
        previous = self.state()
        host = hostname(request["hostname"])
        if previous and host != previous["hostname"]:
            raise StorageError("The storage endpoint cannot be moved while buckets exist. Renew the certificate using the same hostname.")
        try:
            command(["systemctl", "is-active", "--quiet", "nginx"], 10)
        except StorageError:
            raise StorageError("Nginx must be running before configuring storage. Start Nginx and retry; existing storage data and configuration were not changed.") from None
        if not previous and self.config.exists() and any(self.config.iterdir()):
            # Candidate certificate directories can survive a failed first setup.
            if any(path.name != "certificates" for path in self.config.iterdir()):
                raise StorageError("The storage configuration directory contains unmanaged files; they were preserved.")
        uid, gid = self.identity()
        safe_directory(self.config, 0o710, 0, gid)
        volume_marker = self.data / ".qonpanel-storage-volume"
        if self.data.exists() and not previous:
            if volume_marker.is_symlink() or (any(self.data.iterdir()) and (not volume_marker.is_file() or volume_marker.read_text() != "qonpanel-storage-volume-v1\n")):
                raise StorageError("The storage data directory contains unmanaged data; it was preserved.")
        safe_directory(self.data, 0o700, uid, gid)
        if not volume_marker.exists():
            atomic_write(volume_marker, b"qonpanel-storage-volume-v1\n", 0o444)
        for path in (self.unit, self.nginx):
            if path.exists() and (path.is_symlink() or not path.read_text().startswith(MARKER)):
                raise StorageError("An unmanaged service or Nginx file already exists; it was preserved.")
        # Never take over an existing website's host name.
        nginx = command(["nginx", "-T"], 20).decode(errors="replace")
        own = self.nginx.read_text() if self.nginx.exists() else ""
        other = nginx.replace(own, "") if own else nginx
        for match in re.finditer(r"\bserver_name\s+([^;]+);", other):
            if host in match.group(1).split():
                raise StorageError("That hostname is already used by Nginx. Choose a separate storage domain.")
        state = previous or {"schema": 1, "hostname": host, "root": {"access_key": secrets.token_hex(16), "secret_key": secrets.token_hex(32)}, "buckets": {}}
        # Store each validated pair in its own immutable directory. Switching the
        # Nginx file atomically can never expose a new cert with an old key, even
        # if the machine loses power during renewal.
        revision = hashlib.sha256((request["fullchain_pem"] + request["private_key_pem"]).encode()).hexdigest()
        certificate_dir = self.config / "certificates" / revision
        safe_directory(certificate_dir)
        chain_path, key_path = certificate_dir / "fullchain.pem", certificate_dir / "private-key.pem"
        paths = [self.state_path, self.config / "s3.json", self.unit, self.nginx]
        snapshot = {path: (path.read_bytes(), stat.S_IMODE(path.stat().st_mode), path.stat().st_gid) if path.exists() else None for path in paths}
        stage = "certificate validation"
        try:
            atomic_write(chain_path, request["fullchain_pem"].encode())
            atomic_write(key_path, request["private_key_pem"].encode())
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(str(chain_path), str(key_path))
            command(["openssl", "x509", "-in", str(chain_path), "-noout", "-checkhost", host, "-checkend", "86400"], 10)
            stage = "verified engine installation"
            install_engine()
            self.write_state(state, gid)
            atomic_write(self.unit, service_template(f"https://{host}").encode(), 0o644)
            atomic_write(self.nginx, nginx_template(host, revision).encode(), 0o644)
            relabel([self.config, self.nginx, Path(ENGINE)])
            stage = "Nginx configuration validation"
            command(["nginx", "-t"], 20)
            stage = "storage service startup"
            command(["systemctl", "daemon-reload"])
            command(["systemctl", "enable", "--now", UNIT], 130)
            stage = "storage readiness"
            self.healthy(state)
            stage = "HTTPS activation"
            command(["systemctl", "reload", "nginx"])
            wait_for_https(state, request["fullchain_pem"])
        except Exception as error:
            reason = str(error) if isinstance(error, StorageError) else "Check the certificate, DNS, dependency or service configuration."
            initial_status = service_status_summary(UNIT)
            rollback_failures = []
            stop_failed = False
            # Disable a newly-created unit while its file still exists; deleting
            # it first makes systemctl disable fail and can leave the engine up.
            if not previous and self.unit.exists():
                try:
                    command(["systemctl", "disable", "--now", UNIT])
                except Exception:
                    stop_failed = True
                    rollback_failures.append("stop new storage service")
            labels = {self.state_path: "restore storage state", self.config / "s3.json": "restore engine credentials",
                      self.unit: "restore service unit", self.nginx: "restore Nginx configuration"}
            for path, saved in snapshot.items():
                # If stopping a new service failed, retain its identity/config so
                # a retry or operator can recover it without losing live keys.
                if stop_failed and saved is None and path in (self.state_path, self.config / "s3.json", self.unit):
                    continue
                try:
                    if saved is None:
                        path.unlink(missing_ok=True)
                    else:
                        atomic_write(path, saved[0], saved[1], saved[2])
                except Exception:
                    rollback_failures.append(labels.get(path, "restore managed configuration"))
            for label, args, timeout in [
                ("reload systemd units", ["systemctl", "daemon-reload"], 60),
                ("validate restored Nginx configuration", ["nginx", "-t"], 20),
                ("reload Nginx", ["systemctl", "reload", "nginx"], 60),
            ]:
                try:
                    command(args, timeout)
                except Exception:
                    rollback_failures.append(label)
            message = f"Storage setup failed during {stage}. {reason} Storage service: {initial_status}. "
            if rollback_failures:
                raise StorageSetupError(message + "Rollback failed for: " + ", ".join(rollback_failures) + ". Inspect the server; object data was retained.", stage) from None
            raise StorageSetupError(message + "Configuration was restored and object data retained.", stage) from None
        return self.status()

    def allocate(self, request):
        state = self.state()
        if not state:
            raise StorageError("Activate Qinpanel storage first.")
        key, bucket, owner = request["connection_id"], bucket_name(request["bucket"]), request["owner_id"]
        record = state["buckets"].get(key)
        if record and (record["owner_id"] != owner or record["bucket"] != bucket):
            raise StorageError("Bucket ownership cannot be changed.")
        if record and record.get("deleted"):
            raise StorageError("This allocation has been removed. Create a new allocation identity.")
        if any(item["bucket"] == bucket and not item.get("deleted_confirmed") and item_id != key for item_id, item in state["buckets"].items()):
            raise StorageError("That bucket is already allocated.")
        new = copy.deepcopy(state)
        supplied = credentials(request["credentials"])
        if request["action"] == "provision":
            if record and record["panel"] != supplied:
                raise StorageError("A different credential already exists for this bucket.")
            new["buckets"][key] = record or {"owner_id": owner, "bucket": bucket, "panel": supplied, "generation": 0, "application": None}
        else:
            generation = request["generation"]
            if not record or generation != record["generation"] + 1:
                # Idempotent retry accepts exactly the same desired key, never rolls back a generation.
                if not (record and generation == record["generation"] and record["application"] == supplied):
                    raise StorageError("The application key changed. Reload before rotating it again.")
            if generation == record["generation"] + 1 and record.get("application"):
                new["buckets"][key]["pending_retired"] = record["application"]
            new["buckets"][key].update(generation=generation, application=supplied)
        _, gid = self.identity()
        try:
            self.write_state(new, gid)
            # A pinned restart makes key removal authoritative, without a reload race.
            command(["systemctl", "restart", UNIT], 130)
            self.healthy(new)
            if request["action"] == "provision":
                code = s3_request(new, "HEAD", f"/{bucket}")
                if code == 404:
                    code = s3_request(new, "PUT", f"/{bucket}")
                if code not in (200, 204):
                    raise StorageError("Bucket provisioning was not confirmed.")
                if s3_request(new, "HEAD", f"/{bucket}", supplied) != 200:
                    raise StorageError("Scoped bucket access was not confirmed.")
            elif s3_request(new, "HEAD", f"/{bucket}", supplied) != 200:
                raise StorageError("Application key access was not confirmed.")
            retired = new["buckets"][key].get("pending_retired")
            if retired:
                if s3_request(new, "HEAD", f"/{bucket}", retired) != 403:
                    raise StorageError("Previous application key revocation was not confirmed.")
                del new["buckets"][key]["pending_retired"]
                self.write_state(new, gid)
        except Exception:
            # Retain the reservation for unknown outcomes, preventing another tenant
            # from claiming a possibly-created bucket. An identical request reconciles it.
            raise StorageError("Storage operation was interrupted. Retry the same allocation/key operation to reconcile; data was retained.") from None
        return {"ready": True, "generation": new["buckets"][key]["generation"]}

    def decommission(self, request):
        state = self.state()
        key = request["connection_id"]
        record = state["buckets"].get(key) if state else None
        if not record or record["owner_id"] != request["owner_id"] or record["bucket"] != request["bucket"]:
            raise StorageError("The bucket allocation does not match its recorded owner.")
        if not record.get("deleted"):
            # The engine atomically rejects nonempty buckets. No recursive
            # delete, filesystem path or force flag is accepted by this helper.
            code = s3_request(state, "DELETE", f"/{record['bucket']}")
            if code == 409:
                return {"removed": False, "not_empty": True}
            if code not in (200, 204, 404):
                raise StorageError("Empty-bucket removal was not confirmed. Reconcile the same removal request.")
            record.update(deleted=True)
        _, gid = self.identity()
        self.write_state(state, gid)
        command(["systemctl", "restart", UNIT], 130)
        self.healthy(state)
        for keys in (record["panel"], record.get("application"), record.get("pending_retired")):
            if keys and s3_request(state, "HEAD", f"/{record['bucket']}", keys) != 403:
                raise StorageError("Bucket key revocation was not confirmed. Resume the same removal operation.")
        record.update(deleted_confirmed=True, application=None)
        record.pop("pending_retired", None)
        self.write_state(state, gid)
        return {"removed": True}


def socket_ready():
    import pwd
    uid = pwd.getpwnam(SERVICE_USER).pw_uid
    for _ in range(90):
        try:
            info = os.lstat(SOCKET)
            if not stat.S_ISSOCK(info.st_mode) or info.st_uid != uid:
                raise StorageError("Unexpected storage socket identity.")
            command(["setfacl", "-m", "u:panel-api:--x,u:www-data:--x", "/run/qonpanel-storage"], 10)
            command(["setfacl", "-m", "u:panel-api:rw-,u:www-data:rw-", SOCKET], 10)
            relabel([Path("/run/qonpanel-storage")])
            return
        except FileNotFoundError:
            time.sleep(1)
    raise StorageError("Storage S3 socket did not become ready.")


def main():
    if sys.platform != "linux" or os.geteuid() != 0:
        raise StorageError("The managed storage helper requires the Linux system agent.")
    if sys.argv[1:] == ["--socket-ready"]:
        socket_ready()
        return
    if sys.argv[1:]:
        raise StorageError("This helper accepts typed stdin only.")
    request = validate_request(json.loads(sys.stdin.buffer.read(196609)))
    import fcntl
    safe_directory(Path("/run/qonpanel-storage-control"))
    fd = os.open("/run/qonpanel-storage-control/control.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise StorageError("Another storage operation is running. Wait and reload its status.") from None
        controller = Controller()
        if request["action"] == "status":
            result = controller.status()
        elif request["action"] == "configure":
            result = controller.configure(request)
        elif request["action"] == "decommission":
            result = controller.decommission(request)
        else:
            result = controller.allocate(request)
        print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except StorageError as error:
        print(json.dumps(error_response(error)))
        # systemd must fail startup when the post-start socket ACL is missing.
        # Typed stdin returns its safe JSON error through the agent instead.
        sys.exit(1 if sys.argv[1:] == ["--socket-ready"] else 0)
    except Exception:
        # Never emit request, PEM, credentials, upstream response or a traceback.
        print(json.dumps({"error": "Managed storage operation failed. Inspect the service journal on the server."}))
        sys.exit(1)
