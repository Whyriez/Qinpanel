"""Actual pre-checkout workflow scripts, fixture repo files and mock DNF only."""
import configparser
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
BASH = str(Path("C:/Program Files/Git/bin/bash.exe")) if os.name == "nt" else shutil.which("bash")
BEGIN = "# AlmaLinux CI repositories: begin (also used before release checkout)."
END = "# AlmaLinux CI repositories: end."


def workflow_script(filename, step):
    text = (ROOT / ".github/workflows" / filename).read_text(encoding="utf-8")
    after = text.split(f"      - name: {step}\n", 1)[1]
    block = after.split("\n      - ", 1)[0]
    return textwrap.dedent(block.split("        run: |\n", 1)[1])


PLATFORM = workflow_script("platform-compatibility.yml", "Bootstrap container tools")
RELEASE = workflow_script("release.yml", "Install native release prerequisites")
MOCKS = r'''
dnf() {
    printf 'dnf %s\n' "$*" >> "$CI_FIXTURE_TRACE"
    case " $* " in
        *' makecache '*)
            n=$(cat "$CI_FIXTURE_ATTEMPTS")
            n=$((n + 1)); printf '%s\n' "$n" > "$CI_FIXTURE_ATTEMPTS"
            if [ "$n" -le "$CI_FIXTURE_FAILURES" ]; then
                echo 'repomd.xml parser error: Document is empty' >&2
                return 1
            fi ;;
    esac
    return 0
}
apt-get() { printf 'apt-get %s\n' "$*" >> "$CI_FIXTURE_TRACE"; }
sleep() { printf 'sleep %s\n' "$*" >> "$CI_FIXTURE_TRACE"; }
command() {
    case "${1:-}:${2:-}" in
        -v:dnf) [ "$CI_FIXTURE_FAMILY" = rpm ] ;;
        -v:curl) [ "$CI_FIXTURE_CURL" = yes ] ;;
        *) builtin command "$@" ;;
    esac
}
'''


@unittest.skipUnless(BASH and Path(BASH).is_file(), "Bash required for hermetic workflow execution")
class AlmaRepositoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="qinpanel-ci-repos-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.repos = self.directory / "repos"
        self.repos.mkdir()
        for name, path, enabled in [("baseos", "BaseOS", 1), ("appstream", "AppStream", 1), ("extras", "extras", 1), ("crb", "CRB", 0)]:
            baseurl_prefix = "" if name == "baseos" else "# "
            scheme = "http" if name in ("baseos", "extras") else "https"
            (self.repos / f"almalinux-{name}.repo").write_text(
                f"[{name}]\nname=AlmaLinux $releasever - {path}\n"
                f"mirrorlist=https://mirrors.almalinux.org/mirrorlist/$releasever/{name}\n"
                f"{baseurl_prefix}baseurl={scheme}://repo.almalinux.org/almalinux/$releasever/{path}/$basearch/os/\n"
                f"enabled={enabled}\ngpgcheck=1\nsslverify=1\ngpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-AlmaLinux\n"
                f"[{name}-source]\nmirrorlist=https://mirrors.almalinux.org/mirrorlist/$releasever/{name}-source\n"
                f"# baseurl=https://vault.almalinux.org/$releasever/{path}/Source/\nenabled=0\ngpgcheck=1\n",
                encoding="utf-8",
            )
        self.vendor = self.repos / "vendor.repo"
        self.vendor.write_text("[vendor]\nmirrorlist=https://vendor.invalid/list\nenabled=0\ngpgcheck=1\n", encoding="utf-8")

    def run_script(self, script, failures=0, family="rpm", curl="yes"):
        trace = self.directory / "trace"
        attempts = self.directory / "attempts"
        trace.write_text("")
        attempts.write_text("0")
        # The only path adaptation: confine actual sed writes to this temp fixture.
        script = script.replace("/etc/yum.repos.d/almalinux*.repo", '"$CI_FIXTURE_REPOS"/almalinux*.repo')
        environment = dict(os.environ, CI_FIXTURE_REPOS=self.repos.as_posix(), CI_FIXTURE_TRACE=trace.as_posix(), CI_FIXTURE_ATTEMPTS=attempts.as_posix(), CI_FIXTURE_FAILURES=str(failures), CI_FIXTURE_FAMILY=family, CI_FIXTURE_CURL=curl)
        # Define the Bash-only apt-get mock first; execute workflow code in POSIX mode.
        result = subprocess.run([BASH, "--noprofile", "--norc", "-e", "-c", MOCKS + "\nset -o posix\n" + script], env=environment, text=True, capture_output=True, timeout=20)
        return result, trace.read_text().splitlines(), int(attempts.read_text())

    def assert_repo_contract(self):
        config = configparser.ConfigParser(interpolation=None)
        config.read(sorted(self.repos.glob("almalinux*.repo")))
        for name, path, enabled in [("baseos", "BaseOS", "1"), ("appstream", "AppStream", "1"), ("extras", "extras", "1"), ("crb", "CRB", "0")]:
            row = config[name]
            self.assertEqual(row["baseurl"], f"https://repo.almalinux.org/almalinux/$releasever/{path}/$basearch/os/")
            self.assertNotIn("mirrorlist", row)
            self.assertEqual(row["enabled"], enabled)
            self.assertEqual(row["gpgcheck"], "1")
            self.assertEqual(row["sslverify"], "1")
            self.assertEqual(row["gpgkey"], "file:///etc/pki/rpm-gpg/RPM-GPG-KEY-AlmaLinux")
            self.assertEqual(config[f"{name}-source"]["enabled"], "0")
            self.assertTrue(config[f"{name}-source"]["baseurl"].startswith("https://vault.almalinux.org/"))
        self.assertEqual(self.vendor.read_text(), "[vendor]\nmirrorlist=https://vendor.invalid/list\nenabled=0\ngpgcheck=1\n")

    def test_both_workflows_share_the_same_precheckout_fix_and_valid_shell(self):
        snippets = [textwrap.dedent(script.split(BEGIN, 1)[1].split(END, 1)[0]).strip() for script in (PLATFORM, RELEASE)]
        self.assertEqual(snippets[0], snippets[1])
        for filename, script in [("platform-compatibility.yml", PLATFORM), ("release.yml", RELEASE)]:
            result = subprocess.run([BASH, "--posix", "-n"], input=script, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            workflow = (ROOT / ".github/workflows" / filename).read_text()
            self.assertLess(workflow.index(BEGIN), workflow.index("uses: actions/checkout@"))

    def test_official_https_and_security_flags_are_preserved_idempotently(self):
        for script in (PLATFORM, RELEASE):
            with self.subTest(workflow="platform" if script == PLATFORM else "release"):
                result, calls, attempts = self.run_script(script)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(attempts, 1)
                self.assert_repo_contract()
                before = {path.name: path.read_bytes() for path in self.repos.iterdir()}
                result, _, _ = self.run_script(script)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(before, {path.name: path.read_bytes() for path in self.repos.iterdir()})
                self.assertTrue(any(" install git " in line for line in calls))

    def test_empty_metadata_is_cleared_and_retried_before_install(self):
        for script in (PLATFORM, RELEASE):
            result, calls, attempts = self.run_script(script, failures=2)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(attempts, 3)
            self.assertEqual(calls.count("dnf -q clean metadata"), 3)
            self.assertEqual([line for line in calls if line.startswith("sleep ")], ["sleep 5", "sleep 10"])
            makecache = [index for index, line in enumerate(calls) if " makecache " in line]
            install = [index for index, line in enumerate(calls) if " install " in line]
            self.assertLess(max(makecache), min(install))
            self.assertTrue(all("--setopt=skip_if_unavailable=False" in calls[index] for index in makecache))

    def test_persistent_metadata_failure_stops_before_install_without_disabling_appstream(self):
        for script in (PLATFORM, RELEASE):
            result, calls, attempts = self.run_script(script, failures=99)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(attempts, 3)
            self.assertFalse(any(" install " in line for line in calls))
            self.assertIn("after three attempts", result.stderr)
            self.assert_repo_contract()

    def test_debian_branch_does_not_change_repository_files_or_invoke_dnf(self):
        before = {path.name: path.read_bytes() for path in self.repos.iterdir()}
        result, calls, attempts = self.run_script(PLATFORM, family="deb")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(attempts, 0)
        self.assertFalse(any(line.startswith("dnf ") for line in calls))
        self.assertIn("apt-get update -qq", calls)
        self.assertEqual(before, {path.name: path.read_bytes() for path in self.repos.iterdir()})

    def test_existing_curl_provider_is_retained_and_missing_curl_gets_minimal(self):
        for available in ("yes", "no"):
            result, calls, _ = self.run_script(PLATFORM, curl=available)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual("dnf -y install curl-minimal" in calls, available == "no")
            self.assertFalse(any(line.endswith("install curl") for line in calls))


if __name__ == "__main__":
    unittest.main()
