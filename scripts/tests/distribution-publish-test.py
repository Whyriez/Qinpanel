"""Temporary release archives and fake Storage; no credentials/network/services."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import sys
from unittest.mock import patch

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("publisher", ROOT / "scripts/publish-distribution.py")
pub = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pub)
BASE = "https://download.arelqon.com/qonpanel"
TAG = "v0.1.0"


class FakeStore:
    public = "https://project.supabase.co/storage/v1/object/public/releases"

    def __init__(self):
        self.objects = {}
        self.calls = []
        self.fail = None

    def preflight(self, largest, tag):
        self.calls.append("preflight")

    def upload(self, name, body, mime, overwrite=False):
        self.calls.append(name)
        if self.fail and self.fail in name:
            raise ValueError("fixture failure")
        if name in self.objects and not overwrite:
            raise ValueError("immutable")
        self.objects[name] = body.read_bytes() if isinstance(body, Path) else body

    def digest(self, url, **options):
        data = self.read(url)
        return None if data is None else hashlib.sha256(data).hexdigest()

    def read(self, url):
        prefix = self.public if url.startswith(self.public) else BASE
        name = url[len(prefix) + 1:]
        return self.objects.get(name)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.github_directory = Path(self.enterContext(tempfile.TemporaryDirectory())) / "github"
        self.bootstrap = ROOT / "scripts/bootstrap-install.sh"
        for target in pub.TARGETS:
            self.archive("qinpanel", target)
        self.checksums()

    def archive(self, brand, target, extra=None, base=BASE):
        root = f"{brand}-{TAG}-{target}"
        with tarfile.open(self.directory / (root + ".tar.gz"), "w:gz") as archive:
            for name in ["bin/panel-api", "bin/panel-agent", "bin/panelctl", "install.sh", "uninstall.sh", "UPGRADE_FROM", "scripts/panel-update-worker.sh", "scripts/bootstrap-install.sh", "web/dist/index.html"] + ([extra] if extra else []):
                data = pub.render_bootstrap(self.bootstrap.read_text(), base) if name.endswith("bootstrap-install.sh") else b"fixture"
                member = tarfile.TarInfo(root + "/" + name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def checksums(self):
        (self.directory / "SHA256SUMS").write_bytes("".join(pub.digest_file(p) + "  " + p.name + "\n" for p in sorted(self.directory.glob("*.tar.gz"))).encode())

    def prepare(self):
        return pub.prepare(self.directory, TAG, BASE, self.bootstrap)

    def test_new_release_publishes_only_qinpanel_archives_and_checksums(self):
        manifest, installer = self.prepare()
        value = json.loads(manifest)
        self.assertEqual(len(value["assets"]), 3)
        self.assertEqual(value["body"], "Qinpanel " + TAG)
        store = FakeStore()
        with patch.object(pub, "public_digest", store.digest):
            pub.publish(store, self.directory, TAG, BASE, manifest, installer)
        for target in pub.TARGETS:
            name = f"qinpanel-{TAG}-{target}.tar.gz"
            self.assertEqual(store.objects[f"releases/{TAG}/{name}"], (self.directory / name).read_bytes())
        self.assertEqual(len([name for name in store.objects if name.endswith(".tar.gz")]), 2)
        for old_brand in ("nusa-panel", "qonpanel"):
            self.assertFalse(any(old_brand in name for name in store.objects))
            self.assertNotIn(old_brand.encode(), manifest)
            self.assertNotIn(old_brand.encode(), store.objects[f"releases/{TAG}/SHA256SUMS"])
        self.assertEqual(store.calls[-2:], ["install.sh", "latest.json"])

    def test_github_lists_only_canonical_archives_with_matching_checksums(self):
        manifest, _ = self.prepare()
        original_checksums = (self.directory / "SHA256SUMS").read_bytes()
        pub.prepare_github_assets(self.directory, self.github_directory, manifest)
        expected = {f"qinpanel-{TAG}-{target}.tar.gz" for target in pub.TARGETS}
        self.assertEqual({path.name for path in self.github_directory.iterdir()}, expected | {"SHA256SUMS"})
        checksums = pub.parse_checksums((self.github_directory / "SHA256SUMS").read_text())
        self.assertEqual(set(checksums), expected)
        for name in expected:
            self.assertEqual((self.directory / name).read_bytes(), (self.github_directory / name).read_bytes())
            self.assertEqual(pub.digest_file(self.github_directory / name), checksums[name])
        self.assertEqual((self.directory / "SHA256SUMS").read_bytes(), original_checksums)
        self.assertNotIn(b"qonpanel", (self.github_directory / "SHA256SUMS").read_bytes())
        self.assertNotIn(b"nusa-panel", (self.github_directory / "SHA256SUMS").read_bytes())

    def test_qinpanel_build_resume_preserves_published_filenames_and_bytes(self):
        manifest, _ = self.prepare()
        pub.prepare_github_assets(self.directory, self.github_directory, manifest)
        self.assertEqual({p.name for p in self.directory.iterdir()}, {p.name for p in self.github_directory.iterdir()})
        for path in self.directory.iterdir():
            self.assertEqual(path.read_bytes(), (self.github_directory / path.name).read_bytes())

    def test_cli_stages_and_publishes_only_qinpanel_downloads(self):
        store = FakeStore()
        argv = ["publish-distribution.py", "--directory", str(self.directory), "--tag", TAG,
                "--github-directory", str(self.github_directory)]
        with patch.object(pub.sys, "argv", argv), patch.dict(pub.os.environ, {"QONPANEL_RELEASE_BASE_URL": BASE}), patch.object(pub, "Store", return_value=store), patch.object(pub, "public_digest", store.digest), patch.object(pub.sys, "stdout", io.StringIO()):
            pub.main()
        self.assertEqual(len(list(self.github_directory.glob("*.tar.gz"))), 2)
        self.assertTrue(all(path.name.startswith("qinpanel-") for path in self.github_directory.glob("*.tar.gz")))
        self.assertEqual(len([name for name in store.objects if name.endswith(".tar.gz")]), 2)
        self.assertEqual(store.calls[-2:], ["install.sh", "latest.json"])

    def test_incomplete_canonical_layout_is_rejected(self):
        (self.directory / f"qinpanel-{TAG}-{pub.TARGETS[0]}.tar.gz").unlink()
        self.checksums()
        with self.assertRaisesRegex(pub.PublicationError, "Unexpected/missing"):
            self.prepare()

    def test_legacy_and_mixed_builds_fail_before_staging_or_storage_access(self):
        for brands in (("nusa-panel",), ("qonpanel",), ("qonpanel", "nusa-panel"),
                       ("qinpanel", "nusa-panel"), ("qinpanel", "qonpanel", "nusa-panel")):
            with self.subTest(brands=brands):
                for path in self.directory.iterdir():
                    path.unlink()
                for brand in brands:
                    for target in pub.TARGETS:
                        self.archive(brand, target)
                self.checksums()
                original = {path.name: path.read_bytes() for path in self.directory.iterdir()}
                argv = ["publish-distribution.py", "--directory", str(self.directory), "--tag", TAG,
                        "--github-directory", str(self.github_directory)]
                with patch.object(pub.sys, "argv", argv), patch.dict(pub.os.environ, {"QONPANEL_RELEASE_BASE_URL": BASE}), patch.object(pub, "Store") as store:
                    with self.assertRaisesRegex(pub.PublicationError, "Historical artifact names cannot be published"):
                        pub.main()
                store.assert_not_called()
                self.assertFalse(self.github_directory.exists())
                self.assertEqual(original, {path.name: path.read_bytes() for path in self.directory.iterdir()})

    def test_github_staging_rejects_historical_names_before_creating_output(self):
        manifest, _ = self.prepare()
        value = json.loads(manifest)
        for brand in ("nusa-panel", "qonpanel"):
            value["assets"][0]["name"] = f"{brand}-{TAG}-{pub.TARGETS[0]}.tar.gz"
            with self.subTest(brand=brand), self.assertRaisesRegex(pub.PublicationError, "GitHub assets must use Qinpanel filenames"):
                pub.prepare_github_assets(self.directory, self.github_directory, json.dumps(value).encode())
            self.assertFalse(self.github_directory.exists())

    def test_legacy_checksum_entry_without_its_archive_is_rejected(self):
        checksums = self.directory / "SHA256SUMS"
        original = checksums.read_bytes()
        for brand in ("nusa-panel", "qonpanel"):
            checksums.write_bytes(original + b"0" * 64 + f"  {brand}-{TAG}-{pub.TARGETS[0]}.tar.gz\n".encode())
            with self.subTest(brand=brand), self.assertRaisesRegex(pub.PublicationError, "Checksum set does not match"):
                self.prepare()

    def test_github_staging_refuses_existing_output_and_changed_archives(self):
        manifest, _ = self.prepare()
        self.github_directory.mkdir()
        marker = self.github_directory / "existing"
        marker.write_bytes(b"retain existing output")
        with self.assertRaisesRegex(pub.PublicationError, "staging directory already exists"):
            pub.prepare_github_assets(self.directory, self.github_directory, manifest)
        self.assertEqual(list(self.github_directory.iterdir()), [marker])
        self.assertEqual(marker.read_bytes(), b"retain existing output")
        package = next(self.directory.glob("qinpanel-*.tar.gz"))
        package.write_bytes(package.read_bytes() + b"changed after prepare")
        with self.assertRaisesRegex(pub.PublicationError, "changed after release verification"):
            pub.prepare_github_assets(self.directory, self.github_directory.parent / "changed", manifest)
        self.checksums()  # Changing the checksum file must not bless changed bytes.
        with self.assertRaisesRegex(pub.PublicationError, "checksum set changed"):
            pub.prepare_github_assets(self.directory, self.github_directory.parent / "forged", manifest)
        self.assertFalse((self.github_directory.parent / "forged").exists())

    def test_github_staging_cannot_add_outputs_inside_the_verified_distribution(self):
        manifest, _ = self.prepare()
        original = {path.name for path in self.directory.iterdir()}
        for destination in (self.directory, self.directory / "github"):
            with self.assertRaisesRegex(pub.PublicationError, "outside the verified distribution"):
                pub.prepare_github_assets(self.directory, destination, manifest)
        self.assertEqual({path.name for path in self.directory.iterdir()}, original)

    def test_metadata_matches_all_archives_and_installer_pins_the_same_source(self):
        manifest, installer = self.prepare()
        value = json.loads(manifest)
        self.assertEqual(len(value["assets"]), 3)
        self.assertFalse(value["draft"])
        self.assertIn(("PACKAGED_RELEASE_BASE_URL='" + BASE + "'").encode(), installer)
        self.assertNotIn(b"SUPABASE_SERVICE_ROLE_KEY", installer + manifest)

    def test_qinpanel_artifact_resume_keeps_its_bundled_installer_bytes_and_identity(self):
        current_bootstrap = self.bootstrap
        legacy = Path(self.temporary.name) / "old-bootstrap.sh"
        legacy.write_text(current_bootstrap.read_text().replace("Qinpanel", "QonPanel"))
        self.bootstrap = legacy
        for target in pub.TARGETS:
            self.archive("qinpanel", target)
        self.checksums()
        legacy.unlink()  # Only explicit release artifacts are accepted by prepare.
        self.bootstrap = current_bootstrap
        manifest, installer = self.prepare()
        expected = pub.render_bootstrap(current_bootstrap.read_text().replace("Qinpanel", "QonPanel"), BASE)
        self.assertEqual(installer, expected)
        self.assertEqual(json.loads(manifest)["body"], "QonPanel " + TAG)

    def test_mixed_generation_archives_fail_before_publication(self):
        current = self.bootstrap
        altered = Path(self.temporary.name) / "altered-bootstrap.sh"
        altered.write_text(current.read_text() + "\n# Different source generation\n")
        self.bootstrap = altered
        self.archive("qinpanel", pub.TARGETS[0])
        altered.unlink()
        self.bootstrap = current
        self.checksums()
        with self.assertRaisesRegex(ValueError, "different bundled bootstraps"):
            self.prepare()

    def test_no_source_debug_maps_credentials_or_unexpected_files_are_published(self):
        for extra in ["src/main.rs", ".env", "web/dist/index.js.map", "docs/private.md", ".git/config", "web/dist/source.tsx"]:
            with self.subTest(extra=extra):
                self.archive("qinpanel", pub.TARGETS[0], extra)
                self.checksums()
                with self.assertRaises(ValueError):
                    self.prepare()
        self.archive("qinpanel", pub.TARGETS[0])
        self.checksums()
        (self.directory / "source.zip").write_bytes(b"private")
        with self.assertRaises(ValueError):
            self.prepare()

    def test_corruption_and_mismatched_bundled_source_are_rejected(self):
        package = next(self.directory.glob("*.tar.gz"))
        package.write_bytes(package.read_bytes() + b"changed")
        with self.assertRaises(ValueError):
            self.prepare()
        self.archive("qinpanel", pub.TARGETS[0], base="https://wrong.example.com")
        self.checksums()
        with self.assertRaises(ValueError):
            self.prepare()

    def test_promotion_is_last_and_retry_is_idempotent(self):
        manifest, installer = self.prepare()
        store = FakeStore()
        with patch.object(pub, "public_digest", store.digest):
            pub.publish(store, self.directory, TAG, BASE, manifest, installer)
            self.assertEqual(store.calls[-2:], ["install.sh", "latest.json"])
            store.calls.clear()
            pub.publish(store, self.directory, TAG, BASE, manifest, installer)
            self.assertEqual(store.calls, ["preflight", "install.sh", "latest.json"])

    def test_partial_failure_or_immutable_conflict_keeps_previous_latest(self):
        manifest, installer = self.prepare()
        for conflict in [False, True]:
            store = FakeStore()
            store.objects["latest.json"] = b"old stable metadata"
            if conflict:
                store.objects["releases/" + TAG + "/SHA256SUMS"] = b"different bytes"
            else:
                store.fail = "qinpanel-"
            with patch.object(pub, "public_digest", store.digest), patch.object(pub, "public_checksum_bytes", store.read), self.assertRaises(ValueError):
                pub.publish(store, self.directory, TAG, BASE, manifest, installer)
            self.assertEqual(store.objects["latest.json"], b"old stable metadata")
            self.assertNotIn("install.sh", store.calls)

    def test_checksum_line_order_resumes_without_overwriting_published_bytes(self):
        manifest, installer = self.prepare()
        store = FakeStore()
        key = "releases/" + TAG + "/SHA256SUMS"
        local = (self.directory / "SHA256SUMS").read_bytes()
        remote = b"\n".join(reversed(local.splitlines())) + b"\n"
        self.assertNotEqual(remote, local)
        store.objects[key] = remote
        with patch.object(pub, "public_digest", store.digest), patch.object(pub, "public_checksum_bytes", store.read):
            pub.publish(store, self.directory, TAG, BASE, manifest, installer)
            self.assertEqual(store.objects[key], remote)
            self.assertNotIn(key, store.calls)
            self.assertEqual(store.calls[-2:], ["install.sh", "latest.json"])
            pub.publish(store, self.directory, TAG, BASE, manifest, installer)
        self.assertEqual(store.objects[key], remote)

    def test_changed_duplicate_or_unstable_checksums_still_block_without_promotion(self):
        manifest, installer = self.prepare()
        key = "releases/" + TAG + "/SHA256SUMS"
        local = (self.directory / "SHA256SUMS").read_bytes()
        first = local.splitlines()[0]
        for remote in [b"0" * 64 + local[64:], local + first + b"\n", local[:-1], b"invalid"]:
            store = FakeStore()
            store.objects[key] = remote
            with patch.object(pub, "public_digest", store.digest), patch.object(pub, "public_checksum_bytes", store.read):
                with self.assertRaisesRegex(pub.PublicationError, "different content: releases/v0.1.0/SHA256SUMS"):
                    pub.publish(store, self.directory, TAG, BASE, manifest, installer)
            self.assertEqual(store.calls, ["preflight"])
            self.assertNotIn("latest.json", store.objects)
            self.assertEqual(store.objects[key], remote)
        with patch.object(pub, "public_checksum_bytes", return_value=local):
            self.assertFalse(pub.same_checksums("fixture", self.directory / "SHA256SUMS", "0" * 64))

    def test_domain_failure_does_not_promote_latest(self):
        manifest, installer = self.prepare()
        store = FakeStore()
        with patch.object(pub, "public_digest", lambda url: None if url.startswith(BASE) else store.digest(url)), self.assertRaises(ValueError):
            pub.publish(store, self.directory, TAG, BASE, manifest, installer)
        self.assertNotIn("latest.json", store.objects)

    def test_pointer_propagation_retries_reads_without_reuploading_or_promoting_early(self):
        manifest, installer = self.prepare()
        store = FakeStore()
        reads = {}

        def delayed_digest(url, **options):
            if "/releases/" in url:
                return store.digest(url)
            self.assertEqual(options, {"limit": 131072, "timeout": 15})
            self.assertNotIn("?", url)  # Verify the URL used by real installers.
            reads[url] = reads.get(url, 0) + 1
            if url.endswith("install.sh"):
                self.assertNotIn("latest.json", store.calls)
                if reads[url] <= (2 if url.startswith(store.public) else 3):
                    return hashlib.sha256(b"old cached installer").hexdigest()
            return store.digest(url)

        with patch.object(pub, "public_digest", delayed_digest), patch.object(pub.time, "sleep") as sleep, patch.object(pub.sys, "stderr", io.StringIO()):
            pub.publish(store, self.directory, TAG, BASE, manifest, installer)
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [2, 4, 8])
        self.assertEqual(store.calls[-2:], ["install.sh", "latest.json"])
        self.assertEqual(store.calls.count("install.sh"), 1)
        self.assertEqual(store.calls.count("latest.json"), 1)
        self.assertEqual(store.objects["latest.json"], manifest)

    def test_persistently_wrong_pointer_fails_with_source_and_filename(self):
        manifest, installer = self.prepare()
        for name in ("install.sh", "latest.json"):
            for broken in (BASE, FakeStore.public):
                with self.subTest(name=name, broken=broken):
                    store = FakeStore()

                    def digest(url, **options):
                        if url == broken + "/" + name:
                            return hashlib.sha256(b"wrong content").hexdigest()
                        return store.digest(url)

                    with patch.object(pub, "public_digest", digest), patch.object(pub.time, "sleep") as sleep, patch.object(pub.sys, "stderr", io.StringIO()), self.assertRaises(pub.PublicationError) as error:
                        pub.publish(store, self.directory, TAG, BASE, manifest, installer)
                    message = str(error.exception)
                    self.assertIn("failed for " + name, message)
                    self.assertIn(("download domain" if broken == BASE else "Supabase") + "=SHA256 mismatch", message)
                    self.assertIn(("Supabase" if broken == BASE else "download domain") + "=verified", message)
                    self.assertIn("pointers may already be updated", message)
                    self.assertEqual([call.args[0] for call in sleep.call_args_list], [2, 4, 8, 16, 30])
                    self.assertEqual(store.calls.count(name), 1)
                    if name == "install.sh":
                        self.assertNotIn("latest.json", store.calls)

    def test_pointer_transient_failures_recover_without_leaking_network_details(self):
        for failure in (None, pub.PublicDownloadError(503), pub.PublicDownloadError(429), OSError("fixture_secret"), pub.http.client.HTTPException("fixture_secret")):
            with self.subTest(failure=type(failure).__name__):
                expected = hashlib.sha256(b"installer").hexdigest()
                values = [failure, expected, expected, expected]

                def digest(url, **options):
                    value = values.pop(0)
                    if isinstance(value, Exception):
                        raise value
                    return value

                with patch.object(pub, "public_digest", digest), patch.object(pub.time, "sleep") as sleep, patch.object(pub.sys, "stderr", io.StringIO()) as output:
                    pub.verify_public_pointer(FakeStore(), BASE, "install.sh", b"installer")
                sleep.assert_called_once_with(2)
                self.assertNotIn("fixture_secret", output.getvalue())
                self.assertEqual(values, [])

    def test_pointer_redirect_permissions_and_oversize_fail_without_retry(self):
        for failure, label in ((pub.PublicDownloadError(302), "HTTP 302"), (pub.PublicDownloadError(401), "HTTP 401"), (pub.PublicDownloadError(403), "HTTP 403"), (pub.PublicationError("fixture_secret"), "invalid or oversized response")):
            expected = hashlib.sha256(b"installer").hexdigest()
            with self.subTest(label=label), patch.object(pub, "public_digest", side_effect=[expected, failure]) as digest, patch.object(pub.time, "sleep") as sleep, self.assertRaises(pub.PublicationError) as error:
                pub.verify_public_pointer(FakeStore(), BASE, "install.sh", b"installer")
            self.assertIn("download domain=" + label, str(error.exception))
            self.assertNotIn("fixture_secret", str(error.exception))
            self.assertEqual(digest.call_count, 2)
            sleep.assert_not_called()

    def test_pointer_missing_response_remains_failure_after_bounded_attempts(self):
        expected = hashlib.sha256(b"installer").hexdigest()
        with patch.object(pub, "public_digest", side_effect=[expected, None] * 6) as digest, patch.object(pub.time, "sleep"), patch.object(pub.sys, "stderr", io.StringIO()), self.assertRaisesRegex(pub.PublicationError, "download domain=HTTP 404"):
            pub.verify_public_pointer(FakeStore(), BASE, "install.sh", b"installer")
        self.assertEqual(digest.call_count, 12)

    def test_pointer_digest_enforces_small_read_bound_and_network_timeout(self):
        calls = []
        payload = b"x" * (pub.POINTER_LIMIT + 1)

        class Connection:
            def __init__(self, host, **options):
                calls.append(options["timeout"])
                self.offset = 0

            def request(self, method, path, **options):
                self.assert_headers = options["headers"]
                calls.append(self.assert_headers)

            def getresponse(self):
                return self

            status = 200

            def read(self, count):
                calls.append(count)
                data = payload[self.offset:self.offset + count]
                self.offset += len(data)
                return data

            def close(self):
                calls.append("closed")

        with patch.object(pub.http.client, "HTTPSConnection", Connection), self.assertRaisesRegex(pub.PublicationError, "exceeds release limit"):
            pub.public_digest(BASE + "/install.sh", limit=pub.POINTER_LIMIT, timeout=15)
        self.assertEqual(calls[0], 15)
        self.assertEqual(calls[2:], [65536, 65536, 1, "closed"])
        self.assertNotIn("Authorization", calls[1])

    def test_checksum_readback_is_bounded_and_rejects_redirects_without_credentials(self):
        calls = []
        status, body = 200, b"fixture checksums"

        class Connection:
            def __init__(self, host, **options):
                pass

            def request(self, method, path, **options):
                calls.append(options["headers"])

            def getresponse(self):
                class Response:
                    def __init__(self):
                        self.status = status

                    def read(self, limit):
                        calls.append(limit)
                        return body[:limit]
                return Response()

            def close(self):
                pass

        with patch.object(pub.http.client, "HTTPSConnection", Connection):
            self.assertEqual(pub.public_checksum_bytes(BASE + "/releases/v0.1.0/SHA256SUMS"), body)
            self.assertEqual(calls[-1], 65537)
            self.assertNotIn("Authorization", calls[0])
            status = 302
            with self.assertRaisesRegex(pub.PublicationError, "HTTP 302"):
                pub.public_checksum_bytes(BASE + "/releases/v0.1.0/SHA256SUMS")
            status, body = 200, b" " * 65537
            with self.assertRaisesRegex(pub.PublicationError, "too large"):
                pub.public_checksum_bytes(BASE + "/releases/v0.1.0/SHA256SUMS")

    def test_https_config_rejects_userinfo_paths_and_project_redirects(self):
        for base in ["http://example.com", "https://user:secret@example.com", "https://example.com/a/../b", "https://example.com?token=secret"]:
            with self.assertRaises(ValueError):
                pub.base_url(base)
        with self.assertRaises(ValueError):
            pub.Store("https://evil.test", "releases", "fixture_key")

    def test_preflight_enforces_public_bucket_limit_and_monotonic_latest(self):
        store = pub.Store("https://project.supabase.co", "releases", "fixture_key")
        for bucket, latest in [({"public": False}, None), ({"public": True, "file_size_limit": 2}, None), ({"public": True}, {"tag_name": "v9.0.0"})]:
            values = [(200, json.dumps(bucket).encode())]
            if latest:
                values.append((200, json.dumps(latest).encode()))
            with patch.object(store, "request", side_effect=values), self.assertRaises(ValueError):
                store.preflight(100, TAG)

    def test_storage_missing_key_envelopes_are_narrowly_classified(self):
        for value in [
            {"statusCode": "404", "error": "not_found", "message": "Object not found", "code": "NoSuchKey"},
            {"statusCode": 404, "code": "NoSuchKey"},
            {"statusCode": "404", "error": "not_found", "message": "Object not found"},
        ]:
            self.assertTrue(pub.storage_object_missing(400, json.dumps(value).encode()))
        for value in [
            {"statusCode": "403", "code": "AccessDenied"},
            {"statusCode": "404", "code": "NoSuchBucket"},
            {"statusCode": "404", "error": "not_found", "message": "Bucket not found"},
            {"statusCode": "404", "error": "not_found", "message": "Object not found", "code": "InvalidJWT"},
            [], None, "NoSuchKey", {},
        ]:
            self.assertFalse(pub.storage_object_missing(400, json.dumps(value).encode()))
        self.assertFalse(pub.storage_object_missing(400, b'not JSON'))
        self.assertFalse(pub.storage_object_missing(400, b'\xff'))
        self.assertFalse(pub.storage_object_missing(400, b' ' * 4097))
        self.assertFalse(pub.storage_object_missing(401, b'{"statusCode":"404","code":"NoSuchKey"}'))
        self.assertTrue(pub.storage_object_missing(404, b''))

    def test_first_release_accepts_only_confirmed_missing_latest_after_bucket_validation(self):
        store = pub.Store("https://project.supabase.co", "releases", "fixture_key")
        for status, body in [(404, b''), (400, b'{"statusCode":"404","code":"NoSuchKey"}'),
                             (400, b'{"statusCode":"404","error":"not_found","message":"Object not found"}')]:
            with patch.object(store, "request", side_effect=[(200, b'{"public":true}'), (status, body)]) as request:
                store.preflight(100, TAG)
                self.assertEqual(request.call_count, 2)
        for status, body in [(400, b'{"statusCode":"403","code":"AccessDenied"}'),
                             (400, b'{"statusCode":"404","code":"NoSuchBucket"}'),
                             (400, b'not JSON'), (401, b'fixture_private_error'), (503, b'fixture_private_error')]:
            with patch.object(store, "request", side_effect=[(200, b'{"public":true}'), (status, body)]):
                with self.assertRaises(pub.PublicationError) as error:
                    store.preflight(100, TAG)
                self.assertIn('HTTP ' + str(status), str(error.exception))
                self.assertNotIn('fixture_private_error', str(error.exception))

    def test_public_readback_normalizes_missing_only_on_supabase_and_bounds_error_body(self):
        calls = []
        body = b'{"statusCode":"404","error":"not_found","message":"Object not found","code":"NoSuchKey"}'

        class Connection:
            def __init__(self, host, **options):
                pass

            def request(self, method, path, **options):
                calls.append(options)

            def getresponse(self):
                class Response:
                    status = 400

                    def read(self, size):
                        calls.append(size)
                        return body[:size]
                return Response()

            def close(self):
                pass

        direct = 'https://project.supabase.co/storage/v1/object/public/releases/file.tar.gz'
        with patch.object(pub.http.client, 'HTTPSConnection', Connection):
            self.assertIsNone(pub.public_digest(direct))
            self.assertEqual(calls[-1], 4097)
            self.assertNotIn('Authorization', calls[0]['headers'])
            for url in [BASE + '/latest.json', 'https://project.supabase.co/other/latest.json']:
                with self.assertRaisesRegex(pub.PublicationError, 'HTTP 400'):
                    pub.public_digest(url)
            for body in [b'{"statusCode":"403","code":"AccessDenied"}', b' ' * 4097]:
                with self.assertRaisesRegex(pub.PublicationError, 'HTTP 400'):
                    pub.public_digest(direct)

    def test_rest_upload_streams_file_and_sets_the_official_upload_headers(self):
        store = pub.Store("https://project.supabase.co", "releases", "fixture_only_secret")
        calls = []

        class Connection:
            def __init__(self, host, **options):
                self.host = host

            def request(self, method, path, body=None, headers=None):
                data = body.read(3) + body.read() if hasattr(body, "read") else body
                calls.append((method, path, data, headers))

            def getresponse(self):
                return type("Response", (), {"status": 200, "read": lambda self, size: b'{}'})()

            def close(self):
                pass

        path = self.directory / "transport.fixture"
        path.write_bytes(b"streamed payload")
        with patch.object(pub.http.client, "HTTPSConnection", Connection):
            store.upload("releases/v0.1.0/test.tar.gz", path, "application/gzip")
            store.upload("latest.json", b"{}", "application/json", overwrite=True)
        self.assertEqual(calls[0][2], path.read_bytes())
        self.assertEqual(calls[0][3]["Content-Length"], str(path.stat().st_size))
        self.assertEqual(calls[0][3]["Cache-Control"], "max-age=31536000")
        self.assertEqual(calls[0][3]["x-upsert"], "false")
        self.assertEqual(calls[1][3]["Cache-Control"], "max-age=0")
        self.assertEqual(calls[1][3]["x-upsert"], "true")

    def test_public_verification_never_follows_redirects_or_sends_credentials(self):
        calls = []

        class Connection:
            def __init__(self, host, **options):
                pass

            def request(self, method, path, **options):
                calls.append(options)

            def getresponse(self):
                return type("Response", (), {"status": 302})()

            def close(self):
                pass

        with patch.object(pub.http.client, "HTTPSConnection", Connection), self.assertRaises(ValueError):
            pub.public_digest(BASE + "/latest.json")
        self.assertNotIn("Authorization", calls[0]["headers"])


if __name__ == "__main__":
    unittest.main()
