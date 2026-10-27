#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("release_gate", Path(__file__).resolve().parents[1] / "verify-release-jobs.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def job(identifier, name, steps=(), conclusion="success"):
    return {"id": identifier, "name": name, "status": "completed", "conclusion": conclusion,
            "steps": [{"name": name, "conclusion": "success"} for name in steps]}


def pages():
    return [{"jobs": [
        job(1, "Build Qinpanel (x86_64-unknown-linux-gnu)"),
        job(2, "Build Qinpanel (aarch64-unknown-linux-gnu)"),
        job(3, "security / Backend (Rust)", ["Native file-worker jail and privilege-drop acceptance"]),
        job(4, "security / Frontend", ["Audit frontend dependencies"]),
        job(5, "platforms"),
        job(6, "Publish Qinpanel release", conclusion="failure"),
    ]}]


class ReleaseSecurityGateTest(unittest.TestCase):
    def test_retry_of_failed_publication_accepts_verified_original_artifacts(self):
        gate.verify(pages())

    def test_old_artifacts_without_security_evidence_are_rejected(self):
        data = pages()
        data[0]["jobs"][2:4] = []
        with self.assertRaises(ValueError):
            gate.verify(data)

    def test_skipped_or_failed_native_step_is_not_evidence(self):
        for result in ["skipped", "failure"]:
            data = pages()
            data[0]["jobs"][2]["steps"][0]["conclusion"] = result
            with self.assertRaises(ValueError):
                gate.verify(data)

    def test_latest_rerun_failure_overrides_an_earlier_success(self):
        data = pages()
        failed = copy.deepcopy(data[0]["jobs"][2])
        failed.update(id=20, conclusion="failure")
        data.append({"jobs": [failed]})
        with self.assertRaises(ValueError):
            gate.verify(data)

    def test_latest_rerun_success_overrides_an_earlier_failure(self):
        data = pages()
        earlier = copy.deepcopy(data[0]["jobs"][2])
        earlier.update(id=0, conclusion="failure")
        data.append({"jobs": [earlier]})
        gate.verify(data)

    def test_a_missing_architecture_is_rejected(self):
        data = pages()
        del data[0]["jobs"][1]
        with self.assertRaises(ValueError):
            gate.verify(data)

    def test_two_jobs_for_one_architecture_do_not_replace_the_missing_target(self):
        data = pages()
        data[0]["jobs"][1]["name"] = "Build QonPanel (x86_64-unknown-linux-gnu)"
        with self.assertRaises(ValueError):
            gate.verify(data)


if __name__ == "__main__":
    unittest.main()
