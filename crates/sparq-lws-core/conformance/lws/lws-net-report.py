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
# dotnet test exits 0 when every test passed, 1 (VSTest) or 2 (Microsoft.Testing.Platform) when
# some failed. Anything else (3 aborted, 5 invalid arguments, 7 session failure, 8 no tests, ...)
# means the run itself went wrong.
TEST_OUTCOME_STATUSES = {0, 1, 2}


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


def check(path, status, min_passed=None, expected_total=None, gha=False):
    """The process exit code for this run, printing what it found."""
    try:
        executed, passed, failures = read(path)
    except ValueError as e:
        print(f"{'::error title=lws-net::' if gha else ''}lws-net: {e} (exit {status})")
        return status or 1
    print(
        f"{'::notice title=lws-net::' if gha else ''}lws-net {passed} of {executed} passed (exit {status})"
    )
    for line in failures:
        print(f"::error title=lws-net failed::{line}" if gha else line)
    if min_passed is None:
        return status
    problems = []
    if status not in TEST_OUTCOME_STATUSES:
        problems.append(f"the harness exited {status}, not with a test outcome")
    if expected_total is not None and executed != expected_total:
        problems.append(f"{executed} tests ran, {expected_total} expected")
    if passed < min_passed:
        problems.append(f"{passed} passed, below the floor of {min_passed}")
    for p in problems:
        print(f"{'::error title=lws-net::' if gha else ''}lws-net: {p}")
    return 1 if problems else 0


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
    assert run(trx(["Passed", "Passed", "Failed"]), 2, **floor) == 0, (
        "a failing test above the floor"
    )
    assert run(trx(["Passed", "Passed", "Failed"]), 1, **floor) == 0, "VSTest's failure status"
    assert run(trx(["Passed", "Passed"]), 3, **floor) == 1, "an aborted run"
    assert run(trx(["Passed", "Passed"]), 2, **floor) == 1, "a run missing a test"
    assert run(trx(["Passed", "Passed", "Passed"]), 7, **floor) == 1, "an infrastructure status"
    assert run(trx(["Passed", "Failed", "Failed"]), 2, **floor) == 1, "below the floor"
    malformed = trx(["Passed"] * 3)[: -len("</TestRun>")]
    assert run(malformed, 0, **floor) == 1, "a report that does not parse"
    assert run("", 0, **floor) == 1, "an empty report"
    no_counters = f'<TestRun xmlns="{NS[1:-1]}"><Results/></TestRun>'
    assert run(no_counters, 0, **floor) == 1, "a report without counters"
    assert run(trx(["Passed", "Failed"]), 2) == 2, "no floor: the harness's own status"
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
