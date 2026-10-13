#!/usr/bin/env python3
"""Read an LWS.net TRX report: print each test that did not pass, and with --min-passed hold the
run to a floor. A run counts only when the report parses, every expected test ran, and the harness
exited with a test outcome (0 or a test failure), not an abort or an infrastructure error.

    lws-net-report.py REPORT.trx --status N [--min-passed P --expected-total T] [--gha]
    lws-net-report.py --self-test
"""

import argparse
import sys
import xml.etree.ElementTree as ET

NS = "{http://microsoft.com/schemas/VisualStudio/TeamTest/2010}"
# LWS.net runs on Microsoft.Testing.Platform, which exits 0 when every test passed and 2 when some
# failed. Anything else (1 a generic failure, 3 aborted, 5 invalid arguments, 7 session failure,
# 8 no tests, ...) means the run itself went wrong.
TEST_OUTCOME_STATUSES = {0, 2}


def read(path):
    """(executed, passed, failures) from the report, raising ValueError when it is unusable."""
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
    failures = []
    # Leaf results only: a data-driven test's parent repeats its rows' outcome.
    for r in root.iter(NS + "UnitTestResult"):
        if r.find(NS + "InnerResults") is None and r.get("outcome") != "Passed":
            msg = r.findtext(f"{NS}Output/{NS}ErrorInfo/{NS}Message") or ""
            failures.append(
                f"{r.get('testName')} ({r.get('outcome')}): {' '.join(msg.split())[:300]}"
            )
    return executed, passed, failures


def exit_code(status):
    """`status` as a process exit code: an out-of-range one is a failure, not its low byte."""
    return status if 0 <= status <= 255 else 1


def problems(path, status, min_passed=None, expected_total=None):
    """(summary, failed tests, why the run does not count): the last is empty when it does."""
    try:
        executed, passed, failures = read(path)
    except ValueError as e:
        return f"{e} (exit {status})", [], ["unreadable report"]
    summary = f"{passed} of {executed} passed (exit {status})"
    why = []
    if min_passed is not None:
        if status not in TEST_OUTCOME_STATUSES:
            why.append(f"the harness exited {status}, not with a test outcome")
        if expected_total is not None and executed != expected_total:
            why.append(f"{executed} tests ran, {expected_total} expected")
        if passed < min_passed:
            why.append(f"{passed} passed, below the floor of {min_passed}")
    return summary, failures, why


def check(path, status, min_passed=None, expected_total=None, gha=False):
    """The process exit code for this run, printing what it found."""
    summary, failures, why = problems(path, status, min_passed, expected_total)
    print(f"{'::notice title=lws-net::' if gha else ''}lws-net {summary}")
    for line in failures:
        print(f"::error title=lws-net failed::{line}" if gha else line)
    for w in why:
        print(f"{'::error title=lws-net::' if gha else ''}lws-net: {w}")
    if why:
        return 1
    # Without a floor the harness's own status stands, unless the report is unreadable.
    return 0 if min_passed is not None else exit_code(status)


def trx(outcomes):
    rows = "".join(
        f'<UnitTestResult testName="t{i}" outcome="{o}"/>' for i, o in enumerate(outcomes)
    )
    n, ok = len(outcomes), outcomes.count("Passed")
    counters = f'<Counters total="{n}" executed="{n}" passed="{ok}" failed="{n - ok}"/>'
    return f'<TestRun xmlns="{NS[1:-1]}"><Results>{rows}</Results><ResultSummary>{counters}</ResultSummary></TestRun>'


def self_test():
    import os
    import tempfile

    def run(text, status, **kw):
        with tempfile.NamedTemporaryFile("w", suffix=".trx", delete=False) as f:
            f.write(text)
        try:
            return check(f.name, status, **kw)
        finally:
            os.unlink(f.name)

    floor = dict(min_passed=2, expected_total=3)

    def why(text, status, **kw):
        with tempfile.NamedTemporaryFile("w", suffix=".trx", delete=False) as f:
            f.write(text)
        try:
            return problems(f.name, status, **kw)[2]
        finally:
            os.unlink(f.name)

    complete = trx(["Passed", "Passed", "Failed"])
    assert why(complete, 2, **floor) == [], "a failing test above the floor"
    assert why(complete, 0, **floor) == [], "status 0"
    assert why(complete, 1, **floor) == ["the harness exited 1, not with a test outcome"], (
        "a generic failure"
    )
    assert why(trx(["Passed", "Passed"]), 3, **floor) == [
        "the harness exited 3, not with a test outcome",
        "2 tests ran, 3 expected",
    ], "an aborted run"
    assert why(trx(["Passed", "Passed"]), 2, **floor) == ["2 tests ran, 3 expected"], "a short run"
    assert why(trx(["Passed"] * 3), 7, **floor) == [
        "the harness exited 7, not with a test outcome"
    ], "an infrastructure status"
    assert why(trx(["Passed", "Failed", "Failed"]), 2, **floor) == [
        "1 passed, below the floor of 2"
    ], "below the floor"
    malformed = trx(["Passed"] * 3)[: -len("</TestRun>")]
    no_counters = f'<TestRun xmlns="{NS[1:-1]}"><Results/></TestRun>'
    for text, case in [(malformed, "malformed"), ("", "empty"), (no_counters, "no counters")]:
        assert why(text, 0, **floor) == ["unreadable report"], case
        assert run(text, 0, **floor) == 1, case
    assert problems("/nonexistent/lws-net.trx", 0, **floor)[2] == ["unreadable report"], "missing"
    assert run(complete, 2, **floor) == 0 and run(complete, 1, **floor) == 1, "exit codes"
    assert run(trx(["Passed", "Failed"]), 2) == 2, "no floor: the harness's own status"
    # An unreadable report fails whatever the status, even one whose low byte is 0.
    for status in (0, 2, 256, 512, -256):
        assert run("", status) == 1 and run("", status, **floor) == 1, status
    assert run(complete, 256) == 1 and run(complete, -256) == 1, "out-of-range status"
    # The command line, as lws-net.sh runs it.
    import subprocess

    for args in (["--status", "256"], ["--status", "512", "--min-passed", "1"]):
        r = subprocess.run(
            [sys.executable, "-B", __file__, "/nonexistent/lws-net.trx", *args],
            capture_output=True,
            text=True,
        )
        assert r.returncode == 1, (args, r.returncode, r.stdout)
        assert "no readable report" in r.stdout, r.stdout
    print("lws-net-report self-test passed")


if __name__ == "__main__":
    a = argparse.ArgumentParser()
    a.add_argument("report", nargs="?")
    a.add_argument("--status", type=int, default=0)
    a.add_argument("--min-passed", type=int)
    a.add_argument("--expected-total", type=int)
    a.add_argument("--gha", action="store_true")
    a.add_argument("--self-test", action="store_true")
    o = a.parse_args()
    if o.self_test:
        self_test()
        sys.exit(0)
    sys.exit(check(o.report, o.status, o.min_passed, o.expected_total, o.gha))
