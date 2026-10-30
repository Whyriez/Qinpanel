"""Disposable GitHub Ubuntu VM acceptance. Never run on a developer/server host.

Exercises the real pinned engine, systemd namespace, Unix permissions and TLS
Nginx path using two tenants' S3 keys. Rust API acceptance remains a separate job.
"""
import datetime
import hashlib
import hmac
import http.client
import importlib.util
import json
import os
from pathlib import Path
import pwd
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.parse
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
HOST = "storage.qonpanel.test"


def run(args, **options):
    return subprocess.run(args, capture_output=True, check=True, **options).stdout


def control(request):
    result = json.loads(run(["python3", "/usr/local/libexec/qonpanel-object-storage"], input=json.dumps(request).encode(), timeout=480))
    if "error" in result:
        raise AssertionError(result["error"])
    return result


def prepare_nginx():
    # apt installation may leave an already-installed service inactive on runners.
    # The fixture owns this disposable VM; production configuration never silently
    # starts a shared Nginx service as part of rolling back storage.
    run(["nginx", "-t"], timeout=20)
    run(["systemctl", "start", "nginx"], timeout=60)
    run(["systemctl", "is-active", "--quiet", "nginx"], timeout=10)


def diagnostics():
    spec = importlib.util.spec_from_file_location("storage_diagnostics", ROOT / "scripts/object-storage-worker.py")
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    for unit in (helper.UNIT, "nginx"):
        print(f"Native diagnostic {unit}: {helper.service_status_summary(unit)}", flush=True)


class LocalHTTPS(http.client.HTTPSConnection):
    def connect(self):
        tcp = socket.create_connection(("127.0.0.1", 443), timeout=15)
        self.sock = ssl.create_default_context().wrap_socket(tcp, server_hostname=HOST)


def request(method, path, keys=None, body=b"", tunnel=False, host=HOST, with_headers=False):
    headers = {"host": host}
    if keys:
        now = datetime.datetime.now(datetime.timezone.utc)
        timestamp, day = now.strftime("%Y%m%dT%H%M%SZ"), now.strftime("%Y%m%d")
        digest = hashlib.sha256(body).hexdigest()
        headers.update({"x-amz-content-sha256": digest, "x-amz-date": timestamp})
        signed = ";".join(sorted(headers))
        parsed = urllib.parse.urlsplit(path)
        query = "&".join(f"{urllib.parse.quote(k, safe='-_.~')}={urllib.parse.quote(v, safe='-_.~')}" for k, v in sorted(urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)))
        canonical = "\n".join([method, parsed.path, query, "".join(f"{k}:{headers[k]}\n" for k in sorted(headers)), signed, digest])
        scope = f"{day}/us-east-1/s3/aws4_request"
        value = f"AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{hashlib.sha256(canonical.encode()).hexdigest()}"
        signing = ("AWS4" + keys["secret_key"]).encode()
        for part in (day, "us-east-1", "s3", "aws4_request"):
            signing = hmac.new(signing, part.encode(), hashlib.sha256).digest()
        signature = hmac.new(signing, value.encode(), hashlib.sha256).hexdigest()
        headers["authorization"] = f"AWS4-HMAC-SHA256 Credential={keys['access_key']}/{scope}, SignedHeaders={signed}, Signature={signature}"
    connection = http.client.HTTPConnection("127.0.0.1", 17891, timeout=15) if tunnel else LocalHTTPS(HOST, timeout=15)
    try:
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        result = (response.status, response.read())
        return (*result, dict(response.getheaders())) if with_headers else result
    finally:
        connection.close()


def presigned_get(path, keys, now=None, host=HOST):
    """Native engine verifies query authentication separately from header signing."""
    now = now or datetime.datetime.now(datetime.timezone.utc)
    timestamp, day = now.strftime("%Y%m%dT%H%M%SZ"), now.strftime("%Y%m%d")
    scope = f"{day}/us-east-1/s3/aws4_request"
    values = {
        "X-Amz-Algorithm": "AWS4-HMAC-SHA256",
        "X-Amz-Credential": f"{keys['access_key']}/{scope}",
        "X-Amz-Date": timestamp, "X-Amz-Expires": "60",
        "X-Amz-SignedHeaders": "host", "response-content-disposition": "attachment",
    }
    query = urllib.parse.urlencode(sorted(values.items()), quote_via=urllib.parse.quote, safe="-_.~")
    canonical = f"GET\n{path}\n{query}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD"
    text = f"AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{hashlib.sha256(canonical.encode()).hexdigest()}"
    signing = ("AWS4" + keys["secret_key"]).encode()
    for part in (day, "us-east-1", "s3", "aws4_request"):
        signing = hmac.new(signing, part.encode(), hashlib.sha256).digest()
    signature = hmac.new(signing, text.encode(), hashlib.sha256).hexdigest()
    return f"{path}?{query}&X-Amz-Signature={signature}"


def wait_for_http01(expected, timeout=30):
    """Reload is asynchronous: require the exact challenge, not just an active unit."""
    deadline = time.monotonic() + timeout
    reason = "No HTTP response received."
    while (remaining := deadline - time.monotonic()) > 0:
        challenge_connection = http.client.HTTPConnection("127.0.0.1", 80, timeout=min(3, remaining))
        try:
            challenge_connection.request("GET", "/.well-known/acme-challenge/native-storage", headers={"host": HOST})
            response = challenge_connection.getresponse()
            sample = response.read(len(expected) + 1)
            matches = sample == expected
            if response.status == 200 and matches:
                return
            reason = f"HTTP {response.status}; sampled_bytes={len(sample)}; expected_bytes={len(expected)}; body_matches={matches}."
        except (OSError, http.client.HTTPException):
            reason = "HTTP connection failed or returned an incomplete response."
        finally:
            challenge_connection.close()
        if (remaining := deadline - time.monotonic()) > 0:
            time.sleep(min(0.5, remaining))
    raise AssertionError("HTTP-01 challenge did not become ready within the deadline. " + reason)


def exercise_listed_object_keys(keys, tunnel=False):
    """Actual S3 list -> decode -> GET/DELETE/HEAD, with distinct collision keys."""
    prefix = "key-roundtrip/"
    names = ["WhatsApp Image 2026-10-02 at 20.58.22.jpeg", "WhatsApp+Image+2026-10-02+at+20.58.22.jpeg", "foto + café %2B.txt"]
    paths, expected = {}, {}
    for index, name in enumerate(names):
        key = prefix + name
        paths[key] = "/alice-media/" + urllib.parse.quote(key, safe="/-_.~")
        expected[key] = f"roundtrip-{index}".encode()
        assert request("PUT", paths[key], keys, expected[key], tunnel=tunnel)[0] in (200,201)
    query = urllib.parse.urlencode({"list-type":"2", "encoding-type":"url", "prefix":prefix, "delimiter":"/"})
    status, body = request("GET", "/alice-media?" + query, keys, tunnel=tunnel)
    assert status == 200
    tree = ET.fromstring(body)
    ns = {"s3":"http://s3.amazonaws.com/doc/2006-03-01/"}
    assert tree.findtext("s3:EncodingType", namespaces=ns) == "url"
    listed = [urllib.parse.unquote_plus(element.text) for element in tree.findall("s3:Contents/s3:Key", ns)]
    assert set(listed) == set(paths), "List encoding must preserve spaces, literal plus, Unicode and literal percent exactly once"
    for index, key in enumerate(listed):
        path = "/alice-media/" + urllib.parse.quote(key, safe="/-_.~")
        assert request("GET", path, keys, tunnel=tunnel) == (200, expected[key])
        assert request("DELETE", path, keys, tunnel=tunnel)[0] in (200,204)
        assert request("HEAD", path, keys, tunnel=tunnel)[0] == 404
        assert request("GET", path, keys, tunnel=tunnel)[0] == 404
        for remaining in listed[index+1:]:
            assert request("GET", paths[remaining], keys, tunnel=tunnel)[0] == 200, "Deleting a spaced filename must not delete the separate literal-plus filename"


def main():
    if os.environ.get("QONPANEL_DISPOSABLE_CI") != "1" or os.environ.get("GITHUB_ACTIONS") != "true" or sys.platform != "linux" or os.geteuid() != 0:
        raise SystemExit("Only an explicitly authorized disposable GitHub Linux VM may run this test.")
    if Path("/etc/qonpanel-storage").exists() or Path("/var/lib/qonpanel-storage").exists():
        raise SystemExit("The CI fixture must start with no storage service/data.")
    run(["install", "-d", "-m", "0755", "/usr/local/libexec"])
    run(["install", "-m", "0755", str(ROOT / "scripts/object-storage-worker.py"), "/usr/local/libexec/qonpanel-object-storage"])
    try:
        pwd.getpwnam("panel-api")
    except KeyError:
        run(["useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin", "panel-api"])
    prepare_nginx()
    with tempfile.TemporaryDirectory(prefix="qonpanel-native-s3-") as directory:
        work = Path(directory)
        run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "3", "-subj", "/CN=Qinpanel disposable CI CA", "-addext", "basicConstraints=critical,CA:TRUE", "-keyout", str(work / "ca.key"), "-out", str(work / "ca.crt")])
        run(["openssl", "req", "-newkey", "rsa:2048", "-nodes", "-subj", f"/CN={HOST}", "-keyout", str(work / "leaf.key"), "-out", str(work / "leaf.csr")])
        (work / "ext").write_text(f"subjectAltName=DNS:{HOST}\nbasicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n")
        run(["openssl", "x509", "-req", "-in", str(work / "leaf.csr"), "-CA", str(work / "ca.crt"), "-CAkey", str(work / "ca.key"), "-CAcreateserial", "-days", "2", "-extfile", str(work / "ext"), "-out", str(work / "leaf.crt")])
        shutil.copyfile(work / "ca.crt", "/usr/local/share/ca-certificates/qonpanel-storage-ci.crt")
        run(["update-ca-certificates"])
        configuration = {"action": "configure", "hostname": HOST, "fullchain_pem": (work / "leaf.crt").read_text() + (work / "ca.crt").read_text(), "private_key_pem": (work / "leaf.key").read_text()}
        try:
            assert control({"action": "status"})["configured"] is False
            assert control(configuration)["running"] is True
            assert control(configuration)["running"] is True  # TLS reconcile preserves data/keys.
            import secrets
            def keys():
                return {"access_key": secrets.token_hex(16), "secret_key": secrets.token_hex(32), "session_token": None}
            alice, bob, rotated = keys(), keys(), keys()
            identities = [
                ("7c03851f-60dd-49d1-a3aa-a2100320e7c9", 2, "alice-media", alice),
                ("ed603234-a37a-446d-b549-83fc0c00b44f", 3, "bob-media", bob),
            ]
            for identifier, owner, bucket, application in identities:
                allocation = {"action": "provision", "connection_id": identifier, "owner_id": owner, "bucket": bucket, "credentials": json.dumps(keys())}
                assert control(allocation)["ready"]
                assert control(allocation)["ready"]  # idempotent native bucket reconciliation
                assert control({**allocation, "action": "application_key", "generation": 1, "credentials": json.dumps(application)})["ready"]
            assert request("GET", "/alice-media?list-type=2", alice)[0] == 200
            exercise_listed_object_keys(alice)
            assert request("PUT", "/alice-media/nested/hello%20world.txt", alice, b"Qinpanel native S3 fixture")[0] in (200, 201)
            assert request("GET", "/alice-media/nested/hello%20world.txt", alice) == (200, b"Qinpanel native S3 fixture")
            signed = presigned_get("/alice-media/nested/hello%20world.txt", alice)
            assert request("GET", signed) == (200, b"Qinpanel native S3 fixture"), "Fresh query-signed GET must read the actual object through TLS Nginx"
            assert request("GET", signed.rsplit("=", 1)[0] + "=" + "0" * 64)[0] == 403, "Invalid signatures must remain denied"
            expired = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(hours=1)
            assert request("GET", presigned_get("/alice-media/nested/hello%20world.txt", alice, expired))[0] == 403
            assert request("GET", presigned_get("/alice-media/nested/hello%20world.txt", alice, host="different.example.test"))[0] == 403, "A rewritten signed Host must not authorize the request"
            assert control({"action": "decommission", "connection_id": identities[0][0], "owner_id": 2, "bucket": "alice-media"}) == {"removed": False, "not_empty": True}
            assert request("GET", "/alice-media/nested/hello%20world.txt")[0] == 403
            access = {"action": "set_bucket_access", "connection_id": identities[0][0], "owner_id": 2, "bucket": "alice-media", "expected_revision": 1, "revision": 2, "public": True}
            assert control(access) == {"ready": True, "public": True, "revision": 2}
            assert control(access)["ready"]  # Idempotent policy retry preserves keys and data.
            public_status, public_body, public_headers = request("GET", "/alice-media/nested/hello%20world.txt", with_headers=True)
            assert (public_status, public_body) == (200, b"Qinpanel native S3 fixture")
            assert public_headers.get("Cache-Control") == "no-store", "Private revocation must not inherit a cached public response"
            assert request("HEAD", "/alice-media/nested/hello%20world.txt")[0] == 200
            for method, path in [("GET", "/alice-media?list-type=2"), ("PUT", "/alice-media/anonymous.txt"), ("DELETE", "/alice-media/nested/hello%20world.txt"), ("GET", "/bob-media/private.txt")]:
                assert request(method, path)[0] == 403, f"Public read must not grant anonymous list/write or another bucket: {method} {path}"
            run(["systemctl", "restart", "qonpanel-object-storage.service"], timeout=130)
            assert request("GET", "/alice-media/nested/hello%20world.txt")[0] == 200
            assert control({**access, "expected_revision": 2, "revision": 3, "public": False})["ready"]
            assert request("GET", "/alice-media/nested/hello%20world.txt")[0] == 403
            assert request("GET", presigned_get("/alice-media/nested/hello%20world.txt", alice))[0] == 200, "Private signed access still uses the existing owner key"
            for method, path in [("GET", "/bob-media?list-type=2"), ("PUT", "/bob-media/forbidden"), ("GET", "/bob-media/forbidden"), ("DELETE", "/bob-media/forbidden"), ("PUT", "/another-bucket"), ("DELETE", "/alice-media"), ("PUT", "/alice-media?policy")]:
                assert request(method, path, alice)[0] == 403, f"Cross-bucket/admin operation must be denied: {method}"
            # The raw filer/master/volume ports live in the service's private network namespace.
            for port in (9333, 8888, 9340, 8333):
                with socket.socket() as probe:
                    probe.settimeout(1)
                    assert probe.connect_ex(("127.0.0.1", port)) != 0
            run(["systemctl", "is-enabled", "--quiet", "qonpanel-object-storage.service"])
            run(["systemctl", "restart", "qonpanel-object-storage.service"], timeout=130)
            assert request("GET", "/alice-media/nested/hello%20world.txt", alice)[0] == 200
            assert control({"action": "application_key", "connection_id": identities[0][0], "owner_id": 2, "bucket": "alice-media", "generation": 2, "credentials": json.dumps(rotated)})["ready"]
            assert request("GET", "/alice-media/nested/hello%20world.txt", alice)[0] == 403
            assert request("GET", "/alice-media/nested/hello%20world.txt", rotated)[0] == 200
            assert request("GET", presigned_get("/alice-media/nested/hello%20world.txt", alice))[0] == 403, "A fresh query signed with the revoked key must fail"
            assert request("GET", presigned_get("/alice-media/nested/hello%20world.txt", rotated)) == (200, b"Qinpanel native S3 fixture")
            before = Path("/etc/qonpanel-storage/state.json").read_bytes()
            assert control({"action": "prepare_http", "hostname": HOST})["prepared"]
            challenge = Path("/var/lib/panel/acme/.well-known/acme-challenge/native-storage")
            challenge.parent.mkdir(parents=True, exist_ok=True)
            challenge.write_text("native HTTP-01 fixture")
            try:
                wait_for_http01(b"native HTTP-01 fixture")
            finally:
                challenge.unlink()
            assert Path("/etc/qonpanel-storage/state.json").read_bytes() == before
            origin = control({"action": "configure_tunnel", "hostname": HOST})
            assert origin["connection_mode"] == "tunnel" and origin["origin_ready"]
            assert origin["endpoint"] == f"https://{HOST}" and not origin["public_access_verified"]
            after = json.loads(Path("/etc/qonpanel-storage/state.json").read_bytes())
            assert after["root"] == json.loads(before)["root"] and after["buckets"] == json.loads(before)["buckets"]
            object_path = "/alice-media/nested/hello%20world.txt"
            exercise_listed_object_keys(rotated, tunnel=True)
            assert request("GET", object_path, rotated, tunnel=True) == (200, b"Qinpanel native S3 fixture")
            assert request("GET", presigned_get(object_path, rotated), tunnel=True) == (200, b"Qinpanel native S3 fixture")
            assert request("GET", object_path, tunnel=True)[0] == 403
            assert control({**access, "expected_revision": 3, "revision": 4})["ready"]
            assert request("GET", object_path, tunnel=True) == (200, b"Qinpanel native S3 fixture")
            assert request("HEAD", object_path, tunnel=True)[0] == 200
            assert request("GET", "/alice-media?list-type=2", tunnel=True)[0] == 403
            assert control({**access, "expected_revision": 4, "revision": 5, "public": False})["ready"]
            assert request("GET", object_path, tunnel=True)[0] == 403
            assert request("GET", "/bob-media?list-type=2", rotated, tunnel=True)[0] == 403
            assert request("GET", presigned_get(object_path, alice), tunnel=True)[0] == 403
            assert request("PUT", "/alice-media/tunnel.txt", rotated, b"loopback fixture", tunnel=True)[0] == 200
            assert request("GET", "/alice-media/tunnel.txt", rotated, tunnel=True) == (200, b"loopback fixture")
            assert request("DELETE", "/alice-media/tunnel.txt", rotated, tunnel=True)[0] == 204
            try:
                request("GET", object_path, rotated, tunnel=True, host="other.example.test")
                raise AssertionError("Unknown hostname must not reach storage")
            except http.client.RemoteDisconnected:
                pass
            assert request("GET", object_path, rotated)[0] == 200, "Keep prior HTTPS while route changes"
            assert control({"action": "configure_tunnel", "hostname": HOST})["origin_ready"]
            assert request("GET", "/bob-media?list-type=2", bob)[0] == 200
            assert request("DELETE", "/alice-media/nested/hello%20world.txt", rotated)[0] == 204
            assert control({"action": "decommission", "connection_id": identities[0][0], "owner_id": 2, "bucket": "alice-media"})["removed"]
            assert request("GET", "/alice-media?list-type=2", rotated)[0] == 403
            assert request("GET", "/bob-media?list-type=2", bob)[0] == 200
            # panel-api has only the authenticated S3 socket, never filer/config/root keys.
            run(["runuser", "-u", "panel-api", "--", "python3", "-c", "import socket;s=socket.socket(socket.AF_UNIX);s.connect('/run/qonpanel-storage/s3.sock');s.close()"])
            denied = subprocess.run(["runuser", "-u", "nobody", "--", "python3", "-c", "import socket;s=socket.socket(socket.AF_UNIX);s.connect('/run/qonpanel-storage/s3.sock')"], capture_output=True)
            assert denied.returncode != 0, "Other Unix users must not bypass the authenticated HTTPS endpoint"
            for path in ["/etc/qonpanel-storage/state.json", "/etc/qonpanel-storage/s3.json", "/run/qonpanel-storage/filer.sock"]:
                denied = subprocess.run(["runuser", "-u", "panel-api", "--", "test", "-r", path], capture_output=True)
                assert denied.returncode != 0, "API must not read native root credentials/control sockets"
            print("Managed storage native acceptance passed: real TLS proxy, engine, tenant isolation, key rotation, restart persistence and Unix access boundaries.")
        except Exception:
            try:
                diagnostics()
            except Exception:
                print("Native service diagnostics unavailable; original failure follows.", flush=True)
            raise
        finally:
            subprocess.run(["systemctl", "disable", "--now", "qonpanel-object-storage.service"], capture_output=True)


if __name__ == "__main__":
    main()
