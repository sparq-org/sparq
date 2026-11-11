#!/usr/bin/env python3
"""Read an LWS.net TRX report and optionally require exactly the named expected failures.

    lws-net-report.py REPORT.trx --status N [--expected-failures FILE --expected-total T] [--gha]
    lws-net-report.py --self-test

A ratcheted run fails on an unlisted failure, an expected failure passing or missing, an
incomplete/inconsistent report, or a harness infrastructure error. Without the ratchet, the
harness's own exit status stands (the lws-test-suite tests.yaml run).
"""

import argparse
import json
import sys
import xml.etree.ElementTree as ET

NS = "{http://microsoft.com/schemas/VisualStudio/TeamTest/2010}"
# Microsoft.Testing.Platform exits 2 for test failures. Other nonzero codes mean the run failed.
TEST_OUTCOME_STATUSES = {0, 2}


def unique_keys(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise ValueError(f"duplicate JSON key: {key}")
        out[key] = value
    return out


def expected_failures(path):
    """Load exact names with a category and evidence; reject malformed or duplicate entries."""
    try:
        with open(path) as f:
            doc = json.load(f, object_pairs_hook=unique_keys)
        if not isinstance(doc, dict) or set(doc) != {"_doc", "expected_failures"}:
            raise ValueError("expected _doc and expected_failures")
        if not isinstance(doc["expected_failures"], list):
            raise ValueError("expected_failures must be a list")
        names = set()
        for entry in doc["expected_failures"]:
            if not isinstance(entry, dict) or set(entry) != {"name", "category", "reason"}:
                raise ValueError("each entry needs name, category and reason")
            if any(not isinstance(entry[k], str) or not entry[k].strip() for k in entry):
                raise ValueError("entry fields must be nonempty strings")
            if entry["category"] not in {"intentional", "runner"}:
                raise ValueError("a server defect cannot be an expected failure")
            if entry["name"] in names:
                raise ValueError(f"duplicate expected failure: {entry['name']}")
            names.add(entry["name"])
        return names
    except (OSError, ValueError) as e:
        raise ValueError(f"unusable expected-failures list: {e}") from None


def read(path, strict=False):
    """(executed, passed, results by exact name), or ValueError for an unusable report."""
    try:
        root = ET.parse(path).getroot()
    except (OSError, ET.ParseError) as e:
        raise ValueError(f"no readable report: {e}") from None
    counters = root.find(f"{NS}ResultSummary/{NS}Counters")
    if counters is None:
        raise ValueError("the report has no result counters")
    try:
        executed, passed = int(counters.get("executed")), int(counters.get("passed"))
    except (TypeError, ValueError):
        raise ValueError("the report's counters are not numbers") from None
    results = {}
    # Leaf results only: a data-driven test's parent repeats its rows' outcome.
    for r in root.iter(NS + "UnitTestResult"):
        if r.find(NS + "InnerResults") is not None:
            continue
        name, outcome = r.get("testName"), r.get("outcome")
        if strict and (not name or name in results):
            raise ValueError(f"missing or duplicate test name: {name!r}")
        if strict and outcome not in {"Passed", "Failed"}:
            raise ValueError(f"test did not finish: {name} ({outcome})")
        msg = r.findtext(f"{NS}Output/{NS}ErrorInfo/{NS}Message") or ""
        results[name] = (outcome, " ".join(msg.split())[:300])
    if strict and (executed != len(results) or passed != sum(o == "Passed" for o, _ in results.values())):
        raise ValueError("the report's counters disagree with its leaf results")
    if strict:
        try:
            total, failed = int(counters.get("total")), int(counters.get("failed"))
        except (TypeError, ValueError):
            raise ValueError("the report's counters are not numbers") from None
        if total != executed or failed != executed - passed:
            raise ValueError("the report's counters disagree with its leaf results")
    return executed, passed, results


def exit_code(status):
    """An out-of-range process exit code is a failure, not its low byte."""
    return status if 0 <= status <= 255 else 1


def problems(path, status, expected=None, expected_total=None):
    """(summary, failed tests, reasons the run does not count)."""
    try:
        executed, passed, results = read(path, strict=expected is not None)
    except ValueError as e:
        return f"{e} (exit {status})", [], ["unreadable report"]
    failed = {name for name, (outcome, _) in results.items() if outcome != "Passed"}
    failures = [f"{name} ({results[name][0]}): {results[name][1]}" for name in sorted(failed)]
    summary = f"{passed} of {executed} passed (exit {status})"
    why = []
    if expected is not None:
        if status not in TEST_OUTCOME_STATUSES:
            why.append(f"the harness exited {status}, not with a test outcome")
        if expected_total is None or expected_total < 1:
            why.append("a named ratchet requires a positive expected total")
        elif executed != expected_total:
            why.append(f"{executed} tests ran, {expected_total} expected")
        for name in sorted(failed - expected):
            why.append(f"unlisted failure: {name}")
        for name in sorted(expected - failed):
            outcome = "passed: remove it from the list" if name in results else "did not run"
            why.append(f"expected failure {outcome}: {name}")
    return summary, failures, why


def check(path, status, expected=None, expected_total=None, gha=False):
    summary, failures, why = problems(path, status, expected, expected_total)
    print(f"{'::notice title=lws-net::' if gha else ''}lws-net {summary}")
    for line in failures:
        print(f"::error title=lws-net failed::{line}" if gha else line)
    for w in why:
        print(f"{'::error title=lws-net::' if gha else ''}lws-net: {w}")
    if why:
        return 1
    return 0 if expected is not None else exit_code(status)


def trx(outcomes):
    rows = "".join(
        f'<UnitTestResult testName="t{i}" outcome="{o}"/>' for i, o in enumerate(outcomes)
    )
    n, ok = len(outcomes), outcomes.count("Passed")
    counters = f'<Counters total="{n}" executed="{n}" passed="{ok}" failed="{n - ok}"/>'
    return f'<TestRun xmlns="{NS[1:-1]}"><Results>{rows}</Results><ResultSummary>{counters}</ResultSummary></TestRun>'


def self_test():
    import contextlib
    import io
    import os
    import subprocess
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        report = os.path.join(tmp, "report.trx")

        def why(text, status=2, **kw):
            with open(report, "w") as f:
                f.write(text)
            return problems(report, status, **kw)[2]

        ratchet = dict(expected={"t2"}, expected_total=3)
        complete = trx(["Passed", "Passed", "Failed"])
        assert why(complete, **ratchet) == [], "exact listed set"
        assert why(trx(["Passed", "Failed", "Failed"]), **ratchet) == ["unlisted failure: t1"]
        assert why(trx(["Passed"] * 3), 0, **ratchet) == [
            "expected failure passed: remove it from the list: t2"
        ]
        assert why(trx(["Passed", "Failed", "Passed"]), **ratchet) == [
            "unlisted failure: t1", "expected failure passed: remove it from the list: t2"
        ], "a new failure cannot replace a passing exception"
        assert why(trx(["Passed"] * 2), **ratchet) == [
            "2 tests ran, 3 expected", "expected failure did not run: t2"
        ]
        for status in (1, 3, 7, 8, 256, -256):
            assert why(complete, status, **ratchet) == [f"the harness exited {status}, not with a test outcome"]
        assert why(complete, 0, **ratchet) == [], "valid test outcome"
        assert why(complete, expected={"t2"}) == ["a named ratchet requires a positive expected total"]
        for bad in ("", complete[:-10], complete.replace('testName="t2"', 'testName="t1"'),
                    complete.replace('total="3"', 'total="9"'),
                    complete.replace('failed="1"', 'failed="0"'),
                    complete.replace('total="3"', 'total="invalid"'),
                    complete.replace('executed="3"', 'executed="9"'),
                    complete.replace('passed="2"', 'passed="3"'),
                    complete.replace('outcome="Failed"', 'outcome="NotExecuted"'),
                    f'<TestRun xmlns="{NS[1:-1]}"><Results/></TestRun>'):
            assert why(bad, **ratchet) == ["unreadable report"]
        assert problems("/nonexistent/lws-net.trx", 0, **ratchet)[2] == ["unreadable report"]
        with contextlib.redirect_stdout(io.StringIO()):
            why(complete)
            assert check(report, 2, **ratchet) == 0
            assert check(report, 2) == 2, "unratcheted run keeps its status"
            assert check(report, 256) == 1
            why(trx(["Passed", "NotExecuted"]))
            assert check(report, 0) == 0, "unratcheted run retains skipped-test handling"
            why("")
            for status in (0, 2, 256, -256):
                assert check(report, status) == 1 and check(report, status, **ratchet) == 1
        entries = os.path.join(tmp, "expected.json")
        entry = dict(name="t2", category="runner", reason="fixture")
        for value, valid in [([entry], True), ([entry, entry], False),
                             ([dict(entry, category="server")], False),
                             ([dict(entry, reason="")], False)]:
            with open(entries, "w") as f:
                json.dump(dict(_doc="fixture", expected_failures=value), f)
            try:
                assert expected_failures(entries) == {"t2"}
                assert valid
            except ValueError:
                assert not valid
        with open(entries, "w") as f:
            f.write('{"_doc":"x","expected_failures":[],"expected_failures":[]}')
        try:
            expected_failures(entries)
            raise AssertionError("duplicate JSON keys accepted")
        except ValueError:
            pass
        with open(entries, "w", encoding="utf-8") as f:
            json.dump(dict(_doc="fixture", expected_failures=[entry]), f)
        for report_text, status, code in [
            (complete, 2, 0),
            (trx(["Passed", "Failed", "Failed"]), 2, 1),
            (trx(["Passed"] * 3), 0, 1),
            (complete.replace('testName="t2"', 'testName="renamed"'), 2, 1),
            (complete.replace('testName="t2"', 'testName="t1"'), 2, 1),
            (complete.replace('total="3"', 'total="9"'), 2, 1),
        ]:
            with open(report, "w", encoding="utf-8") as f:
                f.write(report_text)
            r = subprocess.run([
                sys.executable, "-B", __file__, report, "--status", str(status),
                "--expected-failures", entries, "--expected-total", "3",
            ], capture_output=True, text=True)
            assert r.returncode == code, (code, r.returncode, r.stdout, r.stderr)
        with open(report, "wb") as f:
            f.write(complete.replace('testName="t2"', 'testName="\xff"').encode("latin-1"))
        assert problems(report, 2, **ratchet)[2] == ["unreadable report"]
        with open(entries, "wb") as f:
            f.write(b'{"_doc":"fixture","expected_failures":[{"name":"\xff"}]}')
        try:
            expected_failures(entries)
            raise AssertionError("invalid UTF-8 accepted")
        except ValueError:
            pass
        for args in (["--status", "256"], ["--status", "512", "--expected-failures", entries, "--expected-total", "3"]):
            r = subprocess.run([sys.executable, "-B", __file__, "/nonexistent/lws-net.trx", *args], capture_output=True, text=True)
            assert r.returncode == 1, (args, r.returncode, r.stdout)
    print("lws-net-report self-test passed")


if __name__ == "__main__":
    a = argparse.ArgumentParser()
    a.add_argument("report", nargs="?")
    a.add_argument("--status", type=int, default=0)
    a.add_argument("--expected-failures")
    a.add_argument("--expected-total", type=int)
    a.add_argument("--gha", action="store_true")
    a.add_argument("--self-test", action="store_true")
    o = a.parse_args()
    if o.self_test:
        self_test()
        sys.exit(0)
    try:
        expected = expected_failures(o.expected_failures) if o.expected_failures else None
    except ValueError as e:
        print(f"lws-net: {e}")
        sys.exit(1)
    if o.expected_total is not None and expected is None:
        a.error("--expected-total requires --expected-failures")
    sys.exit(check(o.report, o.status, expected, o.expected_total, o.gha))
