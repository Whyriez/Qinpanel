"""Real GnuPG smoke tests against disposable files/homedir, never host services."""
import importlib.util
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("backup_worker", Path(__file__).parents[1] / "server-backup-worker.py")
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
gpg = "C:/Program Files/Git/usr/bin/gpg.exe" if Path("C:/Program Files/Git/usr/bin/gpg.exe").exists() else shutil.which("gpg")


@unittest.skipUnless(gpg, "GnuPG is not available on this test runner")
class CryptoTests(unittest.TestCase):
    def test_real_encryption_roundtrip_wrong_passphrase_and_corruption(self):
        with tempfile.TemporaryDirectory(prefix="qonpanel-crypto-") as directory:
            root = Path(directory)
            engine = worker.Engine(root)
            engine.base.mkdir(parents=True)
            engine.work=root / "w"
            engine.work.mkdir()

            def translated(argument):
                if "Git" in gpg and re.match(r"^[A-Za-z]:[\\/]", argument):
                    return "/" + argument[0].lower() + argument[2:].replace("\\", "/")
                return argument

            def execute(args, data=None, **_kwargs):
                command = [gpg] + [translated(arg) for arg in args[1:]]
                if "Git" in gpg:
                    command = ["C:/Program Files/Git/bin/bash.exe", "--noprofile", "--norc", "-c", 'exec /usr/bin/gpg "$@"', "fixture"] + command[1:]
                result = subprocess.run(command, input=data, capture_output=True, timeout=45)
                if result.returncode:
                    raise RuntimeError("GPG rejected the disposable fixture: " + result.stderr.decode(errors="replace"))
                return result

            engine.execute = execute
            source, encrypted, plain = root / "source", root / "encrypted.qpb", root / "plain"
            source.write_bytes(b"private fixture application and master key" * 4096)
            try:
                engine.crypt(source, encrypted, "fixture passphrase for cryptography")
                self.assertNotIn(b"private fixture", encrypted.read_bytes())
                engine.crypt(encrypted, plain, "fixture passphrase for cryptography", decrypt=True)
                self.assertEqual(source.read_bytes(), plain.read_bytes())
                with self.assertRaises(RuntimeError):
                    engine.crypt(encrypted, root / "wrong", "wrong fixture passphrase", decrypt=True)
                content = bytearray(encrypted.read_bytes()); content[-20] ^= 1; encrypted.write_bytes(content)
                with self.assertRaises(RuntimeError):
                    engine.crypt(encrypted, root / "corrupt", "fixture passphrase for cryptography", decrypt=True)
            finally:
                # Kill only the agent bound to this disposable homedir.
                command = str(Path(gpg).with_name("gpgconf.exe" if gpg.endswith(".exe") else "gpgconf"))
                cleanup = [command, "--homedir", translated(str(engine.work / "gnupg")), "--kill", "gpg-agent"]
                if "Git" in gpg:
                    cleanup = ["C:/Program Files/Git/bin/bash.exe", "--noprofile", "--norc", "-c", 'exec /usr/bin/gpgconf "$@"', "fixture"] + cleanup[1:]
                subprocess.run(cleanup, capture_output=True, timeout=10)


if __name__ == "__main__":
    unittest.main()
