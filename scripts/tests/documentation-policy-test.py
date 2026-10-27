#!/usr/bin/env python3
"""Check the publication policy without requiring ignored local documentation."""

import subprocess
from pathlib import Path


def main() -> None:
    repo = Path(__file__).resolve().parents[2]
    tracked = subprocess.check_output(
        ["git", "ls-files", "-z"], cwd=repo
    ).decode("utf-8").split("\0")
    forbidden = sorted(
        path
        for path in tracked
        if (path.lower().endswith(".md") and path != "README.md")
        or path.lower().startswith("docs/")
    )
    if forbidden:
        raise SystemExit("Local documentation is tracked by Git:\n" + "\n".join(forbidden))
    print("PASS only the root README may be tracked as Markdown")

    probes = [
        "PUBLICATION-PROBE.md",
        "crates/documentation-probe/README.md",
        "crates/documentation-probe/notes.MD",
        "web/public/documentation-probe/notes.Md",
        "docs/documentation-probe/private.txt",
        "AGENTS.md",
    ]
    ignored = subprocess.check_output(
        ["git", "check-ignore", "--no-index", "--stdin", "-z"],
        cwd=repo,
        input=("\0".join(probes) + "\0").encode("utf-8"),
    ).decode("utf-8").split("\0")[:-1]
    if ignored != probes:
        raise SystemExit("Markdown and local docs must remain ignored at every depth")
    print("PASS root/nested Markdown, case variants, docs and agent hook stay ignored")

    readme_ignored = subprocess.run(
        ["git", "check-ignore", "--no-index", "-q", "--", "README.md"], cwd=repo
    )
    if readme_ignored.returncode != 1:
        raise SystemExit("The public root README.md must not be ignored")
    print("PASS root README.md remains eligible for GitHub publication")


if __name__ == "__main__":
    main()
