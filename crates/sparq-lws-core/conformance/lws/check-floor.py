#!/usr/bin/env python3
"""Fail when the latest Touchstone run passes fewer tests in any module than floor.json records.

    check-floor.py <runs-dir> [floor.json]
    check-floor.py --self-test

Modules are the first path segment of a test id (core, auth, index), except notifications/webhook.
Raise the floor in the same change that makes more tests pass; never lower it to go green.
A test floor.json lists under "supported" probes the absence of a feature this server supports: it
counts as a pass only when inapplicable, and a pass of it (or its absence from the run) fails the
check, since that means the feature was lost.
"""
import glob
import json
import os
import sys
from collections import Counter


def module(test_id: str) -> str:
    path = test_id.split("#")[0]
    return "notifications/webhook" if path.startswith("notifications/") else path.split("/")[0]


# Touchstone's outcomes (harness-core results/Outcome.java).
OUTCOMES = {"passed", "failed", "cantTell", "inapplicable", "untested"}


def malformed(tests) -> list:
    """Why `tests` is not a list of uniquely identified results, each with a known outcome."""
    if not isinstance(tests, list):
        return ["the report's tests are not a list"]
    out, seen = [], set()
    for t in tests:
        if not (isinstance(t, dict) and isinstance(t.get("id"), str) and isinstance(t.get("level"), str)):
            out.append(f"  malformed result: {t!r}")
        elif t.get("outcome") not in OUTCOMES:
            out.append(f"  unknown outcome {t.get('outcome')!r}: {t['id']}")
        elif t["id"] in seen:
            out.append(f"  duplicate result: {t['id']}")
        else:
            seen.add(t["id"])
    return out


def check(tests: list, floor_doc: dict) -> tuple:
    """Whether `tests` (a report's) hold `floor_doc` (floor.json's), and the lines to print. A
    report that is not well formed (see `malformed`) holds nothing."""
    bad = malformed(tests)
    if bad:
        return False, bad
    floor, supported = floor_doc["passed"], set(floor_doc.get("supported", []))
    ok, out = True, []
    passed, total = Counter(), Counter()
    for t in tests:
        m = module(t["id"])
        total[m] += 1
        if t["id"] in supported:
            held = t["outcome"] == "inapplicable"
            passed[m] += held
            if not held:
                ok = False
                out.append(f"  {t['outcome']:9} supported feature lost: {t['id']}")
        else:
            passed[m] += t["outcome"] == "passed"
    for missing in sorted(supported - {t["id"] for t in tests}):
        ok = False
        out.append(f"  missing   supported feature not run: {missing}")
    for m in sorted(set(total) | set(floor)):
        want = floor.get(m, 0)
        line = f"{m:24} {passed[m]:4} passed of {total[m]:4} (floor {want})"
        if passed[m] < want:
            ok = False
            line += "  BELOW FLOOR"
        out.append(line)
    for t in tests:
        if t["outcome"] in ("failed", "cantTell"):
            out.append(f"  {t['outcome']:9} {t['level']:6} {t['id']}")
    return ok, out


def self_test():
    probe = "core/linksets#linkset-put-405-when-unsupported"
    floor_doc = {"supported": [probe], "passed": {"core": 4}}

    def run(probe_outcome, other="passed"):
        tests = [
            {"id": "core/a#x", "outcome": "passed", "level": "MUST"},
            {"id": "core/a#y", "outcome": "passed", "level": "MUST"},
            {"id": "core/a#z", "outcome": other, "level": "MUST"},
        ]
        if probe_outcome is not None:
            tests.append({"id": probe, "outcome": probe_outcome, "level": "MUST"})
        return check(tests, floor_doc)[0]

    # The probe counts toward the floor only while it is inapplicable.
    assert run("inapplicable")
    # Passing it means the feature was lost, even though the count would hold.
    assert not run("passed")
    assert not run("failed")
    assert not run(None)
    # An unrelated failure is not hidden by the probe.
    assert not run("inapplicable", other="failed")
    assert not run("passed", other="failed")
    # A report counts each test once, with a known outcome: a repeated probe cannot stand in
    # for an unrelated failure, and a malformed result fails the check.
    def tests_with(*extra):
        base = [{"id": f"core/a#{i}", "outcome": "passed", "level": "MUST"} for i in "xy"]
        return base + list(extra)
    inapp = {"id": probe, "outcome": "inapplicable", "level": "MUST"}
    failed = {"id": "core/a#z", "outcome": "failed", "level": "MUST"}
    assert not check(tests_with(failed, inapp, dict(inapp)), floor_doc)[0]
    assert not check(tests_with({"id": "core/a#x", "outcome": "passed", "level": "MUST"}, inapp), floor_doc)[0]
    for odd in ({"id": "core/a#z", "outcome": "ok", "level": "MUST"},
                {"id": "core/a#z", "outcome": "passed"},
                {"id": 7, "outcome": "passed", "level": "MUST"},
                "core/a#z"):
        assert not check(tests_with(odd, inapp), floor_doc)[0], odd
    assert not check({"core/a#x": "passed"}, floor_doc)[0]
    assert check(tests_with({"id": "core/a#z", "outcome": "passed", "level": "MUST"}, inapp), floor_doc)[0]
    # Without the probe listed, a module is held to its plain pass count.
    assert not check([{"id": "core/a#x", "outcome": "passed", "level": "MUST"}], {"passed": {"core": 2}})[0]
    print("check-floor self-test passed")


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return 0
    runs = sys.argv[1]
    floor_path = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(__file__), "floor.json")
    reports = sorted(glob.glob(os.path.join(runs, "*", "report.json")))
    if not reports:
        print(f"no Touchstone report under {runs}", file=sys.stderr)
        return 2
    report = json.load(open(reports[-1]))
    ok, out = check(report["tests"], json.load(open(floor_path)))
    print("\n".join(out))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
