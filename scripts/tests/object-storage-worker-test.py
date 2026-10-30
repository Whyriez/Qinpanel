"""Hermetic lifecycle fixtures. No engine, network, root or host service is run."""
import copy
import importlib.util
import json
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, Mock, call, patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("managed_storage", ROOT / "scripts/object-storage-worker.py")
worker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(worker)
RAW_COMMAND = worker.command


def keys(seed="a"):
    return {"access_key": seed * 32, "secret_key": seed * 64, "session_token": None}


def operation(action="provision", identifier="7c03851f-60dd-49d1-a3aa-a2100320e7c9", bucket="alice-media", owner=2):
    value = {"action": action, "connection_id": identifier, "bucket": bucket, "owner_id": owner, "credentials": json.dumps(keys())}
    if action == "application_key":
        value.update(generation=1, credentials=json.dumps(keys("b")))
    return value


class FixtureController(worker.Controller):
    def state(self):
        return json.loads(self.state_path.read_text()) if self.state_path.exists() else None

    def identity(self):
        return 900, 900

    def healthy(self, state):
        pass


class HTTPSReadinessTests(unittest.TestCase):
    def setUp(self):
        self.state = {"hostname": "storage.example.test", "root": keys()}
        self.chain = "-----BEGIN CERTIFICATE-----\nfixture\n-----END CERTIFICATE-----"
        self.tcp, self.tls = MagicMock(), MagicMock()
        self.tls.__enter__.return_value = self.tls
        self.tls.getpeercert.return_value = b"installed-leaf"
        self.context = Mock()
        self.context.wrap_socket.return_value = self.tls
        self.now = 0

        def sleep(seconds):
            self.assertGreater(seconds, 0)
            self.now += seconds

        for target, name, replacement in [
            (worker.ssl, "PEM_cert_to_DER_cert", Mock(return_value=b"installed-leaf")),
            (worker.ssl, "create_default_context", Mock(return_value=self.context)),
            (worker.socket, "create_connection", Mock(return_value=self.tcp)),
            (worker, "s3_request", Mock(return_value=200)),
            (worker.time, "monotonic", lambda: self.now),
            (worker.time, "sleep", sleep),
        ]:
            context = patch.object(target, name, replacement)
            context.start()
            self.addCleanup(context.stop)

    def test_listener_delay_old_leaf_and_gateway_warmup_are_retried(self):
        worker.socket.create_connection.side_effect = [ConnectionRefusedError("PRIVATE"), self.tcp, self.tcp, self.tcp]
        self.tls.getpeercert.side_effect = [b"old-leaf", b"installed-leaf", b"installed-leaf"]
        worker.s3_request.side_effect = [502, 200]
        worker.wait_for_https(self.state, self.chain, timeout=3)
        self.assertEqual(worker.socket.create_connection.call_count, 4)
        self.assertEqual(worker.s3_request.call_count, 2)
        self.assertEqual(self.now, 1.5)
        self.context.wrap_socket.assert_called_with(self.tcp.__enter__.return_value, server_hostname=self.state["hostname"])
        worker.s3_request.assert_called_with(self.state, "GET", "/", public=True, timeout=1.5)

    def test_untrusted_or_wrong_hostname_certificate_never_sends_signed_request(self):
        self.context.wrap_socket.side_effect = worker.ssl.SSLCertVerificationError(1, "PRIVATE_CERTIFICATE_DETAIL")
        with self.assertRaisesRegex(worker.StorageError, "trust or hostname verification failed") as failure:
            worker.wait_for_https(self.state, self.chain, timeout=1)
        self.assertNotIn("PRIVATE", str(failure.exception))
        self.assertEqual(self.now, 1)
        worker.s3_request.assert_not_called()

    def test_wrong_installed_leaf_never_sends_signed_request(self):
        self.tls.getpeercert.return_value = b"different-leaf"
        with self.assertRaisesRegex(worker.StorageError, "did not present the installed certificate"):
            worker.wait_for_https(self.state, self.chain, timeout=1)
        worker.s3_request.assert_not_called()

    def test_permanent_connection_failure_is_bounded_and_redacted(self):
        worker.socket.create_connection.side_effect = OSError("PRIVATE_CONNECTION_DETAIL")
        with self.assertRaisesRegex(worker.StorageError, "listener connection failed or timed out") as failure:
            worker.wait_for_https(self.state, self.chain, timeout=1)
        self.assertEqual(self.now, 1)
        self.assertEqual(worker.socket.create_connection.call_count, 2)
        self.assertNotIn("PRIVATE", str(failure.exception))
        worker.s3_request.assert_not_called()

    def test_tls_protocol_and_http_failures_remain_distinguishable(self):
        for error, expected in [
            (worker.ssl.SSLError("PRIVATE"), "TLS handshake failed"),
            (worker.http.client.RemoteDisconnected("PRIVATE"), "invalid or interrupted HTTP response"),
        ]:
            with self.subTest(expected=expected):
                self.now = 0
                self.context.wrap_socket.side_effect = error if isinstance(error, worker.ssl.SSLError) else None
                worker.s3_request.side_effect = error
                with self.assertRaisesRegex(worker.StorageError, expected) as failure:
                    worker.wait_for_https(self.state, self.chain, timeout=1)
                self.assertNotIn("PRIVATE", str(failure.exception))

    def test_permanent_s3_denial_does_not_claim_readiness(self):
        worker.s3_request.return_value = 403
        with self.assertRaisesRegex(worker.StorageError, "HTTPS proxy returned HTTP 403"):
            worker.wait_for_https(self.state, self.chain, timeout=1)
        self.assertEqual(self.now, 1)


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.controller = FixtureController(root / "config", root / "data", root / "unit", root / "nginx")
        self.controller.acme = root / "acme.conf"
        self.controller.config.mkdir()
        self.controller.data.mkdir()
        self.state = {"schema": 1, "hostname": "storage.example.test", "root": keys("r"), "buckets": {}}
        self.writes = []

        def write(path, content, mode=0o600, gid=None):
            self.writes.append((path.name, mode, gid))
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        self.stack = []
        def native_s3(state, method, path, supplied=None, **options):
            if supplied:
                for record in state["buckets"].values():
                    if not record.get("deleted") and supplied in (record["panel"], record.get("application")):
                        return 200
                return 403
            return 200
        for name, replacement in [("atomic_write", write), ("command", Mock(return_value=b"")), ("s3_request", Mock(side_effect=native_s3))]:
            context = patch.object(worker, name, replacement)
            context.start()
            self.addCleanup(context.stop)
        self.controller.write_state(self.state, 900)

    def test_wire_rejects_paths_commands_owner_forgery_and_malformed_credentials(self):
        for change in [{"path": "/tmp/evil"}, {"owner_id": 0}, {"owner_id": True}, {"bucket": "../../etc"}, {"connection_id": "bad"}, {"credentials": "not-json"}]:
            value = operation()
            value.update(change)
            with self.subTest(change=change), self.assertRaises(worker.StorageError):
                worker.validate_request(value)
        self.assertEqual(worker.validate_request(operation()), operation())

    def test_status_rejects_fields_not_present_in_the_typed_wire_contract(self):
        self.assertEqual(worker.validate_request({"action": "status"}), {"action": "status"})
        for field in ["path", "command", "binary_url", "service"]:
            with self.subTest(field=field), self.assertRaises(worker.StorageError):
                worker.validate_request({"action": "status", field: "untyped-fixture-value"})

    def test_domain_injection_and_numeric_hosts_are_rejected(self):
        for host in ["127.0.0.1", "https://example.com", "x;return 200;#", "a\nserver.test", "*.example.com", "EXAMPLE.com"]:
            with self.subTest(host=host), self.assertRaises(worker.StorageError):
                worker.hostname(host)
        self.assertEqual(worker.hostname("storage.example.com"), "storage.example.com")

    def test_provision_replay_preserves_credentials_owner_and_bucket(self):
        request = operation()
        self.assertTrue(self.controller.allocate(request)["ready"])
        original = self.controller.state()
        self.assertTrue(self.controller.allocate(request)["ready"])
        self.assertEqual(original, self.controller.state())
        for change in [{"owner_id": 3}, {"bucket": "bob-media"}, {"credentials": json.dumps(keys("c"))}]:
            with self.subTest(change=change), self.assertRaises(worker.StorageError):
                self.controller.allocate(dict(request, **change))

    def test_buckets_cannot_be_reassigned_through_another_allocation(self):
        self.controller.allocate(operation())
        with self.assertRaises(worker.StorageError):
            self.controller.allocate(operation(identifier="ed603234-a37a-446d-b549-83fc0c00b44f", owner=3))

    def test_authentication_has_no_anonymous_or_global_tenant_identity(self):
        self.controller.allocate(operation())
        self.controller.allocate(operation(action="application_key"))
        document = worker.auth_document(self.controller.state())
        self.assertEqual(len(document["identities"]), 3)
        root, panel, app = document["identities"]
        self.assertIn("Admin", root["actions"])
        for identity in [panel, app]:
            self.assertEqual(identity["actions"], ["Read:alice-media", "List:alice-media", "Write:alice-media"])
            self.assertNotEqual(identity["name"], "anonymous")
        self.assertNotEqual(panel["credentials"], app["credentials"])
        self.assertIn(("state.json", 0o600, None), self.writes)
        self.assertIn(("s3.json", 0o640, 900), self.writes)

    def test_rotation_is_monotonic_and_keeps_panel_keys(self):
        request = operation()
        self.controller.allocate(request)
        first = operation(action="application_key")
        self.controller.allocate(first)
        second = dict(first, generation=2, credentials=json.dumps(keys("c")))
        self.controller.allocate(second)
        record = self.controller.state()["buckets"][request["connection_id"]]
        self.assertEqual(record["panel"], keys())
        self.assertEqual(record["application"], keys("c"))
        with self.assertRaises(worker.StorageError):
            self.controller.allocate(first)

    def test_unknown_key_outcome_retries_health_and_same_key(self):
        self.controller.allocate(operation())
        request = operation(action="application_key")
        with patch.object(self.controller, "healthy", side_effect=worker.StorageError("interrupted")):
            with self.assertRaises(worker.StorageError):
                self.controller.allocate(request)
        worker.command.reset_mock()
        self.assertTrue(self.controller.allocate(request)["ready"])
        worker.command.assert_any_call(["systemctl", "restart", worker.UNIT], 130)
        self.assertEqual(self.controller.state()["buckets"][request["connection_id"]]["application"], keys("b"))

    def test_failed_provision_retains_reservation_without_deleting_data(self):
        sentinel = self.controller.data / "existing-object"
        sentinel.write_text("keep")
        with patch.object(worker, "s3_request", return_value=503):
            with self.assertRaises(worker.StorageError):
                self.controller.allocate(operation())
        self.assertIn(operation()["connection_id"], self.controller.state()["buckets"])
        self.assertEqual(sentinel.read_text(), "keep")
        self.assertTrue(self.controller.allocate(operation())["ready"])

    def test_nonempty_bucket_cannot_be_removed_and_its_keys_remain_active(self):
        self.controller.allocate(operation())
        old = copy.deepcopy(self.controller.state())
        request = {key: value for key, value in operation().items() if key != "credentials"}
        request["action"] = "decommission"
        with patch.object(worker, "s3_request", return_value=409):
            self.assertEqual(self.controller.decommission(request), {"removed": False, "not_empty": True})
        self.assertEqual(self.controller.state(), old)

    def test_empty_removal_revokes_both_identities_and_reconciles_without_recreating_bucket(self):
        self.controller.allocate(operation())
        self.controller.allocate(operation(action="application_key"))
        request = {key: value for key, value in operation().items() if key != "credentials"}
        request["action"] = "decommission"
        self.assertTrue(self.controller.decommission(request)["removed"])
        self.assertEqual(len(worker.auth_document(self.controller.state())["identities"]), 1)
        worker.s3_request.reset_mock()
        self.assertTrue(self.controller.decommission(request)["removed"])
        self.assertTrue(all(call.args[1] == "HEAD" for call in worker.s3_request.call_args_list))
        with self.assertRaises(worker.StorageError):
            self.controller.allocate(operation())

    def test_rotation_does_not_claim_success_until_old_key_is_denied_by_engine(self):
        self.controller.allocate(operation())
        first = operation(action="application_key")
        self.controller.allocate(first)
        second = dict(first, generation=2, credentials=json.dumps(keys("c")))
        with patch.object(worker, "s3_request", return_value=200), self.assertRaises(worker.StorageError):
            self.controller.allocate(second)
        self.assertIn("pending_retired", self.controller.state()["buckets"][first["connection_id"]])
        self.assertTrue(self.controller.allocate(second)["ready"])

    def test_setup_rolls_back_config_and_preserves_existing_data(self):
        (self.controller.data / ".qonpanel-storage-volume").write_text("qonpanel-storage-volume-v1\n")
        sentinel = self.controller.data / "objects"
        sentinel.write_text("original")
        old = self.controller.state_path.read_bytes()
        request = {"action": "configure", "hostname": self.state["hostname"], "fullchain_pem": "fixture-cert", "private_key_pem": "fixture-key"}
        with patch.object(worker, "safe_directory"), patch.object(worker, "install_engine"), patch.object(worker.ssl, "SSLContext"), patch.object(worker, "wait_for_https", side_effect=worker.StorageError("Storage HTTPS readiness timed out. Storage HTTPS listener connection failed or timed out.")):
            with self.assertRaisesRegex(worker.StorageError, "HTTPS activation.*HTTPS readiness timed out"):
                self.controller.configure(request)
        self.assertEqual(self.controller.state_path.read_bytes(), old)
        self.assertFalse(self.controller.nginx.exists())
        self.assertEqual(sentinel.read_text(), "original")

    def test_tunnel_wire_needs_only_hostname_and_rejects_untyped_targets(self):
        for action in ("configure_tunnel", "prepare_http"):
            value = {"action": action, "hostname": self.state["hostname"]}
            self.assertEqual(worker.validate_request(value), value)
            for extra in ("port", "path", "url", "fullchain_pem", "service", "command"):
                with self.subTest(action=action, extra=extra), self.assertRaises(worker.StorageError):
                    worker.validate_request(dict(value, **{extra: "untrusted"}))

    def access_request(self, public=True, expected=1, revision=2):
        return {"action":"set_bucket_access", "connection_id":operation()["connection_id"], "owner_id":2, "bucket":"alice-media", "expected_revision":expected, "revision":revision, "public":public}

    def prepare_access(self):
        self.controller.allocate(operation())
        self.controller.nginx.write_bytes(worker.nginx_template(self.state["hostname"], "a"*64).encode())

    def test_public_access_wire_is_typed_and_never_accepts_global_paths_or_other_owner(self):
        value = self.access_request()
        self.assertEqual(worker.validate_request(value), value)
        for change in ({"path":"/etc"}, {"public":"true"}, {"revision":1}, {"expected_revision":0}, {"command":"restart"}):
            with self.subTest(change=change), self.assertRaises(worker.StorageError):
                worker.validate_request(dict(value, **change))
        self.prepare_access()
        with self.assertRaises(worker.StorageError):
            self.controller.set_bucket_access(dict(value, owner_id=3))

    def test_anonymous_read_is_bucket_scoped_without_list_write_or_private_bucket_access(self):
        self.prepare_access()
        self.controller.allocate(operation(identifier="ed603234-a37a-446d-b549-83fc0c00b44f",bucket="bob-media",owner=3))
        before = self.controller.state()
        with patch.object(worker, "anonymous_request", side_effect=[404,403]):
            self.assertTrue(self.controller.set_bucket_access(self.access_request())["public"])
        after = self.controller.state()
        anonymous = next(item for item in worker.auth_document(after)["identities"] if item["name"]=="anonymous")
        self.assertEqual(anonymous, {"name":"anonymous","actions":["Read:alice-media"]})
        self.assertEqual(after["root"], before["root"])
        for key, record in before["buckets"].items():
            self.assertEqual(after["buckets"][key]["panel"], record["panel"])
            self.assertEqual(after["buckets"][key]["owner_id"], record["owner_id"])

    def test_private_revokes_anonymous_and_same_revision_retry_preserves_keys(self):
        self.prepare_access()
        with patch.object(worker, "anonymous_request", side_effect=[404,403,404,403,403,403]):
            first = self.controller.set_bucket_access(self.access_request())
            keys_before = self.controller.state()["buckets"][operation()["connection_id"]]["panel"]
            self.assertEqual(self.controller.set_bucket_access(self.access_request()),first)
            self.assertFalse(self.controller.set_bucket_access(self.access_request(False,2,3))["public"])
        self.assertFalse(any(item["name"]=="anonymous" for item in worker.auth_document(self.controller.state())["identities"]))
        self.assertEqual(self.controller.state()["buckets"][operation()["connection_id"]]["panel"],keys_before)
        with self.assertRaises(worker.StorageError):
            self.controller.set_bucket_access(self.access_request())

    def test_restart_unknown_outcome_retains_desired_revision_for_reconciliation(self):
        self.prepare_access()
        with patch.object(worker,"command",side_effect=worker.StorageError("restart failed")),self.assertRaises(worker.StorageError):
            self.controller.set_bucket_access(self.access_request())
        self.assertEqual(self.controller.state()["buckets"][operation()["connection_id"]]["access_revision"],2)
        with patch.object(worker,"anonymous_request",side_effect=[404,403]):
            self.assertTrue(self.controller.set_bucket_access(self.access_request())["ready"])

    def test_publication_refuses_anonymous_list_and_failed_revoke_never_reports_success(self):
        self.prepare_access()
        for request, outcomes in [(self.access_request(),[404,200]),(self.access_request(False,2,3),[404])]:
            with patch.object(worker,"anonymous_request",side_effect=outcomes), self.assertRaises(worker.StorageError):
                self.controller.set_bucket_access(request)

    def test_existing_proxy_cache_policy_upgrades_once_without_changing_host_uri_or_tls(self):
        self.prepare_access()
        old = self.controller.nginx.read_bytes().replace(worker.CACHE_RULE.encode(),b"")
        self.controller.nginx.write_bytes(old)
        with patch.object(worker,"anonymous_request",side_effect=[404,403,404,403]):
            self.controller.set_bucket_access(self.access_request())
            upgraded = self.controller.nginx.read_bytes()
            self.controller.set_bucket_access(self.access_request())
        self.assertEqual(self.controller.nginx.read_bytes(),upgraded)
        self.assertEqual(upgraded.replace(worker.CACHE_RULE.encode(),b""),old)
        self.assertIn(b'add_header Cache-Control "no-store" always;',upgraded)

    def test_retained_https_origin_is_upgraded_even_when_tunnel_already_has_no_store(self):
        self.prepare_access()
        tunnel = worker.tunnel_template(self.state["hostname"])
        legacy = worker.nginx_template(self.state["hostname"], "a"*64).replace(worker.CACHE_RULE, "")
        self.controller.nginx.write_text(tunnel + "# Retained HTTPS origin\n" + legacy)
        with patch.object(worker, "anonymous_request", side_effect=[404,403,404,403]):
            self.controller.set_bucket_access(self.access_request())
            first = self.controller.nginx.read_bytes()
            self.controller.set_bucket_access(self.access_request())
        self.assertEqual(first, self.controller.nginx.read_bytes())
        self.assertEqual(first.count(worker.CACHE_RULE.encode()), 2)
        self.assertIn(b"listen 443 ssl", first)
        self.assertIn(b"listen 127.0.0.1:17891", first)

    def test_modified_proxy_is_preserved_instead_of_silently_publishing_without_cache_policy(self):
        self.prepare_access()
        proxy = self.controller.nginx.read_bytes().replace(worker.CACHE_RULE.encode(), b"").replace(b"        proxy_set_header Host", b"    proxy_set_header Host")
        self.controller.nginx.write_bytes(proxy)
        before = self.controller.state_path.read_bytes()
        with self.assertRaises(worker.StorageError):
            self.controller.set_bucket_access(self.access_request())
        self.assertEqual(before, self.controller.state_path.read_bytes())
        self.assertEqual(proxy, self.controller.nginx.read_bytes())

    def test_cache_policy_failure_rolls_back_before_publishing_bucket(self):
        self.prepare_access()
        old = self.controller.nginx.read_bytes().replace(worker.CACHE_RULE.encode(),b"")
        self.controller.nginx.write_bytes(old)
        before = self.controller.state_path.read_bytes()
        failed = False
        def command(args,timeout=60):
            nonlocal failed
            if args==["nginx","-t"] and not failed:
                failed=True
                raise worker.StorageError("bad config")
            return b""
        with patch.object(worker,"command",side_effect=command),self.assertRaises(worker.StorageError):
            self.controller.set_bucket_access(self.access_request())
        self.assertEqual(self.controller.nginx.read_bytes(),old)
        self.assertEqual(self.controller.state_path.read_bytes(),before)

    def test_tunnel_listener_is_loopback_only_and_preserves_s3_request(self):
        config = worker.tunnel_template(self.state["hostname"])
        self.assertIn("listen 127.0.0.1:17891 default_server", config)
        self.assertNotIn("listen 17891", config)
        self.assertIn("return 444;", config)
        self.assertIn("proxy_set_header Host $http_host;", config)
        self.assertIn("proxy_set_header Authorization $http_authorization;", config)
        self.assertIn(f"proxy_pass http://unix:{worker.SOCKET};", config)
        self.assertNotIn("rewrite", config)
        self.assertNotIn("ssl_certificate", config)

    def test_tunnel_setup_preserves_objects_keys_and_previous_https_during_publication(self):
        self.controller.allocate(operation())
        before = self.controller.state()
        previous_config = worker.nginx_template(before["hostname"], "a" * 64)
        self.controller.nginx.write_text(previous_config)
        sentinel = self.controller.data / "objects"
        sentinel.write_bytes(b"existing object")
        request = {"action": "configure_tunnel", "hostname": before["hostname"]}
        with patch.object(worker, "safe_directory"), patch.object(worker, "install_engine"), patch.object(worker, "wait_for_tunnel") as ready, patch.object(worker.ssl, "SSLContext") as ssl_context:
            result = self.controller.configure(request)
            self.assertEqual(result["connection_mode"], "tunnel")
            self.assertTrue(result["origin_ready"])
            self.assertFalse(result["public_access_verified"])
            self.assertEqual(result["endpoint"], "https://storage.example.test")
            self.assertEqual(result["origin_url"], "http://127.0.0.1:17891")
            ssl_context.assert_not_called()
            ready.assert_called_once()
            first = self.controller.nginx.read_text()
            self.controller.configure(request)
            self.assertEqual(self.controller.nginx.read_text(), first)
        self.assertIn(previous_config, first)
        after = self.controller.state()
        self.assertEqual(after["root"], before["root"])
        self.assertEqual(after["buckets"], before["buckets"])
        self.assertEqual(sentinel.read_bytes(), b"existing object")

    def test_fresh_tunnel_activation_never_requires_or_writes_certificates(self):
        self.controller.state_path.unlink()
        (self.controller.config / "s3.json").unlink()
        with patch.object(worker, "safe_directory"), patch.object(worker, "install_engine"), patch.object(worker, "wait_for_tunnel"), patch.object(worker.ssl, "SSLContext") as tls:
            result = self.controller.configure({"action": "configure_tunnel", "hostname": self.state["hostname"]})
            tls.assert_not_called()
        self.assertTrue(result["configured"] and result["origin_ready"])
        self.assertEqual(result["connection_mode"], "tunnel")
        self.assertFalse((self.controller.config / "certificates").exists())
        self.assertNotIn("listen 443", self.controller.nginx.read_text())

    def test_tunnel_origin_failure_restores_prior_https_state_and_configuration(self):
        old = self.controller.state_path.read_bytes()
        self.controller.nginx.write_text(worker.nginx_template(self.state["hostname"], "b" * 64))
        old_nginx = self.controller.nginx.read_bytes()
        with patch.object(worker, "safe_directory"), patch.object(worker, "install_engine"), patch.object(worker, "wait_for_tunnel", side_effect=worker.StorageError("loopback not ready")):
            with self.assertRaisesRegex(worker.StorageError, "Tunnel origin activation.*loopback not ready"):
                self.controller.configure({"action": "configure_tunnel", "hostname": self.state["hostname"]})
        self.assertEqual(self.controller.state_path.read_bytes(), old)
        self.assertEqual(self.controller.nginx.read_bytes(), old_nginx)

    def test_tunnel_port_collision_preserves_prior_state(self):
        old = self.controller.state_path.read_bytes()
        probe = MagicMock()
        probe.__enter__.return_value.bind.side_effect = OSError("occupied")
        with patch.object(worker, "safe_directory"), patch.object(worker.socket, "socket", return_value=probe), patch.object(worker, "install_engine") as install:
            with self.assertRaisesRegex(worker.StorageError, "already in use"):
                self.controller.configure({"action": "configure_tunnel", "hostname": self.state["hostname"]})
            install.assert_not_called()
        self.assertEqual(self.controller.state_path.read_bytes(), old)

    def test_status_does_not_equate_engine_health_with_origin_or_public_health(self):
        with patch.object(worker, "s3_request", side_effect=[200, OSError("listener down")]):
            result = self.controller.status()
        self.assertTrue(result["running"])
        self.assertFalse(result["origin_ready"])
        self.assertFalse(result["public_access_verified"])
        self.assertEqual(result["connection_mode"], "https")

    def test_http01_preparation_is_separate_from_tls_and_never_creates_engine_keys(self):
        before = self.controller.state_path.read_bytes()
        self.assertTrue(self.controller.prepare_http({"hostname": self.state["hostname"]})["prepared"])
        self.assertEqual(self.controller.state_path.read_bytes(), before)
        config = self.controller.acme.read_text()
        self.assertIn("listen 80;", config)
        self.assertIn("root /var/lib/panel/acme;", config)
        self.assertIn("location ^~ /.well-known/acme-challenge/", config)
        self.assertIn("location / { return 404; }", config)
        self.assertFalse(self.controller.nginx.exists())

    def test_http01_conflicting_hostname_is_not_taken_over(self):
        with patch.object(worker, "command", return_value=b"server_name storage.example.test;"):
            with self.assertRaisesRegex(worker.StorageError, "already used"):
                self.controller.prepare_http({"hostname": self.state["hostname"]})
        self.assertFalse(self.controller.acme.exists())

    def test_http01_reload_failure_restores_old_challenge_configuration(self):
        old = worker.acme_template(self.state["hostname"])
        self.controller.acme.write_bytes(old.encode())
        failed = False
        def command(args, timeout=60):
            nonlocal failed
            if args == ["systemctl", "reload", "nginx"] and not failed:
                failed = True
                raise worker.StorageError("reload failed")
            return b""
        with patch.object(worker, "command", side_effect=command), self.assertRaisesRegex(worker.StorageError, "previous configuration restored"):
            self.controller.prepare_http({"hostname": self.state["hostname"]})
        self.assertEqual(self.controller.acme.read_text(), old)

    def test_inactive_nginx_is_rejected_before_config_or_data_changes(self):
        before = self.controller.state_path.read_bytes()
        request = {"hostname": self.state["hostname"], "fullchain_pem": "fixture-chain", "private_key_pem": "fixture-key"}
        with patch.object(worker, "command", side_effect=worker.StorageError("inactive")), patch.object(worker, "install_engine") as install:
            with self.assertRaisesRegex(worker.StorageError, "Nginx must be running"):
                self.controller.configure(request)
            install.assert_not_called()
        self.assertEqual(self.controller.state_path.read_bytes(), before)
        self.assertFalse(self.controller.unit.exists())
        self.assertFalse(self.controller.nginx.exists())
        self.assertEqual(list(self.controller.data.iterdir()), [])

    def test_setup_waits_for_https_listener_after_nginx_reload(self):
        chain = "-----BEGIN CERTIFICATE-----\nfixture\n-----END CERTIFICATE-----"
        request = {"hostname": self.state["hostname"], "fullchain_pem": chain, "private_key_pem": "fixture-key"}
        tcp, tls = MagicMock(), MagicMock()
        tls.__enter__.return_value = tls
        tls.getpeercert.return_value = b"installed-leaf"
        context = Mock()
        context.wrap_socket.return_value = tls
        before = self.controller.state()
        with patch.object(worker, "safe_directory"), patch.object(worker, "install_engine"), patch.object(worker.ssl, "SSLContext"), patch.object(worker.ssl, "create_default_context", return_value=context), patch.object(worker.ssl, "PEM_cert_to_DER_cert", return_value=b"installed-leaf"), patch.object(worker.socket, "create_connection", side_effect=[ConnectionRefusedError("PRIVATE"), tcp]) as connect, patch.object(worker.time, "sleep"):
            self.assertTrue(self.controller.configure(request)["running"])
        self.assertEqual(connect.call_count, 2)
        context.wrap_socket.assert_called_with(tcp.__enter__.return_value, server_hostname=self.state["hostname"])
        self.assertEqual(self.controller.state(), before)
        worker.command.assert_any_call(["systemctl", "reload", "nginx"])

    def test_rollback_failure_keeps_initial_stage_status_and_preserves_live_recovery_keys(self):
        for stop_fails in [False, True]:
            with self.subTest(stop_fails=stop_fails):
                self.controller.state_path.unlink(missing_ok=True)
                (self.controller.config / "s3.json").unlink(missing_ok=True)
                self.controller.unit.unlink(missing_ok=True)
                self.controller.nginx.unlink(missing_ok=True)
                (self.controller.data / ".qonpanel-storage-volume").write_text("qonpanel-storage-volume-v1\n")
                sentinel = self.controller.data / "objects"
                sentinel.write_text("retain these objects")
                commands = []
                def execute(args, timeout=60):
                    commands.append(args)
                    if args[:2] == ["systemctl", "show"]:
                        return b"Result=exit-code\nExecMainStatus=23\nSecretKey=DO_NOT_PRINT\n"
                    if args == ["systemctl", "enable", "--now", worker.UNIT]:
                        raise worker.StorageError("fixture engine startup failure")
                    if args == ["systemctl", "reload", "nginx"] or (stop_fails and args == ["systemctl", "disable", "--now", worker.UNIT]):
                        raise RuntimeError("DO_NOT_PRINT")
                    return b""
                request = {"hostname": "storage.example.test", "fullchain_pem": "fixture-chain", "private_key_pem": "fixture-key"}
                with patch.object(worker, "safe_directory"), patch.object(worker, "install_engine"), patch.object(worker.ssl, "SSLContext"), patch.object(worker, "command", side_effect=execute):
                    with self.assertRaises(worker.StorageError) as failure:
                        self.controller.configure(request)
                message = str(failure.exception)
                for expected in ["storage service startup", "fixture engine startup failure", "ExecMainStatus=23", "Rollback failed for:", "reload Nginx", "object data was retained"]:
                    self.assertIn(expected, message)
                self.assertNotIn("DO_NOT_PRINT", message)
                self.assertEqual(sentinel.read_text(), "retain these objects")
                self.assertEqual(self.controller.unit.exists(), stop_fails)
                self.assertEqual(self.controller.state_path.exists(), stop_fails)
                self.assertEqual((self.controller.config / "s3.json").exists(), stop_fails)
                self.assertFalse(self.controller.nginx.exists())
                self.assertIn(["nginx", "-t"], commands)
                if stop_fails:
                    self.assertNotIn(self.controller.state()["root"]["secret_key"], message)

    def test_diagnostics_accept_only_machine_state_not_commands_credentials_or_paths(self):
        self.assertEqual(worker.error_response(worker.StorageSetupError("safe", "PRIVATE_STAGE")), {"error": "safe"})
        self.assertEqual(worker.error_response(worker.StorageError("safe")), {"error": "safe"})
        raw = b"LoadState=loaded\nActiveState=failed\nResult=exit-code\nExecMainStatus=1\nExecStart=SECRET_COMMAND\nEnvironment=SECRET_KEY\nSubState=PRIVATE VALUE\n"
        with patch.object(worker, "command", return_value=raw):
            message = worker.service_status_summary(worker.UNIT)
        self.assertIn("Result=exit-code", message)
        for secret in ["SECRET", "PRIVATE", "ExecStart", "Environment"]:
            self.assertNotIn(secret, message)
        with patch.object(worker, "command", side_effect=RuntimeError("PRIVATE")):
            self.assertEqual(worker.service_status_summary(worker.UNIT), "unavailable")

    def test_engine_install_failure_reports_stage_and_preserves_existing_state(self):
        before = {path: path.read_bytes() for path in (self.controller.state_path, self.controller.config / "s3.json")}
        request = {"hostname": self.state["hostname"], "fullchain_pem": "fixture-chain", "private_key_pem": "fixture-key"}
        with patch.object(worker, "safe_directory"), patch.object(worker.ssl, "SSLContext"), patch.object(worker, "install_engine", side_effect=worker.StorageError("Engine download exceeded its size or time limit.")):
            with self.assertRaises(worker.StorageSetupError) as failure:
                self.controller.configure(request)
        response = worker.error_response(failure.exception)
        self.assertEqual(response["stage"], "verified engine installation")
        self.assertIn("Configuration was restored", response["error"])
        self.assertNotIn("fixture-key", json.dumps(response))
        self.assertNotIn(self.state["root"]["secret_key"], json.dumps(response))
        for path, content in before.items():
            self.assertEqual(path.read_bytes(), content)
        self.assertFalse(self.controller.unit.exists())
        self.assertNotIn(unittest.mock.call(["systemctl", "enable", "--now", worker.UNIT], 130), worker.command.call_args_list)

    def test_command_failure_and_timeout_never_return_raw_output_or_argv(self):
        args = ["nginx", "-t", "PRIVATE_ARG"]
        with patch.object(worker.subprocess, "run", return_value=subprocess.CompletedProcess(args, 1, b"PRIVATE_STDOUT", b"PRIVATE_STDERR")):
            with self.assertRaisesRegex(worker.StorageError, r"nginx -t, exit 1") as failure:
                RAW_COMMAND(args)
        self.assertNotIn("PRIVATE", str(failure.exception))
        with patch.object(worker.subprocess, "run", side_effect=subprocess.TimeoutExpired(args, 10, output=b"PRIVATE")):
            with self.assertRaisesRegex(worker.StorageError, "timed out") as failure:
                RAW_COMMAND(args)
        self.assertNotIn("PRIVATE", str(failure.exception))

    def test_native_fixture_explicitly_starts_and_checks_nginx_without_real_commands(self):
        spec = importlib.util.spec_from_file_location("native_fixture", ROOT / "scripts/tests/managed-storage-native-test.py")
        native = importlib.util.module_from_spec(spec)
        # pwd is Linux-only. Importing the fixture here never enters its guarded main.
        with patch.dict(sys.modules, {"pwd": Mock()}):
            spec.loader.exec_module(native)
        with patch.object(native, "run") as execute:
            native.prepare_nginx()
        self.assertEqual(execute.call_args_list, [
            call(["nginx", "-t"], timeout=20),
            call(["systemctl", "start", "nginx"], timeout=60),
            call(["systemctl", "is-active", "--quiet", "nginx"], timeout=10),
        ])

    def test_existing_endpoint_cannot_move_or_take_over_custom_nginx(self):
        with self.assertRaises(worker.StorageError):
            self.controller.configure({"hostname": "elsewhere.example.test"})
        self.controller.nginx.write_text("# custom nginx config")
        with patch.object(worker, "safe_directory"), self.assertRaises(worker.StorageError):
            self.controller.configure({"hostname": self.state["hostname"]})
        self.assertEqual(self.controller.nginx.read_text(), "# custom nginx config")

    def test_first_setup_stops_new_unit_before_removing_its_file_on_rollback(self):
        self.controller.state_path.unlink()
        (self.controller.config / "s3.json").unlink()
        stopped = []
        def execute(args, timeout=60):
            if args == ["systemctl", "disable", "--now", worker.UNIT]:
                stopped.append(self.controller.unit.exists())
            return b""
        request = {"action": "configure", "hostname": "storage.example.test", "fullchain_pem": "fixture-chain", "private_key_pem": "fixture-key"}
        with patch.object(worker, "safe_directory"), patch.object(worker, "install_engine"), patch.object(worker.ssl, "SSLContext"), patch.object(worker, "command", side_effect=execute), patch.object(worker, "wait_for_https", side_effect=worker.StorageError("fixture HTTPS unavailable")):
            with self.assertRaises(worker.StorageError):
                self.controller.configure(request)
        self.assertEqual(stopped, [True])
        self.assertFalse(self.controller.unit.exists())
        self.assertIsNone(self.controller.state())

    def test_network_namespace_only_exposes_authenticated_s3_socket(self):
        unit = worker.service_template("https://storage.example.test")
        for requirement in ["PrivateNetwork=true", "User=qonpanel-storage", "-s3.iam=false", "-s3.autoCreateBucket=false", "-webdav=false", "-admin.ui=false", "WantedBy=multi-user.target", "-s3.config=/etc/qonpanel-storage/s3.json"]:
            self.assertIn(requirement, unit)
        nginx = worker.nginx_template("storage.example.test", "a" * 64)
        self.assertIn("proxy_pass http://unix:/run/qonpanel-storage/s3.sock;", nginx)
        self.assertIn("proxy_set_header Host $http_host;", nginx)
        self.assertNotIn("rewrite", nginx.replace("# Signature paths and Host must reach S3 unchanged. No URI suffix/rewrite.", ""))

    def test_release_download_digests_are_pinned_for_both_architectures(self):
        self.assertEqual(set(worker.ASSETS), {"x86_64", "aarch64"})
        for _, digest in worker.ASSETS.values():
            self.assertRegex(digest, r"^[0-9a-f]{64}$")
        with self.assertRaises(worker.StorageError):
            worker.ReleaseRedirect().redirect_request(None, None, 302, "", {}, "http://169.254.169.254/latest")


if __name__ == "__main__":
    unittest.main()
