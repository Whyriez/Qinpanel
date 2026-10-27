"""Disposable Linux CI only: probe sockets, never apply firewall rules."""
import configparser
import os
from pathlib import Path
import subprocess
import sys
import uuid


def main():
    if not (sys.platform == "linux" and os.geteuid() == 0 and
            os.environ.get("GITHUB_ACTIONS") == "true" and
            os.environ.get("QONPANEL_DISPOSABLE_CI") == "1"):
        raise SystemExit("Requires explicit disposable root Linux CI environment")
    root = Path(__file__).resolve().parents[2]
    for name, expected in [("panel-agent", True), ("panel-api", False)]:
        unit = configparser.ConfigParser(strict=False, interpolation=None)
        unit.read(root / "templates/systemd" / (name + ".service"))
        families = unit["Service"]["RestrictAddressFamilies"]
        assert ("AF_NETLINK" in families.split()) is expected
        capabilities = "CAP_NET_ADMIN" if expected else ""
        program = """
import errno, socket, sys
try:
    with socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, 12) as connection:
        connection.bind((0, 0))
except OSError as error:
    assert sys.argv[1] == 'denied' and error.errno == errno.EAFNOSUPPORT, error
else:
    assert sys.argv[1] == 'allowed', 'API unexpectedly obtained Netlink'
print('Netlink socket probe passed')
"""
        # A private network namespace isolates this from runner networking.
        # No packets, firewall mutations or application daemons are involved.
        subprocess.run([
            "systemd-run", "--quiet", "--wait", "--pipe", "--collect",
            "--unit=qinpanel-netlink-" + uuid.uuid4().hex,
            "--property=PrivateNetwork=yes", "--property=NoNewPrivileges=yes",
            "--property=RuntimeMaxSec=30", "--property=RestrictAddressFamilies=" + families,
            "--property=CapabilityBoundingSet=" + capabilities,
            sys.executable, "-B", "-c", program, "allowed" if expected else "denied",
        ], check=True, timeout=45)
        print("PASS " + name + " address-family boundary")


if __name__ == "__main__":
    main()
