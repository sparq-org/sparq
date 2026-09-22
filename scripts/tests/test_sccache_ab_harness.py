#!/usr/bin/env python3
# [OPUS-5] sq-6vshe.15 item 3 (issue #5164) — INSPECTION tests for the sccache A/B
# harness (.github/workflows/sccache-ab.yml + scripts/sccache-ab-namespace.sh +
# scripts/sccache_ab_verdict.py).
"""test_sccache_ab_harness.py — the A/B's validity conditions, pinned structurally.

WHY A TEST SUITE FOR A DISPATCH-ONLY EXPERIMENT.

Every property that makes this an A/B rather than two unrelated builds is a single
line of YAML or shell that is INVISIBLE when absent. Nothing goes red if one is
dropped: the workflow still runs, still emits two columns of plausible seconds, and
still prints a confident verdict. The failure mode is not a broken build — it is a
WRONG NUMBER written permanently into research/ci-mergequeue-speedup-2026-07.md §3.2,
in a research record whose whole purpose is to stop someone re-litigating the question
later. The design record predicts a NEGATIVE result, which means a silently broken
harness produces exactly the answer everyone already expects and nobody re-checks.

So the validity conditions are asserted here:

  1. CARGO_INCREMENTAL=0. sccache refuses to cache incremental compilation outright.
     Without this the treatment arm caches nothing, runs slightly slower than control
     (wrapper overhead), and "measures" a negative result that is really a harness bug.
  2. The experiment never writes a production cache — every rust-cache step is
     `save-if: "false"`.
  3. Exactly one writer of the sccache namespace (`prime`); the measure trials are
     READ_ONLY. This is both the sq-6vshe.5 poisoning discipline and what keeps the
     five trials independent samples instead of a warming sequence.
  4. The control arm is genuinely unwrapped — every sccache step in `measure` is
     guarded to the treatment arm.
  5. Both arms compile the SAME thing, and that thing is what ci.yml's build-archive
     job compiles. A drift in ci.yml's feature set silently turns this into a
     measurement of a different build.
  6. All three jobs are DECLARED in .github/advisory-registry.json. A dispatch on main
     lands check-runs on main's head SHA, which the push-triggered ci-summary gate
     polls; undeclared, this experiment would GATE main (fail-closed, by design).
  7. Third-party actions are SHA-pinned (repo code-scanning posture).

Plus behavioural coverage of the two scripts: the namespace script's key schema (it
must discriminate toolchain and feature-set, and FAIL CLOSED rather than emit a key
when a component is unavailable), and the verdict script's own mutation self-test.

Hermetic: stdlib only (no PyYAML, no network, no gh) so it runs anywhere — the same
rule test_mergequeue_cache_posture.py follows, and the reason both suites can run on a
box with no Python packaging at all. The job splitter is IMPORTED from
scripts/check-advisory-registry.py rather than re-implemented, so this suite and the
registry gate can never disagree about what a job is or what its name is.

Run:  python3 scripts/tests/test_sccache_ab_harness.py
"""
from __future__ import annotations

import importlib.util
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
AB_YML = WORKFLOWS / "sccache-ab.yml"
CI_YML = WORKFLOWS / "ci.yml"
DOCS_QUALITY = WORKFLOWS / "docs-quality.yml"
REGISTRY = REPO_ROOT / ".github" / "advisory-registry.json"
NAMESPACE_SH = REPO_ROOT / "scripts" / "sccache-ab-namespace.sh"
VERDICT_PY = REPO_ROOT / "scripts" / "sccache_ab_verdict.py"

SELF_PATH = "scripts/tests/test_sccache_ab_harness.py"


def _load_registry_checker():
    """Import scripts/check-advisory-registry.py (hyphenated, so not importable by name)."""
    path = REPO_ROOT / "scripts" / "check-advisory-registry.py"
    spec = importlib.util.spec_from_file_location("_advisory_registry", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_REG = _load_registry_checker()

# A step list item inside a job block: `      - ` at exactly 6 spaces.
_STEP_START = re.compile(r"^      - ")


def _is_comment(line: str) -> bool:
    return line.lstrip().startswith("#")


def ab_jobs() -> dict[str, dict]:
    """job_id -> {"name": ..., "block_text": ...} for sccache-ab.yml."""
    return {j["id"]: j for j in _REG.parse_jobs(AB_YML.read_text(encoding="utf-8"))}


def steps_of(block_text: str) -> list[list[str]]:
    """Split a job block into its `steps:` list items (each a list of raw lines)."""
    lines = block_text.split("\n")
    starts = [i for i, l in enumerate(lines) if _STEP_START.match(l)]
    out = []
    for n, start in enumerate(starts):
        end = starts[n + 1] if n + 1 < len(starts) else len(lines)
        out.append(lines[start:end])
    return out


def step_field(body: list[str], key: str) -> str | None:
    """First value of `key:` anywhere in a step body (comments skipped)."""
    pat = re.compile(rf"^\s*-?\s*{re.escape(key)}:\s*(.*?)\s*$")
    for line in body:
        if _is_comment(line):
            continue
        m = pat.match(line)
        if m:
            return m.group(1)
    return None


def unquote(value: str | None) -> str | None:
    if value is None:
        return None
    v = value.strip()
    if len(v) >= 2 and v[0] == v[-1] and v[0] in ("'", '"'):
        return v[1:-1]
    return v


class TestExperimentValidity(unittest.TestCase):
    """The conditions without which the two columns of numbers mean nothing."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.text = AB_YML.read_text(encoding="utf-8")
        cls.jobs = ab_jobs()
        cls.assertIn_ = None

    def test_all_three_jobs_are_present(self) -> None:
        self.assertEqual(sorted(self.jobs), ["measure", "prime", "verdict"])

    def test_cargo_incremental_is_zero(self) -> None:
        # Validity condition 1 — the one that fails SILENTLY and produces a confident
        # false negative. sccache classifies every incremental request as non-cacheable
        # (`not_cached: {"incremental": N}`), so the treatment arm degrades to
        # control-plus-overhead with zero hits AND zero misses.
        self.assertRegex(
            self.text,
            r'(?m)^  CARGO_INCREMENTAL: "0"$',
            "sccache-ab.yml must set CARGO_INCREMENTAL: '0' at workflow scope — sccache "
            "cannot cache incremental compilation, so without it the treatment arm "
            "caches NOTHING and the experiment reports a negative result that is really "
            "a harness bug.",
        )

    def test_the_experiment_never_saves_a_production_cache(self) -> None:
        # Validity condition 2. This workflow restores the SAME `test-workspace`
        # rust-cache key production uses; a save from here would rewrite the cache
        # under measurement and perturb every real CI job that restores it after.
        found = 0
        for job_id, job in self.jobs.items():
            for body in steps_of(job["block_text"]):
                if "Swatinem/rust-cache@" not in "\n".join(body):
                    continue
                found += 1
                self.assertEqual(
                    unquote(step_field(body, "save-if")),
                    "false",
                    f"job {job_id}: a rust-cache step in the A/B harness must set "
                    "save-if: 'false' — an experiment that writes the cache it is "
                    "measuring is measuring itself, and poisons every real CI job "
                    "that restores the same key afterwards.",
                )
        self.assertGreaterEqual(
            found, 2, "expected a rust-cache step in both prime and measure"
        )

    def test_prime_is_the_only_writer_and_trials_are_read_only(self) -> None:
        # Validity condition 3. If the trials could write, trial 1 would warm trial 2
        # and the five samples would be a warming sequence, biasing the median down.
        self.assertIn(
            "SCCACHE_GHA_RW_MODE=READ_ONLY",
            self.jobs["measure"]["block_text"],
            "the measure trials must run sccache READ_ONLY: it is the sq-6vshe.5 "
            "poisoning discipline AND what keeps the trials independent samples.",
        )
        self.assertNotIn(
            "READ_ONLY",
            self.jobs["prime"]["block_text"],
            "the prime job is the designated WRITER; making it read-only leaves the "
            "namespace empty and every treatment trial becomes a 100%-miss run.",
        )
        self.assertRegex(
            self.jobs["measure"]["block_text"],
            r"(?m)^    needs: prime$",
            "measure must need prime, or the trials race the namespace they read.",
        )

    def test_every_sccache_step_in_measure_is_treatment_only(self) -> None:
        # Validity condition 4 — the control arm must be the production configuration,
        # bit for bit. An unguarded sccache step would wrap BOTH arms and the A/B would
        # compare a thing to itself.
        guarded = 0
        for body in steps_of(self.jobs["measure"]["block_text"]):
            blob = "\n".join(l for l in body if not _is_comment(l))
            if not re.search(r"sccache-action|SCCACHE_PATH|SCCACHE_GHA", blob):
                continue
            # The result-recording step reads RUSTC_WRAPPER to DETECT contamination, so
            # it must run in BOTH arms; it is identified and exempted by name.
            if "Record the trial result" in (step_field(body, "name") or ""):
                continue
            guarded += 1
            self.assertEqual(
                step_field(body, "if"),
                "matrix.arm == 'treatment'",
                f"step {step_field(body, 'name')!r} touches sccache but is not guarded "
                "to the treatment arm — it would wrap the control build too, and the "
                "A/B would compare a thing to itself.",
            )
        self.assertGreaterEqual(
            guarded, 3, "expected the install, enable, zero-stats and stats steps to be guarded"
        )

    def test_both_arms_share_one_build_invocation(self) -> None:
        # Validity condition 5a. Two separately-written commands drift; one shared
        # command cannot.
        invocations = [
            body
            for body in steps_of(self.jobs["measure"]["block_text"])
            if any("cargo nextest archive" in l for l in body if not _is_comment(l))
        ]
        self.assertEqual(
            len(invocations),
            1,
            "the measure job must contain exactly ONE `cargo nextest archive` "
            "invocation shared by both arms; per-arm copies drift apart silently.",
        )
        self.assertIsNone(
            step_field(invocations[0], "if"),
            "the measured build step must not be arm-conditional — it is the one thing "
            "the two arms must have in common.",
        )

    def test_feature_set_matches_the_ci_job_under_test(self) -> None:
        # Validity condition 5b — a DRIFT GUARD. ci.yml's build-archive features are
        # load-bearing (#363) and are the thing this experiment claims to model. If
        # ci.yml's set changes and this file's does not, the harness keeps running and
        # silently measures a build that no longer exists in production.
        m_ab = re.search(r"(?m)^  ARCHIVE_FEATURES: (\S+)$", self.text)
        self.assertIsNotNone(m_ab, "sccache-ab.yml must declare ARCHIVE_FEATURES")
        m_ci = re.search(
            r"cargo nextest archive --workspace --all-targets --features (\S+)",
            CI_YML.read_text(encoding="utf-8"),
        )
        self.assertIsNotNone(
            m_ci,
            "could not find ci.yml's `cargo nextest archive` invocation — if that job "
            "was renamed or restructured, re-point this drift guard.",
        )
        self.assertEqual(
            m_ab.group(1),
            m_ci.group(1),
            "sccache-ab.yml's ARCHIVE_FEATURES has drifted from ci.yml's build-archive "
            "feature set; the experiment would measure a different build than the one "
            "it claims to be about.",
        )

    def test_the_measured_step_times_only_the_build(self) -> None:
        # Setup (installing sccache, restoring rust-cache) is NOT part of the compile
        # cost under test; if the timer spanned it, the treatment arm would be charged
        # for its own installer and the comparison would be unfair the other way.
        body = next(
            b
            for b in steps_of(self.jobs["measure"]["block_text"])
            if any("cargo nextest archive" in l for l in b if not _is_comment(l))
        )
        blob = "\n".join(body)
        self.assertIn("date +%s%N", blob, "the build step must bracket itself with a timer")
        self.assertNotIn("sccache-action", blob)
        self.assertNotIn("rustup", blob)


class TestNonGatingIsDeclaredNotAssumed(unittest.TestCase):
    """A dispatch on main puts these check-runs on the SHA the gate polls (sq-huwr8)."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.text = AB_YML.read_text(encoding="utf-8")
        cls.jobs = ab_jobs()
        cls.registry = json.loads(REGISTRY.read_text(encoding="utf-8"))["jobs"]

    def test_workflow_is_dispatch_only(self) -> None:
        lines = self.text.split("\n")
        start = next(i for i, l in enumerate(lines) if re.match(r'^(on|"on"):', l))
        end = len(lines)
        for i in range(start + 1, len(lines)):
            if lines[i] and not lines[i][0].isspace() and not _is_comment(lines[i]):
                end = i
                break
        triggers = [
            m.group(1)
            for l in lines[start + 1:end]
            if not _is_comment(l)
            for m in [re.match(r"^  ([a-z_]+):", l)]
            if m
        ]
        self.assertEqual(
            triggers,
            ["workflow_dispatch"],
            "the A/B is an 11-build experiment; it must never run on push, "
            f"pull_request, merge_group or a schedule. Found: {triggers}",
        )

    def test_every_job_is_declared_advisory(self) -> None:
        required = ("owner_bead", "promotion_criteria", "registered", "workflow", "job_id")
        for job_id, job in self.jobs.items():
            name = job["name"]
            self.assertIn(
                name,
                self.registry,
                f"job {job_id!r} (name {name!r}) is not declared in "
                ".github/advisory-registry.json. Undeclared check-runs GATE — a "
                "dispatch on main would block every push-to-main on an experiment.",
            )
            entry = self.registry[name]
            for field in required:
                self.assertIn(field, entry, f"registry entry {name!r} lacks {field!r}")
            self.assertEqual(entry["workflow"], "sccache-ab.yml")
            self.assertEqual(entry["job_id"], job_id)

    def test_registry_keys_carry_a_literal_anchor(self) -> None:
        # ci_summary_gate.py REFUSES an expression-only key: it compiles to `.+` and
        # would neutralise every check-run on the commit, `gate` included (#3774 2(b)).
        for job in self.jobs.values():
            self.assertTrue(
                _REG._has_literal_anchor(job["name"]),
                f"job name {job['name']!r} has no literal text outside its "
                "expressions; as a registry key it would match every check-run.",
            )

    def test_third_party_actions_are_sha_pinned(self) -> None:
        # The trailing `# vX.Y.Z` version comment is REQUIRED by the pinning
        # convention, so it must be tolerated here — an anchored `\s*$` matches
        # nothing and this test passes vacuously over an empty list, which is what
        # the count sentinel below exists to catch.
        uses = re.findall(r"(?m)^\s*(?:- )?uses:\s*(\S+)\s*(?:#.*)?$", self.text)
        self.assertGreaterEqual(len(uses), 5, "expected several actions in this workflow")
        for ref in uses:
            self.assertRegex(
                ref,
                r"^[^@]+@[0-9a-f]{40}$",
                f"`uses: {ref}` is not pinned to a full commit SHA "
                "(repo code-scanning / Scorecard posture).",
            )


class TestNamespaceKeySchema(unittest.TestCase):
    """sq-6vshe.5: no key may alias across toolchain / Cargo.lock / feature-set."""

    RUSTC_A = 'echo "rustc 1.90.0 (aaaaaaa 2026-01-01)"; echo "host: x86_64-unknown-linux-gnu"'
    RUSTC_B = 'echo "rustc 1.91.0 (bbbbbbb 2026-02-01)"; echo "host: x86_64-unknown-linux-gnu"'

    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.tmp = Path(self._td.name)
        (self.tmp / "Cargo.lock").write_text("# lock v1\n", encoding="utf-8")
        (self.tmp / "bin").mkdir()

    def tearDown(self) -> None:
        self._td.cleanup()

    def _run(self, features: str | None, rustc_body: str | None,
             github_env: Path | None = None) -> subprocess.CompletedProcess:
        rustc = self.tmp / "bin" / "rustc"
        # rustc_body None => a toolchain that FAILS: exit 1, empty stdout. That is the
        # shape observed on the authoring box, and the shape the script must refuse.
        rustc.write_text(
            "#!/bin/sh\nexit 1\n" if rustc_body is None else f"#!/bin/sh\n{rustc_body}\n",
            encoding="utf-8",
        )
        rustc.chmod(rustc.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        env = dict(os.environ)
        env["PATH"] = f"{self.tmp / 'bin'}:{env['PATH']}"
        env.pop("GITHUB_ENV", None)
        env.pop("ARCHIVE_FEATURES", None)
        if features is not None:
            env["ARCHIVE_FEATURES"] = features
        if github_env is not None:
            env["GITHUB_ENV"] = str(github_env)
        return subprocess.run(
            ["bash", str(NAMESPACE_SH)], cwd=self.tmp, env=env,
            capture_output=True, text=True,
        )

    def test_deterministic(self) -> None:
        a = self._run("f1,f2", self.RUSTC_A)
        b = self._run("f1,f2", self.RUSTC_A)
        self.assertEqual(a.returncode, 0, a.stderr)
        self.assertEqual(a.stdout.strip(), b.stdout.strip())
        self.assertRegex(a.stdout.strip(), r"^sccache-ab-[0-9a-f]{16}$")

    def test_toolchain_changes_the_namespace(self) -> None:
        a = self._run("f1,f2", self.RUSTC_A)
        b = self._run("f1,f2", self.RUSTC_B)
        self.assertEqual(b.returncode, 0, b.stderr)
        self.assertNotEqual(
            a.stdout.strip(),
            b.stdout.strip(),
            "two rustc versions produced the same namespace — cross-rustc aliasing is "
            "exactly what sq-6vshe.5 forbids ('cross-rustc hits are how they rot').",
        )

    def test_feature_set_changes_the_namespace(self) -> None:
        a = self._run("f1,f2", self.RUSTC_A)
        b = self._run("f1,f3", self.RUSTC_A)
        self.assertNotEqual(
            a.stdout.strip(), b.stdout.strip(),
            "cross-family hits are how caches thrash at the 10 GB GHA cap (sq-6vshe.5).",
        )

    def test_lockfile_changes_the_namespace(self) -> None:
        a = self._run("f1,f2", self.RUSTC_A)
        (self.tmp / "Cargo.lock").write_text("# lock v2\n", encoding="utf-8")
        b = self._run("f1,f2", self.RUSTC_A)
        self.assertNotEqual(a.stdout.strip(), b.stdout.strip())

    def test_fails_closed_when_rustc_is_unavailable(self) -> None:
        # THE REGRESSION THIS SUITE EXISTS FOR. The first version of the script piped
        # `rustc -vV` straight into the digest; `set -euo pipefail` does NOT abort a
        # failing command inside a `{ ... } | sha256sum` command substitution, so a
        # broken toolchain contributed an EMPTY string and the script still printed a
        # confident namespace — one that aliased across every rustc version. Caught by
        # running the script on a box where `rustc -vV` exits 1.
        res = self._run("f1,f2", None)
        self.assertEqual(
            res.returncode, 1,
            "a failing `rustc -vV` must be FATAL; emitting a namespace without the "
            f"toolchain component aliases across toolchains. stdout={res.stdout!r}",
        )
        self.assertNotIn("sccache-ab-", res.stdout)
        self.assertIn("alias across toolchains", res.stderr)

    def test_fails_closed_without_a_feature_set(self) -> None:
        res = self._run(None, self.RUSTC_A)
        self.assertEqual(res.returncode, 1)
        self.assertNotIn("sccache-ab-", res.stdout)
        self.assertIn("alias across feature sets", res.stderr)

    def test_fails_closed_without_a_lockfile(self) -> None:
        (self.tmp / "Cargo.lock").unlink()
        res = self._run("f1,f2", self.RUSTC_A)
        self.assertEqual(res.returncode, 1)
        self.assertNotIn("sccache-ab-", res.stdout)
        self.assertIn("alias across dependency graphs", res.stderr)

    def test_exports_to_github_env(self) -> None:
        gh_env = self.tmp / "gh_env"
        res = self._run("f1,f2", self.RUSTC_A, github_env=gh_env)
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertRegex(
            gh_env.read_text(encoding="utf-8").strip(),
            r"^SCCACHE_GHA_VERSION=sccache-ab-[0-9a-f]{16}$",
        )

    def test_the_workflow_actually_calls_this_script(self) -> None:
        # A key schema nothing invokes is a comment.
        jobs = ab_jobs()
        for job_id in ("prime", "measure"):
            self.assertIn(
                "scripts/sccache-ab-namespace.sh",
                jobs[job_id]["block_text"],
                f"job {job_id} must compute the sccache key namespace, or it writes "
                "and reads an unnamespaced key that aliases across toolchains.",
            )


class TestVerdictScript(unittest.TestCase):
    def test_self_test_passes(self) -> None:
        # The verdict script carries its own mutation tripwires (a disabled
        # zero-hits/contamination/bar check makes one of these cases go red).
        res = subprocess.run(
            [sys.executable, str(VERDICT_PY), "--self-test"],
            capture_output=True, text=True,
        )
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)

    def test_workflow_invokes_the_verdict_script(self) -> None:
        self.assertIn("scripts/sccache_ab_verdict.py", AB_YML.read_text(encoding="utf-8"))

    def test_verdict_job_runs_even_when_trials_die(self) -> None:
        # A skipped verdict job is how a broken experiment gets quietly re-read later
        # as "we measured it and it did not help".
        self.assertRegex(
            ab_jobs()["verdict"]["block_text"],
            r"(?m)^    if: always\(\)$",
            "the verdict job must run with if: always() so a partial run is reported "
            "as INCONCLUSIVE out loud rather than silently skipped.",
        )


class TestSuiteIsWiredIntoCi(unittest.TestCase):
    """A structural test that never runs is a comment. Pin its own call site."""

    def test_docs_quality_invokes_this_suite(self) -> None:
        self.assertIn(
            SELF_PATH,
            DOCS_QUALITY.read_text(encoding="utf-8"),
            "this suite must be invoked by docs-quality.yml or it silently stops gating.",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
