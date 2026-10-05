#!/usr/bin/env python3
# Pre-publish PyPI distribution-name availability check (bead sq-ed5).
#
# WHY THIS SCRIPT EXISTS — the executable half of docs/release.md §0a.
# The release runbook (docs/release.md §0a "Crate-name availability") records a DATED
# snapshot of every registry name we publish under and ends with the standing
# instruction: "Re-run before the first publish — registries change." For crates.io the
# re-run is `cargo publish`'s own pre-flight ("name is already taken" aborts the upload),
# but the PyPI distribution name has no equivalent local pre-flight before a maturin/twine
# upload — so that "re-run" was a MANUAL `curl pypi.org` step with nothing to actually run.
# This script makes it runnable.
#
# HISTORY (honesty): the *decision* sq-ed5 originally tracked — "is the bare `sparq` name
# free?" — was already resolved by sq-8slf: `sparq` is TAKEN by an unrelated package, so the
# wheel ships as the distribution name **`sparq-rdf`** (import name stays `sparq`). This
# script does NOT re-litigate that; it turns the recurring "re-run before publish" check into
# a one-command pre-flight for whatever distribution name `pyproject.toml` currently declares,
# so a name regression (or `sparq-rdf` itself being squatted before first publish) is caught
# BEFORE the upload rather than by a failed/clobbered publish.
#
# WHAT IT CHECKS: the PyPI JSON API at https://pypi.org/pypi/<dist>/json. Per the same
# convention §0a uses for crates.io: HTTP 404 = the project does not exist = the name is
# AVAILABLE; HTTP 200 = the project exists = the name is TAKEN. (PyPI normalises distribution
# names per PEP 503 — lowercase, runs of `-_.` collapse to a single `-` — and the JSON
# endpoint resolves the normalised form, so `Sparq_RDF` and `sparq-rdf` hit the same project.)
#
# TWO MODES:
#   * default / --self-test : HERMETIC. Exercises the pure classify_status() decision table
#     (200/404/other -> verdict) + the pyproject name reader. NO network, NO subprocess — so
#     it runs in the docs-quality `ci-scripts` job alongside the other gate self-tests and
#     can never flake on registry availability or a CI sandbox with no egress.
#   * --check : LIVE. Performs the actual PyPI lookup (stdlib urllib only) for the maintainer
#     to run by hand at release time. This mode is NOT wired into CI (it needs network); it is
#     the "re-run before the first publish" tool docs/release.md §0a points at.
#
# EXIT (live --check): 0 if the declared name is AVAILABLE (safe to publish), 1 if TAKEN,
# 2 on an indeterminate network/HTTP error (so a flaky lookup is never mistaken for "free").
# A `--expect available|taken` lets the maintainer assert the expected state and fail on drift
# (e.g. pin "we publish as sparq-rdf and expect it AVAILABLE until our own first upload").
#
# Usage:
#   check-pypi-name.py --self-test                 # hermetic logic self-test (CI)
#   check-pypi-name.py --check                      # LIVE: look up the pyproject dist name
#   check-pypi-name.py --check --name sparq         # LIVE: look up an explicit name
#   check-pypi-name.py --check --expect available   # LIVE: assert availability, fail on drift
#
# stdlib-only.

from __future__ import annotations

import argparse
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PYPROJECT = REPO_ROOT / "crates" / "sparq-py" / "pyproject.toml"

# PyPI JSON metadata endpoint. 404 => project does not exist (name free); 200 => taken.
PYPI_JSON_URL = "https://pypi.org/pypi/{name}/json"

# Verdicts returned by classify_status().
AVAILABLE = "available"  # 404: no such project — the name is free to publish
TAKEN = "taken"  # 200: the project exists — the name is in use
INDETERMINATE = "indeterminate"  # any other status / transport error — unknown


def read_dist_name(pyproject: Path = PYPROJECT) -> str:
    """Return the PyPI *distribution* name declared by `[project] name` in pyproject.toml.

    This is the `pip install <name>` / PyPI-project name (currently `sparq-rdf`), NOT the
    import/module name (`sparq`, pinned by `[tool.maturin] module-name`). Reading it from the
    manifest is what keeps this check honest: it always validates the name we would ACTUALLY
    publish under, so a future rename can never silently check the wrong string."""
    with pyproject.open("rb") as fh:
        data = tomllib.load(fh)
    try:
        name = data["project"]["name"]
    except KeyError as exc:  # pragma: no cover - manifest is malformed
        raise KeyError(
            f"no [project] name in {pyproject} (cannot determine the PyPI distribution name)"
        ) from exc
    if not isinstance(name, str) or not name.strip():
        raise ValueError(f"[project] name in {pyproject} is empty/non-string: {name!r}")
    return name.strip()


def classify_status(status: int | None) -> str:
    """Pure decision table mapping an HTTP status (or None for a transport error) to a
    verdict. 404 -> AVAILABLE (no such project), 200 -> TAKEN (project exists), anything
    else (incl. None / 5xx / 403) -> INDETERMINATE — we never claim a name is free off a
    non-404, so a registry hiccup can't green-light clobbering an existing project."""
    if status == 404:
        return AVAILABLE
    if status == 200:
        return TAKEN
    return INDETERMINATE


def _fetch_status(name: str, *, timeout: float = 10.0) -> int | None:
    """LIVE: return the HTTP status of the PyPI JSON endpoint for `name`, or None on a
    transport error. 404 surfaces as an HTTPError we read the code from (not an exception)
    — that is the SUCCESS path for "name is free", so it must not be treated as failure."""
    url = PYPI_JSON_URL.format(name=urllib.parse.quote(name, safe=""))
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 (https only)
            return resp.status
    except urllib.error.HTTPError as exc:
        # 404 (free) and 403/410/etc. all arrive here with a real status code.
        return exc.code
    except (urllib.error.URLError, TimeoutError, OSError):
        # DNS / connection / timeout — indeterminate, not "available".
        return None


def check_live(name: str, *, timeout: float = 10.0) -> str:
    """LIVE: look up `name` on PyPI and return a verdict (AVAILABLE/TAKEN/INDETERMINATE)."""
    return classify_status(_fetch_status(name, timeout=timeout))


def _verdict_line(name: str, verdict: str) -> str:
    if verdict == AVAILABLE:
        return f"PyPI name '{name}': AVAILABLE (404 — no such project; safe to publish)."
    if verdict == TAKEN:
        return f"PyPI name '{name}': TAKEN (200 — project exists)."
    return (
        f"PyPI name '{name}': INDETERMINATE (no 200/404 — network or registry error). "
        "Re-run; do NOT treat as available."
    )


def self_test() -> int:
    """Hermetic self-test of the pure decision table + the pyproject name reader. No
    network, no subprocess — mirrors scripts/coverage-gate.py / check-install-action-tool.py
    --self-test so a regression in the gate's own logic is caught in the ci-scripts job."""
    failures = 0

    def expect(label: str, got, want):
        nonlocal failures
        if got != want:
            failures += 1
            print(f"  FAIL {label}: got {got!r}, want {want!r}")
        else:
            print(f"  ok   {label}")

    # classify_status decision table.
    expect("404 -> available", classify_status(404), AVAILABLE)
    expect("200 -> taken", classify_status(200), TAKEN)
    expect("403 -> indeterminate", classify_status(403), INDETERMINATE)
    expect("500 -> indeterminate", classify_status(500), INDETERMINATE)
    expect("301 -> indeterminate", classify_status(301), INDETERMINATE)
    expect("None (transport err) -> indeterminate", classify_status(None), INDETERMINATE)

    # The distribution name actually declared in pyproject.toml. Per §0a / sq-8slf this is
    # `sparq-rdf` (the bare `sparq` is taken); pin it so a silent rename to the taken name
    # — which would reintroduce exactly the sq-ed5/sq-8slf problem — fails this self-test.
    name = read_dist_name()
    expect("pyproject [project] name == 'sparq-rdf'", name, "sparq-rdf")
    expect("declared dist name is non-empty", bool(name and name.strip()), True)

    # Verdict lines render the right phrasing per verdict.
    expect(
        "available verdict mentions safe to publish",
        "safe to publish" in _verdict_line("x", AVAILABLE),
        True,
    )
    expect("taken verdict mentions TAKEN", "TAKEN" in _verdict_line("x", TAKEN), True)
    expect(
        "indeterminate verdict warns not-available",
        "do NOT treat as available" in _verdict_line("x", INDETERMINATE),
        True,
    )

    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description=(
            "Pre-publish PyPI distribution-name availability check (sq-ed5; the runnable "
            "half of docs/release.md §0a). Default mode is the hermetic self-test; pass "
            "--check for a live PyPI lookup before a release."
        )
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="run the hermetic decision-table + pyproject-reader self-test and exit (CI).",
    )
    ap.add_argument(
        "--check",
        action="store_true",
        help="LIVE: query PyPI for the distribution name (needs network; not run in CI).",
    )
    ap.add_argument(
        "--name",
        help="distribution name to check (default: [project] name from pyproject.toml).",
    )
    ap.add_argument(
        "--expect",
        choices=[AVAILABLE, TAKEN],
        help="assert the live verdict equals this; exit non-zero on drift.",
    )
    ap.add_argument(
        "--timeout",
        type=float,
        default=10.0,
        help="live HTTP timeout in seconds (default 10).",
    )
    args = ap.parse_args(argv)

    # Default to the self-test so a bare invocation in CI is hermetic + can't flake.
    if args.self_test or not args.check:
        return self_test()

    name = args.name or read_dist_name()
    verdict = check_live(name, timeout=args.timeout)
    print(_verdict_line(name, verdict))

    if args.expect is not None:
        if verdict == INDETERMINATE:
            print(
                f"::error::could not determine PyPI status for '{name}' "
                f"(expected '{args.expect}') — re-run."
            )
            return 2
        if verdict != args.expect:
            print(f"::error::PyPI name '{name}' is {verdict}, expected {args.expect}.")
            return 1
        return 0

    # No --expect: report-only. Map verdict to a conventional exit (0 free, 1 taken, 2 unknown)
    # so a maintainer can also use it in a `&&` chain before `maturin upload`.
    return {AVAILABLE: 0, TAKEN: 1, INDETERMINATE: 2}[verdict]


if __name__ == "__main__":
    raise SystemExit(main())
