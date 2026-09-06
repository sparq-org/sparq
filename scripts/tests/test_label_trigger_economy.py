#!/usr/bin/env python3
# [OPUS-5] sparq-org/sparq#5215 — INSPECTION test for the LABEL-TRIGGER CHECK-RUN
# ECONOMY of the PR-triggered heavy workflows. 🤖 SPARQ agent.
#
# THE PROBLEM THIS PINS. A workflow run started by a label event that every job
# then `if:`-guards off is NOT free: it still ANNOUNCES a `skipped` check-run for
# every JOB DEFINITION in each workflow the event started. (A job-level `if:` is
# evaluated before the matrix expands — which is why GitHub documents the `matrix`
# context as unavailable in `jobs.<id>.if`, and why #5215's counts show one row per
# skipped job whether or not it carries a matrix.) #5215 measured, on PR #5081:
# 538 of 639 check-runs on ONE head SHA were `skipped` no-ops, spread over ~13
# generations of the same four workflows, and reading that paginated set costs the
# agent-registry dispatcher 7 requests per PR. Those figures are QUOTED from the
# issue's measurement, not re-measured here. The workflows' own trigger notes record
# that the review pipeline flips `review:*`/`status:*` labels several times per
# round; each such state transition is a REMOVE+ADD PAIR, so subscribing to BOTH
# `labeled` and `unlabeled` doubled the number of guarded no-op generations.
#
# THE FIX AND ITS SOUNDNESS ARGUMENT. `unlabeled` was dropped from the
# `on.pull_request.types` of the label-triggered heavy workflows; `labeled` stays,
# so the `ci-full` / `bench-full` / `fuzz-full` opt-ins still trigger a fresh run
# the moment they are APPLIED. Not re-running when one is REMOVED is sound ONLY
# because every label these workflows read is a MONOTONE OPT-IN TO MORE WORK:
#   * `ci-full`    -> ci_select.py `--full` (mode=full, nothing selection-skipped),
#                     and escapes the draft-tier scope reduction;
#   * `bench-full` -> ADDS the well-known-suite toolchains to the bench lane;
#   * `fuzz-full`  -> opts BACK IN to the randomized fuzz budget / draft escape.
# Removing such a label can therefore only ever ask for LESS than the run already
# standing on that head SHA, so skipping the removal event leaves a strict SUPERSET
# of the required coverage — it can never under-test. (The narrower selection
# resumes on the next push, i.e. the `synchronize` event.)
#
# WHAT WOULD BREAK THE ARGUMENT, AND WHY THIS FILE EXISTS. A future label whose
# REMOVAL should ENABLE work — anything read as `!contains(...labels..., 'x')`, a
# `skip-ci`/`no-bench`-shaped opt-OUT — would make the missing `unlabeled` trigger
# an UNDER-TEST, silently. Nothing else in the repo would go red. So this suite
# pins BOTH halves of the change together:
#   T1  the trigger sets: `labeled` present, `unlabeled` absent, in each of the
#       five label-triggered heavy workflows;
#   T2  repo-wide: no workflow reintroduces `unlabeled` (an explicit, empty
#       allowlist makes any future exception a reviewable diff rather than a
#       silent regression of the check-run budget);
#   T3  MONOTONICITY: every `github.event.pull_request.labels.*.name` and
#       `github.event.label.name` reference in those workflows is a POSITIVE
#       membership test over {ci-full, bench-full, fuzz-full}. A negated form, or
#       any other label name, REDs — that is the tripwire for the under-test above.
#   T4  the parser is non-vacuous (it detects `unlabeled` and a negated label
#       condition in synthetic fixtures), and this suite is wired into CI.
#
# NOT claimed here: this removes a CLASS of no-op generations, not all of them. Each
# SURVIVING generation still announces one skipped row per job definition, and a
# `labeled` flip for a non-escape label is still a guarded no-op run. Removing that
# remainder needs an architectural change (the four heavy workflows would have to
# stop being *started* by irrelevant label events, which in GitHub Actions means
# reusable-workflow indirection and therefore a repo-wide check-run RENAME) — out of
# scope here and deliberately not attempted. Nothing about WHAT any lane verifies
# changes: no job `if:`, no matrix, and no `["labeled","unlabeled"]` guard was
# touched.
#
# Hermetic: stdlib only (no PyYAML, no network, no gh) so it runs anywhere.
# Run:  python3 scripts/tests/test_label_trigger_economy.py

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
DOCS_QUALITY = WORKFLOWS / "docs-quality.yml"

# The heavy PR-triggered workflows that subscribe to label events. Each one is a
# `ci-select.yml` caller (or, for vectorized-feature-off.yml, an in-step selector
# user) whose no-op label-flip runs were the #5215 check-run amplifier.
LABEL_TRIGGERED = (
    "ci.yml",
    "feature-matrix.yml",
    "bench.yml",
    "fuzz.yml",
    "vectorized-feature-off.yml",
)

# The ONLY labels these workflows may read, and every one of them is a monotone
# opt-in to MORE work. Adding a name here is a deliberate assertion that the label
# can never make a lane do LESS.
MONOTONE_OPT_IN_LABELS = ("ci-full", "bench-full", "fuzz-full")

# Deliberately EMPTY: no workflow currently needs the `unlabeled` PR event. A future
# workflow that genuinely does must add itself here, which makes the check-run cost
# a reviewed decision instead of an invisible one.
UNLABELED_ALLOWLIST: tuple[str, ...] = ()

_LABELS_REF = "github.event.pull_request.labels.*.name"
_LABEL_NAME_REF = "github.event.label.name"

# `contains(github.event.pull_request.labels.*.name, '<allowed>')`
_POSITIVE_LABELS_CONTAINS = re.compile(
    r"contains\(\s*"
    + re.escape(_LABELS_REF)
    + r"\s*,\s*'(?P<label>[^']+)'\s*\)"
)
# `contains(fromJSON('[...]'), github.event.label.name)` — membership of the fired
# label in a literal allow-list — or `github.event.label.name == '<allowed>'`.
_POSITIVE_LABEL_NAME_IN_SET = re.compile(
    r"contains\(\s*fromJSON\(\s*'(?P<set>\[[^']*\])'\s*\)\s*,\s*"
    + re.escape(_LABEL_NAME_REF)
    + r"\s*\)"
)
_POSITIVE_LABEL_NAME_EQ = re.compile(
    re.escape(_LABEL_NAME_REF) + r"\s*==\s*'(?P<label>[^']+)'"
)


# --------------------------------------------------------------------------- #
# Minimal structural helpers. PyYAML would flatten the dense comment blocks these
# workflows carry and is not installed in every environment, so we walk lines —
# the same approach as scripts/tests/test_mergequeue_cache_posture.py.
# --------------------------------------------------------------------------- #
def _lines(text: str) -> list[str]:
    return text.split("\n")


def _is_comment(line: str) -> bool:
    return line.lstrip().startswith("#")


def _on_block(text: str) -> list[str]:
    """The top-level `on:` block's lines, comments included."""
    lines = _lines(text)
    start = next(
        (i for i, l in enumerate(lines) if re.match(r'^(on|"on"):', l)), None
    )
    if start is None:
        return []
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line[0].isspace() and not _is_comment(line):
            end = i
            break
    return lines[start:end]


def pull_request_types(text: str) -> list[str] | None:
    """The `types:` list declared under `on.pull_request:`.

    None when the workflow has no `pull_request` trigger, or declares one with no
    explicit `types:` (which means GitHub's default set — opened/synchronize/
    reopened — and therefore no label events at all).

    Comment lines are skipped throughout: every one of these workflows carries a
    prose note that NAMES `unlabeled` while explaining why it is not a trigger, and
    counting that text would invert this suite's verdict.
    """
    block = _on_block(text)
    pr_at = None
    pr_indent = 0
    for i, line in enumerate(block):
        if _is_comment(line):
            continue
        m = re.match(r"^(\s+)pull_request:\s*$", line)
        if m:
            pr_at = i
            pr_indent = len(m.group(1))
            break
    if pr_at is None:
        return None
    for line in block[pr_at + 1 :]:
        if not line.strip() or _is_comment(line):
            continue
        indent = len(line) - len(line.lstrip())
        if indent <= pr_indent:
            break  # left the pull_request mapping
        m = re.match(r"^\s+types:\s*\[(?P<items>[^\]]*)\]\s*$", line)
        if m:
            return [t.strip() for t in m.group("items").split(",") if t.strip()]
    return None


def code_lines(text: str) -> list[tuple[int, str]]:
    """Every non-comment line, as (1-based line number, text)."""
    return [
        (n, l) for n, l in enumerate(_lines(text), start=1) if not _is_comment(l)
    ]


def label_reference_violations(text: str) -> list[str]:
    """Non-monotone label reads: every one is a reason `unlabeled` must return.

    A reference is CLEAN iff it is a positive membership test over
    MONOTONE_OPT_IN_LABELS. Anything else — a negation, an unlisted label name, a
    reference shape this parser does not recognise — is reported, because the
    soundness of dropping the `unlabeled` trigger rests entirely on "removing a
    label can only ever ask for LESS work".
    """
    violations: list[str] = []
    for lineno, line in code_lines(text):
        # 1. `contains(labels.*.name, '<x>')` — every occurrence must be positive
        #    and name an allow-listed label.
        if _LABELS_REF in line:
            matches = list(_POSITIVE_LABELS_CONTAINS.finditer(line))
            if len(matches) != line.count(_LABELS_REF):
                violations.append(
                    f"line {lineno}: `{_LABELS_REF}` is read in an unrecognised "
                    f"shape (expected `contains({_LABELS_REF}, '<label>')`): {line.strip()}"
                )
            for m in matches:
                if m.group("label") not in MONOTONE_OPT_IN_LABELS:
                    violations.append(
                        f"line {lineno}: label '{m.group('label')}' is not a declared "
                        f"monotone opt-in {MONOTONE_OPT_IN_LABELS}"
                    )
                if line[: m.start()].rstrip().endswith("!"):
                    violations.append(
                        f"line {lineno}: NEGATED label read `!contains(...'"
                        f"{m.group('label')}')` — removing that label would ENABLE "
                        f"work, so the `unlabeled` trigger cannot be dropped"
                    )
        # 2. `github.event.label.name` — the label that FIRED this event. Allowed
        #    only as positive membership over the same allow-list.
        if _LABEL_NAME_REF in line:
            named: list[str] = []
            ok = 0
            for m in _POSITIVE_LABEL_NAME_IN_SET.finditer(line):
                ok += 1
                named.extend(re.findall(r"'?\"([^\"]+)\"'?", m.group("set")))
                if line[: m.start()].rstrip().endswith("!"):
                    violations.append(
                        f"line {lineno}: NEGATED `!contains(fromJSON(...), "
                        f"{_LABEL_NAME_REF})` — a label whose REMOVAL enables work"
                    )
            for m in _POSITIVE_LABEL_NAME_EQ.finditer(line):
                ok += 1
                named.append(m.group("label"))
            if ok != line.count(_LABEL_NAME_REF):
                violations.append(
                    f"line {lineno}: `{_LABEL_NAME_REF}` is read in an unrecognised "
                    f"shape: {line.strip()}"
                )
            for label in named:
                if label not in MONOTONE_OPT_IN_LABELS:
                    violations.append(
                        f"line {lineno}: label '{label}' is not a declared monotone "
                        f"opt-in {MONOTONE_OPT_IN_LABELS}"
                    )
    return violations


class TestLabelTriggerSets(unittest.TestCase):
    """T1/T2 — the trigger sets themselves."""

    def test_heavy_workflows_still_trigger_on_labeled(self) -> None:
        # The `ci-full`/`bench-full`/`fuzz-full` opt-ins are worthless if APPLYING
        # them does not start a run. This is the half of the contract #5215 must
        # not have broken.
        for name in LABEL_TRIGGERED:
            with self.subTest(workflow=name):
                types = pull_request_types((WORKFLOWS / name).read_text())
                self.assertIsNotNone(
                    types, f"{name}: expected an explicit on.pull_request.types list"
                )
                self.assertIn(
                    "labeled",
                    types,
                    f"{name}: dropping `labeled` would make the ci-full/bench-full/"
                    f"fuzz-full opt-ins unable to start a run",
                )

    def test_heavy_workflows_do_not_trigger_on_unlabeled(self) -> None:
        for name in LABEL_TRIGGERED:
            with self.subTest(workflow=name):
                types = pull_request_types((WORKFLOWS / name).read_text())
                self.assertNotIn(
                    "unlabeled",
                    types or [],
                    f"{name}: `unlabeled` doubles the guarded no-op runs (#5215) and "
                    f"buys nothing while every label read here is a monotone opt-in",
                )

    def test_no_workflow_reintroduces_unlabeled(self) -> None:
        offenders = []
        for path in sorted(WORKFLOWS.glob("*.yml")):
            if path.name in UNLABELED_ALLOWLIST:
                continue
            types = pull_request_types(path.read_text())
            if types and "unlabeled" in types:
                offenders.append(path.name)
        self.assertEqual(
            [],
            offenders,
            "these workflows subscribe to `unlabeled`, which costs one skipped "
            "check-run per job definition on every label REMOVAL (#5215). Add the "
            "workflow to UNLABELED_ALLOWLIST with a reason if it genuinely needs it: "
            f"{offenders}",
        )


class TestLabelReadsAreMonotone(unittest.TestCase):
    """T3 — the invariant that makes T1/T2 sound."""

    def test_every_label_read_is_a_positive_opt_in(self) -> None:
        for name in LABEL_TRIGGERED:
            with self.subTest(workflow=name):
                violations = label_reference_violations(
                    (WORKFLOWS / name).read_text()
                )
                self.assertEqual(
                    [],
                    violations,
                    f"{name}: a label read that is not a monotone opt-in breaks the "
                    f"soundness argument for dropping the `unlabeled` trigger — "
                    f"either make it monotone or restore `unlabeled` for this "
                    f"workflow via UNLABELED_ALLOWLIST.\n  "
                    + "\n  ".join(violations),
                )

    def test_ci_select_still_reads_the_ci_full_label(self) -> None:
        # Non-vacuity for the suite as a whole: if the `ci-full` override ever
        # stopped being read, every assertion above would pass over a workflow set
        # that no longer has the behaviour this file reasons about.
        text = (WORKFLOWS / "ci-select.yml").read_text()
        self.assertIn(
            f"contains({_LABELS_REF}, 'ci-full')",
            text,
            "ci-select.yml no longer reads the ci-full label — the monotone-opt-in "
            "premise of this suite needs re-deriving",
        )


class TestParserNonVacuity(unittest.TestCase):
    """T4a — the parser can actually see what it claims to reject."""

    _WITH_UNLABELED = (
        "name: fixture\n"
        "on:\n"
        "  # prose that merely MENTIONS unlabeled must not count\n"
        "  pull_request:\n"
        "    types: [opened, synchronize, labeled, unlabeled]\n"
        "jobs: {}\n"
    )
    _WITHOUT_UNLABELED = (
        "name: fixture\n"
        "on:\n"
        "  # this comment names unlabeled on purpose\n"
        "  pull_request:\n"
        "    types: [opened, synchronize, labeled]\n"
        "jobs: {}\n"
    )
    _NO_TYPES = "name: fixture\non:\n  pull_request:\n  push:\njobs: {}\n"

    def test_detects_unlabeled_in_a_types_list(self) -> None:
        self.assertIn("unlabeled", pull_request_types(self._WITH_UNLABELED))

    def test_a_comment_naming_unlabeled_is_not_a_trigger(self) -> None:
        # Every workflow this suite guards carries exactly such a comment.
        self.assertEqual(
            ["opened", "synchronize", "labeled"],
            pull_request_types(self._WITHOUT_UNLABELED),
        )

    def test_absent_types_list_reads_as_no_label_events(self) -> None:
        self.assertIsNone(pull_request_types(self._NO_TYPES))

    def test_detects_a_negated_label_read(self) -> None:
        bad = f"    if: \"!contains({_LABELS_REF}, 'ci-full')\"\n"
        self.assertTrue(
            any("NEGATED" in v for v in label_reference_violations(bad)),
            "the monotonicity check failed to flag a negated label read",
        )

    def test_detects_an_undeclared_label(self) -> None:
        bad = f"    if: contains({_LABELS_REF}, 'skip-ci')\n"
        violations = label_reference_violations(bad)
        self.assertTrue(
            any("skip-ci" in v for v in violations),
            "the monotonicity check failed to flag an undeclared label",
        )

    def test_accepts_the_real_guard_shapes(self) -> None:
        good = (
            "    if: >-\n"
            "      github.event_name != 'pull_request' ||\n"
            "      !contains(fromJSON('[\"labeled\",\"unlabeled\"]'), github.event.action) ||\n"
            f"      {_LABEL_NAME_REF} == 'ci-full'\n"
            f"    X: ${{{{ contains(fromJSON('[\"ci-full\",\"bench-full\"]'), {_LABEL_NAME_REF}) }}}}\n"
            f"    Y: ${{{{ contains({_LABELS_REF}, 'fuzz-full') }}}}\n"
        )
        self.assertEqual([], label_reference_violations(good))


class TestSuiteIsWiredIntoCi(unittest.TestCase):
    """T4b — an unrun guard guards nothing."""

    def test_docs_quality_invokes_this_suite(self) -> None:
        self.assertIn(
            "python3 scripts/tests/test_label_trigger_economy.py",
            DOCS_QUALITY.read_text(),
            "docs-quality.yml must run this suite, or the label-trigger economy "
            "regresses silently",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
