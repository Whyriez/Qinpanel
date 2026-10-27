#!/usr/bin/env python3
"""Verify original build/security jobs before reusing immutable release artifacts."""
import json
import sys

PUBLISH_JOBS = {"Publish Qinpanel release", "Publish QonPanel release"}
REQUIRED_STEPS = {
    "Native file-worker jail and privilege-drop acceptance",
    "Audit frontend dependencies",
}


def verify(pages):
    latest = {}
    for page in pages:
        for job in page.get("jobs", []):
            name = job.get("name", "")
            if name in PUBLISH_JOBS:
                continue
            if name not in latest or job.get("id", 0) > latest[name].get("id", 0):
                latest[name] = job
    jobs = list(latest.values())
    builds = [job for job in jobs if job["name"].startswith(("Build Qinpanel (", "Build QonPanel ("))]
    if len(builds) != 2 or any(job.get("conclusion") != "success" for job in builds):
        raise ValueError("Both original architecture builds must pass")
    targets = {job["name"].rsplit("(", 1)[-1].removesuffix(")") for job in builds}
    if targets != {"x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"}:
        raise ValueError("Original artifacts must cover both supported architectures")
    if any(job.get("status") != "completed" or job.get("conclusion") not in {"success", "skipped"} for job in jobs):
        raise ValueError("Original build/platform/security jobs must complete successfully")
    successful_steps = {
        step.get("name") for job in jobs if job.get("conclusion") == "success"
        for step in job.get("steps", []) if step.get("conclusion") == "success"
    }
    if not REQUIRED_STEPS.issubset(successful_steps):
        raise ValueError("Original artifacts lack passing security checks; rebuild from the reviewed source")


if __name__ == "__main__":
    try:
        with open(sys.argv[1], encoding="utf-8") as source:
            verify(json.load(source))
    except (ValueError, OSError, IndexError, TypeError, KeyError) as error:
        sys.exit(f"Release source verification failed: {error}")
    print("Original build, platform and security jobs verified")
