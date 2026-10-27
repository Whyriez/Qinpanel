#!/usr/bin/env python3
"""Publish only verified release payloads. CI credentials never enter artifacts."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import ssl
import sys
import tarfile
import time
import urllib.parse

LIMIT = 536_870_912
TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")
MARKER = "PACKAGED_RELEASE_BASE_URL=''"
POINTER_RETRY_DELAYS = (0, 2, 4, 8, 16, 30)
POINTER_LIMIT = 131072


class PublicationError(ValueError):
    """Controlled operator message; never includes an upstream response or key."""


class PublicDownloadError(PublicationError):
    def __init__(self, status):
        self.status = status
        super().__init__("Public download failed (HTTP " + str(status) + ")")


def storage_object_missing(status, data):
    # Storage REST can wrap a missing key in HTTP 400. Other failures stay fatal.
    if status == 404:
        return True
    if status != 400 or len(data) > 4096:
        return False
    try:
        value = json.loads(data)
    except (ValueError, UnicodeError):
        return False
    if not isinstance(value, dict) or str(value.get("statusCode")) != "404":
        return False
    return value.get("code") == "NoSuchKey" or (
        "code" not in value
        and value.get("error") == "not_found"
        and value.get("message") == "Object not found"
    )



def base_url(value):
    if len(value) > 512 or not re.fullmatch(r"https://[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?(?:/[A-Za-z0-9_-]+)*", value):
        raise PublicationError("Invalid HTTPS distribution URL")
    host = urllib.parse.urlsplit(value).hostname
    if not host or "." not in host or ".." in host:
        raise PublicationError("Invalid distribution hostname")
    return value


def render_bootstrap(source, base):
    base_url(base)
    if source.count(MARKER) != 1:
        raise PublicationError("Bootstrap source does not have one release marker")
    return source.replace(MARKER, "PACKAGED_RELEASE_BASE_URL='" + base + "'").encode()


def digest_file(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def parse_checksums(text):
    declared = {}
    for line in text.splitlines():
        match = re.fullmatch(r"([a-f0-9]{64})  ([A-Za-z0-9_.-]+)", line)
        if not match or match[2] in declared:
            raise PublicationError("Invalid/duplicate checksum entry")
        declared[match[2]] = match[1]
    return declared


def verify_archive(path, tag, base):
    root = path.name[:-7]
    seen = set()
    bootstrap = None
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            name = PurePosixPath(member.name)
            if not name.parts or name.is_absolute() or ".." in name.parts or name.parts[0] != root or member.name in seen:
                raise PublicationError("Unsafe or duplicate archive member")
            seen.add(member.name)
            if not member.isdir() and not member.isfile():
                raise PublicationError("Release must not contain links or special files")
            if any(p in (".git", ".github", "docs", "src", "node_modules") or p.startswith(".env") for p in name.parts):
                raise PublicationError("Private source/configuration found in release")
            if name.suffix.lower() in (".rs", ".ts", ".tsx", ".map", ".md") or name.name in ("Cargo.toml", "Cargo.lock"):
                raise PublicationError("Private source/debug documentation found in release")
            if member.name == root + "/scripts/bootstrap-install.sh":
                if not member.isfile() or member.size > 131072:
                    raise PublicationError("Bundled bootstrap too large")
                bootstrap = archive.extractfile(member).read()
                if "PACKAGED_RELEASE_BASE_URL='" + base + "'" not in bootstrap.decode():
                    raise PublicationError("Bundled bootstrap and publisher source differ")
    required = ["bin/panel-api", "bin/panel-agent", "bin/panelctl", "install.sh", "uninstall.sh", "UPGRADE_FROM", "scripts/bootstrap-install.sh", "scripts/panel-update-worker.sh", "web/dist/index.html"]
    if not all(root + "/" + name in seen for name in required):
        raise PublicationError("Incomplete release payload")
    return bootstrap


def prepare(directory, tag, base, bootstrap):
    base_url(base)
    if not re.fullmatch(r"v(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})", tag):
        raise PublicationError("Distribution requires a stable version tag")
    actual = {p.name for p in directory.iterdir()}
    compatibility = {f"{brand}-{tag}-{target}.tar.gz" for brand in ("qonpanel", "nusa-panel") for target in TARGETS}
    canonical = {f"qinpanel-{tag}-{target}.tar.gz" for target in TARGETS}
    # Resuming an original build must not rename or repackage its immutable set.
    new_layout = compatibility | canonical
    expected = new_layout if actual == new_layout | {"SHA256SUMS"} else compatibility
    if actual != expected | {"SHA256SUMS"} or any(p.is_symlink() or not p.is_file() for p in directory.iterdir()):
        raise PublicationError("Unexpected/missing release file; only explicit packaged assets may be published")
    declared = parse_checksums((directory / "SHA256SUMS").read_text())
    if set(declared) != expected:
        raise PublicationError("Checksum set does not match release artifacts")
    assets = []
    # Retain the helper signature, but publish the installer from the verified
    # build. Reusing old artifacts after a rebrand must not regenerate its bytes
    # or immutable release metadata from the current checkout's brand.
    packaged_bootstrap = None
    for name in sorted(expected):
        path = directory / name
        size = path.stat().st_size
        if not 0 < size <= LIMIT or digest_file(path) != declared[name]:
            raise PublicationError("Invalid release size or checksum")
        bundled = verify_archive(path, tag, base)
        if packaged_bootstrap is not None and packaged_bootstrap != bundled:
            raise PublicationError("Release archives contain different bundled bootstraps")
        packaged_bootstrap = bundled
        assets.append({"name": name, "size": size, "state": "uploaded", "digest": "sha256:" + declared[name]})
    assets.append({"name": "SHA256SUMS", "state": "uploaded", "size": (directory / "SHA256SUMS").stat().st_size})
    if packaged_bootstrap is None:
        raise PublicationError("Bundled installer is missing")
    brand = "Qinpanel" if b"Qinpanel Installer" in packaged_bootstrap else "QonPanel"
    manifest = {"tag_name": tag, "draft": False, "prerelease": False, "body": brand + " " + tag, "assets": assets}
    return json.dumps(manifest, separators=(",", ":")).encode(), packaged_bootstrap


def prepare_github_assets(directory, destination, manifest):
    """Select canonical assets from verified payloads without rewriting archives."""
    if destination.resolve().is_relative_to(directory.resolve()):
        raise PublicationError("Keep GitHub staging outside the verified distribution directory")
    assets = [asset for asset in json.loads(manifest)["assets"] if asset["name"].endswith(".tar.gz")]
    verified_checksums = {asset["name"]: asset["digest"].removeprefix("sha256:") for asset in assets}
    archives = list(verified_checksums)
    canonical = [name for name in archives if name.startswith("qinpanel-")]
    selected = canonical or archives
    checksums = parse_checksums((directory / "SHA256SUMS").read_text())
    if checksums != verified_checksums:
        raise PublicationError("GitHub checksum set changed after release verification")
    try:
        destination.mkdir()  # Never mix stale assets with the verified selection.
    except FileExistsError:
        raise PublicationError("GitHub staging directory already exists; use a new output directory") from None
    for name in selected:
        shutil.copyfile(directory / name, destination / name)
        if digest_file(destination / name) != checksums[name]:
            raise PublicationError("GitHub asset changed after release verification")
    if canonical:
        (destination / "SHA256SUMS").write_bytes("".join(checksums[name] + "  " + name + "\n" for name in sorted(selected)).encode())
    else:
        # Original artifacts may predate the canonical filenames. Preserve their
        # bytes/names, including the original checksum file, when resuming.
        shutil.copyfile(directory / "SHA256SUMS", destination / "SHA256SUMS")
        print("Original artifacts use historical filenames; preserving the original release assets.")


class Store:
    def __init__(self, project, bucket, key):
        if not re.fullmatch(r"https://[a-z0-9]+\.supabase\.co", project):
            raise PublicationError("Use the HTTPS Supabase project URL")
        if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,62}", bucket) or not key or any(ord(c) < 33 or ord(c) > 126 for c in key):
            raise PublicationError("Missing/invalid bucket or CI secret")
        self.host = urllib.parse.urlsplit(project).hostname
        self.bucket = bucket
        self.key = key
        self.public = project + "/storage/v1/object/public/" + bucket

    def request(self, method, path, body=None, content_type="application/octet-stream", overwrite=False):
        conn = http.client.HTTPSConnection(self.host, timeout=600, context=ssl.create_default_context())
        try:
            headers = {"Authorization": "Bearer " + self.key, "apikey": self.key}
            if method == "GET":
                headers["Cache-Control"] = "no-cache"
            if body is not None:
                headers.update({"Content-Type": content_type, "Cache-Control": "max-age=0" if overwrite else "max-age=31536000", "x-upsert": "true" if overwrite else "false"})
                headers["Content-Length"] = str(body.stat().st_size if isinstance(body, Path) else len(body))
            if isinstance(body, Path):
                with body.open("rb") as stream:
                    conn.request(method, path, body=stream, headers=headers)
            else:
                conn.request(method, path, body=body, headers=headers)
            response = conn.getresponse()
            status = response.status
            data = response.read(65537)
            if len(data) > 65536:
                raise PublicationError("Storage response too large")
            return status, data
        finally:
            conn.close()

    def preflight(self, largest, tag):
        status, data = self.request("GET", "/storage/v1/bucket/" + self.bucket)
        if status != 200:
            raise PublicationError("Cannot access release bucket; check the Actions secret and bucket")
        bucket = json.loads(data)
        if not isinstance(bucket, dict):
            raise PublicationError("Invalid bucket metadata")
        limit = bucket.get("file_size_limit")
        if bucket.get("public") is not True or limit is not None and largest > int(limit):
            raise PublicationError("Use a public artifact bucket with a sufficient file size limit")
        status, data = self.request("GET", "/storage/v1/object/public/" + self.bucket + "/latest.json")
        if storage_object_missing(status, data):
            return
        if status != 200:
            raise PublicationError("Cannot check current stable release (HTTP " + str(status) + "); check Storage access")
        previous_metadata = json.loads(data)
        if not isinstance(previous_metadata, dict):
            raise PublicationError("Invalid previous stable metadata")
        previous = previous_metadata.get("tag_name", "")
        if not isinstance(previous, str) or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", previous):
            raise PublicationError("Invalid previous stable metadata")
        if tuple(map(int, previous[1:].split('.'))) > tuple(map(int, tag[1:].split('.'))):
            raise PublicationError("An older workflow must not downgrade latest")

    def upload(self, name, body, content_type, overwrite=False):
        path = "/storage/v1/object/" + self.bucket + "/" + name
        status, _ = self.request("POST", path, body, content_type, overwrite)
        if status not in (200, 201):
            raise PublicationError("Storage upload failed (HTTP " + str(status) + "); check size limits/credentials; existing versions are immutable")


def public_digest(url, *, limit=LIMIT, timeout=600):
    parsed = urllib.parse.urlsplit(url)
    base_url("https://" + parsed.netloc)
    conn = http.client.HTTPSConnection(parsed.hostname, timeout=timeout, context=ssl.create_default_context())
    try:
        conn.request("GET", parsed.path, headers={"Cache-Control": "no-cache", "Accept-Encoding": "identity"})
        response = conn.getresponse()
        if response.status == 404:
            return None
        if (response.status == 400
                and re.fullmatch(r"[a-z0-9]+\.supabase\.co", parsed.hostname or "")
                and parsed.path.startswith("/storage/v1/object/public/")
                and storage_object_missing(response.status, response.read(4097))):
            return None
        if response.status != 200:
            raise PublicDownloadError(response.status)
        checksum, size = hashlib.sha256(), 0
        while True:
            chunk = response.read(min(65536, limit - size + 1))
            if not chunk:
                break
            size += len(chunk)
            if size > limit:
                raise PublicationError("Public download exceeds release limit")
            checksum.update(chunk)
        return checksum.hexdigest()
    finally:
        conn.close()


def public_checksum_bytes(url):
    parsed = urllib.parse.urlsplit(url)
    base_url("https://" + parsed.netloc)
    conn = http.client.HTTPSConnection(parsed.hostname, timeout=60, context=ssl.create_default_context())
    try:
        conn.request("GET", parsed.path, headers={"Cache-Control": "no-cache", "Accept-Encoding": "identity"})
        response = conn.getresponse()
        if response.status != 200:
            raise PublicationError("Cannot compare existing SHA256SUMS (HTTP " + str(response.status) + ")")
        data = response.read(65537)
        if len(data) > 65536:
            raise PublicationError("Existing checksum manifest too large")
        return data
    finally:
        conn.close()


def same_checksums(url, path, existing):
    data = public_checksum_bytes(url)
    # Preserve the published bytes and manifest size; only line order may differ.
    if len(data) != path.stat().st_size or hashlib.sha256(data).hexdigest() != existing:
        return False
    try:
        return parse_checksums(data.decode("utf-8")) == parse_checksums(path.read_text())
    except (UnicodeError, PublicationError):
        return False


def pointer_observation(url, expected):
    try:
        actual = public_digest(url, limit=POINTER_LIMIT, timeout=15)
    except PublicDownloadError as error:
        return "HTTP " + str(error.status), error.status in (408, 429) or 500 <= error.status <= 599
    except PublicationError:
        return "invalid or oversized response", False
    except (OSError, http.client.HTTPException):
        return "network error", True
    if actual is None:
        return "HTTP 404", True
    if actual != expected:
        return "SHA256 mismatch (expected " + expected[:12] + ", received " + actual[:12] + ")", True
    return "verified", False


def verify_public_pointer(store, base, name, body):
    expected = hashlib.sha256(body).hexdigest()
    # Read the ordinary installer/updater URLs: a cache-busted response alone
    # cannot prove that users will receive the newly promoted pointer.
    for attempt, delay in enumerate(POINTER_RETRY_DELAYS, start=1):
        if delay:
            time.sleep(delay)
        origin, origin_retry = pointer_observation(store.public + "/" + name, expected)
        domain, domain_retry = pointer_observation(base + "/" + name, expected)
        if origin == domain == "verified":
            return
        detail = "Supabase=" + origin + "; download domain=" + domain
        if ((origin != "verified" and not origin_retry)
                or (domain != "verified" and not domain_retry)
                or attempt == len(POINTER_RETRY_DELAYS)):
            raise PublicationError(
                "Public pointer verification failed for " + name + ": " + detail
                + ". Check the bucket, Worker route and cache; pointers may already be updated."
                + " Retry with the original artifacts; do not rebuild or overwrite this version."
            )
        print("Waiting for " + name + " propagation (attempt " + str(attempt) + "/"
              + str(len(POINTER_RETRY_DELAYS)) + "; " + detail + ")", file=sys.stderr)


def publish(store, directory, tag, base, manifest, installer):
    store.preflight(max(p.stat().st_size for p in directory.iterdir()), tag)
    versioned = [("releases/" + tag + "/" + p.name, p, "application/gzip" if p.name.endswith(".tar.gz") else "text/plain") for p in sorted(directory.iterdir(), key=lambda item: item.name)]
    versioned += [("releases/" + tag + "/install.sh", installer, "text/plain"), ("releases/" + tag + "/release.json", manifest, "application/json")]
    # Never overwrite a version. A retry skips identical objects; conflicts stop.
    for name, body, mime in versioned:
        expected = digest_file(body) if isinstance(body, Path) else hashlib.sha256(body).hexdigest()
        existing = public_digest(store.public + "/" + name)
        if existing is None:
            try:
                store.upload(name, body, mime)
            except (OSError, ValueError):
                # Recover an ambiguous successful POST only after read-back.
                if public_digest(store.public + "/" + name) != expected:
                    raise
        elif existing != expected:
            if isinstance(body, Path) and body.name == "SHA256SUMS" and same_checksums(store.public + "/" + name, body, existing):
                expected = existing
            else:
                raise PublicationError("Version already exists with different content: " + name + "; retry the original build artifacts or use a new version")
        if public_digest(store.public + "/" + name) != expected or public_digest(base + "/" + name) != expected:
            raise PublicationError("Published object/domain verification failed; latest was not promoted")
    # Publish mutable pointers last. Old latest remains valid on partial failure.
    for name, body, mime in [("install.sh", installer, "text/plain"), ("latest.json", manifest, "application/json")]:
        store.upload(name, body, mime, overwrite=True)
        verify_public_pointer(store, base, name, body)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=Path("dist"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--github-directory", type=Path, help="Stage canonical GitHub assets; Storage retains compatibility downloads")
    args = parser.parse_args()
    base = os.environ.get("QONPANEL_RELEASE_BASE_URL", "")
    manifest, installer = prepare(args.directory, args.tag, base, Path("scripts/bootstrap-install.sh"))
    if args.github_directory is not None:
        prepare_github_assets(args.directory, args.github_directory, manifest)
    store = Store(os.environ.get("SUPABASE_URL", ""), os.environ.get("SUPABASE_BUCKET", "qonpanel-releases"), os.environ.get("SUPABASE_SERVICE_ROLE_KEY", ""))
    publish(store, args.directory, args.tag, base, manifest, installer)
    print("Verified release published: " + base + "/install.sh")


if __name__ == "__main__":
    try:
        main()
    except PublicationError as error:
        print("Distribution publication failed: " + str(error), file=sys.stderr)
        sys.exit(1)
    except (ValueError, OSError, http.client.HTTPException, tarfile.TarError):
        # Do not echo raw network responses or credential-bearing exception text.
        print("Distribution publication failed. Check bucket limits, Actions configuration and domain availability; no credentials were printed.", file=sys.stderr)
        sys.exit(1)
