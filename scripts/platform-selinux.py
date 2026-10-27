#!/usr/bin/python3
"""Fixed Qinpanel SELinux policy migration. No API paths or arbitrary commands.

The injectable root/executor are used only by imported hermetic tests.
"""
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys

STATE = "var/lib/qonpanel-platform/selinux-transaction.json"
CONTEXTS = {
    r"/etc/qonpanel-storage(/.*)?": "cert_t",
    r"/run/qonpanel-storage(/.*)?": "httpd_var_run_t",
    r"/usr/local/libexec/qonpanel-object-storage": "bin_t",
    r"/usr/local/lib/qonpanel-storage/weed": "bin_t",
    r"/etc/panel/nginx(/.*)?": "httpd_config_t",
    r"/etc/panel/php(/.*)?": "httpd_config_t",
    r"/etc/panel/ssl(/.*)?": "cert_t",
    r"/etc/panel/panel-tls(/.*)?": "cert_t",
    r"/run/panel/php(/.*)?": "httpd_var_run_t",
    r"/run/panel/python(/.*)?": "httpd_var_run_t",
    r"/var/log/panel(/.*)?": "httpd_log_t",
    r"/home/p_[a-z0-9_]+/sites(/.*)?": "httpd_user_rw_content_t",
    r"/home/p_[a-z0-9_]+/tmp(/.*)?": "httpd_tmp_t",
    r"/var/lib/panel/mail(/.*)?": "mail_spool_t",
    r"/var/spool/postfix/qpanel-dkim(/.*)?": "dkim_milter_data_t",
    r"/etc/dovecot/qpanel-users": "dovecot_passwd_t",
    r"/etc/dovecot/qpanel-userdb": "dovecot_passwd_t",
    r"/usr/local/libexec/qonpanel-mail-auth": "bin_t",
}
BOOLEANS = ("httpd_can_network_connect", "httpd_enable_homedirs")
RELABEL = ("etc/qonpanel-storage", "run/qonpanel-storage", "usr/local/libexec/qonpanel-object-storage", "etc/panel/nginx", "etc/panel/php", "etc/panel/ssl", "etc/panel/panel-tls",
           "run/panel/php", "run/panel/python", "var/log/panel", "var/lib/panel/mail",
           "var/spool/postfix/qpanel-dkim", "etc/dovecot", "usr/local/libexec/qonpanel-mail-auth")


class NativeBooleanStore:
    """Use the installed semanage binding to preserve local override absence.

    semanage export reports active boolean values on some versions, so it cannot
    distinguish a temporary runtime change from the persistent policy value.
    """
    def read(self):
        import seobject
        values = seobject.booleanRecords().get_all(True)
        return {name: "on" if values[name][0] else "off" for name in BOOLEANS if name in values}

    def remove(self, name):
        if name not in BOOLEANS:
            raise ValueError("Boolean outside managed scope")
        import seobject
        seobject.booleanRecords().delete(name)


class Policy:
    def __init__(self, root=Path("/"), execute=None, boolean_store=None):
        self.root = root.resolve()
        self.file = self.root / STATE
        self.execute = execute or self.process
        # Imported fixtures can never fall back to the host's SELinux store.
        self.boolean_store = boolean_store if boolean_store is not None else (NativeBooleanStore() if self.root == Path("/") else None)

    @staticmethod
    def process(args):
        return subprocess.run(args, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C"}).stdout.decode()

    def save(self, actions):
        self.file.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        temporary = self.file.with_suffix(".tmp")
        with temporary.open("w", encoding="utf-8") as stream:
            os.chmod(temporary, 0o600)
            json.dump(actions, stream)
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(self.file)

    def supported(self):
        release = (self.root / "etc/os-release").read_text()
        fields = dict(line.split("=", 1) for line in release.splitlines() if "=" in line)
        return fields.get("ID", "").strip('"') == "almalinux"

    def apply(self):
        if not self.supported():
            return
        if self.file.exists():
            raise ValueError("Unfinished SELinux migration: run the installed helper --rollback first")
        # Do not re-enable a policy disabled by the server owner. With SELinux
        # present, retain the current enforcing/permissive mode throughout.
        if not (self.root / "sys/fs/selinux/enforce").exists():
            return
        if self.boolean_store is None:
            raise ValueError("An isolated boolean store is required for fixtures")
        overrides = self.boolean_store.read()
        if any(value not in ("on", "off") for name, value in overrides.items() if name in BOOLEANS):
            raise ValueError("Unrecognized persistent boolean state")
        actions = []
        local = {}
        for line in self.execute(["semanage", "export"]).splitlines():
            words = shlex.split(line)
            if words and words[0] == "fcontext" and "-t" in words:
                local[words[-1]] = words[words.index("-t") + 1]
        self.save(actions)
        for pattern, context in CONTEXTS.items():
            if pattern in local:
                if local[pattern] != context:
                    raise ValueError("Custom SELinux file context preserved: " + pattern)
                continue
            actions.append(["context", pattern])
            self.save(actions)
            self.execute(["semanage", "fcontext", "-a", "-t", context, pattern])
        for boolean in BOOLEANS:
            state = self.execute(["getsebool", boolean]).strip().split()[-1]
            if state not in ("on", "off"):
                raise ValueError("Unrecognized SELinux boolean state")
            # A runtime 'on' may still revert to 'off' on reboot. Persist the
            # required value and journal both dimensions for exact rollback.
            if state != "on" or overrides.get(boolean) != "on":
                actions.append(["boolean", boolean, state, overrides.get(boolean)])
                self.save(actions)
                self.execute(["setsebool", "-P", boolean, "on"])
        port = (self.root / "etc/panel/public-port").read_text().strip()
        if not port.isdecimal() or not 1024 <= int(port) <= 65535 or int(port) == 7700:
            raise ValueError("Invalid managed panel port")
        owners = set()
        for line in self.execute(["semanage", "port", "-l"]).splitlines():
            parts = line.split()
            if len(parts) < 3 or parts[1] != "tcp" or parts[0] in ("unreserved_port_t", "ephemeral_port_t", "port_t"):
                continue
            for entry in "".join(parts[2:]).split(","):
                if re.fullmatch(r"\d+(?:-\d+)?", entry):
                    bounds = list(map(int, entry.split("-")))
                    if bounds[0] <= int(port) <= bounds[-1]:
                        owners.add(parts[0])
        if owners - {"http_port_t"}:
            raise ValueError("Panel port has a custom SELinux type; existing policy preserved")
        if not owners:
            actions.append(["port", port])
            self.save(actions)
            self.execute(["semanage", "port", "-a", "-t", "http_port_t", "-p", "tcp", port])
        self.relabel()

    def relabel(self):
        for name in RELABEL:
            path = self.root / name
            if path.exists():
                self.execute(["restorecon", "-RF", "--", str(path)])
        home = self.root / "home"
        if home.exists():
            for tenant in home.iterdir():
                if not tenant.is_symlink() and re.fullmatch(r"p_[a-z0-9_]{1,28}", tenant.name):
                    for name in ("sites", "tmp"):
                        if (tenant / name).exists():
                            self.execute(["restorecon", "-RF", "--", str(tenant / name)])

    def rollback(self):
        if not self.file.exists():
            return
        if self.file.is_symlink() or self.file.stat().st_size > 16384:
            raise ValueError("Invalid SELinux migration journal")
        actions = json.loads(self.file.read_text())
        for action in actions:
            if not (isinstance(action, list) and (len(action) == 2 and action[0] == "context" and action[1] in CONTEXTS or
                len(action) == 3 and action[0] == "boolean" and action[1] in BOOLEANS and action[2] == "off" or
                len(action) == 4 and action[0] == "boolean" and action[1] in BOOLEANS and action[2] in ("on", "off") and action[3] in (None, "on", "off") or
                len(action) == 2 and action[0] == "port" and isinstance(action[1], str) and action[1].isdecimal() and 1024 <= int(action[1]) <= 65535)):
                raise ValueError("Invalid SELinux rollback action")
        while actions:
            action = actions[-1]
            if action[0] == "context":
                # An intent can survive a failed command. Remove only a local
                # rule with the exact value this migration intended to add.
                rules = [shlex.split(line) for line in self.execute(["semanage", "export"]).splitlines()]
                matches = [rule for rule in rules if rule and rule[0] == "fcontext" and rule[-1] == action[1] and "-t" in rule]
                if matches:
                    if any(rule[rule.index("-t") + 1] != CONTEXTS[action[1]] for rule in matches):
                        raise ValueError("SELinux context changed after migration; manual recovery required")
                    self.execute(["semanage", "fcontext", "-d", action[1]])
            elif action[0] == "port":
                rules = [shlex.split(line) for line in self.execute(["semanage", "export"]).splitlines()]
                matches = [rule for rule in rules if rule and rule[0] == "port" and rule[-1] == action[1] and "-t" in rule and "-p" in rule and rule[rule.index("-p") + 1] == "tcp"]
                if matches:
                    if any(rule[rule.index("-t") + 1] != "http_port_t" for rule in matches):
                        raise ValueError("SELinux port changed after migration; manual recovery required")
                    self.execute(["semanage", "port", "-d", "-p", "tcp", action[1]])
            else:
                if len(action) == 3:  # Read the earlier journal representation.
                    self.execute(["setsebool", "-P", action[1], action[2]])
                else:
                    if self.boolean_store is None:
                        raise ValueError("An isolated boolean store is required for fixtures")
                    if action[3] is None:
                        if action[1] in self.boolean_store.read():
                            self.boolean_store.remove(action[1])
                    else:
                        self.execute(["setsebool", "-P", action[1], action[3]])
                    self.execute(["setsebool", action[1], action[2]])
            actions.pop()
            self.save(actions)
        self.relabel()
        self.file.unlink()


def main():
    if sys.platform != "linux" or os.geteuid() != 0:
        raise SystemExit("This helper is for root on a managed Linux server only")
    os.umask(0o077)
    policy = Policy()
    if sys.argv[1:] == ["--apply"]:
        policy.apply()
    elif sys.argv[1:] == ["--rollback"]:
        policy.rollback()
    elif sys.argv[1:] == ["--commit"]:
        policy.file.unlink(missing_ok=True)
    else:
        raise SystemExit("Usage: qonpanel-selinux --apply|--rollback|--commit")


if __name__ == "__main__":
    main()
