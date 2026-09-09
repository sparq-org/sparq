#!/usr/bin/env python3
# [SONNET-4.6] sq-qcnn.31: Structural guard C1 — every feature-gated test module/
# target must have a CI executor. Design: research/test-quality-program-plan.md Wave 2
# (W2-G1). Authored by the SPARQ Sonnet agent (sq-qcnn.31).
#
# INVARIANT (fail-closed): an unmapped feature-gated test FAILS CI rather than
# silently never running. Static + deterministic: wired as a GATING step in
# .github/workflows/feature-matrix.yml (the "setup" job, free of the words
# "advisory"/"informational").
#
# BACKGROUND: ci.yml's `cargo nextest archive` carries exactly the default-OFF
# features `approx-ann`, `filtered-ann`, and `vec-predicate`. Test files behind
# `#![cfg(feature = "X")]` for any OTHER default-OFF feature compile to an EMPTY
# binary there — cargo runs it, finds 0 tests, and reports success. Such tests need
# a CI executor that passes the right `--features` set. This script makes the class
# STRUCTURAL: every gated test must map to an executor or an explicit allowlist entry;
# a new gated test with no wiring FAILS the gate immediately instead of silently never
# running.
#
# SCANNED SURFACES
#   (a) crates/*/tests/**/*.rs with any ``#![cfg(feature = "F")]`` inner attribute.
#       The feature set extracted is the union of ALL ``feature = "F"`` references in
#       any ``#![cfg(...)]`` attribute in the file (conservative: AND semantics for
#       compound cfg — see LIMITATIONS below).
#   (b) [[test]] / [[bench]] entries in crates/*/Cargo.toml with required-features.
#
# CI EXECUTORS (each may cover a gated test)
#   1. Feature-matrix leg: .github/feature-matrix.d/*.yml, assembled by
#      scripts/assemble-feature-matrix.py. A leg covers a test when the leg's crate
#      matches AND the executor's ACTIVATED feature set (default features + explicit
#      features + transitive implications from the crate's [features] table) is a
#      superset of the test's required features.
#   2. coverage.sh measure() feature-set: scripts/coverage.sh measure() case-arm
#      features for each PER_COMMIT_CRATE. Same superset check.
#   3. Explicit allowlist: scripts/check-feature-test-execution.allowlist.json.
#      Each entry covers a specific (crate, features) pair with a written reason.
#
# LIMITATIONS
#   - ``cfg(any(feature = "X", feature = "Y"))`` is treated conservatively as
#     requiring BOTH X and Y (the union of all feature names in any cfg attribute).
#     A test that only needs X OR Y might be falsely reported if neither executor
#     provides both. Such tests should be added to the allowlist with a note.
#   - Example targets (``[[example]]``) and ``bin`` / ``lib`` targets are NOT
#     scanned — only ``[[test]]``/``[[bench]]`` with required-features and tests/
#     files with inner cfg attributes. Inline ``#[cfg(feature = "F")] #[test] fn...``
#     inside ``src/`` files are not detected; use ``#![cfg(feature = "F")]`` in a
#     dedicated tests/ file or a ``required-features`` Cargo.toml entry.
#
# USAGE
#   python3 scripts/check-feature-test-execution.py --check
#       Scan the real crates/; exit 0 if all gated tests have an executor or are
#       in the allowlist; exit 1 with findings on any unmapped test.
#   python3 scripts/check-feature-test-execution.py --self-test
#       Run against the built-in negative fixture (a synthetic crate with an
#       ungated-in-CI feature-gated test); exit 0 if the detector CORRECTLY reports
#       a violation (guard is working); exit 1 if the fixture is unexpectedly clean
#       (detector is broken — this would be a CI false-green failure).
#
# DEPENDENCIES: tomllib (Python 3.11+). PyYAML required for fragment loading (same
# install as the assemble-feature-matrix.py step above in the feature-matrix setup
# job; the self-test does NOT require PyYAML).

from __future__ import annotations

import argparse
import json
import os
import re
import sys

try:
    import tomllib
except ImportError:  # pragma: no cover
    sys.stderr.write(
        "error: tomllib not available; Python 3.11+ is required\n"
    )
    sys.exit(2)

from pathlib import Path

try:
    import yaml as _yaml_mod
except ImportError:  # pragma: no cover
    _yaml_mod = None  # type: ignore[assignment]  # fallback: subprocess

REPO_ROOT = Path(__file__).resolve().parent.parent
CRATES_DIR = REPO_ROOT / "crates"
FRAGMENT_DIR = REPO_ROOT / ".github" / "feature-matrix.d"
COVERAGE_SH = REPO_ROOT / "scripts" / "coverage.sh"
ALLOWLIST_PATH = REPO_ROOT / "scripts" / "check-feature-test-execution.allowlist.json"

# Negative fixture for --self-test: a synthetic crate directory whose tests/ contains a
# feature-gated test NOT wired to any CI executor. The detector MUST report a violation
# when scanning this directory; if it does not, the guard is broken and --self-test exits 1.
FIXTURE_CRATES_DIR = (
    Path(__file__).resolve().parent
    / "tests"
    / "fixtures"
    / "check-fte"
    / "crates"
)


# ---------------------------------------------------------------------------
# Feature closure computation
# ---------------------------------------------------------------------------

def _compute_closure(enabled: frozenset[str], features_table: dict[str, list[str]]) -> frozenset[str]:
    """Transitively expand a set of enabled features using a crate's features table.

    External dep references (``dep:xxx``) and cross-crate feature references
    (containing ``/``) are filtered — only intra-crate feature names are followed.
    Returns a new frozenset containing all transitively implied features.
    """
    closure: set[str] = set(enabled)
    changed = True
    while changed:
        changed = False
        for feat in list(closure):
            for implied in features_table.get(feat, []):
                if implied.startswith("dep:") or "/" in implied:
                    continue
                if implied not in closure:
                    closure.add(implied)
                    changed = True
    return frozenset(closure)


def _load_features_table(cargo_toml: Path) -> dict[str, list[str]]:
    """Return the ``[features]`` table from a Cargo.toml, or {} on any error."""
    try:
        with open(cargo_toml, "rb") as fh:
            data = tomllib.load(fh)
        return data.get("features", {})
    except Exception:
        return {}


def executor_feature_set(
    crate: str,
    extra_features_csv: str,
    crates_dir: Path,
) -> frozenset[str]:
    """Compute the full set of activated features for one executor invocation.

    Mirrors how cargo's ``--features`` flag works: it ADDS the named features to
    the crate's DEFAULT feature set (no ``--no-default-features`` is used by any of
    our executors, so defaults are always active). Returns the transitive closure
    over ``default_features | extra_features``.
    """
    cargo_toml = crates_dir / crate / "Cargo.toml"
    table = _load_features_table(cargo_toml)
    defaults = frozenset(
        f for f in table.get("default", [])
        if not f.startswith("dep:") and "/" not in f
    )
    extra = frozenset(
        f.strip() for f in extra_features_csv.split(",") if f.strip()
    )
    return _compute_closure(defaults | extra, table)


# ---------------------------------------------------------------------------
# CI executor loading — feature-matrix fragments
# ---------------------------------------------------------------------------

def _load_legs_via_yaml(fragment_dir: Path) -> list[dict[str, object]]:
    """Load all legs from .github/feature-matrix.d/*.yml using PyYAML."""
    assert _yaml_mod is not None
    legs: list[dict[str, object]] = []
    for frag_path in sorted(fragment_dir.glob("*.yml")):
        with open(frag_path, encoding="utf-8") as fh:
            data = _yaml_mod.safe_load(fh)
        if not isinstance(data, list):
            continue
        for leg in data:
            if isinstance(leg, dict) and "crate" in leg and "features" in leg:
                legs.append(leg)
    return legs


def _load_legs_via_subprocess(fragment_dir: Path) -> list[dict[str, object]]:
    """Fallback: call assemble-feature-matrix.py as a subprocess to get the legs."""
    import subprocess  # noqa: PLC0415

    assembler = REPO_ROOT / "scripts" / "assemble-feature-matrix.py"
    result = subprocess.run(
        [sys.executable, str(assembler)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(
            "error: assemble-feature-matrix.py failed: {}\n".format(
                result.stderr[:400]
            )
        )
        sys.exit(1)
    return json.loads(result.stdout)["include"]  # type: ignore[return-value]


def load_fragment_executors(
    fragment_dir: Path,
    crates_dir: Path,
) -> dict[str, list[frozenset[str]]]:
    """Return ``{crate: [frozenset(activated_features), ...]}``, one per matrix leg."""
    if _yaml_mod is not None:
        legs = _load_legs_via_yaml(fragment_dir)
    else:
        legs = _load_legs_via_subprocess(fragment_dir)

    executors: dict[str, list[frozenset[str]]] = {}
    for leg in legs:
        crate = str(leg["crate"])
        features_csv = str(leg.get("features", ""))
        activated = executor_feature_set(crate, features_csv, crates_dir)
        executors.setdefault(crate, []).append(activated)
    return executors


# ---------------------------------------------------------------------------
# CI executor loading — coverage.sh
# ---------------------------------------------------------------------------

def _parse_coverage_case_extras(coverage_sh_path: Path) -> dict[str, frozenset[str]]:
    """Parse extra features from the measure() case block in coverage.sh.

    Uses a line-by-line FSM keyed on ``^    <crate-name>)$`` (the 4-space-indented
    case arm header pattern used throughout coverage.sh) to avoid false matches from
    comment text that also contains ``sparq-XXX)`` literals.

    Returns ``{crate: frozenset(extra_features)}`` — only the features the arm adds
    (callers combine with the crate's default features via executor_feature_set).
    """
    text = coverage_sh_path.read_text(encoding="utf-8")

    in_case = False
    current_crate: str | None = None
    current_features: set[str] = set()
    case_extras: dict[str, frozenset[str]] = {}

    for line in text.splitlines():
        stripped = line.strip()

        # Case block start/end
        if stripped == 'case "$crate" in':
            in_case = True
            continue
        if in_case and stripped == "esac":
            if current_crate is not None:
                case_extras[current_crate] = frozenset(current_features)
            in_case = False
            current_crate = None
            current_features = set()
            continue
        if not in_case:
            continue

        # Case arm header: exactly 4-space indent + crate name + ) + end-of-line.
        # This precise pattern avoids matching sparq-XXX) in comment text.
        header_m = re.match(r"^    (sparq-[a-z0-9-]+)\)$", line)
        if header_m:
            if current_crate is not None:
                case_extras[current_crate] = frozenset(current_features)
            current_crate = header_m.group(1)
            current_features = set()
            continue

        # Arm terminator: any line whose stripped form ends with ``;;``
        if stripped.endswith(";;"):
            feat_m = re.search(r"--features\s+([^\s)'\"]+)", line)
            if feat_m:
                current_features.update(
                    f.strip() for f in feat_m.group(1).split(",") if f.strip()
                )
            if current_crate is not None:
                case_extras[current_crate] = frozenset(current_features)
            current_crate = None
            current_features = set()
            continue

        # Non-terminator line within an arm: accumulate any --features flag
        if current_crate is not None:
            feat_m = re.search(r"--features\s+([^\s)'\"]+)", line)
            if feat_m:
                current_features.update(
                    f.strip() for f in feat_m.group(1).split(",") if f.strip()
                )

    return case_extras


def _parse_per_commit_crates(coverage_sh_path: Path) -> list[str]:
    """Return the PER_COMMIT_CRATES list from coverage.sh (non-comment lines only)."""
    text = coverage_sh_path.read_text(encoding="utf-8")
    m = re.search(r"PER_COMMIT_CRATES=\(", text)
    if not m:
        return []
    start = m.end()
    end_m = re.search(r"^\)", text[start:], re.MULTILINE)
    if not end_m:
        return []
    arr_text = text[start: start + end_m.start()]
    crates: list[str] = []
    for line in arr_text.splitlines():
        # Strip inline comments (shell # starts a comment)
        line = line.split("#")[0]
        crates.extend(re.findall(r"\b(sparq-[a-z0-9-]+)\b", line))
    # Preserve order; deduplicate
    seen: set[str] = set()
    return [c for c in crates if not (c in seen or seen.add(c))]  # type: ignore[func-returns-value]


def load_coverage_executors(
    coverage_sh_path: Path,
    crates_dir: Path,
) -> dict[str, list[frozenset[str]]]:
    """Return coverage.sh executors: ``{crate: [frozenset(activated_features)]}``."""
    per_commit = _parse_per_commit_crates(coverage_sh_path)
    case_extras = _parse_coverage_case_extras(coverage_sh_path)
    executors: dict[str, list[frozenset[str]]] = {}
    for crate in per_commit:
        extra_csv = ",".join(case_extras.get(crate, frozenset()))
        activated = executor_feature_set(crate, extra_csv, crates_dir)
        executors.setdefault(crate, []).append(activated)
    return executors


# ---------------------------------------------------------------------------
# Allowlist loading
# ---------------------------------------------------------------------------

def load_allowlist(allowlist_path: Path) -> list[dict[str, object]]:
    """Load the explicit allowlist.

    Returns an empty list if the file does not exist. Each entry must have
    ``crate`` (str), ``features`` (list[str]), and ``reason`` (str).
    """
    if not allowlist_path.exists():
        return []
    with open(allowlist_path, encoding="utf-8") as fh:
        data = json.load(fh)
    entries = data.get("allowlist", [])
    # Basic schema validation: each entry needs at minimum crate + features + reason
    for i, entry in enumerate(entries):
        for key in ("crate", "features", "reason"):
            if key not in entry:
                sys.stderr.write(
                    "error: allowlist entry [{}] is missing required key '{}'\n".format(
                        i, key
                    )
                )
                sys.exit(1)
    return entries


def _allowlist_covers(
    entry: dict[str, object],
    crate: str,
    required_features: frozenset[str],
) -> bool:
    """Return True if an allowlist entry covers a specific (crate, features) pair.

    Match condition: ``entry.crate == crate`` AND ``required_features == frozenset(entry.features)``
    (exact feature-set equality). This prevents a broad ``{X, Y}`` entry from silently
    covering a narrower ``{X}`` test that DOES have a CI executor.
    """
    if entry.get("crate") != crate:
        return False
    entry_feats = frozenset(str(f) for f in entry.get("features", []))
    return required_features == entry_feats


# ---------------------------------------------------------------------------
# Test surface scanning
# ---------------------------------------------------------------------------

def _extract_inner_cfg_features(content: str) -> frozenset[str]:
    """Extract feature names from ``#![cfg(...)]`` inner attributes in a file.

    Conservatively collects ALL ``feature = "F"`` references in any
    ``#![cfg(...)]`` attribute (simple, all(), any(), nested forms). Returns
    the union as a frozenset. Returns frozenset() when no feature cfgs are found.

    NOTE: ``any()`` compound forms are treated as AND (requiring all mentioned
    features) for fail-closed conservatism — see the LIMITATIONS note at the top.
    """
    features: set[str] = set()
    for m in re.finditer(
        r"#!\[cfg\([^)]*(?:\([^)]*\)[^)]*)*\)\]",
        content,
    ):
        expr = m.group(0)
        for feat_m in re.finditer(r'feature\s*=\s*"([^"]+)"', expr):
            features.add(feat_m.group(1))
    return frozenset(features)


def scan_gated_tests(
    crates_dir: Path,
) -> list[tuple[str, str, frozenset[str]]]:
    """Scan crates_dir for feature-gated tests.

    Returns ``[(crate_name, test_identifier, required_features), ...]``.
    ``test_identifier`` is one of:
      - ``"tests/<relpath>.rs"`` for a tests/ file with ``#![cfg(feature = ...)]``
      - ``"[[test]] <name>"`` or ``"[[bench]] <name>"`` for a Cargo.toml target

    Each (crate_name, test_identifier) pair appears at most once (the feature set
    is the union across all inner cfg attributes in the file).
    """
    gated: list[tuple[str, str, frozenset[str]]] = []
    seen_ids: set[tuple[str, str]] = set()

    for crate_name in sorted(os.listdir(crates_dir)):
        crate_dir = crates_dir / crate_name

        # --- (a) tests/ files with #![cfg(feature = "F")] inner attributes ---
        tests_dir = crate_dir / "tests"
        if tests_dir.is_dir():
            for rs_file in sorted(tests_dir.rglob("*.rs")):
                try:
                    content = rs_file.read_text(encoding="utf-8")
                except OSError:
                    continue
                features = _extract_inner_cfg_features(content)
                if not features:
                    continue
                rel = str(rs_file.relative_to(crate_dir / "tests"))
                test_id = "tests/{}".format(rel)
                key = (crate_name, test_id)
                if key not in seen_ids:
                    seen_ids.add(key)
                    gated.append((crate_name, test_id, features))

        # --- (b) [[test]] / [[bench]] Cargo.toml targets with required-features ---
        cargo_toml = crate_dir / "Cargo.toml"
        if cargo_toml.is_file():
            try:
                with open(cargo_toml, "rb") as fh:
                    data = tomllib.load(fh)
            except Exception:
                continue
            for section_key in ("test", "bench"):
                for target in data.get(section_key, []):
                    required = target.get("required-features", [])
                    if not required:
                        continue
                    tname = str(target.get("name", "unnamed"))
                    test_id = "[[{}]] {}".format(section_key, tname)
                    key = (crate_name, test_id)
                    if key not in seen_ids:
                        seen_ids.add(key)
                        gated.append((crate_name, test_id, frozenset(required)))

    return gated


# ---------------------------------------------------------------------------
# Coverage check
# ---------------------------------------------------------------------------

def _is_covered(
    crate: str,
    required_features: frozenset[str],
    all_executors: dict[str, list[frozenset[str]]],
) -> bool:
    """Return True if any executor for ``crate`` activates all ``required_features``."""
    for activated in all_executors.get(crate, []):
        if required_features.issubset(activated):
            return True
    return False


def check(
    crates_dir: Path,
    fragment_dir: Path,
    coverage_sh: Path,
    allowlist_path: Path,
) -> tuple[bool, list[str]]:
    """Run the C1 structural guard over ``crates_dir``.

    Returns ``(ok, findings)`` where ``ok`` is True iff there are no unmapped
    gated tests. ``findings`` lists human-readable violation messages.
    Fails closed: any I/O or parse error in the executor-loading path is
    propagated (the script exits non-zero) rather than silently skipping.
    """
    # Build merged executor map: {crate: [frozenset(activated_features), ...]}
    matrix_execs = load_fragment_executors(fragment_dir, crates_dir)
    cov_execs = load_coverage_executors(coverage_sh, crates_dir)
    allowlist = load_allowlist(allowlist_path)

    all_executors: dict[str, list[frozenset[str]]] = {}
    for src in (matrix_execs, cov_execs):
        for crate, activated_list in src.items():
            all_executors.setdefault(crate, []).extend(activated_list)

    # Scan for feature-gated tests
    gated = scan_gated_tests(crates_dir)

    findings: list[str] = []
    for crate, test_id, required_features in gated:
        if _is_covered(crate, required_features, all_executors):
            continue
        if any(_allowlist_covers(e, crate, required_features) for e in allowlist):
            continue
        findings.append(
            "UNMAPPED: {}/{} requires features {{{}}} "
            "but no CI executor covers it and it has no allowlist entry "
            "(see scripts/check-feature-test-execution.allowlist.json)".format(
                crate,
                test_id,
                ", ".join(sorted(required_features)),
            )
        )

    return (len(findings) == 0, findings)


# ---------------------------------------------------------------------------
# Self-test against the negative fixture
# ---------------------------------------------------------------------------

def run_self_test(fixture_crates_dir: Path) -> bool:
    """Verify the detector correctly identifies the negative fixture's violation.

    Returns True (self-test PASSES) when the check DOES report at least one
    finding for the fixture crate (the guard is working). Returns False when the
    fixture is unexpectedly clean (the guard is broken — a false green).

    The fixture uses an empty executor map (no matrix legs, no coverage.sh) and
    an empty allowlist so the single gated test in the fixture has no coverage.
    """
    if not fixture_crates_dir.exists():
        sys.stderr.write(
            "error: fixture directory not found: {}\n".format(fixture_crates_dir)
        )
        return False

    # Empty executor sources — the fixture has no CI executors by design
    empty_executors: dict[str, list[frozenset[str]]] = {}
    empty_allowlist: list[dict[str, object]] = []

    gated = scan_gated_tests(fixture_crates_dir)
    if not gated:
        sys.stderr.write(
            "self-test FAILED: no feature-gated tests found in fixture directory {}; "
            "the negative fixture may be missing or malformed\n".format(
                fixture_crates_dir
            )
        )
        return False

    findings: list[str] = []
    for crate, test_id, required_features in gated:
        if not _is_covered(crate, required_features, empty_executors):
            if not any(
                _allowlist_covers(e, crate, required_features) for e in empty_allowlist
            ):
                findings.append(
                    "UNMAPPED: {}/{} -> {{{}}}".format(
                        crate, test_id, ", ".join(sorted(required_features))
                    )
                )

    if findings:
        print(
            "self-test PASSED: detector correctly found {} violation(s) "
            "in the negative fixture:".format(len(findings))
        )
        for f in findings:
            print("  {}".format(f))
        return True

    sys.stderr.write(
        "self-test FAILED: detector found NO violations in the negative fixture — "
        "the guard is broken (false green). Fixture: {}\n".format(fixture_crates_dir)
    )
    return False


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description="C1 structural guard: every feature-gated test must have a CI executor."
    )
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--check",
        action="store_true",
        help="Scan crates/ and exit 0 if all gated tests are covered; exit 1 on any gap.",
    )
    group.add_argument(
        "--self-test",
        action="store_true",
        help=(
            "Run against the built-in negative fixture; exit 0 if the detector "
            "correctly reports a violation (guard is working); exit 1 otherwise."
        ),
    )
    args = parser.parse_args()

    if args.self_test:
        ok = run_self_test(FIXTURE_CRATES_DIR)
        return 0 if ok else 1

    # --check: scan the real crates/
    ok, findings = check(CRATES_DIR, FRAGMENT_DIR, COVERAGE_SH, ALLOWLIST_PATH)
    if ok:
        print(
            "check-feature-test-execution: OK — all feature-gated tests have a CI executor "
            "or an allowlist entry"
        )
        return 0

    print(
        "check-feature-test-execution: FAIL — {} feature-gated test(s) have no CI "
        "executor and no allowlist entry:".format(len(findings))
    )
    for f in findings:
        print("  {}".format(f))
    print()
    print(
        "To fix: add a feature-matrix leg (.github/feature-matrix.d/<crate>.yml) that "
        "enables the required features, OR add an entry to "
        "scripts/check-feature-test-execution.allowlist.json with a written reason."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
