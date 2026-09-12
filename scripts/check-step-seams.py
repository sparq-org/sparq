#!/usr/bin/env python3
# [OPUS-5] sparq-org/sparq#5371 — the STEP-level YAML seam gate. 🤖 SPARQ agent.
#
# THE HOLE THIS CLOSES. The repo has strong protection at the JOB level (the required
# `ci-summary / gate` aggregator plus `.github/advisory-registry.json`, enforced by
# scripts/check-advisory-registry.py) and strong protection INSIDE the Python modules
# (mutation-checked unit tests). Nothing generic sat between them, at the level of an
# individual workflow STEP.
#
# Concretely, `.github/workflows/triage-area.yml` runs the classifier's two hermetic
# suites as the pre-apply guard before the half-hourly `--apply` cron mutates live issue
# labels. DELETING that step — or giving it `continue-on-error: true`, or `if: false` —
# leaves the job green, the aggregator green and every Python test still passing, while
# the classifier applies unverified `area:` labels to live issues with nothing red
# anywhere. The #4567 reviewer measured the shape: 18/18 Python mutants died, and EVERY
# surviving mutant lived in a workflow `if:`, a step, or a call site.
#
# The repo's answer so far has been one hand-written unittest per lane (the YAML-seam
# classes in test_readiness_visibility.py, test_release_publish_guard.py,
# test_bench_hardzone_wiring.py, test_ci_execution_latency_alarm.py, …). Those work, but
# they are opt-in prose: a NEW safety step is covered only if someone remembers to write
# a new test class for it. This checker is the DECLARATIVE mechanism — pinning a seam is
# a JSON entry — plus one sweep that needs no declaration at all.
#
# FIVE DIRECTIONS over .github/workflows/*.yml + .github/step-seam-registry.json:
#
#  S1 IDENTITY   — every declared seam must BIND: the `workflow` file exists, it has a
#                  job with that `job_id`, and that job has a step whose `name:` is
#                  EXACTLY `step_name`. Deleting the step, renaming it, renaming the job
#                  or moving it to another workflow all RED here. (Fail-closed in the
#                  useful direction: an un-bindable declaration is an offence, never a
#                  silent skip — that is the failure mode C4 of the advisory registry was
#                  added to fix, and it applies verbatim one level down.)
#
#  S2 NO-SWALLOW — a declared seam's step must carry NO `continue-on-error` key at all,
#                  and no `if:` other than the exact expression the entry pins in
#                  `step_if`. The same two rules apply to the ENCLOSING JOB (`job_if`):
#                  `if: false` on the job disables the step just as completely, and
#                  job-level `continue-on-error` hides the step's failure from the
#                  aggregator. ABSENCE is required rather than falsity because
#                  `continue-on-error: ${{ … }}` cannot be statically shown to be false
#                  — the four spellings `true`, `'true'`, `"true"` and `${{ true }}` all
#                  survived a looser reading in PR #4192's review.
#
#  S3 BODY       — every fragment in the entry's `must_run` must appear in the step's
#                  `run:` text AFTER shell comments are stripped. S1 catches a deleted
#                  step; S3 catches a GUTTED one — dropping `--apply`, or commenting out
#                  one of the two self-test invocations, leaves a green step with the
#                  right name doing less than it says.
#
#  S4 SWEEP      — repo-wide and needing NO declaration: no step that invokes a
#                  GATE-CLASSIFIED command may carry a truthy `continue-on-error` unless
#                  its job is DECLARED advisory in .github/advisory-registry.json, or the
#                  seam registry carries an explicit `swallow_waivers` entry for it. Gate
#                  classification is not re-implemented here — it is imported from
#                  scripts/check-advisory-registry.py, so the two checkers share ONE
#                  definition of "this command fails the build on a violation" (python,
#                  shell, node and npm aliases resolved through package.json) and cannot
#                  drift apart. This is the half that covers steps nobody has pinned yet.
#
#  S5 VACUITY    — the registry must be non-empty and every entry must carry all of
#                  {workflow, job_id, step_name, why, owner_issue, registered}; and the
#                  S4 sweep must have classified at least one gate command across the
#                  live workflows. A structural checker whose parser silently stops
#                  matching reports a confident all-clear forever; S5 is what makes that
#                  RED instead. (Measured on the tree this landed against: the sweep sees
#                  gate-classified commands in dozens of steps, so the guard is not
#                  trivially satisfied.)
#
# SCOPE, HONESTLY. This is a STATIC check over the checked-in YAML. It cannot know
# whether a step's command is the right one, and it does not attempt to enumerate every
# load-bearing step in the repo — the registry is a pinned set that grows by declaration.
# What it does guarantee is that a pinned seam cannot be deleted, renamed, disabled,
# swallowed or gutted without a red gate, and that no gate-classified step anywhere can
# acquire `continue-on-error: true` silently.
#
# USAGE
#   check-step-seams.py              # check the live workflows (default)
#   check-step-seams.py --root DIR   # use DIR as repo root
#   check-step-seams.py --self-test  # hermetic mutation table, no repo access
#
# Stdlib only, plus the sibling classifier import. No PyYAML: the same
# indentation-driven parsing check-advisory-registry.py uses, extended one level down
# from jobs to steps.

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

REGISTRY_PATH = Path(".github") / "step-seam-registry.json"
ADVISORY_REGISTRY_PATH = Path(".github") / "advisory-registry.json"

# Every seam declaration must carry all of these. `why` and `owner_issue` are as
# required as the identity triple on purpose: an entry nobody can attribute is an entry
# nobody will ever knowingly retire, and the registry decays into noise a future author
# deletes wholesale. Same discipline as the advisory registry's REQUIRED_FIELDS.
REQUIRED_SEAM_FIELDS = ("workflow", "job_id", "step_name", "why", "owner_issue", "registered")
REQUIRED_WAIVER_FIELDS = ("workflow", "job_id", "step_name", "why", "owner_issue", "registered")

# A step block's own keys sit at exactly 8 spaces once the `- ` list marker is
# normalised away; `with:`/`env:` sub-keys and block-scalar bodies are deeper.
_STEP_KEY_RE = re.compile(r"^ {8}([A-Za-z][\w-]*):\s*(.*)$")
_BLOCK_SCALAR_HEADS = ("|", ">", "|-", ">-", "|+", ">+")


# ---------------------------------------------------------------------------
# Gate classification — imported, never re-implemented
# ---------------------------------------------------------------------------

def load_classifier(scripts_dir: Path | None = None):
    """Import scripts/check-advisory-registry.py for its workflow parser + classifier.

    S4's whole value is that it applies the SAME "is this a gate command" rule the
    advisory registry's C3 applies, one level down. Re-implementing the rule here would
    let the two definitions drift, and a drift in this direction is invisible: the sweep
    would simply stop seeing a class of gate and keep printing `all clear`. A failure to
    import is therefore fatal, not degraded operation.
    """
    path = (scripts_dir or Path(__file__).resolve().parent) / "check-advisory-registry.py"
    spec = importlib.util.spec_from_file_location("_sparq_advisory_registry", path)
    if spec is None or spec.loader is None:      # pragma: no cover — unreachable if the file exists
        raise ImportError(f"cannot load the gate classifier from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# ---------------------------------------------------------------------------
# Step-level parsing (stdlib-only, one level below parse_jobs)
# ---------------------------------------------------------------------------

def _unquote(value: str) -> str:
    """Read a YAML scalar the way GitHub Actions does: quotes, then comments.

    ORDER IS LOAD-BEARING and was wrong here first time round. The repo's step names
    routinely carry an issue reference, and the two spellings parse DIFFERENTLY:

      name: "Self-test ci-summary #3505 mutations (…)"   -> the # is inside quotes, so it
                                                            is part of the name
      name: Self-test ci-free-disk.sh (… guards + #462 fold)
                                                         -> a plain scalar ends at the
                                                            first ` #`, so YAML (and
                                                            therefore GitHub) sees the
                                                            name TRUNCATED at `+`

    Stripping the comment first mangles the quoted form into `"Self-test ci-summary`,
    which matches nothing — a seam pinned on such a step could never bind. Handling the
    quote first reproduces both cases exactly, so a registry entry can be written against
    the name GitHub actually renders.
    """
    value = value.strip()
    if value[:1] in ('"', "'"):
        quote = value[0]
        i = 1
        while i < len(value):
            if quote == "'" and value.startswith("''", i):
                i += 2                                  # YAML's escaped single quote
                continue
            if quote == '"' and value[i] == "\\":
                i += 2                                  # a backslash escape in a double-quoted scalar
                continue
            if value[i] == quote:
                inner = value[1:i]
                return inner.replace("''", "'") if quote == "'" else inner
            i += 1
        return value[1:]                                # unterminated quote: best effort
    return re.split(r"\s+#", value, maxsplit=1)[0].strip()


def job_fields(block_text: str) -> dict[str, str]:
    """Job-level scalar keys (4-space indent) — `if:` and `continue-on-error:`.

    Deliberately scalar-only: a block-scalar `if:` is not idiomatic and a nested mapping
    is not a value S2 reasons about. A key present with an unreadable value still lands
    in the dict (as the raw text), so it can never read as absent.
    """
    fields: dict[str, str] = {}
    for match in re.finditer(r"^ {4}(if|continue-on-error):\s*(.*)$", block_text, re.MULTILINE):
        fields.setdefault(match.group(1), _unquote(match.group(2)))
    return fields


def parse_steps(block_text: str) -> list[dict]:
    """Split a job block into its steps.

    Each dict has `name` (the `name:` value, or "" for an unnamed step), `fields` (the
    step's own scalar keys) and `text` (the raw step block, for run-command extraction).

    The steps region is bounded at the first subsequent line indented <= 4, so a
    job-level key written AFTER `steps:` cannot pull unrelated list items (a `needs:`
    entry, a `strategy.matrix` row) into the step set.
    """
    lines = block_text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if re.match(r"^ {4}steps:\s*(?:#.*)?$", line):
            start = i + 1
            break
    if start is None:
        return []                                   # a `uses:` (reusable-workflow) job has no steps

    end = len(lines)
    for i in range(start, len(lines)):
        line = lines[i]
        if not line.strip():
            continue
        if len(line) - len(line.lstrip()) <= 4:
            end = i
            break
    region = lines[start:end]

    marks = [i for i, line in enumerate(region) if line.startswith("      - ")]
    steps: list[dict] = []
    for n, first in enumerate(marks):
        last = marks[n + 1] if n + 1 < len(marks) else len(region)
        text = "\n".join(region[first:last])
        fields = _step_fields(region[first:last])
        steps.append({"name": fields.get("name", ""), "fields": fields, "text": text})
    return steps


def _step_fields(step_lines: list[str]) -> dict[str, str]:
    """Scalar keys of ONE step, with block scalars and nested mappings consumed whole.

    Normalising the `- ` marker to two spaces puts the first key at the same indent as
    every continuation key, so `- name: x` and a following `  if: false` are read
    uniformly. Anything deeper than 8 spaces belongs to a `with:`/`env:` mapping or a
    `run: |` body and is skipped — that is what stops a `continue-on-error` mentioned
    inside a shell heredoc from reading as the step's own key.
    """
    lines = list(step_lines)
    if lines and lines[0].startswith("      - "):
        lines[0] = "        " + lines[0][8:]

    fields: dict[str, str] = {}
    i = 0
    while i < len(lines):
        match = _STEP_KEY_RE.match(lines[i])
        if not match:
            i += 1
            continue
        key, inline = match.group(1), match.group(2).strip()
        i += 1
        if inline and inline not in _BLOCK_SCALAR_HEADS:
            # `run:`/`if:` inline values keep their text; a `#` inside a shell command is
            # a shell comment, not a YAML one, so only the scalar keys S2 reads are
            # comment-stripped.
            fields.setdefault(key, inline if key == "run" else _unquote(inline))
            continue
        body: list[str] = []
        while i < len(lines):
            line = lines[i]
            if line.strip() and (len(line) - len(line.lstrip())) <= 8:
                break
            body.append(line)
            i += 1
        fields.setdefault(key, "\n".join(body))
    return fields


def _is_truthy_swallow(value: str) -> bool:
    """Is this `continue-on-error:` value anything other than a literal false?

    `false` is the one spelling that provably swallows nothing. Everything else —
    including `${{ inputs.soft }}`, whose runtime value is unknowable here — counts.
    """
    return _unquote(value).strip().lower() not in ("false", "'false'", '"false"')


# ---------------------------------------------------------------------------
# The five checks
# ---------------------------------------------------------------------------

def check_seams(workflows: dict[str, str], registry: dict, advisory_jobs: dict,
                classifier) -> list[str]:
    """Run S1–S5 over already-loaded workflow texts. Returns offence strings."""
    offences: list[str] = []
    npm_scripts = getattr(classifier, "_npm_scripts_for_check", {})

    # Index every job and step once: (workflow, job_id) -> {name, fields, steps}.
    live: dict[tuple[str, str], dict] = {}
    for filename, text in sorted(workflows.items()):
        for job in classifier.parse_jobs(text):
            live[(filename, job["id"])] = {
                "name": job["name"],
                "fields": job_fields(job["block_text"]),
                "steps": parse_steps(job["block_text"]),
            }

    seams = registry.get("seams") or []
    waivers = registry.get("swallow_waivers") or []

    # ---- S5a: the declarations themselves must be well-formed ----
    if not seams:
        offences.append(
            "S5: .github/step-seam-registry.json declares no seams. An empty registry "
            "makes S1-S3 vacuous — every pinned-step guarantee this gate advertises "
            "would hold trivially. Declare the load-bearing steps or delete the gate."
        )
    for label, entries, required in (("seam", seams, REQUIRED_SEAM_FIELDS),
                                     ("swallow_waiver", waivers, REQUIRED_WAIVER_FIELDS)):
        for index, entry in enumerate(entries):
            if not isinstance(entry, dict):
                offences.append(f"S5: {label} #{index} is not an object")
                continue
            missing = [field for field in required if not entry.get(field)]
            if missing:
                offences.append(
                    f"S5: {label} #{index} ({entry.get('step_name', '?')!r}) is missing "
                    f"required field(s) {missing}. `why`/`owner_issue`/`registered` are as "
                    f"required as the identity triple: an entry nobody can attribute is one "
                    f"nobody can ever knowingly retire."
                )

    # ---- S1/S2/S3: the declared seams ----
    for entry in seams:
        if not isinstance(entry, dict):
            continue
        workflow = entry.get("workflow") or ""
        job_id = entry.get("job_id") or ""
        step_name = entry.get("step_name") or ""
        if not (workflow and job_id and step_name):
            continue                                # already reported by S5

        where = f"{workflow}:{job_id}:{step_name!r}"
        job = live.get((workflow, job_id))
        if job is None:
            offences.append(
                f"S1: seam {where} does not bind — {workflow!r} has no job {job_id!r} "
                f"(the file may be gone, or the job renamed). A declaration that binds to "
                f"nothing polices nothing: fix the pair, or delete the entry if the seam "
                f"is genuinely retired."
            )
            continue

        step = next((s for s in job["steps"] if s["name"] == step_name), None)
        if step is None:
            present = [s["name"] for s in job["steps"] if s["name"]]
            offences.append(
                f"S1: seam {where} is GONE — {workflow}:{job_id} has no step with that "
                f"exact name. This is the deletion/rename the seam registry exists to "
                f"catch: the job stays green and the aggregator stays green without it. "
                f"Steps present: {present}"
            )
            continue

        # S2 — step level.
        if "continue-on-error" in step["fields"]:
            offences.append(
                f"S2: seam {where} carries `continue-on-error: "
                f"{step['fields']['continue-on-error']}`. A pinned safety step must fail "
                f"its job; absence is required rather than a false value because "
                f"`${{{{ … }}}}` cannot be statically shown false. Remove the key, or "
                f"retire the seam deliberately by deleting its registry entry."
            )
        expected_step_if = entry.get("step_if")
        actual_step_if = step["fields"].get("if")
        if actual_step_if != expected_step_if and not (expected_step_if is None and actual_step_if is None):
            offences.append(
                f"S2: seam {where} has step `if: {actual_step_if!r}` but its entry pins "
                f"step_if={expected_step_if!r}. `if: false` — or any condition that is "
                f"false in practice — makes the step skip while the job and the gate stay "
                f"green. Pin the new expression in `step_if` if it is intended."
            )

        # S2 — job level. A job-level `if:` or `continue-on-error:` disables or hides the
        # step just as completely, so the seam is only as pinned as its job.
        if "continue-on-error" in job["fields"]:
            offences.append(
                f"S2: the job carrying seam {where} has `continue-on-error: "
                f"{job['fields']['continue-on-error']}`, so the step's failure never reds "
                f"the job's check-run and the seam guards nothing."
            )
        expected_job_if = entry.get("job_if")
        actual_job_if = job["fields"].get("if")
        if actual_job_if != expected_job_if and not (expected_job_if is None and actual_job_if is None):
            offences.append(
                f"S2: the job carrying seam {where} has `if: {actual_job_if!r}` but its "
                f"entry pins job_if={expected_job_if!r}. A job-level condition skips every "
                f"step inside it, the seam included. Pin the new expression in `job_if` if "
                f"it is intended."
            )

        # S3 — the step must still DO what it is pinned for.
        must_run = entry.get("must_run") or []
        if must_run:
            commands = "\n".join(classifier.extract_run_commands(step["text"]))
            for fragment in must_run:
                if fragment not in commands:
                    offences.append(
                        f"S3: seam {where} no longer runs {fragment!r}. The step is still "
                        f"there and still green, but it has been gutted — commenting an "
                        f"invocation out or dropping a flag is the same outcome as deleting "
                        f"the step, minus the evidence."
                    )

    # ---- S4: the repo-wide swallow sweep ----
    waived = {(w.get("workflow"), w.get("job_id"), w.get("step_name")) for w in waivers
              if isinstance(w, dict)}
    gate_steps_seen = 0
    for (filename, job_id), job in sorted(live.items()):
        job_declared_advisory = job["name"] in advisory_jobs
        for step in job["steps"]:
            hits = classifier.find_gate_scripts_in_block(step["text"], npm_scripts)
            if not hits:
                continue
            gate_steps_seen += 1
            swallow = step["fields"].get("continue-on-error")
            if swallow is None or not _is_truthy_swallow(swallow):
                continue
            if job_declared_advisory or (filename, job_id, step["name"]) in waived:
                continue
            offences.append(
                f"S4: {filename}:{job_id}:{step['name']!r} runs gate-classified command(s) "
                f"{hits} with `continue-on-error: {swallow}`. The step reports its failure "
                f"and the job still concludes success, so the gate it implements enforces "
                f"nothing. Either drop `continue-on-error`, or make the demotion VISIBLE: "
                f"declare the job in .github/advisory-registry.json (a gate allowed to fail "
                f"IS advisory), or add a `swallow_waivers` entry here when only this one "
                f"step is best-effort inside an otherwise-gating job."
            )

    # ---- S5b: the sweep must not have gone blind ----
    if workflows and gate_steps_seen == 0:
        offences.append(
            "S5: the S4 sweep classified NO gate command in any step across "
            f"{len(workflows)} workflow file(s). The repo runs many; seeing none means the "
            "step parser or the imported classifier stopped matching, and a blind sweep "
            "reports a confident all-clear forever."
        )
    return offences


# ---------------------------------------------------------------------------
# Disk loading
# ---------------------------------------------------------------------------

def load_workflows(root: Path) -> dict[str, str]:
    directory = root / ".github" / "workflows"
    if not directory.is_dir():
        return {}
    workflows: dict[str, str] = {}
    for path in sorted(directory.iterdir()):
        if path.suffix not in (".yml", ".yaml"):
            continue
        try:
            workflows[path.name] = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
    return workflows


def _load_json(path: Path, key: str | None = None):
    """Read a JSON registry. A missing/broken file yields an empty mapping, which S5
    then reports as an offence — never a silent pass."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return {}
    if key is not None:
        value = data.get(key)
        return value if isinstance(value, dict) else {}
    return data if isinstance(data, dict) else {}


def check_repo(root: Path) -> list[str]:
    classifier = load_classifier(root / "scripts")
    classifier._npm_scripts_for_check = classifier.load_npm_scripts(root)
    workflows = load_workflows(root)
    if not workflows:
        return [f"S5: no workflow files found under {root}/.github/workflows"]
    registry = _load_json(root / REGISTRY_PATH)
    advisory_jobs = _load_json(root / ADVISORY_REGISTRY_PATH, key="jobs")
    return check_seams(workflows, registry, advisory_jobs, classifier)


# ---------------------------------------------------------------------------
# Self-test — a mutation table, not a smoke test
# ---------------------------------------------------------------------------

_FIXTURE_WORKFLOW = """\
name: cron-lane

on:
  schedule:
    - cron: '25,55 * * * *'

permissions:
  issues: write

jobs:
  classify:
    name: clear the park
    runs-on: ubuntu-latest
    env:
      GH_TOKEN: ${{ github.token }}
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
      - name: Self-test before it writes
        run: |
          set -euo pipefail
          python3 scripts/triage-area.py --self-test
          python3 scripts/tests/test_triage_area.py
      - name: Apply
        run: python3 scripts/triage-area.py --apply
"""

# A second file so the S4 sweep has a gate-classified step to see in every case — its
# absence is itself an offence (S5b), which would otherwise mask the mutants below.
_FIXTURE_GATE_WORKFLOW = """\
name: lints

on: [pull_request]

jobs:
  lint:
    name: lints
    runs-on: ubuntu-latest
    steps:
      - name: Enforce the ratchet
        run: python3 scripts/coverage-gate.py
"""

_FIXTURE_SEAM = {
    "workflow": "cron-lane.yml",
    "job_id": "classify",
    "step_name": "Self-test before it writes",
    "must_run": ["scripts/triage-area.py --self-test", "scripts/tests/test_triage_area.py"],
    "why": "fixture",
    "owner_issue": 5371,
    "registered": "2026-09-01",
}


def _fixture_registry(**overrides) -> dict:
    seam = dict(_FIXTURE_SEAM)
    seam.update(overrides)
    return {"seams": [seam]}


def self_test() -> int:
    """Every mutant below must produce at least one offence; the baselines exactly none.

    The table is the point of the file: a structural checker that is never shown a
    mutation it fails to catch is indistinguishable from one that always prints `all
    clear`. Each case names the real-world edit it stands for.
    """
    classifier = load_classifier()
    classifier._npm_scripts_for_check = {}

    def run(workflow: str, registry: dict, advisory: dict | None = None,
            gate_workflow: str = _FIXTURE_GATE_WORKFLOW) -> list[str]:
        return check_seams({"cron-lane.yml": workflow, "lints.yml": gate_workflow},
                           registry, advisory or {}, classifier)

    clean = _fixture_registry()
    cases: list[tuple[str, list[str], int, str]] = [
        ("baseline: the seam is present, unconditional and intact",
         run(_FIXTURE_WORKFLOW, clean), 0,
         "a well-formed registry over a well-formed workflow must be silent"),

        ("S1: the pinned step is DELETED",
         run(_FIXTURE_WORKFLOW.replace(
             "      - name: Self-test before it writes\n"
             "        run: |\n"
             "          set -euo pipefail\n"
             "          python3 scripts/triage-area.py --self-test\n"
             "          python3 scripts/tests/test_triage_area.py\n", ""), clean), 1,
         "#5371's headline mutant: delete the pre-apply guard, job stays green"),

        ("S1: the pinned step is RENAMED",
         run(_FIXTURE_WORKFLOW.replace("Self-test before it writes", "Self-test"), clean), 1,
         "a rename un-binds the declaration exactly as a deletion does"),

        ("S1: the job is renamed out from under the seam",
         run(_FIXTURE_WORKFLOW.replace("  classify:", "  classify2:"), clean), 1,
         "the job_id half of the identity pair must bind too"),

        ("S1: the workflow file is gone",
         check_seams({"lints.yml": _FIXTURE_GATE_WORKFLOW}, clean, {}, classifier), 1,
         "a stale declaration must RED, never silently skip"),

        ("S2: `continue-on-error: true` added to the pinned step",
         run(_FIXTURE_WORKFLOW.replace(
             "      - name: Self-test before it writes\n",
             "      - name: Self-test before it writes\n        continue-on-error: true\n"),
             clean), 1,
         "#5371's other headline mutant: the guard runs, fails, and nothing reds"),

        ("S2: the QUOTED spelling `continue-on-error: 'true'`",
         run(_FIXTURE_WORKFLOW.replace(
             "      - name: Self-test before it writes\n",
             "      - name: Self-test before it writes\n        continue-on-error: 'true'\n"),
             clean), 1,
         "PR #4192's review found the quoted spelling surviving a looser reading"),

        ("S2: the EXPRESSION spelling `continue-on-error: ${{ true }}`",
         run(_FIXTURE_WORKFLOW.replace(
             "      - name: Self-test before it writes\n",
             "      - name: Self-test before it writes\n"
             "        continue-on-error: ${{ true }}\n"), clean), 1,
         "an expression cannot be shown false, so absence — not falsity — is the rule"),

        ("S2: `if: false` added to the pinned step",
         run(_FIXTURE_WORKFLOW.replace(
             "      - name: Self-test before it writes\n",
             "      - name: Self-test before it writes\n        if: false\n"), clean), 1,
         "the measured shape of every uncaught mutant in this repo is a workflow `if:`"),

        ("S2: `if: false` added to the enclosing JOB",
         run(_FIXTURE_WORKFLOW.replace(
             "    runs-on: ubuntu-latest\n", "    runs-on: ubuntu-latest\n    if: false\n"),
             clean), 1,
         "a job-level condition skips the seam without touching the step at all"),

        ("S2: job-level `continue-on-error: true`",
         run(_FIXTURE_WORKFLOW.replace(
             "    runs-on: ubuntu-latest\n",
             "    runs-on: ubuntu-latest\n    continue-on-error: true\n"), clean), 1,
         "the step reds, the job concludes success, the aggregator sees nothing"),

        ("S2: a job `if:` the entry PINS is accepted",
         run(_FIXTURE_WORKFLOW.replace(
             "    runs-on: ubuntu-latest\n",
             "    runs-on: ubuntu-latest\n    if: github.event_name == 'schedule'\n"),
             _fixture_registry(job_if="github.event_name == 'schedule'")), 0,
         "pinning by EQUALITY lets a legitimately-conditional lane be declared, while "
         "still redding when that condition is edited"),

        ("S2: a pinned job `if:` that has since been EDITED",
         run(_FIXTURE_WORKFLOW.replace(
             "    runs-on: ubuntu-latest\n",
             "    runs-on: ubuntu-latest\n    if: false\n"),
             _fixture_registry(job_if="github.event_name == 'schedule'")), 1,
         "equality, not containment: narrowing a pinned condition must RED"),

        ("S3: the step keeps its name but DROPS one invocation",
         run(_FIXTURE_WORKFLOW.replace(
             "          python3 scripts/tests/test_triage_area.py\n", ""), clean), 1,
         "a gutted step is a deleted step minus the evidence"),

        ("S3: the invocation is COMMENTED OUT rather than removed",
         run(_FIXTURE_WORKFLOW.replace(
             "          python3 scripts/tests/test_triage_area.py",
             "          # python3 scripts/tests/test_triage_area.py"), clean), 1,
         "shell comments are stripped before matching, so this cannot hide"),

        ("S4: a gate step acquires `continue-on-error: true` in an undeclared job",
         run(_FIXTURE_WORKFLOW, clean, gate_workflow=_FIXTURE_GATE_WORKFLOW.replace(
             "      - name: Enforce the ratchet\n",
             "      - name: Enforce the ratchet\n        continue-on-error: true\n")), 1,
         "the no-declaration-needed half: this needs no registry entry to be caught"),

        ("S4: the same step, but its job is DECLARED advisory",
         run(_FIXTURE_WORKFLOW, clean, advisory={"lints": {}},
             gate_workflow=_FIXTURE_GATE_WORKFLOW.replace(
                 "      - name: Enforce the ratchet\n",
                 "      - name: Enforce the ratchet\n        continue-on-error: true\n")), 0,
         "a gate allowed to fail IS advisory — declaring it is the honest escape hatch"),

        ("S4: the same step, waived explicitly in the seam registry",
         run(_FIXTURE_WORKFLOW,
             {"seams": [dict(_FIXTURE_SEAM)],
              "swallow_waivers": [{"workflow": "lints.yml", "job_id": "lint",
                                   "step_name": "Enforce the ratchet", "why": "fixture",
                                   "owner_issue": 5371, "registered": "2026-09-01"}]},
             gate_workflow=_FIXTURE_GATE_WORKFLOW.replace(
                 "      - name: Enforce the ratchet\n",
                 "      - name: Enforce the ratchet\n        continue-on-error: true\n")), 0,
         "the per-step waiver for one best-effort step inside an otherwise-gating job"),

        ("S4: `continue-on-error: false` on a gate step is NOT an offence",
         run(_FIXTURE_WORKFLOW, clean, gate_workflow=_FIXTURE_GATE_WORKFLOW.replace(
             "      - name: Enforce the ratchet\n",
             "      - name: Enforce the ratchet\n        continue-on-error: false\n")), 0,
         "a literal false swallows nothing; flagging it would train authors to ignore S4"),

        ("S4: a non-gate step with `continue-on-error: true` is NOT an offence",
         run(_FIXTURE_WORKFLOW, clean, gate_workflow=_FIXTURE_GATE_WORKFLOW.replace(
             "      - name: Enforce the ratchet\n        run: python3 scripts/coverage-gate.py\n",
             "      - name: Enforce the ratchet\n        run: python3 scripts/coverage-gate.py\n"
             "      - name: Upload the log\n        continue-on-error: true\n"
             "        run: python3 scripts/upload.py\n")), 0,
         "best-effort artifact steps are legitimate; S4 is scoped to gate commands"),

        ("S5: an entry missing a required field",
         run(_FIXTURE_WORKFLOW, {"seams": [{k: v for k, v in _FIXTURE_SEAM.items()
                                            if k != "why"}]}), 1,
         "an unattributable entry is one nobody can ever knowingly retire"),

        ("S5: an EMPTY registry cannot pass",
         run(_FIXTURE_WORKFLOW, {"seams": []}), 1,
         "S1-S3 are vacuous over an empty registry, so emptiness itself is the offence"),

        ("S5: a sweep that classifies NO gate command anywhere",
         check_seams({"cron-lane.yml": _FIXTURE_WORKFLOW}, clean, {}, classifier), 1,
         "a blind sweep reports a confident all-clear forever"),
    ]

    failures = 0
    for label, offences, expected, rationale in cases:
        got = len(offences)
        ok = got == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}: {got} offence(s) (want {expected})")
        if not ok:
            print(f"         [{rationale}]")
            for offence in offences:
                print(f"         offence: {offence}")
            failures += 1

    failures += _parser_cases()

    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


def _parser_cases() -> int:
    """The step parser's own edge cases.

    Every check above is only as sound as this parser: if `parse_steps` miscounts, the
    mutation table can pass while the live sweep sees nothing.
    """
    failures = 0

    def expect(label: str, actual, wanted) -> None:
        nonlocal failures
        ok = actual == wanted
        print(f"  [{'PASS' if ok else 'FAIL'}] parser: {label}")
        if not ok:
            print(f"         got {actual!r}, want {wanted!r}")
            failures += 1

    block = """\
  job:
    name: a job
    needs:
      - other
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@abc
        with:
          persist-credentials: false
      - name: heredoc mentioning the key
        run: |
          cat <<'EOF'
          continue-on-error: true
          EOF
      - name: "quoted name with #3505 inside"
        continue-on-error: "true"
      - name: plain name with a + #462 trailing comment
        run: echo hi
    outputs:
      x: y
"""
    steps = parse_steps(block)
    # `needs:` is a 6-space list BEFORE steps:, and `outputs:` closes the region: neither
    # may be read as a step.
    expect("four steps, no needs/outputs bleed", len(steps), 4)
    expect("unnamed `uses:` step has an empty name", steps[0]["name"], "")
    expect("a run: heredoc is not read as a step key",
           "continue-on-error" in steps[1]["fields"], False)
    expect("a `with:` sub-key is not read as a step key",
           "persist-credentials" in steps[0]["fields"], False)
    # The two `#` spellings, which parse differently — see _unquote. Getting these
    # backwards makes an affected step un-pinnable, and the repo has both in use.
    expect("a `#` inside a QUOTED name is part of the name",
           steps[2]["name"], "quoted name with #3505 inside")
    expect("a plain scalar ENDS at ` #`, exactly as GitHub reads it",
           steps[3]["name"], "plain name with a +")
    expect("a quoted swallow is still detected as truthy",
           _is_truthy_swallow(steps[2]["fields"]["continue-on-error"]), True)
    expect("a literal false is not truthy", _is_truthy_swallow("false"), False)
    expect("an expression is treated as truthy", _is_truthy_swallow("${{ inputs.soft }}"), True)
    expect("a reusable-workflow job yields no steps",
           parse_steps("  call:\n    uses: ./.github/workflows/x.yml\n"), [])
    return failures


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="#5371: step-level workflow safety-seam check (S1-S5)"
    )
    parser.add_argument("--root", type=Path, default=REPO_ROOT,
                        help="repo root (default: inferred from script location)")
    parser.add_argument("--self-test", action="store_true",
                        help="run the hermetic mutation table and exit")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    offences = check_repo(args.root)
    if not offences:
        print("check-step-seams: all clear (S1 + S2 + S3 + S4 + S5)")
        return 0

    print(f"check-step-seams: {len(offences)} offence(s):\n")
    for offence in offences:
        print(f"  {offence}")
    print()
    print("  S1/S2/S3 concern the seams pinned in .github/step-seam-registry.json. If a "
          "seam is genuinely retired, DELETE its entry in the same change that removes "
          "the step — that is the deliberate, reviewable act the registry exists to force.")
    print("  S4 needs no declaration: a gate-classified step may not be allowed to fail "
          "silently. Drop `continue-on-error`, declare the job advisory, or add a "
          "`swallow_waivers` entry.")
    print("  S5 is the anti-vacuity guard: a registry that declares nothing, an entry "
          "nobody can attribute, or a sweep that matches nothing.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
