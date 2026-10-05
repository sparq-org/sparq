#!/usr/bin/env python3
# [FABLE-5] Formal-verification manifest completeness + drift gate (bead sq-63ups).
# 🤖 SPARQ agent.
#
# THE INVARIANT THIS GATE ENFORCES: a Kani proof cannot exist half-wired. Every
# `#[kani::proof]` harness in the workspace is (a) inventoried in
# ci/formal-verification.toml, (b) covered by a `proves` mapping so the
# change-coupled PR selector (scripts/fv_select.py) re-runs it when the verified
# code changes, and (c) — for `nightly = true` suites — present verbatim in the
# kani.yml matrix. Before this gate, three harnesses sat in source for weeks
# running in NO CI lane at all because kani.yml's hard-coded list silently
# drifted from the source inventory; that class of rot is now diff-visible.
#
# FOUR CHECK DIRECTIONS:
#   D1 coverage      — every workspace file containing a `#[cfg(kani)]` module or a
#                      `#[kani::proof]` harness is matched by >= 1 suite's `proves`.
#   D2 harness drift — per crate, the set of `#[kani::proof]` fn names in source
#                      EQUALS the union of that crate's manifest harness lists
#                      (both directions red: an unlisted new harness AND a stale
#                      manifest entry). A harness listed in two suites is an error.
#   D3 kani.yml drift— the kani.yml `matrix.suite` entries EQUAL the manifest's
#                      `nightly = true` suites: same names, same crate, same
#                      features, same harness sets.
#   D4 honesty       — `pr_gate = false` requires `pr_gate_blocked_by` (enforced
#                      by the shared manifest loader in scripts/fv_select.py).
#   D5 budget        — [OPUS-5] sq-ko0jt. In BOTH proof workflows (kani.yml nightly,
#                      formal-verification.yml PR-gating), the proof step's
#                      `KANI_STEP_BUDGET` must leave room under the job's
#                      `timeout-minutes` backstop, and must be at least one
#                      `KANI_HARNESS_TIMEOUT`. See WHY D5 EXISTS below.
#
# WHY D5 EXISTS: the per-harness ceiling bounds ONE proof, never a SUITE. The compare
# ORDER-law suite has 26 harnesses, so 26 × 20m = 520m of worst-case wall clock sits
# behind a 90m (nightly) / 120m (PR) job backstop — and hitting a backstop CANCELS the
# job, discarding the per-harness summary and concluding `cancelled` rather than
# `failure`. Cancelled-with-no-verdict is precisely the failure that hid the Miri lane
# for 22 consecutive nights. The proof loops therefore bound themselves against
# `KANI_STEP_BUDGET` (clamping each harness to the time remaining and reporting the rest
# as NOT RUN). That defence only holds while the budget stays BELOW the backstop, which
# is a two-file config relation no reviewer reliably re-checks — so it is pinned here.
# Raising a workflow's `timeout-minutes` without raising the budget is safe (the loop
# simply stops earlier); raising the budget to or past the backstop is not, and reds.
#
# Parsing is COMMENT-AWARE line scanning: an attribute counts only when the
# (stripped) line starts with it and is not a `//` comment — kani.yml's history
# includes `#[kani::proof]` mentions inside doc comments (sparq-vectors store.rs)
# that a naive grep double-counts. Block comments are not tracked (no harness
# attribute lives inside one; the self-test pins the comment cases we have).
#
# Usage:
#   check-fv-manifest.py                 # gate the live tree (repo root = cwd's git root)
#   check-fv-manifest.py --root DIR      # explicit root (tests)
#   check-fv-manifest.py --self-test     # hermetic should-pass + should-fail fixtures
#
# Dependencies: stdlib + PyYAML (only for parsing kani.yml; hash-pinned install in
# the workflow — .github/requirements/docs-quality.txt). Exit 0 clean / 1 offences.

from __future__ import annotations

import argparse
import importlib.util
import re
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

_FN_RE = re.compile(r"\bfn\s+(\w+)")

# D5. Minutes of a proof job that are NOT available to the proof step: checkout +
# `dtolnay/rust-toolchain` + `Swatinem/rust-cache` restore + the cold
# `cargo install --locked kani-verifier` (which COMPILES the driver) + `cargo kani
# setup` (CBMC download), plus reserve for writing the step summary. Deliberately
# generous: under-reserving reintroduces exactly the cancellation this rule prevents.
SETUP_ALLOWANCE_MIN = 15

# `timeout(1)` duration syntax, restricted to the forms these workflows use.
_DURATION_RE = re.compile(r"^(\d+)([hms]?)$")
_DURATION_SCALE = {"h": 3600, "m": 60, "s": 1, "": 1}


def _load_fv_select():
    """Import scripts/fv_select.py by path (scripts/ is not a package) so the
    manifest loader + proves-glob matcher are shared, not duplicated."""
    path = Path(__file__).resolve().parent / "fv_select.py"
    spec = importlib.util.spec_from_file_location("fv_select", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)  # type: ignore[union-attr]
    return mod


# --------------------------------------------------------------------------- #
# Source scanning (comment-aware)
# --------------------------------------------------------------------------- #
def scan_rust_file(text: str) -> tuple[bool, list[str]]:
    """-> (has_kani_cfg_module, [harness fn names]) for one .rs file."""
    lines = text.splitlines()
    has_cfg = False
    harnesses: list[str] = []
    i = 0
    n = len(lines)
    while i < n:
        s = lines[i].strip()
        i += 1
        if s.startswith("//"):
            continue  # line/doc comment — attribute mentions in prose don't count
        if s.startswith("#[cfg(kani)") or s.startswith("#[cfg(all(kani"):
            has_cfg = True
        if s.startswith("#[kani::proof]"):
            # Scan forward past interleaved attributes/comments for the fn name.
            j = i
            while j < n:
                t = lines[j].strip()
                j += 1
                if t.startswith("//") or t.startswith("#[") or not t:
                    continue
                m = _FN_RE.search(t)
                if m:
                    harnesses.append(m.group(1))
                break
    return has_cfg, harnesses


def crate_name(cargo_toml: Path) -> str | None:
    import tomllib

    try:
        with open(cargo_toml, "rb") as fh:
            data = tomllib.load(fh)
        return data.get("package", {}).get("name")
    except (OSError, tomllib.TOMLDecodeError):
        return None


def scan_workspace(root: Path) -> tuple[dict[str, list[str]], set[str], dict[str, str]]:
    """Scan crates/**/*.rs.

    -> (file -> harness names (only files with >= 1), files with a #[cfg(kani)]
    module or harness, file -> owning crate name). Paths repo-relative POSIX.
    """
    harness_files: dict[str, list[str]] = {}
    kani_files: set[str] = set()
    owners: dict[str, str] = {}
    crate_cache: dict[Path, str | None] = {}
    for rs in sorted((root / "crates").rglob("*.rs")):
        try:
            text = rs.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if "kani" not in text:
            continue  # cheap pre-filter
        has_cfg, harnesses = scan_rust_file(text)
        if not has_cfg and not harnesses:
            continue
        rel = rs.relative_to(root).as_posix()
        kani_files.add(rel)
        if harnesses:
            harness_files[rel] = harnesses
        # Nearest ancestor Cargo.toml owns the file.
        for parent in rs.parents:
            ct = parent / "Cargo.toml"
            if ct.exists():
                if ct not in crate_cache:
                    crate_cache[ct] = crate_name(ct)
                name = crate_cache[ct]
                if name:
                    owners[rel] = name
                break
    return harness_files, kani_files, owners


# --------------------------------------------------------------------------- #
# kani.yml matrix
# --------------------------------------------------------------------------- #
def load_kani_matrix(kani_yml: Path) -> list[dict]:
    import yaml  # PyYAML: hash-pinned install in the fv-manifest job

    with open(kani_yml, encoding="utf-8") as fh:
        doc = yaml.safe_load(fh)
    try:
        suites = doc["jobs"]["kani"]["strategy"]["matrix"]["suite"]
    except (KeyError, TypeError) as exc:
        raise RuntimeError(f"{kani_yml}: cannot locate jobs.kani.strategy.matrix.suite: {exc}")
    out = []
    for s in suites:
        out.append(
            {
                "name": str(s.get("name", "")),
                "crate": str(s.get("crate", "")),
                "features": str(s.get("features", "") or "").strip(),
                "harnesses": sorted(str(s.get("harnesses", "")).split()),
            }
        )
    return out


# --------------------------------------------------------------------------- #
# D5 — proof-step budget vs the job backstop (both proof workflows)
# --------------------------------------------------------------------------- #
def parse_duration_seconds(text: str) -> int:
    """`timeout(1)` duration -> seconds, for the h/m/s/bare forms the `to_seconds`
    shell helper in both proof workflows accepts. Deliberately STRICTER than that
    helper: the shell falls through to a bare-seconds `echo` for a malformed value
    (so `70min` would silently become a nonsense budget), whereas anything outside
    those forms raises here and reds the gate."""
    m = _DURATION_RE.match(str(text).strip())
    if not m:
        raise ValueError(f"{text!r} is not a timeout(1) duration (e.g. 20m, 90m, 1200s)")
    return int(m.group(1)) * _DURATION_SCALE[m.group(2)]


def load_budget(path: Path) -> dict:
    """Read the proof step's budget knobs out of a proof workflow: the workflow-level
    `env` pair plus the `kani` job's `timeout-minutes` backstop."""
    import yaml  # PyYAML: hash-pinned install in the fv-manifest job

    with open(path, encoding="utf-8") as fh:
        doc = yaml.safe_load(fh)
    env = (doc or {}).get("env") or {}
    job = ((doc or {}).get("jobs") or {}).get("kani") or {}
    return {
        "file": path.name,
        "harness_timeout": env.get("KANI_HARNESS_TIMEOUT"),
        "step_budget": env.get("KANI_STEP_BUDGET"),
        "job_timeout_minutes": job.get("timeout-minutes"),
    }


def check_budgets(budgets: list[dict]) -> list[str]:
    problems: list[str] = []
    for b in budgets:
        f = b["file"]
        for key, env_name in (("harness_timeout", "KANI_HARNESS_TIMEOUT"),
                              ("step_budget", "KANI_STEP_BUDGET")):
            if b[key] is None:
                problems.append(
                    f"D5: {f}: workflow-level env.{env_name} is missing — the proof loop "
                    f"would fall back to its hard-coded default, so the budget the job "
                    f"actually enforces would not be reviewable in the workflow"
                )
        if b["job_timeout_minutes"] is None:
            problems.append(
                f"D5: {f}: jobs.kani.timeout-minutes is missing — without a backstop there "
                f"is nothing to keep the step budget under"
            )
        if any(b[k] is None for k in ("harness_timeout", "step_budget", "job_timeout_minutes")):
            continue
        try:
            harness_s = parse_duration_seconds(b["harness_timeout"])
            budget_s = parse_duration_seconds(b["step_budget"])
        except ValueError as exc:
            problems.append(f"D5: {f}: {exc}")
            continue
        backstop_s = int(b["job_timeout_minutes"]) * 60
        allowance_s = SETUP_ALLOWANCE_MIN * 60
        if budget_s + allowance_s > backstop_s:
            problems.append(
                f"D5: {f}: KANI_STEP_BUDGET ({b['step_budget']}) + the {SETUP_ALLOWANCE_MIN}m "
                f"setup allowance exceeds jobs.kani.timeout-minutes "
                f"({b['job_timeout_minutes']}m) — the job can be CANCELLED at its backstop "
                f"mid-suite, discarding the per-harness verdict table. Lower the budget or "
                f"raise the backstop."
            )
        if harness_s > budget_s:
            problems.append(
                f"D5: {f}: KANI_HARNESS_TIMEOUT ({b['harness_timeout']}) exceeds "
                f"KANI_STEP_BUDGET ({b['step_budget']}) — even the FIRST harness would be "
                f"cut short of its own ceiling, so no suite could ever pass"
            )
    return problems


# --------------------------------------------------------------------------- #
# The checks
# --------------------------------------------------------------------------- #
def run_checks(
    suites: list[dict],
    harness_files: dict[str, list[str]],
    kani_files: set[str],
    owners: dict[str, str],
    kani_matrix: list[dict],
    path_matches,
) -> list[str]:
    problems: list[str] = []

    # D1 — every kani-bearing file is covered by >= 1 suite's proves.
    all_proves = [(s["id"], pat) for s in suites for pat in s["proves"]]
    for rel in sorted(kani_files):
        if not any(path_matches(rel, pat) for _sid, pat in all_proves):
            problems.append(
                f"D1: {rel} contains a #[cfg(kani)] module but NO manifest suite's "
                f"`proves` covers it — its proofs are invisible to the change-coupled "
                f"PR selector (add it to a [[suite]].proves in ci/formal-verification.toml)"
            )

    # D2 — per-crate harness-set equality, both directions + uniqueness.
    manifest_by_crate: dict[str, list[str]] = {}
    seen: dict[str, str] = {}
    for s in suites:
        for h in s["harnesses"]:
            if h in seen:
                problems.append(
                    f"D2: harness {h!r} listed by two suites ({seen[h]!r} and {s['id']!r})"
                )
            seen[h] = s["id"]
            manifest_by_crate.setdefault(s["crate"], []).append(h)
    source_by_crate: dict[str, list[str]] = {}
    for rel, names in harness_files.items():
        crate = owners.get(rel)
        if crate is None:
            problems.append(f"D2: {rel} has harnesses but no resolvable owning crate")
            continue
        source_by_crate.setdefault(crate, []).extend(names)
    for crate in sorted(set(manifest_by_crate) | set(source_by_crate)):
        src = set(source_by_crate.get(crate, []))
        man = set(manifest_by_crate.get(crate, []))
        for h in sorted(src - man):
            problems.append(
                f"D2: {crate}: source harness {h!r} (#[kani::proof]) is NOT in the "
                f"manifest — a proof running in no wired suite"
            )
        for h in sorted(man - src):
            problems.append(
                f"D2: {crate}: manifest harness {h!r} does not exist in source — "
                f"stale manifest entry (renamed/removed harness?)"
            )

    # D3 — kani.yml matrix == the nightly = true manifest suites.
    nightly = {s["name"]: s for s in suites if s.get("nightly") is True}
    matrix = {m["name"]: m for m in kani_matrix}
    for name in sorted(set(nightly) - set(matrix)):
        problems.append(f"D3: nightly suite {name!r} is missing from the kani.yml matrix")
    for name in sorted(set(matrix) - set(nightly)):
        problems.append(f"D3: kani.yml matrix entry {name!r} has no nightly manifest suite")
    for name in sorted(set(nightly) & set(matrix)):
        s, m = nightly[name], matrix[name]
        if s["crate"] != m["crate"]:
            problems.append(f"D3: {name!r}: crate mismatch ({s['crate']!r} vs {m['crate']!r})")
        if str(s.get("features", "")).strip() != m["features"]:
            problems.append(
                f"D3: {name!r}: features mismatch ({s.get('features', '')!r} vs {m['features']!r})"
            )
        if sorted(s["harnesses"]) != m["harnesses"]:
            problems.append(
                f"D3: {name!r}: harness list drift (manifest {sorted(s['harnesses'])} "
                f"vs kani.yml {m['harnesses']})"
            )

    return problems


def check_tree(root: Path, manifest: Path, kani_yml: Path, fv_yml: Path | None = None) -> list[str]:
    fv = _load_fv_select()
    try:
        suites = fv.load_manifest(str(manifest))  # raises on shape/D4 problems
    except Exception as exc:  # noqa: BLE001 — any manifest problem is a finding
        return [f"manifest: {exc}"]
    harness_files, kani_files, owners = scan_workspace(root)
    try:
        matrix = load_kani_matrix(kani_yml)
    except Exception as exc:  # noqa: BLE001
        return [f"kani.yml: {exc}"]
    problems = run_checks(suites, harness_files, kani_files, owners, matrix, fv.path_matches)
    # D5 — both proof workflows carry the same loop and the same budget contract.
    budget_ymls = [kani_yml] + ([fv_yml] if fv_yml is not None else [])
    try:
        problems += check_budgets([load_budget(p) for p in budget_ymls])
    except Exception as exc:  # noqa: BLE001 — an unreadable workflow is itself a finding
        problems.append(f"D5: cannot read a proof workflow's budget: {exc}")
    return problems


# --------------------------------------------------------------------------- #
# Hermetic self-test — a consistent fixture tree must pass; each drift class
# must fail. Runs before the real check in the fv-manifest job so a broken
# checker can never green the gate.
# --------------------------------------------------------------------------- #
_FIXTURE_RS = """\
//! Fixture crate. The next line is prose, not a harness: `#[kani::proof]`.
// #[cfg(kani)] in a comment must not count either.
#[cfg(kani)]
mod kani_proofs {
    use super::*;
    #[kani::proof]
    #[kani::unwind(9)]
    fn h1() {}
    #[kani::proof]
    // interleaved comment
    fn h2() {}
}
"""

_FIXTURE_MANIFEST = """\
schema = 1
[[suite]]
id = "a"
name = "crate-a (proofs)"
crate = "crate-a"
features = "--features f"
harnesses = ["h1", "h2"]
proves = ["crates/ca/src/x.rs"]
nightly = true
pr_gate = true
"""

_FIXTURE_KANI_YML = """\
env:
  KANI_HARNESS_TIMEOUT: "20m"
  KANI_STEP_BUDGET: "70m"
jobs:
  kani:
    timeout-minutes: 90
    strategy:
      matrix:
        suite:
          - name: "crate-a (proofs)"
            crate: crate-a
            features: "--features f"
            harnesses: >-
              h1
              h2
"""

# D5 reads only `env` + `jobs.kani.timeout-minutes`; the PR workflow's matrix is a
# `fromJSON(...)` expression, so it has no static suite list to drift-check.
_FIXTURE_FV_YML = """\
env:
  KANI_HARNESS_TIMEOUT: "20m"
  KANI_STEP_BUDGET: "100m"
jobs:
  kani:
    timeout-minutes: 120
"""


def self_test() -> int:
    failures: list[str] = []

    def build(td: Path, rs: str = _FIXTURE_RS, man: str = _FIXTURE_MANIFEST,
              yml: str = _FIXTURE_KANI_YML,
              fv_yml: str = _FIXTURE_FV_YML) -> tuple[Path, Path, Path, Path]:
        (td / "crates/ca/src").mkdir(parents=True, exist_ok=True)
        (td / "crates/ca/Cargo.toml").write_text('[package]\nname = "crate-a"\n')
        (td / "crates/ca/src/x.rs").write_text(rs)
        (td / "fv.toml").write_text(man)
        (td / "kani.yml").write_text(yml)
        (td / "formal-verification.yml").write_text(fv_yml)
        return td, td / "fv.toml", td / "kani.yml", td / "formal-verification.yml"

    def expect(problems: list[str], want_clean: bool, label: str, needle: str = "") -> None:
        if want_clean and problems:
            failures.append(f"{label}: expected clean, got {problems}")
        if not want_clean:
            if not problems:
                failures.append(f"{label}: expected a finding, got clean")
            elif needle and not any(needle in p for p in problems):
                failures.append(f"{label}: no finding mentions {needle!r}: {problems}")

    with tempfile.TemporaryDirectory() as s:
        td = Path(s)
        # 0. consistent tree passes (and the comment decoys are NOT counted).
        expect(check_tree(*build(td)), True, "consistent fixture")
        # 1. new source harness not in the manifest.
        expect(check_tree(*build(td, rs=_FIXTURE_RS.replace(
            "fn h2() {}", "fn h2() {}\n    #[kani::proof]\n    fn h3() {}"))),
            False, "unlisted source harness", "h3")
        # 2. stale manifest harness (removed from source).
        expect(check_tree(*build(td, rs=_FIXTURE_RS.replace(
            "    #[kani::proof]\n    // interleaved comment\n    fn h2() {}\n", ""))),
            False, "stale manifest harness", "h2")
        # 3. kani.yml harness drift.
        expect(check_tree(*build(td, yml=_FIXTURE_KANI_YML.replace("\n              h2", ""))),
               False, "kani.yml harness drift", "h2")
        # 4. kani.yml missing the nightly suite entirely.
        expect(check_tree(*build(td, yml=_FIXTURE_KANI_YML.replace(
            "crate-a (proofs)", "crate-a (renamed)"))),
            False, "kani.yml suite-name drift", "crate-a")
        # 5. an uncovered #[cfg(kani)] file (no proves entry).
        build(td)
        (td / "crates/ca/src/y.rs").write_text("#[cfg(kani)]\nmod kani_proofs {}\n")
        expect(check_tree(td, td / "fv.toml", td / "kani.yml", td / "formal-verification.yml"),
               False, "uncovered cfg(kani) file", "y.rs")
        (td / "crates/ca/src/y.rs").unlink()
        # 6. features drift between manifest and kani.yml.
        expect(check_tree(*build(td, yml=_FIXTURE_KANI_YML.replace(
            '"--features f"', '""'))),
            False, "features drift", "features")
        # 7. D5 — the step budget leaves no room under the job backstop, so the job can be
        #    CANCELLED mid-suite and the per-harness verdict table is lost.
        expect(check_tree(*build(td, yml=_FIXTURE_KANI_YML.replace(
            'KANI_STEP_BUDGET: "70m"', 'KANI_STEP_BUDGET: "85m"'))),
            False, "D5 budget overruns the nightly backstop", "D5")
        # 8. D5 — the same relation on the GATING PR workflow, which has its own backstop.
        expect(check_tree(*build(td, fv_yml=_FIXTURE_FV_YML.replace(
            'KANI_STEP_BUDGET: "100m"', 'KANI_STEP_BUDGET: "115m"'))),
            False, "D5 budget overruns the PR backstop", "formal-verification.yml")
        # 9. D5 — a budget smaller than one harness ceiling: the first harness is cut short
        #    of its own limit, so no suite could ever pass.
        expect(check_tree(*build(td, yml=_FIXTURE_KANI_YML.replace(
            'KANI_STEP_BUDGET: "70m"', 'KANI_STEP_BUDGET: "10m"'))),
            False, "D5 budget below one harness ceiling", "KANI_HARNESS_TIMEOUT")
        # 10. D5 — the budget knob deleted entirely (the loop would silently fall back to
        #     its hard-coded default, making the enforced budget unreviewable).
        expect(check_tree(*build(td, yml=_FIXTURE_KANI_YML.replace(
            '  KANI_STEP_BUDGET: "70m"\n', ''))),
            False, "D5 missing budget knob", "KANI_STEP_BUDGET")
        # 11. D5 — raising the backstop without touching the budget is SAFE and must stay
        #     clean (the loop simply stops earlier); only the inverse is a finding.
        expect(check_tree(*build(td, yml=_FIXTURE_KANI_YML.replace(
            "timeout-minutes: 90", "timeout-minutes: 150"))),
            True, "D5 raised backstop stays clean")

    if failures:
        for f in failures:
            print(f"::error::check-fv-manifest self-test FAILED: {f}")
        return 1
    print(
        "check-fv-manifest self-test: consistent fixture passes; all 6 drift classes and "
        "4 D5 budget classes red; a raised backstop stays clean."
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="FV manifest completeness + drift gate.")
    p.add_argument("--root", type=Path, default=REPO_ROOT)
    p.add_argument("--manifest", type=Path)
    p.add_argument("--kani-yml", type=Path)
    p.add_argument("--fv-yml", type=Path, help="formal-verification.yml (D5 budget check)")
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args(argv)
    if args.self_test:
        return self_test()
    root = args.root
    manifest = args.manifest or root / "ci" / "formal-verification.toml"
    kani_yml = args.kani_yml or root / ".github" / "workflows" / "kani.yml"
    fv_yml = args.fv_yml or root / ".github" / "workflows" / "formal-verification.yml"
    problems = check_tree(root, manifest, kani_yml, fv_yml)
    if problems:
        for prob in problems:
            print(f"::error::{prob}")
        print(
            f"check-fv-manifest: {len(problems)} problem(s). The manifest "
            f"(ci/formal-verification.toml), the source `#[kani::proof]` inventory and "
            f"the kani.yml matrix must move together (D1-D4), and each proof workflow's "
            f"step budget must stay under its job backstop (D5) — see the findings above."
        )
        return 1
    print(
        "check-fv-manifest: manifest, source harness inventory and kani.yml matrix agree; "
        "both proof workflows' step budgets fit under their job backstops."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
