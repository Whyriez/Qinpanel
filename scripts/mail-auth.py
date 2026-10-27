#!/usr/bin/python3 -I
"""Dovecot 2.3 checkpassword bridge for AlmaLinux's Argon2id-less build.

Fixed passdb, reply executable and library names. Credentials enter fd 3 only;
no shell, network, password logging, setuid/setgid, or hash conversion.
"""
import ctypes
import os
from pathlib import Path
import re
import stat
import sys

PASSDB = "/etc/dovecot/qpanel-users"
REPLY = "/usr/libexec/dovecot/checkpassword-reply"
LIBRARIES = ("/usr/lib64/libsodium.so.23", "/usr/lib64/libsodium.so.26")
MAX_DATABASE = 16 * 1024 * 1024
ADDRESS = re.compile(r"[a-z0-9_+-][a-z0-9._+-]{0,63}@[a-z0-9.-]{1,253}\Z")
PHC = re.compile(r"\$argon2id\$v=19\$m=([0-9]{1,6}),t=([0-9]{1,2}),p=([0-9])\$[A-Za-z0-9+/]{16,64}\$[A-Za-z0-9+/]{22,86}\Z")
HOME = re.compile(r"/var/lib/panel/mail/([1-5][0-9]{4})/[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}\Z")


class Sodium:
    def __init__(self):
        filename = next((name for name in LIBRARIES if Path(name).is_file()), None)
        if filename is None:
            raise RuntimeError("Required password verifier is unavailable")
        self.library = ctypes.CDLL(filename)
        self.library.sodium_init.argtypes = []
        self.library.sodium_init.restype = ctypes.c_int
        if self.library.sodium_init() < 0:
            raise RuntimeError("Password verifier initialization failed")
        self.library.crypto_pwhash_str_verify.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_ulonglong]
        self.library.crypto_pwhash_str_verify.restype = ctypes.c_int

    def verify(self, password, encoded):
        validate_hash(encoded)
        secret = ctypes.create_string_buffer(password)
        try:
            return self.library.crypto_pwhash_str_verify(encoded.encode("ascii"), secret, len(password)) == 0
        finally:
            ctypes.memset(secret, 0, len(secret))


def validate_hash(encoded):
    match = PHC.fullmatch(encoded)
    if not match or not (8 <= int(match[1]) <= 262144 and 1 <= int(match[2]) <= 10 and 1 <= int(match[3]) <= 4):
        raise ValueError("Invalid or unbounded password hash")


def credentials(payload):
    if len(payload) > 2048:
        raise ValueError("Oversized authentication input")
    parts = payload.split(b"\0")
    if len(parts) not in (3, 4) or parts[-1] != b"" or not 1 <= len(parts[1]) <= 1024:
        raise ValueError("Invalid authentication input")
    name = parts[0].decode("ascii")
    if len(name) > 254 or not ADDRESS.fullmatch(name) or ".." in name.split("@", 1)[0] or name.split("@", 1)[0].endswith("."):
        raise ValueError("Invalid mailbox name")
    return name, parts[1]


def authenticate(payload, database, verifier):
    name, password = credentials(payload)
    if len(database.encode("utf-8")) > MAX_DATABASE:
        raise ValueError("Oversized authentication database")
    found = None
    for line in database.splitlines():
        fields = line.split(":", 7)
        if fields[0] != name:
            continue
        if found is not None or len(fields) != 8 or not fields[1].startswith("{ARGON2ID}"):
            raise ValueError("Invalid mailbox record")
        encoded = fields[1][10:]
        validate_hash(encoded)
        if not fields[2].isdecimal() or not 10000 <= int(fields[2]) <= 59999 or fields[3] != fields[2] or fields[4] or fields[6]:
            raise ValueError("Invalid mailbox identity")
        home = HOME.fullmatch(fields[5])
        if not home or home[1] != fields[2]:
            raise ValueError("Invalid mailbox home")
        found = (encoded, fields[2], fields[3], fields[5])
    if found is None or not verifier.verify(password, found[0]):
        return None
    # No uid/gid transition: untrusted tenant processes cannot ptrace this helper
    # and forge the reply. Dovecot receives only validated userdb extra fields.
    return {"USER": name, "HOME": found[3], "userdb_uid": found[1], "userdb_gid": found[2],
            "userdb_home": found[3], "EXTRA": "userdb_uid userdb_gid userdb_home",
            "PATH": "/usr/sbin:/usr/bin:/sbin:/bin"}


def read_database():
    descriptor = os.open(PASSDB, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(descriptor, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o027 or metadata.st_size > MAX_DATABASE:
            raise ValueError("Unsafe authentication database")
        data = stream.read(MAX_DATABASE + 1)
    if len(data) > MAX_DATABASE:
        raise ValueError("Oversized authentication database")
    return data.decode("utf-8")


def main():
    if sys.platform != "linux":
        return 111
    if sys.argv[1:] == ["--check"]:
        Sodium()
        return 0 if Path(REPLY).is_file() else 111
    if (sys.argv[1:] != [REPLY] or os.environ.get("ORIG_UID") != str(os.getuid())
            or os.geteuid() != os.getuid() or os.getegid() != os.getgid()
            or "AUTHORIZED" in os.environ):
        return 111
    import resource
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    with os.fdopen(3, "rb", closefd=True) as stream:
        payload = stream.read(2049)
    try:
        credentials(payload)
    except (ValueError, UnicodeError):
        return 1
    environment = authenticate(payload, read_database(), Sodium())
    if environment is None:
        return 1
    # Dovecot's reply binary requires proof that the authentication process UID
    # was retained. Never forward an unvalidated ORIG_UID or set INSECURE_SETUID.
    environment["ORIG_UID"] = str(os.getuid())
    # execve accepts a complete fixed environment. Client AUTH_* / LD_* / UID,
    # AUTHORIZED and INSECURE_SETUID values can never alter the reply protocol.
    os.execve(REPLY, [REPLY], environment)
    return 111


if __name__ == "__main__":
    try:
        result = main()
    except Exception:
        result = 111
    sys.exit(result)
