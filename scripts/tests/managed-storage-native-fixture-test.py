"""Hermetic readiness regressions; never calls native main() or host services."""
import importlib.util
from pathlib import Path
import symtable
import sys
import types
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
SOURCE = Path(__file__).with_name("managed-storage-native-test.py")
SPEC = importlib.util.spec_from_file_location("native_storage_fixture", SOURCE)
native = importlib.util.module_from_spec(SPEC)
# pwd is only used inside gated main(); keep fixture unit tests portable to Windows.
if sys.platform == "win32":
    with patch.dict(sys.modules, {"pwd": types.ModuleType("pwd")}):
        SPEC.loader.exec_module(native)
else:
    SPEC.loader.exec_module(native)


class Http01ReadinessTests(unittest.TestCase):
    def setUp(self):
        self.now = 0
        self.expected = b"native HTTP-01 fixture"
        self.connections = []
        self.responses = []
        self.outcomes = []

        def connect(*args, **kwargs):
            connection = Mock()
            response = Mock()
            status, body = self.outcomes.pop(0)
            if isinstance(status, Exception):
                connection.request.side_effect = status
            response.status = status
            response.read.side_effect = lambda limit: body[:limit]
            connection.getresponse.return_value = response
            self.connections.append(connection)
            self.responses.append(response)
            return connection

        def sleep(seconds):
            self.assertGreater(seconds, 0)
            self.now += seconds

        self.constructor = Mock(side_effect=connect)
        for target, name, replacement in [
            (native.http.client, "HTTPConnection", self.constructor),
            (native.time, "monotonic", lambda: self.now),
            (native.time, "sleep", sleep),
        ]:
            context = patch.object(target, name, replacement)
            context.start()
            self.addCleanup(context.stop)

    def assert_closed(self):
        for connection in self.connections:
            connection.close.assert_called_once_with()
            connection.request.assert_called_once_with("GET", "/.well-known/acme-challenge/native-storage", headers={"host": native.HOST})

    def test_old_default_vhost_and_missing_challenge_retry_until_exact_bytes(self):
        self.outcomes = [(200, b"old default nginx page"), (404, b"not found"), (200, self.expected)]
        native.wait_for_http01(self.expected, timeout=2)
        self.assertEqual(self.now, 1)
        self.assertEqual(len(self.connections), 3)
        self.assert_closed()
        for response in self.responses:
            response.read.assert_called_once_with(len(self.expected) + 1)

    def test_connection_and_incomplete_http_are_retried_without_emitting_raw_details(self):
        self.outcomes = [(OSError("PRIVATE CONNECTION"), b""), (native.http.client.RemoteDisconnected("PRIVATE RESPONSE"), b""), (200, self.expected)]
        native.wait_for_http01(self.expected, timeout=2)
        self.assert_closed()

    def test_permanent_forbidden_is_bounded_and_reports_status_without_body(self):
        self.outcomes = [(403, b"PRIVATE RESPONSE BODY")] * 2
        with self.assertRaisesRegex(AssertionError, "HTTP 403") as error:
            native.wait_for_http01(self.expected, timeout=1)
        self.assertNotIn("PRIVATE", str(error.exception))
        self.assertEqual(self.now, 1)
        self.assertEqual(len(self.connections), 2)
        self.assert_closed()

    def test_status_200_with_wrong_body_or_extra_bytes_is_never_ready(self):
        for body in (b"wrong", self.expected + b"x"):
            with self.subTest(body=body):
                self.outcomes = [(200, body)] * 2
                with self.assertRaisesRegex(AssertionError, "HTTP 200.*body_matches=False"):
                    native.wait_for_http01(self.expected, timeout=1)
        self.assertEqual(len(self.connections), 4)
        self.assert_closed()

    def test_correct_bytes_on_wrong_status_are_not_accepted(self):
        self.outcomes = [(404, self.expected)] * 2
        with self.assertRaisesRegex(AssertionError, "HTTP 404.*body_matches=True"):
            native.wait_for_http01(self.expected, timeout=1)
        self.assert_closed()

    def test_imported_http_module_is_not_shadowed_in_native_main_or_probe(self):
        symbols = symtable.symtable(SOURCE.read_text(encoding="utf-8"), str(SOURCE), "exec")
        for scope in symbols.get_children():
            if scope.get_name() in ("main", "wait_for_http01"):
                self.assertTrue(scope.lookup("http").is_global(), scope.get_name())


if __name__ == "__main__":
    unittest.main()
