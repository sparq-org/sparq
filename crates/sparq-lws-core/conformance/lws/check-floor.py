#!/usr/bin/env python3
"""Fail unless the latest Touchstone run passes exactly as many tests in each module as floor.json
records.

    check-floor.py <runs-dir> [floor.json]
    check-floor.py --self-test

Modules are the first path segment of a test id (core, auth, index), except notifications/webhook.
Raise the floor in the same change that makes more tests pass (more passes than the floor fail
the check too, so a new pass cannot hide a lost one later); never lower it to go green.

floor.json's "supported" maps a `*-when-unsupported` test, which probes the absence of a feature
this server supports, to the reason it must be inapplicable for: Touchstone's report that a named
precondition does not hold because a named assertion of it got a value matching a pattern (the
feature advertised). Such a test counts as a pass only when it is inapplicable for that reason. A
pass of it, any other outcome or reason, or its absence from the run fails the check, since each
means the feature was lost or not seen.
"""
import glob
import json
import os
import re
import sys
from collections import Counter


def module(test_id: str) -> str:
    path = test_id.split("#")[0]
    return "notifications/webhook" if path.startswith("notifications/") else path.split("/")[0]


# Touchstone's outcomes (harness-core results/Outcome.java) and the members of a result in its
# report.json.
OUTCOMES = {"passed", "failed", "cantTell", "inapplicable", "untested"}
FIELDS = {"anchor", "detail", "durationMillis", "id", "label", "level", "outcome", "reason", "requirements"}


def unique_keys(pairs):
    """A JSON object's members, refusing one named twice (json.load would keep the last)."""
    keys = [k for k, _ in pairs]
    if len(keys) != len(set(keys)):
        raise ValueError(f"a JSON object names a member twice: {sorted(k for k in set(keys) if keys.count(k) > 1)}")
    return dict(pairs)


def load(path):
    with open(path) as f:
        return json.load(f, object_pairs_hook=unique_keys)


def inapplicable_for(reason: str, why: dict) -> bool:
    """Whether `reason` is Touchstone's (TestExecution.java) for `why`'s precondition failing on
    its assertion with a value `why["got"]` matches: "precondition '<label>' does not hold:
    <assertion> expected <expected>, got <actual>"."""
    head = f"precondition '{why['precondition']}' does not hold: {why['assertion']} expected "
    if not reason.startswith(head) or ", got " not in reason[len(head):]:
        return False
    got = reason[len(head):].rsplit(", got ", 1)[1]
    return re.fullmatch(why["got"], got) is not None


def valid_pattern(pattern: str) -> bool:
    try:
        re.compile(pattern)
        return True
    except re.error:
        return False


def parse(tests) -> tuple:
    """A report's results as `{id: (outcome, level, reason)}`, and why it is not well formed: each
    result is an object of known members with a string id and level, a known outcome and a string
    or absent reason, and no id is repeated. Nothing is counted from a report that is not well
    formed."""
    if not isinstance(tests, list):
        return {}, ["the report's tests are not a list"]
    results, bad = {}, []
    for t in tests:
        if not isinstance(t, dict):
            bad.append(f"  malformed result: {t!r}")
            continue
        test_id, outcome, level, reason = t.get("id"), t.get("outcome"), t.get("level"), t.get("reason")
        if set(t) - FIELDS:
            bad.append(f"  unknown members {sorted(set(t) - FIELDS)}: {test_id!r}")
        elif not (isinstance(test_id, str) and isinstance(level, str) and isinstance(reason, (str, type(None)))):
            bad.append(f"  malformed result: {t!r}")
        elif outcome not in OUTCOMES:
            bad.append(f"  unknown outcome {outcome!r}: {test_id}")
        elif test_id in results:
            bad.append(f"  duplicate result: {test_id}")
        else:
            results[test_id] = (outcome, level, reason or "")
    return results, bad


def check(tests, floor_doc: dict) -> tuple:
    """Whether `tests` (a report's) hold `floor_doc` (floor.json's), and the lines to print."""
    results, bad = parse(tests)
    supported = floor_doc.get("supported", {})
    if not isinstance(supported, dict):
        supported, bad = {}, bad + ["  floor.json's supported is not an object"]
    for test_id, why in supported.items():
        if not test_id.endswith("-when-unsupported"):
            bad.append(f"  a supported probe must be a *-when-unsupported test: {test_id}")
        # The precondition and its assertion are named literally, so no other reason for being
        # inapplicable (an earlier step failing, another assertion of the precondition, a
        # capability not declared) can stand in; only the value the assertion got is a pattern.
        elif not (
            isinstance(why, dict)
            and set(why) == {"precondition", "assertion", "got"}
            and all(isinstance(v, str) and v for v in why.values())
            and valid_pattern(why["got"])
        ):
            bad.append(f"  a supported probe needs a precondition, an assertion and a got pattern: {test_id}")
    if bad:
        return False, bad
    floor = floor_doc["passed"]
    ok, out = True, []
    passed, total = Counter(), Counter()
    for test_id, (outcome, _, reason) in results.items():
        m = module(test_id)
        total[m] += 1
        if test_id in supported:
            held = outcome == "inapplicable" and inapplicable_for(reason, supported[test_id])
            passed[m] += held
            if not held:
                ok = False
                out.append(f"  {outcome:9} supported feature lost or not seen: {test_id} ({reason})")
        else:
            passed[m] += outcome == "passed"
    for missing in sorted(set(supported) - set(results)):
        ok = False
        out.append(f"  missing   supported feature not run: {missing}")
    for m in sorted(set(total) | set(floor)):
        want = floor.get(m, 0)
        line = f"{m:24} {passed[m]:4} passed of {total[m]:4} (floor {want})"
        if passed[m] < want:
            ok = False
            line += "  BELOW FLOOR"
        elif passed[m] > want:
            ok = False
            line += "  ABOVE FLOOR: raise it in floor.json"
        out.append(line)
    for test_id, (outcome, level, _) in results.items():
        if outcome in ("failed", "cantTell"):
            out.append(f"  {outcome:9} {level:6} {test_id}")
    return ok, out


def self_test():
    probe = "core/linksets#linkset-put-405-when-unsupported"
    seen = (
        "precondition 'the linkset does not advertise PUT (otherwise this test does not apply)' "
        "does not hold: header Allow matches expected matches /(?i)^(?!.*\\bPUT\\b)/, got GET, HEAD, PUT, PATCH"
    )
    floor_doc = load(os.path.join(os.path.dirname(os.path.abspath(__file__)), "floor.json"))
    floor_doc = {"supported": {probe: floor_doc["supported"][probe]}, "passed": {"core": 4}}

    def result(test_id, outcome="passed", **kw):
        return {"id": test_id, "outcome": outcome, "level": "MUST", **kw}

    def tests_with(*extra):
        return [result("core/a#x"), result("core/a#y")] + list(extra)

    inapp = result(probe, "inapplicable", reason=seen)
    z = result("core/a#z")

    # The probe counts toward the floor only while inapplicable because the feature is advertised.
    assert check(tests_with(z, inapp), floor_doc)[0]
    for reason in (
        None,
        "precondition 'the linkset does not advertise PUT (otherwise this test does not apply)' "
        "does not hold: status expected 200, got 500",
        seen.replace("PUT, PATCH", "PATCH"),
        seen.replace("PUT, PATCH", "PATCH, X-PUT"),
        seen.replace("PUT, PATCH", "PATCH") + "\n  exchange: PUT -> 405",
        seen + " (PUT refused)",
        "precondition 'find its linkset' does not hold: " + seen.split(": ", 1)[1],
    ):
        assert not check(tests_with(z, result(probe, "inapplicable", reason=reason)), floor_doc)[0], reason
    # Passing it means the feature was lost, even though the count would hold.
    for outcome in ("passed", "failed", "cantTell", "untested"):
        assert not check(tests_with(z, result(probe, outcome)), floor_doc)[0], outcome
    assert not check(tests_with(z), floor_doc)[0]
    # An unrelated failure is not hidden by the probe, nor a lost pass by a new one.
    assert not check(tests_with(result("core/a#z", "failed"), inapp), floor_doc)[0]
    assert not check(tests_with(z, inapp, result("core/a#w")), floor_doc)[0]
    # A report counts each test once, with a known outcome: a repeated probe cannot stand in for an
    # unrelated failure, and a malformed result fails the check.
    assert parse(tests_with(inapp, dict(inapp)))[1] == [f"  duplicate result: {probe}"]
    assert not check(tests_with(result("core/a#z", "failed"), inapp, dict(inapp)), floor_doc)[0]
    for odd in (
        result("core/a#z", "ok"),
        {"id": "core/a#z", "outcome": "passed"},
        result(7),
        result("core/a#z", extra=1),
        result("core/a#z", reason=3),
        "core/a#z",
    ):
        assert not check(tests_with(odd, inapp), floor_doc)[0], odd
    assert not check({"core/a#x": "passed"}, floor_doc)[0]
    # A member named twice in one result is refused as the report is read.
    try:
        json.loads('{"id": "core/a#z", "outcome": "failed", "outcome": "passed"}', object_pairs_hook=unique_keys)
        raise AssertionError("a repeated member was read")
    except ValueError:
        pass
    # Only a *-when-unsupported test may stand in for a pass.
    unsupported_z = {**floor_doc, "supported": {"core/a#z": ".*", **floor_doc["supported"]}}
    assert not check(tests_with(result("core/a#z", "inapplicable", reason="no"), inapp), unsupported_z)[0]
    # The precondition and assertion are named literally; only what it got is a pattern.
    why = floor_doc["supported"][probe]
    for bad_why in (
        ".*",
        {**why, "got": "("},
        {**why, "got": 3},
        {**why, "precondition": ""},
        {"precondition": why["precondition"], "got": ".*"},
        {**why, "extra": "x"},
    ):
        assert not check(tests_with(z, inapp), {**floor_doc, "supported": {probe: bad_why}})[0], bad_why
    # Even a got pattern that takes anything holds only the named precondition and assertion.
    loose = {**floor_doc, "supported": {probe: {**why, "got": ".*"}}}
    assert check(tests_with(z, inapp), loose)[0]
    for reason in (
        "precondition 'find its linkset' does not hold: " + seen.split(": ", 1)[1],
        seen.split(": ", 1)[0] + ": status expected 200, got 500",
        "precondition 'x|.*' does not hold: header Allow matches expected y, got PUT",
    ):
        assert not check(tests_with(z, result(probe, "inapplicable", reason=reason)), loose)[0], reason
    assert not check(tests_with(z, inapp), {**floor_doc, "supported": [probe]})[0]
    # Without the probe listed, a module is held to its plain pass count.
    assert not check([result("core/a#x")], {"passed": {"core": 2}})[0]
    assert check([result("core/a#x"), result("core/a#y")], {"passed": {"core": 2}})[0]
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
    try:
        report, floor_doc = load(reports[-1]), load(floor_path)
    except ValueError as e:
        print(f"  malformed report or floor: {e}")
        return 1
    ok, out = check(report["tests"], floor_doc)
    print("\n".join(out))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
