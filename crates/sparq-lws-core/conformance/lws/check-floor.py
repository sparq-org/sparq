#!/usr/bin/env python3
"""Fail when the latest Touchstone run passes fewer tests in any module than floor.json records.

    check-floor.py <runs-dir> [floor.json]

Modules are the first path segment of a test id (core, auth, index), except notifications/webhook.
Raise the floor in the same change that makes more tests pass; never lower it to go green.
"""
import glob
import json
import os
import sys
from collections import Counter


def module(test_id: str) -> str:
    path = test_id.split("#")[0]
    return "notifications/webhook" if path.startswith("notifications/") else path.split("/")[0]


def main() -> int:
    runs = sys.argv[1]
    floor_path = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(__file__), "floor.json")
    reports = sorted(glob.glob(os.path.join(runs, "*", "report.json")))
    if not reports:
        print(f"no Touchstone report under {runs}", file=sys.stderr)
        return 2
    report = json.load(open(reports[-1]))
    passed, total = Counter(), Counter()
    for t in report["tests"]:
        m = module(t["id"])
        total[m] += 1
        passed[m] += t["outcome"] == "passed"
    floor = json.load(open(floor_path))["passed"]
    ok = True
    for m in sorted(set(total) | set(floor)):
        want = floor.get(m, 0)
        line = f"{m:24} {passed[m]:4} passed of {total[m]:4} (floor {want})"
        if passed[m] < want:
            ok = False
            line += "  BELOW FLOOR"
        print(line)
    for t in report["tests"]:
        if t["outcome"] in ("failed", "cantTell"):
            print(f"  {t['outcome']:9} {t['level']:6} {t['id']}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
