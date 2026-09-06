#!/usr/bin/env python3
# [OPUS-4.8] Hermetic tests for the dynamic feature-matrix assembler (bead sq-ibrze).
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# GATE-CRITICAL invariant under test: the per-crate fragment files in
# `.github/feature-matrix.d/*.yml`, assembled by scripts/assemble-feature-matrix.py,
# emit the EXACT set of `opt-in <name>` check-run names that the single required
# `ci-summary / gate` aggregator + branch protection discover as REQUIRED. If the
# split into fragments ever renamed/dropped/added a gating leg, branch protection
# would stop recognising a required check (letting an unverified PR merge, or blocking
# the repo on an "expected but missing" check). This test locks that set against the
# byte-for-byte pre-split golden snapshot.
#
# Hermetic: imports the assembler module + reads the committed fragments + golden file
# on disk (committed files, not git/network state), so the run is deterministic. NO
# network; the only subprocesses are local runs of the workflow's failure-attribution
# script (TestAssembleFailureAnnotation, sq-2wo5t) and of the group runner/reporter
# with stubbed cargo/gh binaries (TestGroupRunner/TestGroupReporter) — still offline.
#
# Run:  python3 scripts/tests/test_feature_matrix_assemble.py
# (stdlib + the same PyYAML the assembler needs; no pytest required — also
#  discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import io
import json
import re
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
ASSEMBLER = REPO_ROOT / "scripts" / "assemble-feature-matrix.py"
GOLDEN = Path(__file__).resolve().parent / "feature-matrix-legnames.golden.txt"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "feature-matrix.yml"
# [FABLE-5] PR #3511 review finding 1: the trusted reporter now lives in its own
# default-branch-owned workflow_run workflow (never PR-head-controlled code).
REPORT_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "feature-matrix-report.yml"
FRAGMENT_DIR = REPO_ROOT / ".github" / "feature-matrix.d"

# The exact exclusion rule ci-summary.yml applies (\b(advisory|informational)\b,
# case-insensitive) — a leg NAME matching this would silently STOP gating.
ADVISORY_RE = re.compile(r"\b(advisory|informational)\b", re.IGNORECASE)


def _load_assembler():
    spec = importlib.util.spec_from_file_location("assemble_feature_matrix", ASSEMBLER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _golden_names():
    names = []
    for line in GOLDEN.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        names.append(s)
    return names


class TestFeatureMatrixAssemble(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.mod = _load_assembler()
        cls.legs = cls.mod.load_legs()
        cls.names = sorted(f"opt-in {leg['name']}" for leg in cls.legs)
        cls.golden = _golden_names()

    # ---- THE gate-critical proof -------------------------------------------------
    def test_leg_names_match_golden_exactly(self):
        """Assembled `opt-in <name>` set == the pre-split golden set, byte-for-byte."""
        # `assertEqual` on sorted lists gives a precise added/removed diff on failure.
        self.assertEqual(
            self.names,
            sorted(self.golden),
            "feature-matrix leg-name set drifted from the gate-critical golden. If you "
            "added a NEW opt-in leg, update scripts/tests/feature-matrix-legnames.golden.txt "
            "DELIBERATELY (and keep the new name free of advisory/informational). If you "
            "did NOT intend to change the gating set, this is a real regression.",
        )

    def test_golden_is_exact_names_output(self):
        """The golden is REGENERATED, never hand-edited: it == `--names` stdout, byte-for-byte.

        [SONNET-4.6] issue #2384. The sibling test above compares the SORTED SETS, so a
        hand-written golden line still passes it while sitting in the wrong sort position
        (or carrying a comment, or a stray blank line). This pins the documented
        regeneration command instead — `python3 scripts/assemble-feature-matrix.py --names
        > scripts/tests/feature-matrix-legnames.golden.txt` — so the only way to update the
        golden is to run it. That matters because the decomposition rule says a leg-NAME-SET
        change scopes BOTH the fragment and this golden, with the golden regenerated.
        """
        buf = io.StringIO()
        argv = sys.argv
        sys.argv = ["assemble-feature-matrix.py", "--names"]
        try:
            with redirect_stdout(buf):
                self.mod.main()
        finally:
            sys.argv = argv
        self.assertEqual(
            buf.getvalue(),
            GOLDEN.read_text(encoding="utf-8"),
            "scripts/tests/feature-matrix-legnames.golden.txt is not the verbatim output "
            "of `python3 scripts/assemble-feature-matrix.py --names`. Do not hand-edit it "
            "— regenerate: python3 scripts/assemble-feature-matrix.py --names > "
            "scripts/tests/feature-matrix-legnames.golden.txt",
        )

    def test_no_duplicate_leg_names(self):
        names = [leg["name"] for leg in self.legs]
        dupes = sorted({n for n in names if names.count(n) > 1})
        self.assertEqual(dupes, [], f"duplicate leg names collapse gating checks: {dupes}")

    def test_no_leg_name_is_advisory_or_informational(self):
        """Every leg must GATE — none may match ci-summary's advisory exclusion."""
        offenders = [n for n in self.names if ADVISORY_RE.search(n)]
        self.assertEqual(
            offenders,
            [],
            "these leg names contain the whole word advisory/informational and would "
            f"silently STOP gating in ci-summary: {offenders}",
        )

    # ---- assembler output contract ----------------------------------------------
    def test_assembled_json_is_fromjson_ready(self):
        """The default output is a single JSON object {"include": [legs]}."""
        import io
        from contextlib import redirect_stdout

        buf = io.StringIO()
        argv = sys.argv
        sys.argv = ["assemble-feature-matrix.py"]
        try:
            with redirect_stdout(buf):
                self.mod.main()
        finally:
            sys.argv = argv
        obj = json.loads(buf.getvalue())
        self.assertIn("include", obj)
        self.assertIsInstance(obj["include"], list)
        self.assertEqual(len(obj["include"]), len(self.legs))
        for leg in obj["include"]:
            self.assertEqual(set(leg.keys()), {"name", "crate", "features", "test"})
            self.assertTrue(leg["features"], "features must be non-empty (cargo rejects bare --features)")
            self.assertIsInstance(leg["test"], bool)

    def test_names_mode_matches_default_mode(self):
        import io
        from contextlib import redirect_stdout

        buf = io.StringIO()
        argv = sys.argv
        sys.argv = ["assemble-feature-matrix.py", "--names"]
        try:
            with redirect_stdout(buf):
                self.mod.main()
        finally:
            sys.argv = argv
        printed = [ln for ln in buf.getvalue().splitlines() if ln.strip()]
        self.assertEqual(printed, self.names)

    # ---- workflow wiring guard (prevents reintroducing the static list) ----------
    def test_workflow_uses_dynamic_fromjson_matrix(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(
            "matrix: ${{ fromJSON(needs.setup.outputs.matrix) }}",
            text,
            "opt-in-features must consume the assembled matrix via fromJSON",
        )
        # The static `include:` list must NOT be reintroduced into the workflow.
        self.assertNotRegex(
            text,
            r"\n\s+include:\s*\n\s+- name:",
            "a static `include:` leg list reappeared in feature-matrix.yml — add legs to "
            "the per-crate fragments under .github/feature-matrix.d/ instead",
        )

    def test_every_fragment_is_a_clean_leg_list(self):
        import yaml  # noqa: PLC0415  (lazy: only when the test runs)

        self.assertTrue(FRAGMENT_DIR.is_dir(), f"missing {FRAGMENT_DIR}")
        frags = sorted(FRAGMENT_DIR.glob("*.yml"))
        self.assertTrue(frags, "no fragment files found")
        for frag in frags:
            data = yaml.safe_load(frag.read_text(encoding="utf-8"))
            self.assertIsInstance(
                data, list, f"{frag.name}: top level must be a YAML list of legs"
            )


# [FABLE-5] sq-fmx4u.3: change-based selection over the assembled leg list
# (filter_legs_by_selection). The load-bearing property is FAIL-CLOSED: the ONLY
# input shape that may drop a leg is the exact mode "selected" with a well-formed
# affected list; every other input (shadow, full, unset, malformed JSON, wrong
# type) yields the FULL leg set — running more is always sound (design §2/§4.3).
class TestSelectionFiltering(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.mod = _load_assembler()
        cls.legs = cls.mod.load_legs()

    def _filter(self, mode, affected):
        return self.mod.filter_legs_by_selection(self.legs, mode, affected)

    def test_selected_keeps_only_affected_crates(self):
        crates = sorted({leg["crate"] for leg in self.legs})
        keep = crates[0]
        got = self._filter("selected", json.dumps([keep]))
        self.assertTrue(got, "expected at least one leg for a real crate")
        self.assertEqual({leg["crate"] for leg in got}, {keep})
        # And every one of that crate's legs survives — filtering is per-crate,
        # never per-leg-subset.
        self.assertEqual(len(got), sum(1 for leg in self.legs if leg["crate"] == keep))

    def test_selected_with_empty_affected_yields_zero_legs(self):
        # Legitimate: a provably-empty closure => no legs; the workflow's `legs`
        # count output then SKIPS the matrix job (never an empty-matrix error).
        # [OPUS-4.8] This IS the orchestration-only / docs-only change-class outcome:
        # ci_select.py returns mode=selected + affected=[] for a diff that touches no
        # Rust (e.g. routing.toml + triage.py), so the whole opt-in feature matrix is
        # correctly assembled to ZERO legs (skipped-by-class) without any missing
        # required check — the gate discovers the reduced set by polling.
        self.assertEqual(self._filter("selected", "[]"), [])

    def test_change_class_adds_no_legs_full_golden_set_preserved(self):
        # [OPUS-4.8] The change-class layer is a SELECTION refinement (ci_select.py),
        # not a fragment change: the assembler's FULL leg set (the gate-name golden
        # contract) must be byte-identical to the committed golden snapshot. If the
        # change-class work ever accidentally added/renamed a leg, this fails.
        import io
        from contextlib import redirect_stdout
        buf = io.StringIO()
        argv = sys.argv
        try:
            sys.argv = ["assemble", "--names"]
            with redirect_stdout(buf):
                self.mod.main()
        finally:
            sys.argv = argv
        emitted = [ln for ln in buf.getvalue().splitlines() if ln.strip()]
        golden = [ln for ln in GOLDEN.read_text(encoding="utf-8").splitlines() if ln.strip()]
        self.assertEqual(sorted(emitted), sorted(golden))

    def test_shadow_full_and_unset_modes_keep_all(self):
        for mode in ("shadow", "full", "", None, "SELECTED", "enforce"):
            self.assertEqual(len(self._filter(mode, "[]")), len(self.legs),
                             f"mode={mode!r} must fail-close to the FULL leg set")

    def test_malformed_affected_fails_closed_to_full(self):
        for bad in (None, "", "not json", '{"a": 1}', '"str"', '[1, 2]'):
            self.assertEqual(len(self._filter("selected", bad)), len(self.legs),
                             f"affected={bad!r} must fail-close to the FULL leg set")

    def test_names_mode_ignores_selection_flags(self):
        # The golden gate-name proof must always dump the FULL set.
        import io
        from contextlib import redirect_stdout

        buf = io.StringIO()
        argv = sys.argv
        sys.argv = ["assemble-feature-matrix.py", "--names",
                    "--select-mode", "selected", "--affected", "[]"]
        try:
            with redirect_stdout(buf):
                self.mod.main()
        finally:
            sys.argv = argv
        printed = [ln for ln in buf.getvalue().splitlines() if ln.strip()]
        self.assertEqual(printed, sorted(f"opt-in {leg['name']}" for leg in self.legs))

    def test_main_applies_selection_to_matrix_json(self):
        import io
        from contextlib import redirect_stdout

        crates = sorted({leg["crate"] for leg in self.legs})
        keep = crates[-1]
        buf = io.StringIO()
        argv = sys.argv
        sys.argv = ["assemble-feature-matrix.py",
                    "--select-mode", "selected", "--affected", json.dumps([keep])]
        try:
            with redirect_stdout(buf):
                self.mod.main()
        finally:
            sys.argv = argv
        obj = json.loads(buf.getvalue())
        self.assertEqual({leg["crate"] for leg in obj["include"]}, {keep})


# [SONNET-4.6] sq-ldg8c: feature-matrix TIER + EVENT awareness.
#
# The load-bearing property is BEHAVIOUR-PRESERVATION at ZERO annotations: with no
# `tier:` field on any fragment (the state at merge), every event emits exactly today's
# full leg set and the check tier is empty. The tier machinery only takes effect once a
# fragment opts in (the separate flip bead sq-s5dvo). Malformed/unknown tier => HARD ERROR
# (never a silent demotion). Design: research/feature-matrix-pyramid.md §3/§5.


def _run_main_json(mod, extra_argv):
    """Run the assembler's main() with argv and return the parsed JSON object."""
    buf = io.StringIO()
    saved = sys.argv
    sys.argv = ["assemble-feature-matrix.py"] + list(extra_argv)
    try:
        with redirect_stdout(buf):
            mod.main()
    finally:
        sys.argv = saved
    return json.loads(buf.getvalue())


def _synth_legs():
    """Synthetic legs exercising both tiers across the engine/rest shard split.
    (The real fragments carry NO annotations, so the demotion path can only be
    exercised with synthetic input — the behaviour-preservation tests below cover
    the real, zero-annotation fragment set.)"""
    return [
        {"name": "a", "crate": "sparq-core", "features": "f1", "test": True, "tier": "test"},
        {"name": "b", "crate": "sparq-engine", "features": "f2", "test": True, "tier": "check"},
        {"name": "c", "crate": "sparq-kb", "features": "f3", "test": False, "tier": "check"},
        {"name": "d", "crate": "sparq-engine", "features": "f4", "test": True, "tier": "test"},
    ]


class TestMergeGroupClassAccounting(unittest.TestCase):
    """[FABLE-5] merge-group change-class (extends #3420/#3421): the expected
    per-class opt-in leg set for a MERGE-GROUP event, via the same composed
    tier+selection path the workflow's assemble step runs. A docs-only/
    orchestration-only queued batch (select emits mode=selected + affected=[]
    over the batch diff) assembles ZERO legs — the `legs` output then skips the
    matrix job, an attributed skip, never a missing required check. An engine
    batch keeps its affected legs; an unresolvable batch (select fail-safe to
    mode=full) keeps the FULL merge-group leg set."""

    @classmethod
    def setUpClass(cls):
        cls.mod = _load_assembler()
        cls.legs = cls.mod.load_legs()

    def _assemble(self, select_mode, affected):
        tiered = self.mod.filter_legs_by_tier(self.legs, "merge_group", "test")
        return self.mod.filter_legs_by_selection(tiered, select_mode, affected)

    def test_docs_only_batch_assembles_zero_legs(self):
        # The #2533-shaped docs-only batch: classify_change => docs-only, the
        # select pre-job => selected + empty closure => zero opt-in legs on the
        # merge_group ref. (Identical outcome for orchestration-only.)
        self.assertEqual(self._assemble("selected", "[]"), [])

    def test_engine_batch_keeps_its_affected_legs(self):
        # An engine batch narrows to the affected closure exactly as on a PR —
        # class gating never removes a leg the selection would keep.
        crates = sorted({leg["crate"] for leg in self.legs})
        keep = crates[0]
        got = self._assemble("selected", json.dumps([keep]))
        self.assertTrue(got, "an engine batch must keep its affected legs")
        self.assertEqual({leg["crate"] for leg in got}, {keep})

    def test_unresolvable_batch_fails_safe_to_full_merge_group_set(self):
        # Any classify/select resolution error => mode=full => the FULL
        # merge-group leg set (fail-safe = cost, never soundness). A classifier
        # mutated back to always-full lands every batch here — visible as the
        # RED docs-only fixtures in test_ci_select.py, and as this full set
        # re-appearing on docs-only groups in the live audit trail.
        full = self._assemble("full", "[]")
        tiered = self.mod.filter_legs_by_tier(self.legs, "merge_group", "test")
        self.assertEqual(full, tiered)


class TestTierFiltering(unittest.TestCase):
    """filter_legs_by_tier / filter_legs_by_shard as pure functions."""

    @classmethod
    def setUpClass(cls):
        cls.mod = _load_assembler()

    def _names(self, legs):
        return sorted(leg["name"] for leg in legs)

    # ---- tiered events (pull_request / merge_group) partition by tier -------------
    def test_pull_request_test_tier_excludes_check_legs(self):
        for event in ("pull_request", "merge_group"):
            got = self.mod.filter_legs_by_tier(_synth_legs(), event, "test")
            self.assertEqual(self._names(got), ["a", "d"],
                             f"{event}: default/test tier must exclude tier:check legs")

    def test_pull_request_default_tier_is_test(self):
        # tier=None defaults to 'test' (the matrix output).
        got = self.mod.filter_legs_by_tier(_synth_legs(), "pull_request", None)
        self.assertEqual(self._names(got), ["a", "d"])

    def test_pull_request_check_tier_selects_only_check_legs(self):
        for event in ("pull_request", "merge_group"):
            got = self.mod.filter_legs_by_tier(_synth_legs(), event, "check")
            self.assertEqual(self._names(got), ["b", "c"],
                             f"{event}: --tier check must emit exactly the demoted legs")

    # ---- backstop events (push/schedule/...) emit ALL as full legs ----------------
    def test_backstop_test_tier_emits_all_legs_regardless_of_tier(self):
        for event in ("push", "schedule", "workflow_dispatch", "weird-unknown", None):
            got = self.mod.filter_legs_by_tier(_synth_legs(), event, "test")
            self.assertEqual(self._names(got), ["a", "b", "c", "d"],
                             f"event={event!r}: backstop must run EVERY leg as a full leg")

    def test_backstop_check_tier_is_empty(self):
        # On a full backstop run everything ran as a full leg => the check tier is empty.
        for event in ("push", "schedule", "workflow_dispatch", "weird-unknown", None):
            got = self.mod.filter_legs_by_tier(_synth_legs(), event, "check")
            self.assertEqual(got, [],
                             f"event={event!r}: the check tier must be empty on a full run")

    def test_malformed_tier_flag_is_a_hard_error(self):
        for bad in ("checkk", "TEST", "full", ""):
            with self.assertRaises(SystemExit) as cm:
                self.mod.filter_legs_by_tier(_synth_legs(), "pull_request", bad)
            self.assertNotEqual(cm.exception.code, 0)

    # ---- shard split --------------------------------------------------------------
    def test_shard_engine_vs_rest_partitions_the_check_legs(self):
        check = self.mod.filter_legs_by_tier(_synth_legs(), "pull_request", "check")
        engine = self.mod.filter_legs_by_shard(check, "engine")
        rest = self.mod.filter_legs_by_shard(check, "rest")
        self.assertEqual(self._names(engine), ["b"], "engine shard == sparq-engine legs")
        self.assertEqual(self._names(rest), ["c"], "rest shard == non-engine legs")
        # The two shards partition the check set exactly (no overlap, no drop).
        self.assertEqual(self._names(engine) + self._names(rest), self._names(check))

    def test_shard_none_is_a_noop(self):
        legs = _synth_legs()
        self.assertEqual(self.mod.filter_legs_by_shard(legs, None), legs)

    def test_malformed_shard_flag_is_a_hard_error(self):
        with self.assertRaises(SystemExit) as cm:
            self.mod.filter_legs_by_shard(_synth_legs(), "nope")
        self.assertNotEqual(cm.exception.code, 0)


class TestTierFieldValidation(unittest.TestCase):
    """load_legs() accepts the optional tier/tier-reason keys and validates the tier
    value, with a fresh temp fragment dir so the real fragments are untouched."""

    def setUp(self):
        self.mod = _load_assembler()
        self._tmp = tempfile.TemporaryDirectory()
        self._dir = Path(self._tmp.name)
        self._orig = self.mod.FRAGMENT_DIR
        self.mod.FRAGMENT_DIR = str(self._dir)

    def tearDown(self):
        self.mod.FRAGMENT_DIR = self._orig
        self._tmp.cleanup()

    def _write(self, body):
        (self._dir / "frag.yml").write_text(body, encoding="utf-8")

    _BASE = (
        "- name: demo\n"
        "  crate: sparq-core\n"
        "  features: jsonld\n"
        "  test: true\n"
    )

    def test_missing_tier_defaults_to_test(self):
        self._write(self._BASE)
        legs = self.mod.load_legs()
        self.assertEqual(legs[0]["tier"], "test")

    def test_explicit_test_and_check_are_accepted(self):
        for value in ("test", "check"):
            self._write(self._BASE + f"  tier: {value}\n")
            legs = self.mod.load_legs()
            self.assertEqual(legs[0]["tier"], value)

    def test_tier_reason_override_key_is_allowed(self):
        self._write(self._BASE + "  tier: check\n  tier-reason: reviewed keep\n")
        legs = self.mod.load_legs()
        self.assertEqual(legs[0]["tier"], "check")

    def test_malformed_tier_value_errors(self):
        for bad in ("bogus", "checkk", "", "3", "[a, b]"):
            self._write(self._BASE + f"  tier: {bad}\n")
            with self.assertRaises(SystemExit) as cm:
                self.mod.load_legs()
            self.assertNotEqual(cm.exception.code, 0, f"tier: {bad} must error")

    def test_null_tier_errors(self):
        # A present-but-null tier (`tier:` with no value) must NOT be silently treated
        # as check; it is malformed => error.
        self._write(self._BASE + "  tier:\n")
        with self.assertRaises(SystemExit):
            self.mod.load_legs()

    def test_unknown_extra_key_still_errors(self):
        self._write(self._BASE + "  bogus-key: 1\n")
        with self.assertRaises(SystemExit):
            self.mod.load_legs()


class TestTierBehaviorPreservation(unittest.TestCase):
    """THE load-bearing invariant on the REAL fragment set: zero annotations =>
    byte-identical full leg set on every event, and an empty check tier."""

    @classmethod
    def setUpClass(cls):
        cls.mod = _load_assembler()
        cls.full_names = sorted(leg["name"] for leg in cls.mod.load_legs())

    def _include_names(self, obj):
        return sorted(leg["name"] for leg in obj["include"])

    def test_no_real_fragment_is_annotated_yet(self):
        # Guards the premise of this whole bead: the demotion flip is a SEPARATE bead.
        legs = self.mod.load_legs()
        self.assertTrue(all(leg["tier"] == "test" for leg in legs),
                        "a fragment carries a tier annotation — that belongs to the "
                        "flip bead sq-s5dvo, not the behaviour-preserving wiring bead")

    def test_pull_request_and_merge_group_emit_the_full_set(self):
        for event in ("pull_request", "merge_group"):
            obj = _run_main_json(self.mod, ["--event", event])
            self.assertEqual(self._include_names(obj), self.full_names,
                             f"{event}: test-tier matrix must be the FULL set unchanged")

    def test_push_emits_the_full_set(self):
        obj = _run_main_json(self.mod, ["--event", "push"])
        self.assertEqual(self._include_names(obj), self.full_names)

    def test_check_tier_is_green_empty_on_every_event(self):
        for event in ("pull_request", "merge_group", "push", "schedule"):
            obj = _run_main_json(self.mod, ["--event", event, "--tier", "check"])
            self.assertEqual(obj["include"], [],
                             f"{event}: check tier must be empty at zero annotations")
            for shard in ("engine", "rest"):
                obj = _run_main_json(
                    self.mod, ["--event", event, "--tier", "check", "--shard", shard]
                )
                self.assertEqual(obj["include"], [],
                                 f"{event}/{shard}: check-tier shard must be empty too")

    def test_emitted_legs_have_exactly_the_four_workflow_keys(self):
        # The internal `tier` key must be stripped from the matrix JSON.
        obj = _run_main_json(self.mod, ["--event", "pull_request"])
        for leg in obj["include"]:
            self.assertEqual(set(leg.keys()), {"name", "crate", "features", "test"})


# [FABLE-5] sq-2wo5t: LOUD assemble-failure attribution.
#
# When the `setup` ("assemble feature matrix") job fails, the matrix output is never
# produced, `opt-in-features` spawns ZERO legs, and EVERY required `opt-in *` check on
# the PR goes expected-but-MISSING — which historically misread as "one specific leg
# failing across multiple unrelated PRs" (the 2026-07-11 false 'spqv-provenance'
# main-regression alarm). The workflow therefore carries a final `if: failure()` step
# that emits a ::error annotation + job-summary block attributing the missing legs to
# the assemble job itself. These tests pin that step's WIRING (present, failure-only,
# LAST — so every earlier guard's failure triggers it) and EXECUTE its script (bash is
# the only subprocess; still offline/deterministic) to prove the annotation actually
# fires with the load-bearing text, non-vacuously.
class TestAssembleFailureAnnotation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import yaml  # noqa: PLC0415  (lazy: only when the test runs)

        workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        cls.setup_steps = workflow["jobs"]["setup"]["steps"]
        cls.failure_steps = [
            s for s in cls.setup_steps if str(s.get("if", "")).strip() == "failure()"
        ]

    def test_setup_job_has_exactly_one_failure_attribution_step(self):
        self.assertEqual(
            len(self.failure_steps),
            1,
            "the setup job must carry exactly ONE `if: failure()` loud-attribution "
            "step (sq-2wo5t) — an assemble failure otherwise reads as a phantom "
            "cross-PR `opt-in *` leg regression",
        )

    def test_failure_attribution_step_is_last(self):
        # LAST is load-bearing: `if: failure()` fires only on an EARLIER step's
        # failure, so any guard step added AFTER it would fail silently again.
        self.assertIs(
            self.setup_steps[-1],
            self.failure_steps[0],
            "the failure-attribution step must be the LAST step of the setup job so "
            "every failure-capable guard step precedes (and thus triggers) it",
        )

    def test_failure_attribution_step_has_no_id_or_outputs_dependency(self):
        # It must be self-contained: no `${{ steps.* }}` reads (an earlier failed
        # step's outputs are empty) and plain `run:` (no action to fetch).
        step = self.failure_steps[0]
        self.assertIn("run", step, "attribution must be a plain run: step")
        self.assertNotIn("uses", step)
        self.assertNotIn("${{ steps.", step["run"])

    def test_failure_annotation_names_the_real_situation(self):
        run = self.failure_steps[0]["run"]
        self.assertIn("::error", run, "must emit a GitHub Actions ::error annotation")
        for needle in (
            "MISSING, not failing",
            "opt-in",
            "feature-matrix-legnames.golden.txt",
            "check-feature-test-execution.py",
        ):
            self.assertIn(
                needle, run,
                f"the loud annotation must mention {needle!r} so triage starts at "
                "the assemble job, not a phantom per-leg regression",
            )

    def test_failure_annotation_script_fires(self):
        """NON-VACUOUS: execute the step's script and assert the annotation + the
        job-summary block are actually produced (catches an unbound var under
        `set -u`, a broken heredoc/quoting, or dropped load-bearing text)."""
        import subprocess  # noqa: PLC0415  (lazy: only this test shells out)

        run = self.failure_steps[0]["run"]
        with tempfile.TemporaryDirectory() as tmp:
            summary = Path(tmp) / "step_summary.md"
            summary.touch()
            proc = subprocess.run(
                ["bash", "-c", run],
                capture_output=True,
                text=True,
                env={"PATH": "/usr/bin:/bin", "GITHUB_STEP_SUMMARY": str(summary)},
                timeout=30,
            )
            self.assertEqual(
                proc.returncode, 0,
                f"the attribution script must not itself fail:\n{proc.stderr}",
            )
            self.assertIn("::error", proc.stdout)
            self.assertIn("MISSING, not failing", proc.stdout)
            summary_text = summary.read_text(encoding="utf-8")
            self.assertIn("opt-in feature matrix NOT generated", summary_text)
            self.assertIn("MISSING, not failing", summary_text)
            self.assertIn("feature-matrix-legnames.golden.txt", summary_text)


# [FABLE-5] CI-economy grouping (maintainer directive 2026-07-18): the per-leg matrix
# is bin-packed into grouped runner jobs (assembler --grouped) and the gate-critical
# `opt-in <name>` check-runs are emitted per leg from INSIDE the group job by
# scripts/run-feature-matrix-group.py. THE load-bearing invariant: grouping is a pure
# PARTITION of the (selection/tier-filtered) leg set — no leg dropped, none duplicated,
# no name changed — so the golden name contract is untouched.
class TestGrouping(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.mod = _load_assembler()
        cls.legs = cls.mod.load_legs()

    def test_grouping_partitions_the_full_leg_set(self):
        groups = self.mod.group_legs(self.legs)
        flat = [leg["name"] for g in groups for leg in g["legs"]]
        self.assertEqual(sorted(flat), sorted(leg["name"] for leg in self.legs),
                         "grouping must partition the leg set exactly (no drop/dup)")
        self.assertEqual(len(flat), len(set(flat)), "no leg may appear twice")

    def test_group_capacity_respected_for_multi_leg_groups(self):
        # A single leg heavier than the capacity may stand alone; any MULTI-leg
        # group must fit the capacity (the <5 min wall-time target).
        for g in self.mod.group_legs(self.legs):
            if g["count"] > 1:
                self.assertLessEqual(
                    g["weight"], self.mod.GROUP_CAPACITY,
                    f"group {g['group']} exceeds the capacity with multiple legs")

    def test_grouping_is_deterministic(self):
        a = self.mod.group_legs(self.legs)
        b = self.mod.group_legs(self.legs)
        self.assertEqual(a, b, "grouping must be deterministic run-to-run")

    def test_groups_are_meaningfully_fewer_than_legs(self):
        groups = self.mod.group_legs(self.legs)
        self.assertLess(len(groups), len(self.legs) / 2,
                        "grouping must collapse the runner count substantially — "
                        "one-leg-per-group would defeat the CI-economy directive")

    def test_grouped_main_output_shape(self):
        obj = _run_main_json(self.mod, ["--grouped", "--event", "pull_request"])
        self.assertIn("include", obj)
        total = 0
        for g in obj["include"]:
            self.assertEqual(set(g.keys()), {"group", "cache_crate", "count", "legs"})
            inner = json.loads(g["legs"])  # legs is a JSON-encoded STRING (matrix scalar)
            self.assertEqual(len(inner), g["count"])
            total += g["count"]
            for leg in inner:
                self.assertEqual(set(leg.keys()), {"name", "crate", "features", "test"})
        self.assertEqual(total, len(self.legs))

    def test_grouped_composes_with_selection(self):
        obj = _run_main_json(self.mod, [
            "--grouped", "--event", "pull_request",
            "--select-mode", "selected", "--affected", "[]"])
        self.assertEqual(obj["include"], [],
                         "an empty affected closure must assemble ZERO groups")
        crates = sorted({leg["crate"] for leg in self.legs})
        keep = crates[0]
        obj = _run_main_json(self.mod, [
            "--grouped", "--event", "pull_request",
            "--select-mode", "selected", "--affected", json.dumps([keep])])
        inner = [leg for g in obj["include"] for leg in json.loads(g["legs"])]
        self.assertTrue(inner)
        self.assertEqual({leg["crate"] for leg in inner}, {keep})

    def test_explicit_weight_overrides_heuristic(self):
        leg = dict(self.legs[0])
        leg["weight"] = 7.5
        self.assertEqual(self.mod.leg_weight(leg), 7.5)
        leg["weight"] = None
        self.assertGreater(self.mod.leg_weight(leg), 0)

    def test_single_overweight_leg_gets_its_own_group(self):
        legs = [
            {"name": "huge", "crate": "c1", "features": "f", "test": True,
             "weight": self.mod.GROUP_CAPACITY * 3},
            {"name": "tiny", "crate": "c2", "features": "f", "test": True,
             "weight": 0.1},
        ]
        groups = self.mod.group_legs(legs)
        huge = [g for g in groups if any(l["name"] == "huge" for l in g["legs"])]
        self.assertEqual(len(huge), 1)
        self.assertEqual(huge[0]["count"], 1,
                         "an over-capacity leg must stand alone, never merged")

    def test_same_crate_legs_cluster_before_packing(self):
        # Two crates, each with legs that fit one bin: no group may interleave
        # them while a same-crate leg still fits — target-dir reuse is the point.
        legs = []
        for crate in ("ca", "cb"):
            for i in range(3):
                legs.append({"name": f"{crate}-{i}", "crate": crate,
                             "features": "f", "test": True, "weight": 1.0})
        for g in self.mod.group_legs(legs):
            self.assertEqual(len({leg["crate"] for leg in g["legs"]}), 1,
                             "fitting same-crate chunks must not be split across "
                             "crates when each crate fits a bin alone")


class TestWeightFieldValidation(unittest.TestCase):
    """load_legs() accepts the optional `weight` key and hard-errors on malformed
    values (a bad weight must never silently skew the packing)."""

    def setUp(self):
        self.mod = _load_assembler()
        self._tmp = tempfile.TemporaryDirectory()
        self._dir = Path(self._tmp.name)
        self._orig = self.mod.FRAGMENT_DIR
        self.mod.FRAGMENT_DIR = str(self._dir)

    def tearDown(self):
        self.mod.FRAGMENT_DIR = self._orig
        self._tmp.cleanup()

    def _write(self, body):
        (self._dir / "frag.yml").write_text(body, encoding="utf-8")

    _BASE = (
        "- name: demo\n"
        "  crate: sparq-core\n"
        "  features: jsonld\n"
        "  test: true\n"
    )

    def test_missing_weight_defaults_to_heuristic(self):
        self._write(self._BASE)
        legs = self.mod.load_legs()
        self.assertIsNone(legs[0]["weight"])
        self.assertGreater(self.mod.leg_weight(legs[0]), 0)

    def test_explicit_positive_weight_is_accepted(self):
        for value in ("2", "0.5", "10.25"):
            self._write(self._BASE + f"  weight: {value}\n")
            legs = self.mod.load_legs()
            self.assertEqual(legs[0]["weight"], float(value))

    def test_malformed_weight_errors(self):
        for bad in ("0", "-1", "abc", "true", ".nan", ".inf", "[1]"):
            self._write(self._BASE + f"  weight: {bad}\n")
            with self.assertRaises(SystemExit) as cm:
                self.mod.load_legs()
            self.assertNotEqual(cm.exception.code, 0, f"weight: {bad} must error")


class TestGroupedWorkflowWiring(unittest.TestCase):
    """Pins the grouped-runner wiring in feature-matrix.yml ACROSS THE TRUSTED
    BOUNDARY (PR #3511 review, CRITICAL finding 1): the matrix is assembled with
    --grouped; the UNPRIVILEGED group job (contents: read only, no persisted
    credentials, NO token in its environment — it executes PR-controlled cargo
    build scripts/proc macros) runs scripts/run-feature-matrix-group.py and uploads
    the per-leg results as an artifact. The reporter is NOT a job in this
    PR-triggered workflow — it lives in the separate default-branch-owned
    feature-matrix-report.yml (TestReportWorkflowTrustBoundary). CRUCIALLY: NO job in
    feature-matrix.yml may hold checks: write (a PR-head-checked-out job that holds it
    and runs the matrix scripts is the pwn-requests hole this PR closes)."""

    @classmethod
    def setUpClass(cls):
        import yaml  # noqa: PLC0415

        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.wf = yaml.safe_load(cls.text)
        cls.job = cls.wf["jobs"]["opt-in-features"]
        cls.setup = cls.wf["jobs"]["setup"]

    def test_assemble_step_uses_grouped_mode(self):
        self.assertIn("assemble-feature-matrix.py --grouped", self.text)

    def test_group_job_runs_the_group_runner_script(self):
        runs = [s.get("run", "") for s in self.job["steps"]]
        self.assertTrue(any("run-feature-matrix-group.py" in r for r in runs),
                        "the group job must run scripts/run-feature-matrix-group.py")

    # ---- the trusted boundary itself (do NOT weaken any of these) ----------------
    def test_group_job_is_unprivileged(self):
        self.assertEqual(
            self.job.get("permissions"), {"contents": "read"},
            "the group job executes PR-controlled build code and must hold ONLY "
            "contents: read — a checks: write token here lets malicious build "
            "scripts/proc macros forge arbitrary check runs (PR #3511 review)")

    def test_group_job_checkout_does_not_persist_credentials(self):
        checkouts = [s for s in self.job["steps"]
                     if "actions/checkout" in str(s.get("uses", ""))]
        self.assertTrue(checkouts, "the group job must check out the repo")
        for s in checkouts:
            self.assertIs(
                (s.get("with") or {}).get("persist-credentials"), False,
                "the group job's checkout must set persist-credentials: false — "
                "PR-controlled build code must never find a token in .git/config")

    def test_group_job_env_has_no_token(self):
        env = {}
        for s in self.job["steps"]:
            env.update(s.get("env", {}) or {})
        for key in ("GH_TOKEN", "GITHUB_TOKEN"):
            self.assertNotIn(key, env,
                             f"the group job must not export {key} while running "
                             "PR-controlled cargo code")
        for key in ("GROUP_LEGS", "GROUP_NAME", "FMG_RESULTS"):
            self.assertIn(key, env, f"group runner step must set {key}")

    def test_NO_job_in_the_pr_workflow_holds_checks_write(self):
        """THE finding-1 invariant: this workflow is PR/merge_group-triggered, so
        every job runs from the (possibly PR-head) merge ref. NONE may hold
        checks: write — the reporter (the only checks: write holder) moved to the
        default-branch-owned feature-matrix-report.yml so a PR cannot edit a script
        it runs to forge check-runs."""
        for job_id, job in self.wf["jobs"].items():
            self.assertNotEqual(
                (job.get("permissions") or {}).get("checks"), "write",
                f"job {job_id!r} holds checks: write in the PR-triggered "
                "feature-matrix.yml — that is the pwn-requests hole PR #3511 closes; "
                "the reporter must live in feature-matrix-report.yml (workflow_run)")

    def test_no_cargo_running_job_holds_checks_write_or_token(self):
        """Workflow-wide sweep: any job that executes cargo (i.e. PR-controlled
        build-time code) must never hold checks: write nor export a token."""
        for job_id, job in self.wf["jobs"].items():
            steps = job.get("steps") or []
            runs = " ".join(str(s.get("run", "") or "") for s in steps)
            if "cargo" not in runs:
                continue
            self.assertNotEqual(
                (job.get("permissions") or {}).get("checks"), "write",
                f"job {job_id!r} executes cargo and must not hold checks: write")
            env = {}
            for s in steps:
                env.update(s.get("env", {}) or {})
            self.assertNotIn("GH_TOKEN", env,
                             f"job {job_id!r} executes cargo and must not export GH_TOKEN")

    def test_group_job_uploads_results_artifact(self):
        uploads = [s for s in self.job["steps"]
                   if "actions/upload-artifact" in str(s.get("uses", ""))]
        self.assertEqual(len(uploads), 1,
                         "the group job must upload exactly one results artifact")
        step = uploads[0]
        self.assertIn("feature-matrix-results-", str(step["with"]["name"]))
        self.assertIn("!cancelled()", str(step.get("if", "")),
                      "a group with FAILED legs must still ship its results so the "
                      "reporter posts the red per-leg check-runs")

    # ---- the setup job feeds the trusted reporter its ground truth ---------------
    def test_setup_uploads_selected_matrix_and_metadata(self):
        """Finding 2: the reporter's completeness ground truth is the run's OWN
        selected leg set (a default-branch re-assemble would not know a leg the PR
        adds). Finding 1: the metadata artifact carries the head SHA the reporter
        validates against the server-supplied workflow_run.head_sha. The setup job
        runs PR-controlled assembler code, so it must hold NO checks: write / token
        — the artifact is DATA the trusted reporter validates."""
        uploads = [s for s in self.setup["steps"]
                   if "actions/upload-artifact" in str(s.get("uses", ""))]
        names = {str((s.get("with") or {}).get("name", "")) for s in uploads}
        self.assertIn("feature-matrix-selected", names,
                      "setup must upload the selected-matrix artifact")
        self.assertIn("feature-matrix-metadata", names,
                      "setup must upload the head-SHA metadata artifact")
        self.assertNotEqual((self.setup.get("permissions") or {}).get("checks"),
                            "write", "setup runs PR-controlled assembler code")

    def test_job_names_are_not_advisory(self):
        for job in (self.job, self.setup):
            self.assertNotRegex(str(job.get("name", "")), ADVISORY_RE,
                                "the group + setup jobs must GATE")

    # ---- PR #3511 review finding 4: the test path triggers the setup self-test ---
    def test_rust_path_filter_covers_the_matrix_scripts_and_tests(self):
        import yaml  # noqa: PLC0415

        steps = self.wf["jobs"]["changes"]["steps"]
        flt = [s for s in steps if "dorny/paths-filter" in str(s.get("uses", ""))]
        self.assertEqual(len(flt), 1)
        filters = yaml.safe_load(flt[0]["with"]["filters"])
        rust = filters["rust"]
        for path in (
            "scripts/assemble-feature-matrix.py",
            "scripts/run-feature-matrix-group.py",
            # finding 4: a test-only weakening must still run the setup self-test.
            "scripts/tests/test_feature_matrix_assemble.py",
            ".github/feature-matrix.d/**",
            ".github/workflows/feature-matrix.yml",
            # round-3 finding 2: a PR changing ONLY the PRIVILEGED reporter workflow
            # must still run setup so the trusted-boundary self-tests fire.
            ".github/workflows/feature-matrix-report.yml",
        ):
            self.assertIn(
                path, rust,
                f"the rust path filter must include {path!r} — a PR changing only "
                "it must still run setup (self-test), the grouped execution and "
                "the reporter (PR #3511 review findings 2 + 4)")


class TestPreMergeC1Job(unittest.TestCase):
    """[OPUS-5] sq-nd4yj: pins the PRE-MERGE C1 job in feature-matrix.yml.

    C1 (scripts/check-feature-test-execution.py) is a WHOLE-TREE invariant — a
    feature-gated test in one crate is covered by an executor declared elsewhere in the
    tree — so a MERGE can break it while NEITHER SIDE breaks it alone. That is the sq-nd4yj
    incident: 700ec341 (#2194's drain merge) dropped the `sparq-algos (topology)` leg that
    #2193 had just landed, while the gated tests stayed, and C1 then RED-ded main and every
    branch until f6a06580. The `setup` job's C1 cannot see that class: on `pull_request` it
    grades `refs/pull/N/merge`, frozen at event time, and GitHub does not re-run the checks
    when the base moves. The `premerge-c1` job re-merges the base tip AS OF ITS OWN RUN and
    re-runs the SAME `--check`. (Scope, stated in the job's own header: it grades CLEAN
    merges only — a conflicting PR has no merge ref for anyone to grade, and a
    locally-resolved push to main runs no PR CI, which is how 700ec341 itself got in.) Each
    assertion below is one of the properties that makes the job able to do that; delete the
    job, or weaken any of them, and this test goes red.

    LIFECYCLE (PR #5253 review round 1). `premerge-c1` narrows the stale-green window
    without closing it: a push to the base branch re-triggers nothing for the open PRs
    targeting it, so this job's completed green can outlive the base it graded. The job
    therefore claims only "head ⊕ THIS named base SHA was clean", and the verdict that
    cannot go stale is `setup`'s C1 on the `merge_group` ref — a SHA that contains the
    latest base by construction. Two tests below pin exactly that division of labour: the
    job must publish the base SHA it graded, and the merge_group path must stay wired.

    TEXT-BASED (block slicing on the job's fixed indent) rather than yaml.safe_load, on
    purpose: PyYAML is an install step in CI but is absent on a bare dev box, where the
    yaml-based classes above cannot run at all. The rest of the file already reads
    WORKFLOW as text for the same kind of contract (the static-`include:` check)."""

    JOB_ID = "premerge-c1"
    JOB_NAME = "pre-merge C1 (feature-gated test execution)"

    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.block = cls._job_block(cls.text, cls.JOB_ID)

    @staticmethod
    def _job_block(text: str, job_id: str, why: str = "") -> str:
        """The `jobs.<job_id>` mapping, verbatim: from its 2-space-indented key line to
        (exclusive) the next line at that indent. Job-level keys sit at 4 spaces and
        steps deeper, so the terminator is unambiguous."""
        lines = text.splitlines()
        try:
            start = lines.index(f"  {job_id}:")
        except ValueError:
            raise AssertionError(
                f"feature-matrix.yml declares no `{job_id}` job — "
                + (why or
                   "the pre-merge C1 guard (sq-nd4yj) is GONE, so a violation that only "
                   "exists in the merged tree (gated test on one side, its matrix leg on "
                   "the other) reaches main and reds C1 on every branch.")
            ) from None
        out = [lines[start]]
        for line in lines[start + 1:]:
            if re.match(r"^  \S", line):
                break
            out.append(line)
        return "\n".join(out)

    @staticmethod
    def _top_level_block(text: str, key: str) -> str:
        """The top-level `<key>:` mapping, verbatim: from its column-0 key line to
        (exclusive) the next column-0 line. Same slicing contract as `_job_block`, one
        indent level up — used for the `on:` trigger block."""
        lines = text.splitlines()
        try:
            start = lines.index(f"{key}:")
        except ValueError:
            raise AssertionError(
                f"feature-matrix.yml has no top-level `{key}:` block"
            ) from None
        out = [lines[start]]
        for line in lines[start + 1:]:
            if re.match(r"^\S", line):
                break
            out.append(line)
        return "\n".join(out)

    def test_runs_the_c1_guard_itself(self):
        """The point of the job: the SAME check, not a re-implementation of it."""
        self.assertIn(
            "python3 scripts/check-feature-test-execution.py --check", self.block,
            "the pre-merge job must run the real C1 guard on the merged tree")

    def test_merges_the_current_base_tip(self):
        """Without this the job grades the same stale tree `setup` already graded."""
        self.assertIn('merge --no-commit --no-ff "origin/${BASE_REF}"', self.block,
                      "the job must test-merge the CURRENT base tip into the PR head")
        self.assertIn("BASE_REF: ${{ github.event.pull_request.base.ref }}", self.block,
                      "BASE_REF must come from the PR's base branch")

    def test_checkout_has_the_merge_base(self):
        """A default pull_request checkout fetches only `refs/pull/N/merge` at depth 1 —
        no `origin/<base_ref>` to merge and no shared ancestor — so the test merge, and
        with it the guard, would never evaluate."""
        self.assertIn("fetch-depth: 0", self.block)
        self.assertIn("persist-credentials: false", self.block)

    def test_test_merge_starts_at_the_pr_head(self):
        """PR #5253 review round 2. On `pull_request`, actions/checkout WITHOUT an explicit
        `ref` lands on `refs/pull/N/merge` — the synthetic merge GitHub froze at event time,
        i.e. the head already merged with the THEN-current base. Merging the current base
        tip into that grades ((stale base ⊕ head) ⊕ current base), not the "PR head ⊕
        current base tip" this job's comments and summary claim; history shape feeds merge
        base resolution and rename detection, so the two are not interchangeable. Pin both
        halves of the fix — the checkout starts at the head SHA, and the merge step
        re-verifies HEAD against it rather than trusting the checkout. Note this asserts
        the merge's STARTING COMMIT, which `test_publishes_the_base_sha_it_graded` does not:
        that one only proves HEAD_SHA reaches the summary, which was true while the bug
        was live."""
        self.assertIn("ref: ${{ github.event.pull_request.head.sha }}", self.block,
                      "the checkout must pin `ref` to the PR head SHA — otherwise the "
                      "test merge starts from the event-time refs/pull/N/merge commit")
        self.assertIn('ACTUAL_HEAD="$(git rev-parse HEAD)"', self.block,
                      "the merge step must resolve the commit it is about to merge from")
        self.assertIn('if [ "${ACTUAL_HEAD}" != "${HEAD_SHA}" ]; then', self.block,
                      "the merge step must FAIL CLOSED when HEAD is not the PR head — a "
                      "merge from any other commit produces a tree the job misreports")

    def test_guard_step_is_gated_on_a_clean_test_merge(self):
        """A conflicted working tree is not the merge result; grading it would be a
        verdict about a tree that will never exist."""
        self.assertIn("if: steps.merge.outputs.merged == 'true'", self.block)
        self.assertIn("git merge --abort", self.block)

    def test_pull_request_only_and_path_filtered(self):
        """merge_group/push already check out an actually-merged tree, so re-merging
        there is meaningless; `rust_changed` keeps a non-Rust PR off this runner."""
        self.assertIn("github.event_name == 'pull_request'", self.block)
        self.assertIn("needs.changes.outputs.rust_changed == 'true'", self.block)
        self.assertIn("needs: [changes]", self.block)

    def test_publishes_the_base_sha_it_graded(self):
        """PR #5253 review round 1. The job resolves the base tip WHILE RUNNING, and no PR
        event re-runs it when the base later advances — so a completed green can be a
        verdict about a base the PR would no longer merge with. The job must therefore
        name the SHA it graded (step output + job summary); without that, an old green and
        a current one are indistinguishable and "the CURRENT base tip" is unfalsifiable."""
        self.assertIn('BASE_SHA="$(git rev-parse "origin/${BASE_REF}")"', self.block,
                      "the job must resolve the base tip to a concrete SHA")
        self.assertIn('echo "base_sha=${BASE_SHA}" >> "$GITHUB_OUTPUT"', self.block,
                      "the graded base SHA must be a step output so later steps can cite it")
        self.assertIn('>> "$GITHUB_STEP_SUMMARY"', self.block,
                      "the graded base SHA must reach the job summary a human reads")
        self.assertIn("HEAD_SHA: ${{ github.event.pull_request.head.sha }}", self.block,
                      "the summary must name the PR head it merged, not just the base")

    def test_unstaleable_c1_verdict_stays_wired_on_merge_group(self):
        """PR #5253 review round 1 — the LIFECYCLE half, not another substring of this job.

        Because this job's verdict is scoped to one base SHA and GitHub re-runs nothing for
        an open PR when the base advances, it CANNOT be the last word on the merge
        candidate; the job header says so. That honesty only holds while the invariant is
        ALSO enforced on a ref whose SHA contains the latest base by construction — the
        merge queue's `merge_group` ref. Assert that path end to end: the workflow triggers
        on merge_group, and the job that runs C1 (`setup`) is not restricted to
        pull_request, so the merge_group run actually grades it. Drop either and the only
        remaining C1 verdict is a staleable one."""
        on_block = self._top_level_block(self.text, "on")
        self.assertIn("  merge_group:", on_block,
                      "feature-matrix.yml must still trigger on merge_group — that run is "
                      "the C1 verdict on the tree that actually ships, and the pre-merge "
                      "job explicitly defers to it")
        setup = self._job_block(
            self.text, "setup",
            why="`setup` is where the whole-tree C1 guard runs on the merge_group ref; "
                "without it the only C1 verdict left is the pre-merge job's, which is "
                "scoped to a base SHA that can go stale (PR #5253 review round 1).")
        self.assertIn("python3 scripts/check-feature-test-execution.py --check", setup,
                      "`setup` must still run the real C1 guard")
        self.assertNotIn("pull_request", setup.split("steps:")[0],
                         "`setup`'s job-level condition must NOT restrict it to "
                         "pull_request — the merge_group run is the C1 verdict that "
                         "cannot go stale")

    def test_job_is_unprivileged(self):
        """It checks out the PR head and merges the base locally; it needs nothing but
        the source."""
        self.assertIn("permissions:\n      contents: read\n", self.block)
        self.assertNotIn("checks: write", self.block)

    def test_job_gates(self):
        """Post-#3773 a check gates unless DECLARED advisory; assert both halves — the
        name carries no advisory token, and nothing declares this check advisory."""
        self.assertIn(f"name: {self.JOB_NAME}", self.block)
        self.assertNotRegex(self.JOB_NAME, ADVISORY_RE)
        registry = json.loads(
            (REPO_ROOT / ".github" / "advisory-registry.json").read_text(encoding="utf-8")
        )
        declared = {k.lower() for k in registry if not k.startswith("_")}
        self.assertNotIn(
            self.JOB_NAME.lower(), declared,
            "the pre-merge C1 check must not be declared advisory — a merge-only C1 "
            "violation must BLOCK the merge, which is the whole point (sq-nd4yj)")


class TestReportWorkflowTrustBoundary(unittest.TestCase):
    """Pins the DEFAULT-BRANCH-owned reporter workflow (PR #3511 review finding 1):
    feature-matrix-report.yml is triggered by workflow_run on feature-matrix, so
    GitHub always runs its (and the reporter scripts') default-branch copy — a PR
    cannot edit them to forge check-runs. It holds checks: write, runs no cargo,
    downloads the triggering run's artifacts, validates the head SHA against the
    server-supplied workflow_run.head_sha, and reports via --report."""

    @classmethod
    def setUpClass(cls):
        import yaml  # noqa: PLC0415

        cls.text = REPORT_WORKFLOW.read_text(encoding="utf-8")
        cls.wf = yaml.safe_load(cls.text)
        # yaml parses the bare `on:` key as boolean True — handle both spellings.
        cls.on = cls.wf.get("on", cls.wf.get(True))
        cls.job = cls.wf["jobs"]["report"]

    def test_triggered_by_workflow_run_on_feature_matrix(self):
        wr = self.on["workflow_run"]
        self.assertEqual(wr["workflows"], ["feature-matrix"],
                         "must fire only on the feature-matrix workflow's completion")
        self.assertIn("completed", wr["types"])

    def test_no_pull_request_trigger(self):
        # A pull_request trigger would take the workflow definition from the PR head
        # — exactly the trust hole workflow_run avoids.
        self.assertNotIn("pull_request", self.on,
                         "the reporter must NOT be pull_request-triggered")

    def test_reporter_holds_checks_write_and_runs_no_cargo(self):
        self.assertEqual((self.job.get("permissions") or {}).get("checks"), "write",
                         "the reporter is the sole checks: write holder")
        runs = " ".join(str(s.get("run", "") or "") for s in self.job["steps"])
        self.assertNotIn("cargo", runs,
                         "the trusted reporter must execute no PR build code")
        self.assertIn("run-feature-matrix-group.py --report", runs)

    def test_reporter_downloads_by_run_id(self):
        downloads = [s for s in self.job["steps"]
                     if "actions/download-artifact" in str(s.get("uses", ""))]
        self.assertTrue(downloads, "the reporter must download the run's artifacts")
        patterns = {str((s.get("with") or {}).get("pattern", "")) for s in downloads}
        self.assertIn("feature-matrix-results-*", patterns)
        for s in downloads:
            with_ = s.get("with") or {}
            self.assertIn("run-id", with_,
                          "download must target the TRIGGERING run by run-id")
            self.assertIn("github-token", with_)

    def test_reporter_validates_head_sha_against_server_value(self):
        # The head SHA is attacker-influenced (from an artifact); it must be checked
        # against github.event.workflow_run.head_sha (server-supplied).
        self.assertIn("workflow_run.head_sha", self.text,
                      "the reporter must compare the artifact head SHA against the "
                      "server-supplied workflow_run.head_sha (finding 1)")
        runs = " ".join(str(s.get("run", "") or "") for s in self.job["steps"])
        self.assertIn("TRIGGER_HEAD_SHA", runs,
                      "the validated head SHA must flow through env, not string "
                      "interpolation")

    def test_reporter_wires_trigger_run_id_from_server_value(self):
        # Finding 2: the summary check's external_id correlation token is the
        # TRIGGERING feature-matrix run id — from github.event.workflow_run.id
        # (server-supplied), not any artifact. Pin the env wiring so it cannot drift
        # to an artifact-derived (attacker-influenced) value.
        report_step = next(
            s for s in self.job["steps"]
            if "run-feature-matrix-group.py --report" in str(s.get("run", "") or "")
        )
        env = report_step.get("env") or {}
        self.assertIn("TRIGGER_RUN_ID", env,
                      "the reporter must pass the triggering run id for correlation")
        self.assertIn("workflow_run.id", str(env["TRIGGER_RUN_ID"]),
                      "TRIGGER_RUN_ID must be the server-supplied workflow_run.id")

    def test_reporter_sets_fork_pr_from_server_event_fields(self):
        # Finding 3: fork tolerance is event-gated, from server fields.
        self.assertIn("workflow_run.event", self.text)
        self.assertIn("workflow_run.head_repository.full_name", self.text)
        runs = " ".join(str(s.get("run", "") or "") for s in self.job["steps"])
        self.assertIn("FMG_FORK_PR", runs)

    def test_reporter_job_name_gates(self):
        self.assertNotRegex(str(self.job.get("name", "")), ADVISORY_RE,
                            "the reporter's summary check must GATE")


class TestGroupRunner(unittest.TestCase):
    """Executes scripts/run-feature-matrix-group.py (EXECUTION mode) with a stubbed
    cargo binary (FMG_CARGO): a failing leg must NOT stop the group, every leg's
    outcome must land in the results file for the trusted reporter, the group must
    exit non-zero naming the failed leg — and the mode must need NO GitHub token
    and make NO API call (the trusted-boundary contract, PR #3511 review)."""

    RUNNER = REPO_ROOT / "scripts" / "run-feature-matrix-group.py"
    SCHEMA = "feature-matrix-group-results/v1"

    def _run(self, legs, fail_on="", with_results_env=True):
        import subprocess  # noqa: PLC0415

        with tempfile.TemporaryDirectory() as tmp:
            tmpdir = Path(tmp)
            cargo_log = tmpdir / "cargo.log"
            summary = tmpdir / "summary.md"
            results = tmpdir / "fmg" / "results.json"
            cargo = tmpdir / "cargo-stub"
            cargo.write_text(
                "#!/bin/bash\n"
                f"echo \"$@\" >> {cargo_log}\n"
                "case \" $* \" in *\" ${FAIL_ON:-__none__} \"*) exit 1;; esac\n"
                "exit 0\n",
                encoding="utf-8",
            )
            cargo.chmod(0o755)
            summary.touch()
            # Deliberately NO GH_TOKEN / gh stub in the environment: execution
            # mode must never need or touch a token (trusted-boundary contract).
            env = {
                "PATH": "/usr/bin:/bin",
                "FMG_CARGO": str(cargo),
                "FAIL_ON": fail_on,
                "GROUP_LEGS": json.dumps(legs),
                "GROUP_NAME": "g01 test-group",
                "GITHUB_STEP_SUMMARY": str(summary),
            }
            if with_results_env:
                env["FMG_RESULTS"] = str(results)
            proc = subprocess.run(
                [sys.executable, str(self.RUNNER)],
                capture_output=True,
                text=True,
                timeout=60,
                env=env,
            )
            results_obj = (
                json.loads(results.read_text(encoding="utf-8"))
                if results.exists()
                else None
            )
            cargo_lines = (
                cargo_log.read_text(encoding="utf-8").splitlines()
                if cargo_log.exists()
                else []
            )
            return proc, results_obj, summary.read_text(encoding="utf-8"), cargo_lines

    LEGS = [
        {"name": "alpha (f1)", "crate": "alpha", "features": "f1", "test": True},
        {"name": "beta (f2)", "crate": "beta", "features": "f2", "test": False},
    ]

    def test_all_green_writes_a_success_result_per_leg(self):
        proc, results, _, _ = self._run(self.LEGS)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(results["schema"], self.SCHEMA)
        self.assertEqual(results["group"], "g01 test-group")
        self.assertEqual(
            results["results"],
            [{"name": "alpha (f1)", "failed_step": None},
             {"name": "beta (f2)", "failed_step": None}],
            "every leg's outcome must land in the results file for the reporter")

    def test_failing_leg_continues_reports_and_reds_the_group(self):
        proc, results, summary, _ = self._run(self.LEGS, fail_on="f1")
        self.assertEqual(proc.returncode, 1, "a failed leg must RED the group")
        # The failure names the exact leg, loudly.
        self.assertIn("opt-in alpha (f1)", proc.stdout)
        self.assertIn("::error", proc.stdout)
        # The group kept going: BOTH legs land in the results (fail-fast: false).
        self.assertEqual(
            results["results"],
            [{"name": "alpha (f1)", "failed_step": "build"},
             {"name": "beta (f2)", "failed_step": None}])
        self.assertIn("alpha (f1)", summary)

    def test_untested_leg_skips_cargo_test(self):
        proc, results, _, cargo_lines = self._run([self.LEGS[1]])
        self.assertEqual(proc.returncode, 0)
        self.assertEqual(results["results"][0]["failed_step"], None)
        self.assertFalse(any(line.startswith("test ") for line in cargo_lines),
                         "an untested leg must not invoke `cargo test`")

    def test_missing_results_env_is_fatal(self):
        # Without FMG_RESULTS the reporter would have nothing to post from —
        # every leg name would silently vanish from the gate's discovery set.
        proc, _, _, cargo_lines = self._run(self.LEGS, with_results_env=False)
        self.assertEqual(proc.returncode, 2)
        self.assertEqual(cargo_lines, [], "must refuse to run any leg without it")

    def test_execution_mode_makes_no_github_api_call(self):
        # No gh binary exists on the stub PATH; a mode that shelled out to gh
        # would crash. Green run == no API call was attempted.
        proc, _, _, _ = self._run(self.LEGS)
        self.assertEqual(proc.returncode, 0)
        self.assertNotIn("check-run", proc.stdout,
                         "execution mode must not attempt check-run emission")


class TestGroupReporter(unittest.TestCase):
    """Executes scripts/run-feature-matrix-group.py --report (the TRUSTED side of
    the split) with a stubbed gh binary. The artifact files it consumes were
    written inside jobs that ran arbitrary PR-controlled build code, so the
    load-bearing properties are: HOSTILE-INPUT validation (leg names must resolve
    in the assembled leg set — never PR-supplied free text; strict schema;
    duplicate names refused), and FAIL-CLOSED posting (only the deterministic
    fork-PR read-only-token denial degrades to a warning; every other Checks API
    failure reds the reporter job — PR #3511 review finding 3)."""

    RUNNER = REPO_ROOT / "scripts" / "run-feature-matrix-group.py"
    SCHEMA = "feature-matrix-group-results/v1"
    FORK_DENIAL = "gh: Resource not accessible by integration (HTTP 403)"
    SUMMARY_NAME = "feature-matrix report"

    VALID = {"include": [
        {"name": "alpha (f1)", "crate": "alpha", "features": "f1", "test": True},
        {"name": "beta (f2)", "crate": "beta", "features": "f2", "test": False},
    ]}

    @classmethod
    def _leg_calls(cls, gh_calls):
        """The per-leg `opt-in <name>` POSTs only (excludes the head-SHA summary)."""
        return [c for c in gh_calls if c["name"] != cls.SUMMARY_NAME]

    @classmethod
    def _summary_calls(cls, gh_calls):
        return [c for c in gh_calls if c["name"] == cls.SUMMARY_NAME]

    def _result_file(self, entries, group="g01 alpha", schema=None):
        return {"schema": schema or self.SCHEMA, "group": group, "results": entries}

    def _run_report(self, files, gh_rc="0", gh_stderr="", valid=None,
                    group_jobs_result="failure", raw_files=None, drop_env=(),
                    fork_pr="false", trigger_run_id="424242"):
        import subprocess  # noqa: PLC0415

        with tempfile.TemporaryDirectory() as tmp:
            tmpdir = Path(tmp)
            results_dir = tmpdir / "fmg-results"
            for i, obj in enumerate(files):
                sub = results_dir / f"feature-matrix-results-{i}"
                sub.mkdir(parents=True)
                (sub / "results.json").write_text(
                    json.dumps(obj), encoding="utf-8")
            for i, raw in enumerate(raw_files or []):
                sub = results_dir / f"raw-{i}"
                sub.mkdir(parents=True)
                (sub / "results.json").write_text(raw, encoding="utf-8")
            results_dir.mkdir(parents=True, exist_ok=True)
            valid_file = tmpdir / "valid-legs.json"
            valid_file.write_text(json.dumps(valid or self.VALID), encoding="utf-8")
            gh_log = tmpdir / "gh.log"
            gh = tmpdir / "gh-stub"
            gh.write_text(
                "#!/bin/bash\n"
                f"cat >> {gh_log}\n"
                f"echo >> {gh_log}\n"
                "if [ -n \"${GH_STDERR:-}\" ]; then echo \"$GH_STDERR\" >&2; fi\n"
                "exit ${GH_RC:-0}\n",
                encoding="utf-8",
            )
            gh.chmod(0o755)
            env = {
                "PATH": "/usr/bin:/bin",
                "FMG_GH": str(gh),
                "GH_RC": gh_rc,
                "GH_STDERR": gh_stderr,
                "FMG_RESULTS_DIR": str(results_dir),
                "FMG_VALID_LEGS_FILE": str(valid_file),
                "REPO": "example/repo",
                "HEAD_SHA": "deadbeef",
                "DETAILS_URL": "https://example.invalid/run/1",
                "GROUP_JOBS_RESULT": group_jobs_result,
                "FMG_RETRY_SLEEP": "0",
                "FMG_FORK_PR": fork_pr,
                # finding 2: the triggering feature-matrix run id, embedded as the
                # summary check-run's external_id for the gate's stale-report defence.
                "TRIGGER_RUN_ID": trigger_run_id,
            }
            for key in drop_env:
                env.pop(key, None)
            if trigger_run_id == "":
                env.pop("TRIGGER_RUN_ID", None)
            proc = subprocess.run(
                [sys.executable, str(self.RUNNER), "--report"],
                capture_output=True,
                text=True,
                timeout=60,
                env=env,
            )
            gh_calls = (
                [json.loads(l) for l in gh_log.read_text().splitlines() if l.strip()]
                if gh_log.exists()
                else []
            )
            return proc, gh_calls

    def test_valid_results_post_terminal_check_runs(self):
        # The two-leg VALID set is COMPLETE here (both legs observed), so the
        # completeness check passes and the reporter greens.
        proc, gh_calls = self._run_report([
            self._result_file([{"name": "alpha (f1)", "failed_step": "clippy"}]),
            self._result_file([{"name": "beta (f2)", "failed_step": None}],
                              group="g02 beta"),
        ], group_jobs_result="success")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        legs = self._leg_calls(gh_calls)
        self.assertEqual([c["name"] for c in legs],
                         ["opt-in alpha (f1)", "opt-in beta (f2)"],
                         "every leg must get its byte-identical `opt-in <name>` run")
        self.assertEqual([c["conclusion"] for c in legs],
                         ["failure", "success"])
        for c in gh_calls:
            self.assertEqual(c["status"], "completed",
                             "check-runs must be TERMINAL (a pending one could "
                             "hold the polling ci-summary gate)")
            self.assertEqual(c["head_sha"], "deadbeef")
        for c in legs:
            self.assertTrue(c["name"].startswith("opt-in "),
                            "the reporter constructs the opt-in prefix itself")
        # Summary metadata comes from the SELECTED set, not the artifact.
        self.assertIn("-p alpha --features f1", legs[0]["output"]["summary"])
        # The head-SHA summary check-run gates the reporter's own verdict (finding 1).
        summary = self._summary_calls(gh_calls)
        self.assertEqual(len(summary), 1, "exactly one head-SHA summary check")
        self.assertEqual(summary[0]["conclusion"], "success")
        self.assertEqual(summary[0]["head_sha"], "deadbeef")
        # finding 2: the summary carries the triggering feature-matrix run id as its
        # external_id, so ci_summary_gate.py can correlate it to the current group run
        # and reject a stale same-SHA report from an earlier run.
        self.assertEqual(summary[0]["external_id"], "424242",
                         "the report must embed TRIGGER_RUN_ID as external_id")

    def test_unassembled_leg_name_is_refused_and_nothing_posts(self):
        # THE forgery test: an artifact naming a non-assembled leg — e.g. trying
        # to spoof the required gate context — must fail validation before any
        # POST happens (never PR-supplied free text).
        for forged in ("ci-summary / gate", "gate", "alpha (f1) ", "totally-new"):
            proc, gh_calls = self._run_report([
                self._result_file([{"name": forged, "failed_step": None}]),
            ])
            self.assertEqual(proc.returncode, 1, f"forged name {forged!r} must fail")
            self.assertEqual(self._leg_calls(gh_calls), [],
                             "no per-leg POST may happen on a forged artifact")
            # A forged name is a superset/mismatch — the reporter constructs NO
            # `opt-in <forged>` context; only a failing summary check may appear.
            for c in gh_calls:
                self.assertEqual(c["name"], self.SUMMARY_NAME)
                self.assertEqual(c["conclusion"], "failure")

    def test_malformed_artifacts_fail_closed_without_posting(self):
        cases = [
            # not JSON at all
            {"raw_files": ["not json {"]},
            # wrong schema tag
            {"files": [self._result_file(
                [{"name": "alpha (f1)", "failed_step": None}], schema="v0")]},
            # extra top-level key
            {"raw_files": [json.dumps({"schema": self.SCHEMA, "group": "g01 a",
                                       "results": [], "extra": 1})]},
            # entry with extra keys
            {"files": [self._result_file(
                [{"name": "alpha (f1)", "failed_step": None, "summary": "pwn"}])]},
            # failed_step outside the enum
            {"files": [self._result_file(
                [{"name": "alpha (f1)", "failed_step": "pwned"}])]},
            # hostile group id (annotation/markdown injection shape)
            {"files": [self._result_file(
                [{"name": "alpha (f1)", "failed_step": None}],
                group="x\n::error::pwn")]},
            # empty results list
            {"files": [self._result_file([])]},
        ]
        for case in cases:
            proc, gh_calls = self._run_report(case.get("files", []),
                                              raw_files=case.get("raw_files"))
            self.assertEqual(proc.returncode, 1, f"case {case!r} must fail closed")
            self.assertEqual(self._leg_calls(gh_calls), [],
                             f"case {case!r} must not POST any per-leg check-run")

    def test_duplicate_leg_across_artifacts_is_refused(self):
        # Duplicate sibling check-runs are the latest-run-confusion attack the
        # split exists to prevent — refuse them from the honest side too.
        proc, gh_calls = self._run_report([
            self._result_file([{"name": "alpha (f1)", "failed_step": None}]),
            self._result_file([{"name": "alpha (f1)", "failed_step": "build"}],
                              group="g02 alpha"),
        ])
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(self._leg_calls(gh_calls), [])

    # A single-leg valid set so completeness (finding 2) is satisfied and these
    # tests isolate the POST-failure semantics (finding 3).
    VALID_ONE = {"include": [
        {"name": "alpha (f1)", "crate": "alpha", "features": "f1", "test": True},
    ]}

    def test_fork_token_denial_degrades_to_warning_ONLY_when_fork_pr(self):
        # Finding 3 (EVENT-GATED): the fork-PR read-only-token denial degrades to a
        # warning ONLY when FMG_FORK_PR=true (server-derived). The group jobs' own
        # conclusions remain the gating signal.
        proc, gh_calls = self._run_report(
            [self._result_file([{"name": "alpha (f1)", "failed_step": None}])],
            gh_rc="1", gh_stderr=self.FORK_DENIAL, group_jobs_result="success",
            valid=self.VALID_ONE, fork_pr="true")
        self.assertEqual(proc.returncode, 0,
                         "on a FORK PR, a read-only-token denial must not fail the "
                         "reporter — the group jobs' own conclusions gate")
        self.assertIn("::warning", proc.stdout)
        # per-leg denial + summary denial, each deterministic (not retried).
        self.assertEqual(len(self._leg_calls(gh_calls)), 1,
                         "deterministic denial is not retried")

    def test_fork_denial_string_on_a_non_fork_run_fails_closed(self):
        # Finding 3: the IDENTICAL "Resource not accessible by integration" error on
        # a push / merge_group / same-repo PR (FMG_FORK_PR=false) is a REAL auth
        # regression and must FAIL CLOSED — the old event-blind logic wrongly
        # tolerated it.
        proc, gh_calls = self._run_report(
            [self._result_file([{"name": "alpha (f1)", "failed_step": None}])],
            gh_rc="1", gh_stderr=self.FORK_DENIAL, group_jobs_result="success",
            valid=self.VALID_ONE, fork_pr="false")
        self.assertEqual(proc.returncode, 1,
                         "the fork-denial string on a NON-fork run must fail closed "
                         "(a real auth regression, finding 3)")
        self.assertIn("::error", proc.stderr)

    def test_unexpected_api_failure_fails_closed_after_retries(self):
        # Auth regression / outage / rate limit / malformed request => the
        # reporter job REDs (finding 3) — never a silent warning.
        proc, gh_calls = self._run_report(
            [self._result_file([{"name": "alpha (f1)", "failed_step": None}])],
            gh_rc="1", gh_stderr="HTTP 502 Bad Gateway", group_jobs_result="success",
            valid=self.VALID_ONE)
        self.assertEqual(proc.returncode, 1,
                         "an unexpected Checks API failure must fail the reporter")
        # The per-leg POST retries 3x (bounded) before failing closed.
        self.assertEqual(len(self._leg_calls(gh_calls)), 3,
                         "bounded retry before failing closed")
        self.assertIn("::error", proc.stderr)

    def test_incomplete_results_fail_closed(self):
        # Finding 2: the observed leg set must EQUAL the selected set exactly. Here
        # the selected set has alpha + beta but only alpha is reported (a malicious
        # group job could omit a failed leg while exiting 0) — the reporter must
        # fail closed with a red summary check and post no phantom for the missing.
        proc, gh_calls = self._run_report(
            [self._result_file([{"name": "alpha (f1)", "failed_step": None}])],
            group_jobs_result="success")
        self.assertEqual(proc.returncode, 1,
                         "a missing selected leg is a completeness violation")
        summary = self._summary_calls(gh_calls)
        self.assertEqual(len(summary), 1)
        self.assertEqual(summary[0]["conclusion"], "failure")
        self.assertIn("completeness", summary[0]["output"]["summary"].lower())
        # The honest observed leg (alpha) is still posted for attribution.
        self.assertEqual([c["name"] for c in self._leg_calls(gh_calls)],
                         ["opt-in alpha (f1)"])

    def test_external_id_correlation_token_on_every_summary_path(self):
        """Finding 2: whichever verdict the reporter reaches, the head-SHA summary
        check-run must carry the triggering feature-matrix run id as its external_id
        (the gate's stale-report correlation token). Checks both a green and a
        fail-closed path, and that a DIFFERENT trigger id propagates verbatim."""
        # green complete path
        proc, gh_calls = self._run_report([
            self._result_file([{"name": "alpha (f1)", "failed_step": None}]),
            self._result_file([{"name": "beta (f2)", "failed_step": None}],
                              group="g02 beta"),
        ], group_jobs_result="success", trigger_run_id="777")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(self._summary_calls(gh_calls)[0]["external_id"], "777")
        # fail-closed (incomplete) path still stamps the token
        proc, gh_calls = self._run_report(
            [self._result_file([{"name": "alpha (f1)", "failed_step": None}])],
            group_jobs_result="success", trigger_run_id="888")
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(self._summary_calls(gh_calls)[0]["external_id"], "888")

    def test_absent_trigger_run_id_omits_external_id(self):
        """Backward-compat: with no TRIGGER_RUN_ID (e.g. a bootstrap run) the summary
        check carries NO external_id key — the gate then degrades to any-report
        matching rather than binding to a token that was never set."""
        proc, gh_calls = self._run_report([
            self._result_file([{"name": "alpha (f1)", "failed_step": None}]),
            self._result_file([{"name": "beta (f2)", "failed_step": None}],
                              group="g02 beta"),
        ], group_jobs_result="success", trigger_run_id="")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertNotIn("external_id", self._summary_calls(gh_calls)[0],
                         "no TRIGGER_RUN_ID => no external_id key on the summary")

    def test_zero_artifacts_with_successful_groups_is_an_inconsistency(self):
        proc, _ = self._run_report([], group_jobs_result="success")
        self.assertEqual(proc.returncode, 1,
                         "green groups + zero artifacts would silently drop every "
                         "per-leg check-run — must fail")
        proc, _ = self._run_report([], group_jobs_result="failure")
        self.assertEqual(proc.returncode, 0,
                         "failed groups gate via ci-summary; nothing to report")
        self.assertIn("::warning", proc.stdout)

    def test_missing_reporter_env_is_fatal(self):
        for key in ("FMG_RESULTS_DIR", "FMG_VALID_LEGS_FILE", "REPO", "HEAD_SHA"):
            proc, gh_calls = self._run_report(
                [self._result_file([{"name": "alpha (f1)", "failed_step": None}])],
                drop_env=(key,))
            self.assertEqual(proc.returncode, 2, f"missing {key} must be fatal")
            self.assertEqual(gh_calls, [])

    def test_real_assembled_legs_pass_reporter_validation(self):
        """End-to-end coherence: EXECUTION-mode output for real assembled legs
        must validate on the REPORT side against the real assembler output —
        pins the schema the two halves share across the trust boundary. Reports
        the FULL leg set so completeness (finding 2) is satisfied."""
        mod = _load_assembler()
        legs = mod.load_legs()
        entries = [{"name": leg["name"], "failed_step": None} for leg in legs]
        valid = {"include": [
            {"name": leg["name"], "crate": leg["crate"],
             "features": leg["features"], "test": leg["test"]}
            for leg in legs
        ]}
        proc, gh_calls = self._run_report(
            [self._result_file(entries, group="g01 real")],
            valid=valid, group_jobs_result="success")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        leg_calls = self._leg_calls(gh_calls)
        self.assertEqual({c["name"] for c in leg_calls},
                         {f"opt-in {leg['name']}" for leg in legs})
        self.assertEqual(len(self._summary_calls(gh_calls)), 1)


class TestTwoFileScopeRuleIsDocumented(unittest.TestCase):
    """[SONNET-4.6] issue #2384 — the leg/golden pairing must be stated where it is read.

    A task that adds/removes/RENAMES a leg but is decomposed with
    `.github/feature-matrix.d/<crate>.yml` as its ONLY permitted file is
    self-contradicting: the leg-name change it asks for also requires the golden update it
    forbids, and a worker that obeys the scope has to decline. (That is exactly what
    happened on sq-19f1i / sq-kf56o.) Edits that cannot move the name set stay single-file.
    The contract therefore lives in the fragment directory itself, and these tests keep it
    from being deleted or drifting.

    Pure file reads — no PyYAML, no assembler import.
    """

    README = FRAGMENT_DIR / "README.md"
    GOLDEN_REL = "scripts/tests/feature-matrix-legnames.golden.txt"
    REGEN_CMD = "python3 scripts/assemble-feature-matrix.py --names"

    def test_fragment_dir_readme_states_both_in_scope_paths(self):
        self.assertTrue(self.README.is_file(), f"missing {self.README}")
        text = self.README.read_text(encoding="utf-8")
        for needle in (".github/feature-matrix.d/<crate>.yml", self.GOLDEN_REL):
            self.assertIn(
                needle, text,
                f"{self.README} must name {needle} as an in-scope file for a "
                "leg-name-set change",
            )

    def test_fragment_dir_readme_gives_the_regeneration_command(self):
        text = self.README.read_text(encoding="utf-8")
        self.assertIn(
            f"{self.REGEN_CMD} > {self.GOLDEN_REL}", text,
            "the README must give the exact regeneration command, since the golden is "
            "regenerated and never hand-edited",
        )

    def test_every_fragment_points_at_the_golden(self):
        """A fragment author/decomposer reading ONLY their crate's file still sees the rule
        — including its name-set qualification, so a non-name edit is not over-scoped."""
        frags = sorted(FRAGMENT_DIR.glob("*.yml"))
        self.assertTrue(frags, "no fragment files found")
        missing = [
            f.name for f in frags
            if "feature-matrix-legnames.golden.txt" not in f.read_text(encoding="utf-8")
        ]
        self.assertEqual(
            missing, [],
            "these fragments never mention the gate-name golden, so a leg-name-set task "
            "scoped from them alone would forbid the golden update it requires. Add the "
            "SCOPE comment "
            f"(see any sibling fragment or {self.README}): {missing}",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
