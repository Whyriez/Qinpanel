"""Hermetic auth protocol tests. Native libsodium verification is CI opt-in."""
import ctypes
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("mail_auth", ROOT / "scripts/mail-auth.py")
auth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth)
HASH = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$" + "A" * 43
HOME = "/var/lib/panel/mail/10001/12345678-1234-4234-8234-123456789abc"
ROW = "box@example.test:{ARGON2ID}" + HASH + ":10001:10001::" + HOME + "::userdb_mail=maildir:" + HOME + "/Maildir\n"


class Verifier:
    def __init__(self):
        self.calls = []

    def verify(self, password, encoded):
        self.calls.append((password, encoded))
        return password == b"fixture password only"


class AuthTests(unittest.TestCase):
    def test_existing_argon_hash_is_verified_without_conversion_or_uid_switch(self):
        verifier = Verifier()
        environment = auth.authenticate(b"box@example.test\0fixture password only\0", ROW, verifier)
        self.assertEqual(verifier.calls, [(b"fixture password only", HASH)])
        self.assertEqual(environment["userdb_uid"], "10001")
        self.assertEqual(environment["userdb_gid"], "10001")
        self.assertEqual(environment["HOME"], HOME)
        self.assertEqual(environment["EXTRA"], "userdb_uid userdb_gid userdb_home")
        self.assertNotIn("LD_PRELOAD", environment)
        self.assertNotIn("AUTHORIZED", environment)
        self.assertNotIn("INSECURE_SETUID", environment)
        self.assertNotIn("fixture password", str(environment))

    def test_unknown_wrong_disabled_and_duplicate_accounts_cannot_authenticate(self):
        self.assertIsNone(auth.authenticate(b"unknown@example.test\0fixture password only\0", ROW, Verifier()))
        self.assertIsNone(auth.authenticate(b"box@example.test\0wrong\0", ROW, Verifier()))
        self.assertIsNone(auth.authenticate(b"box@example.test\0fixture password only\0", "", Verifier()))
        with self.assertRaises(ValueError):
            auth.authenticate(b"box@example.test\0fixture password only\0", ROW + ROW, Verifier())

    def test_identity_home_scheme_and_memory_injection_fail_before_verification(self):
        for row in (ROW.replace(":10001:10001:", ":0:0:"), ROW.replace(":10001:10001:", ":10001:33:"),
                    ROW.replace(HOME, "/root"), ROW.replace("{ARGON2ID}", "{PLAIN}"),
                    ROW.replace("m=19456", "m=999999"), ROW.replace("t=2", "t=99"),
                    ROW.replace("$argon2id$", "$argon2i$")):
            verifier = Verifier()
            with self.assertRaises(ValueError):
                auth.authenticate(b"box@example.test\0fixture password only\0", row, verifier)
            self.assertFalse(verifier.calls)

    def test_fd_protocol_is_bounded_and_cannot_supply_a_reply_command(self):
        for data in (b"box@example.test\0password", b"box@example.test\0\0", b"box@example.test\0" + b"x" * 2049 + b"\0",
                     b"bad:name@example.test\0password\0", b"box@example.test\0password\0timestamp\0extra\0"):
            with self.assertRaises(ValueError):
                auth.credentials(data)
        result = subprocess.run([sys.executable, str(ROOT / "scripts/mail-auth.py"), "/tmp/arbitrary-reply"], capture_output=True, check=False)
        self.assertEqual(result.returncode, 111)
        self.assertEqual(result.stdout + result.stderr, b"")

    @unittest.skipUnless(os.environ.get("QONPANEL_TEST_SODIUM") == "1", "native libsodium runs in disposable Linux CI only")
    def test_native_sodium_verifies_argon2id_and_rejects_wrong_password(self):
        sodium = auth.Sodium()
        generate = sodium.library.crypto_pwhash_str_alg
        generate.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_ulonglong, ctypes.c_ulonglong, ctypes.c_size_t, ctypes.c_int]
        generate.restype = ctypes.c_int
        output = ctypes.create_string_buffer(128)
        password = b"fixture password only"
        self.assertEqual(generate(output, password, len(password), 2, 19456 * 1024, 2), 0)
        encoded = output.value.decode("ascii")
        self.assertTrue(sodium.verify(password, encoded))
        self.assertFalse(sodium.verify(b"incorrect fixture", encoded))
        row = ROW.replace(HASH, encoded)
        self.assertEqual(auth.authenticate(b"box@example.test\0fixture password only\0", row, sodium)["userdb_uid"], "10001")

    @unittest.skipUnless(os.environ.get("QONPANEL_GENERATED_PASSDB"), "Rust-generated passdb runs in disposable package CI")
    def test_native_sodium_accepts_existing_rust_argon2id_mailbox_hash(self):
        with patch.object(auth, "PASSDB", os.environ["QONPANEL_GENERATED_PASSDB"]):
            database = auth.read_database()
        sodium = auth.Sodium()
        result = auth.authenticate(b"admin@example.com\0SafeFixture123!\0", database, sodium)
        self.assertEqual(result["userdb_uid"], "10001")
        self.assertIsNone(auth.authenticate(b"admin@example.com\0wrong fixture password\0", database, sodium))

        def exchange(password, original_uid):
            incoming, writer = os.pipe()
            reader, outgoing = os.pipe()
            child = os.fork()
            if child == 0:
                # Duplicate away from fd 3/4 before closing the original pipe set.
                source = os.dup(incoming)
                sink = os.dup(outgoing)
                for descriptor in (incoming, writer, reader, outgoing):
                    os.close(descriptor)
                os.dup2(source, 3, inheritable=True)
                os.dup2(sink, 4, inheritable=True)
                os.close(source)
                os.close(sink)
                auth.PASSDB = os.environ["QONPANEL_GENERATED_PASSDB"]
                os.environ["ORIG_UID"] = original_uid
                sys.argv = ["mail-auth", auth.REPLY]
                try:
                    result = auth.main()
                except Exception:
                    result = 111
                os._exit(result)
            os.close(incoming)
            os.close(outgoing)
            try:
                os.write(writer, b"admin@example.com\0" + password + b"\0")
            except BrokenPipeError:
                pass
            os.close(writer)
            with os.fdopen(reader, "rb") as response:
                wire = response.read(4096)
            _, status = os.waitpid(child, 0)
            return os.waitstatus_to_exitcode(status), wire

        code, wire = exchange(b"SafeFixture123!", str(os.getuid()))
        self.assertEqual(code, 0)
        self.assertIn(b"userdb_uid=10001", wire)
        self.assertIn(b"user=admin@example.com", wire)
        self.assertNotIn(b"SafeFixture123!", wire)
        self.assertEqual(exchange(b"invalid fixture", str(os.getuid())), (1, b""))
        self.assertEqual(exchange(b"SafeFixture123!", "99999"), (111, b""))


if __name__ == "__main__":
    unittest.main()
