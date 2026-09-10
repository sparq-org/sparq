#!/usr/bin/env python3
# [SONNET-4.6] sq-qcnn.32 — advisory-job registry + gate-intent placement check.
# [OPUS-5] #3773 — extended: language-agnostic gate detection (C3) + stable-identity
# binding of every declaration (C4). 🤖 SPARQ agent.
#
# THE REGISTRY IS NOW LOAD-BEARING. Since #3773, `.github/advisory-registry.json` is
# the ONLY thing that can make a check-run non-gating: scripts/ci_summary_gate.py
# excludes a check iff its name is DECLARED here, and everything else GATES. Before
# #3773 the gate inferred advisory status from the display NAME
# (`\b(advisory|informational)\b`), which silently neutralised four real gates. This
# checker is therefore the registry's integrity gate.
#
# Checks FOUR directions across .github/workflows/*.yml:
#
#  C2: every job whose NAME matches \b(advisory|informational)\b must have an entry
#      in .github/advisory-registry.json with {owner_bead, promotion_criteria,
#      registered, job_id}. (Keeping the NAME-token rule here is naming HYGIENE, not
#      a gating decision: a job that *looks* advisory must either be declared or
#      lose the misleading token — the gate itself no longer reads the token at all.)
#
#  C3: no run-step that invokes a GATE-CLASSIFIED command may live inside an
#      advisory-named job without an explicit `gate_script_waiver` entry in the
#      registry for that job. #3773: gate classification is now LANGUAGE-AGNOSTIC —
#      python, shell, node AND npm scripts (resolved through package.json to the
#      command they actually run). The old rule only matched `scripts/*gate*.py`, so
#      a gate implemented in shell or npm needed no waiver at all and the registry
#      could not see it: that is the structural reason the site determinism
#      grep-gate (`npm run test:e2e:grep-gate` → `bash e2e/support/no-timeout-gate.sh`)
#      and the gui `no-sleep-gate.sh` sat inside advisory jobs, gating nothing.
#
#  C4: every registry entry must BIND to a live job identity — `workflow` +
#      `job_id` must resolve to a real job whose current YAML `name:` is EXACTLY the
#      entry's key. This is what makes "renaming a job cannot silently flip whether
#      it gates" enforceable: the gate can only match a check-run by its display
#      name, so a rename un-matches the declaration (the job GATES — fail-closed)
#      and C4 REDs here until the declaration is deliberately updated. It also
#      catches STALE entries whose job was deleted or renamed away, an entry with NO
#      identity pair at all, and a key with no literal anchor outside its
#      `${{ … }}` expressions.
#
# [OPUS-5] #3774 review (gpt-5.6-sol) — the two C4 holes that reached this file:
#   (a) C4 used to `continue` past an identity-less entry "already reported by C2".
#       C2 only iterates jobs whose NAME carries the advisory/informational token, so
#       an identity-less entry keyed on a NON-token name (`clippy (gate) + fmt
#       (non-blocking)`) was reported by nobody: this checker printed
#       `all clear (C2 + C3 + C4)` while ci_summary_gate.py excluded the real clippy
#       leg from the gating set. C4 REPORTS it now, and the gate's
#       REGISTRY_REQUIRED_FIELDS was widened to the same five fields so the GATE
#       refuses it too — the checker alone was never sufficient.
#   (b) An expression-ONLY key (from the idiomatic `name: ${{ matrix.label }}`)
#       compiled to `.+` in the gate and matched every check-run including `gate`,
#       while C4 was satisfied because the key did equal the live YAML `name:`.
#       Both sides now require a literal anchor.
#
# MOTIVATION: #1679 (gate-intent G3 self-test sat in an advisory job = provably
# non-gating until caught); #1656 (gui-mock-ipc gated without recorded probation
# evidence); #3773 (name-based exclusion neutralising four real gates).
#
# ADVISORY-JOB DEFINITION (C2 only): a job whose `name:` field (at 4-space indent)
# contains the whole word "advisory" or "informational" (case-insensitive).
# Workflow-level `name:` at indent 0 is NOT checked — only job-level `name:` values.
#
# GATE-CLASSIFIED COMMANDS (C3), derived from what the workflows ACTUALLY RUN:
#   * a script path in a `run:` step whose FILENAME carries a gate-ish token
#     (`gate`, `ratchet`, `tripwire`, `guard`) in any of .py/.sh/.bash/.mjs/.js/.ts —
#     e.g. mutants-gate.py, coverage-gate.py, no-timeout-gate.sh, no-sleep-gate.sh,
#     tauri-driver-drift-tripwire.sh;
#   * an `npm run <script>` whose SCRIPT NAME carries a gate-ish token; and
#   * an `npm run <script>` whose RESOLVED command (looked up across every
#     package.json in the repo) is itself gate-classified — this is what catches
#     `npm run test:e2e:grep-gate`, and it keeps working if the npm alias is renamed
#     to something innocuous.
#
# REGISTRY: .github/advisory-registry.json (seeded by sq-qcnn.32).
# Each key is the job's `name:` value as written in the workflow YAML (unexpanded,
# including any ${{ matrix.xxx }} expressions) — the string ci_summary_gate.py matches
# a live check-run against. Required fields per entry:
#   owner_bead         — the bead responsible for tracking this advisory job
#   promotion_criteria — what must be true before the declaration is removed
#   registered         — ISO-8601 date the entry was first recorded
#   workflow           — the workflow FILE the job lives in (C4 identity, half 1)
#   job_id             — the YAML key under `jobs:` (C4 identity, half 2)
# Optional:
#   gate_script_waiver — present iff the job legitimately runs a gate-classified
#                        command; its value is a human-readable justification
#
# Usage:
#   check-advisory-registry.py              # check live workflows (default)
#   check-advisory-registry.py --root DIR   # use DIR as repo root
#   check-advisory-registry.py --self-test  # hermetic negative fixtures (all classes)
#
# Exit 0 = all clean; exit 1 = one or more offences.
# stdlib-only.

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Naming-HYGIENE regex (C2 only). NOT a gating decision — ci_summary_gate.py deleted
# its name rule in #3773 and now excludes only DECLARED names.
ADVISORY_RE = re.compile(r"\b(advisory|informational)\b", re.IGNORECASE)

# [OPUS-5] #3773 — the repo's own vocabulary for "this step fails the build on a
# violation". Kept deliberately small and whole-word-ish so the classifier stays
# precise: a false positive costs a waiver, a false NEGATIVE costs a silent gate.
GATE_TOKEN_RE = re.compile(r"(?i)(?:^|[^a-z0-9])(gate|ratchet|tripwire|guard)(?:[^a-z0-9]|$)")
# Script invocations in a run: block, any language. The filename is what we classify.
SCRIPT_PATH_RE = re.compile(r"[\w./-]+\.(?:py|sh|bash|mjs|cjs|js|ts)\b")
# `npm run <script>` (also `npm run -w pkg <script>`, `npm --prefix x run <script>`).
NPM_RUN_RE = re.compile(r"\bnpm\s+(?:[^\s]+\s+)*?run\s+(?:-[^\s]+\s+(?:[^\s-][^\s]*\s+)?)*([\w:.@/-]+)")

# Required fields in every registry entry. [OPUS-5] #3773 adds the C4 identity pair.
# This set is MIRRORED by ci_summary_gate.py's REGISTRY_REQUIRED_FIELDS — the gate must
# refuse exactly what this checker refuses, or an entry the checker rejects can still
# buy a runtime exclusion (#3774 review, gpt-5.6-sol finding 2(a)).
REQUIRED_FIELDS = ("owner_bead", "promotion_criteria", "registered", "workflow", "job_id")

# [OPUS-5] #3774 review (finding 2(b)) — a `${{ … }}` in a registry key compiles to an
# unbounded `.+` in ci_summary_gate.py, so a key with no LITERAL, non-whitespace
# character outside its expressions matches every check-run name (`gate` included).
# Mirrors ci_summary_gate.py's `_YAML_EXPR_RE` / `registry_key_has_literal_anchor`.
YAML_EXPR_RE = re.compile(r"\$\{\{.*?\}\}")


def _has_literal_anchor(key: str) -> bool:
    """Does this registry key pin at least one literal character outside `${{ … }}`?"""
    return any(part.strip() for part in YAML_EXPR_RE.split(key or ""))


# ---------------------------------------------------------------------------
# Lightweight GitHub Actions workflow parser (stdlib-only)
# ---------------------------------------------------------------------------

def _job_name_from_block(block: str, fallback_id: str) -> str:
    """Extract the `name:` value from a job block (4-space indent).

    Returns the fallback job-ID if no `name:` is present.
    Handles both quoted and unquoted values, including ${{ matrix.xxx }} expressions.
    """
    m = re.search(r"^    name:\s+(.+?)(?:\s*#.*)?$", block, re.MULTILINE)
    if not m:
        return fallback_id
    val = m.group(1).rstrip()
    # Strip surrounding quote pair (double or single)
    if len(val) >= 2 and val[0] == val[-1] and val[0] in ('"', "'"):
        val = val[1:-1]
    return val


def parse_jobs(text: str) -> list[dict]:
    """Parse a GitHub Actions workflow file into a list of job dicts.

    Each dict has:
      id         — the YAML key under `jobs:` (e.g. "markdownlint")
      name       — the job's `name:` value, or the id if absent
      block_text — the raw text of the job block (for gate-script scanning)

    Only the `jobs:` top-level section is parsed; other sections are ignored.
    Relies on the fixed 2-space indentation of job IDs in GitHub Actions YAML.
    """
    # Find the start of the jobs: section.
    m = re.search(r"^jobs:\s*$", text, re.MULTILINE)
    if not m:
        return []
    jobs_text = text[m.end():]

    # Find all 2-space-indented job keys (exactly 2 spaces, then a YAML key).
    # Pattern: start of line, exactly 2 spaces, identifier chars, colon,
    # optional whitespace/comment.
    job_key_re = re.compile(
        r"^  ([a-zA-Z][a-zA-Z0-9_-]*):\s*(?:#.*)?$", re.MULTILINE
    )

    matches = list(job_key_re.finditer(jobs_text))
    if not matches:
        return []

    results = []
    for i, match in enumerate(matches):
        job_id = match.group(1)
        start = match.start()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(jobs_text)
        block_text = jobs_text[start:end]
        job_name = _job_name_from_block(block_text, job_id)
        results.append({"id": job_id, "name": job_name, "block_text": block_text})

    return results


def _is_gate_ish(name: str) -> bool:
    """Does this filename / npm-script name carry a gate-ish token?"""
    return bool(GATE_TOKEN_RE.search(name))


def _strip_shell_comments(line: str) -> str:
    """Drop a trailing/leading shell comment. Crude but adequate: the classifier
    only cares whether a real command mentions a gate-ish script."""
    stripped = line.lstrip()
    if stripped.startswith("#"):
        return ""
    return re.split(r"\s#", line, maxsplit=1)[0].rstrip()


def extract_run_commands(block_text: str) -> list[str]:
    """Return the shell text of every `run:` step in a job block.

    [OPUS-5] #3773 — classification must read what the job RUNS, not its prose. The
    previous whole-block scan also matched script paths mentioned in YAML comments
    (gui.yml's tauri-e2e header comment names `support/no-sleep-gate.sh`, which
    attributed the gate to the WRONG job), and shell comments inside a run block are
    likewise not invocations. Handles inline `run: cmd` and block scalars
    (`run: |` / `run: >`), whose body is every following more-indented line.
    """
    commands: list[str] = []
    lines = block_text.splitlines()
    i = 0
    run_re = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
    while i < len(lines):
        match = run_re.match(lines[i])
        if not match:
            i += 1
            continue
        indent, inline = match.group(1), match.group(2).strip()
        i += 1
        if inline and inline not in ("|", ">", "|-", ">-", "|+", ">+"):
            commands.append(_strip_shell_comments(inline))
            continue
        body: list[str] = []
        while i < len(lines):
            line = lines[i]
            if line.strip() and (len(line) - len(line.lstrip())) <= len(indent):
                break
            body.append(_strip_shell_comments(line))
            i += 1
        commands.append("\n".join(body))
    return commands


def load_npm_scripts(root: Path) -> dict[str, list[str]]:
    """Map every npm script NAME in the repo to the command(s) it runs.

    [OPUS-5] #3773 — this is the "derive the gate set from what the workflows actually
    run" half: a workflow's `npm run X` is classified by the command X expands to, not
    by how X is spelled. Every package.json outside node_modules contributes (the repo
    is an npm workspace: root + site/ + js/ + gui/**/ + packages/**), so a script name
    may map to several commands; ANY gate-classified expansion counts.
    """
    scripts: dict[str, list[str]] = {}
    for path in sorted(root.rglob("package.json")):
        if "node_modules" in path.parts or ".git" in path.parts:
            continue
        try:
            data = json.loads(path.read_text(encoding="utf-8", errors="replace"))
        except (json.JSONDecodeError, OSError):
            continue
        table = data.get("scripts")
        if not isinstance(table, dict):
            continue
        for name, command in table.items():
            if isinstance(name, str) and isinstance(command, str):
                scripts.setdefault(name, []).append(command)
    return scripts


def classify_command(command: str, npm_scripts: dict[str, list[str]],
                     _depth: int = 0) -> list[str]:
    """Gate-classified invocations inside ONE command/run-block, any language.

    Returns human-readable descriptors (empty = not gate-classified). Recurses through
    `npm run X` into X's own command(s) so an npm alias cannot hide a gate; the depth
    bound keeps a self-referential script table from looping.
    """
    hits: list[str] = []
    for script in SCRIPT_PATH_RE.findall(command):
        if _is_gate_ish(Path(script).name):
            hits.append(script)
    if _depth < 3:
        for npm_script in NPM_RUN_RE.findall(command):
            if _is_gate_ish(npm_script):
                hits.append(f"npm run {npm_script}")
                continue
            for expansion in npm_scripts.get(npm_script, []):
                inner = classify_command(expansion, npm_scripts, _depth + 1)
                if inner:
                    hits.append(f"npm run {npm_script} -> {inner[0]}")
                    break
    # Stable, de-duplicated order so offence text is diff-comparable.
    seen: set[str] = set()
    return [h for h in hits if not (h in seen or seen.add(h))]


def find_gate_scripts_in_block(block_text: str,
                               npm_scripts: dict[str, list[str]] | None = None) -> list[str]:
    """Return every gate-classified invocation a job block actually RUNS (any language)."""
    hits: list[str] = []
    for command in extract_run_commands(block_text):
        hits.extend(classify_command(command, npm_scripts or {}))
    seen: set[str] = set()
    return [h for h in hits if not (h in seen or seen.add(h))]


# ---------------------------------------------------------------------------
# Registry loading
# ---------------------------------------------------------------------------

def load_registry(root: Path) -> dict:
    """Load and return the advisory-registry.json `jobs` mapping.

    Returns an empty dict if the file does not exist (so the check
    fails with C2 offences rather than crashing).
    """
    registry_path = root / ".github" / "advisory-registry.json"
    if not registry_path.exists():
        return {}
    try:
        data = json.loads(registry_path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return {}
    return data.get("jobs", {})


# ---------------------------------------------------------------------------
# Main check
# ---------------------------------------------------------------------------

def check_workflows(root: Path) -> list[str]:
    """Scan all workflow files under root/.github/workflows.

    Returns a list of human-readable offence descriptions (empty = clean).
    Each entry is prefixed with C2, C3 or C4 to identify the violated direction.
    """
    registry = load_registry(root)
    wf_dir = root / ".github" / "workflows"
    if not wf_dir.is_dir():
        return [f"C2: .github/workflows/ not found under {root}"]

    npm_scripts = load_npm_scripts(root)
    offences: list[str] = []
    # (workflow file, job_id) -> job name, for the C4 identity binding below.
    live_jobs: dict[tuple[str, str], str] = {}

    for path in sorted(wf_dir.iterdir()):
        if path.suffix not in (".yml", ".yaml"):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue

        for job in parse_jobs(text):
            job_name = job["name"]
            live_jobs[(path.name, job["id"])] = job_name
            if not ADVISORY_RE.search(job_name):
                continue  # Not an advisory/informational job.

            # C2: must have a registry entry with all required fields.
            entry = registry.get(job_name)
            if entry is None:
                offences.append(
                    f"C2: {path.name}: job {job_name!r} is advisory/informational "
                    f"but has no entry in .github/advisory-registry.json "
                    f"(so ci_summary_gate.py GATES it — declare it or drop the token)"
                )
            else:
                missing = [f for f in REQUIRED_FIELDS if not entry.get(f)]
                if missing:
                    offences.append(
                        f"C2: {path.name}: job {job_name!r} registry entry is missing "
                        f"required fields: {missing}"
                    )

            # C3: no gate-classified command (ANY language) without a waiver.
            gate_scripts = find_gate_scripts_in_block(job["block_text"], npm_scripts)
            if gate_scripts:
                has_waiver = bool(
                    entry.get("gate_script_waiver") if entry is not None else False
                )
                if not has_waiver:
                    offences.append(
                        f"C3: {path.name}: advisory job {job_name!r} invokes "
                        f"gate-classified command(s) {gate_scripts} without a "
                        f"`gate_script_waiver` in the registry"
                    )

    # C4 — every declaration must BIND to a live job identity whose CURRENT name is
    # exactly the key. This is the rename-invariance enforcement (see the header).
    offences.extend(check_registry_bindings(registry, live_jobs))
    return offences


def check_registry_bindings(registry: dict,
                            live_jobs: dict[tuple[str, str], str]) -> list[str]:
    """C4: bind each registry key to (workflow, job_id) and pin the job's name.

    [OPUS-5] #3773. The gate can only recognise a declared-advisory check by its
    DISPLAY NAME, so a declaration that drifts from the job it describes silently
    stops applying. Fail-closed is the gate's job (a renamed job GATES); making the
    drift LOUD is this check's job.

    [OPUS-5] #3774 review (gpt-5.6-sol, finding 2(a)). This used to `continue` past an
    entry with no `workflow`/`job_id` "already reported by C2" — FALSE. C2 only
    iterates jobs whose NAME carries an advisory/informational token, so an
    identity-less entry whose key is not token-named (`clippy (gate) + fmt
    (non-blocking)`) was reported by NOBODY: the checker printed `all clear` while the
    gate excluded the real clippy leg. C4 now REPORTS it, and it is C4's to report
    because the missing pair is precisely C4's binding input.
    Finding 2(b): a key with no literal anchor outside its `${{ … }}` expressions is
    also reported — its binding LOOKS correct (the key does equal the live `name:`)
    while the gate would compile it to `.+` and exclude every check-run including
    `gate`.
    """
    offences: list[str] = []
    for key, entry in sorted(registry.items()):
        if not isinstance(entry, dict):
            offences.append(f"C4: registry entry {key!r} is not an object")
            continue
        if not _has_literal_anchor(key):
            offences.append(
                f"C4: registry key {key!r} has no literal (non-expression) anchor. It "
                f"compiles to an unbounded `.+` in ci_summary_gate.py, so it would "
                f"whole-name-match EVERY check-run — `gate` included — from one "
                f"registry line. Frame the expression with literal text (e.g. "
                f"`GUI build ({key}, advisory)`)."
            )
        workflow = entry.get("workflow") or ""
        job_id = entry.get("job_id") or ""
        if not workflow or not job_id:
            missing = [f for f in ("workflow", "job_id") if not entry.get(f)]
            offences.append(
                f"C4: registry entry {key!r} is missing {missing} — it has NO job "
                f"identity to bind to, so no check can police it (C2 only sees "
                f"advisory/informational-NAMED jobs, so it does not report this key). "
                f"ci_summary_gate.py refuses such an entry outright (it declares "
                f"nothing and the check keeps GATING): add the workflow file name and "
                f"the job's YAML key, or delete the entry."
            )
            continue
        actual = live_jobs.get((workflow, job_id))
        if actual is None:
            offences.append(
                f"C4: registry entry {key!r} points at {workflow}:{job_id}, which does "
                f"not exist — STALE declaration. A declaration that binds to nothing "
                f"can still match a future job of the same name: delete it, or fix the "
                f"workflow/job_id pair."
            )
        elif actual != key:
            offences.append(
                f"C4: registry entry {key!r} binds to {workflow}:{job_id}, whose job "
                f"name is now {actual!r}. The declaration no longer matches the job, so "
                f"the job GATES (fail-closed). Rename the registry KEY to {actual!r} if "
                f"the lane is still meant to be advisory, or delete the entry to "
                f"promote it."
            )
    return offences


# ---------------------------------------------------------------------------
# Self-test — hermetic negative fixtures
# ---------------------------------------------------------------------------

# C2 negative fixture: an advisory job NOT present in a minimal registry.
_FIXTURE_C2_WORKFLOW = """\
name: test-workflow
on: [pull_request]
jobs:
  my-check:
    name: some check (advisory)
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
"""

# C2 clean fixture: the same job IS in the registry.
_FIXTURE_C2_REGISTRY_CLEAN = {
    "some check (advisory)": {
        "workflow": "test.yml",
        "job_id": "my-check",
        "owner_bead": "sq-test",
        "promotion_criteria": "once clean",
        "registered": "2026-01-01",
    }
}

# C3 negative fixture: a gate-classified script inside an advisory job with
# NO waiver in the registry.
_FIXTURE_C3_WORKFLOW = """\
name: test-workflow
on: [pull_request]
jobs:
  ratchet:
    name: mutation ratchet (cargo-mutants, advisory)
    runs-on: ubuntu-latest
    steps:
      - run: |
          python3 scripts/mutants-gate.py --check outcomes.json
"""

# C3 registry entry WITHOUT a gate_script_waiver → C3 violation.
_FIXTURE_C3_REGISTRY_NO_WAIVER = {
    "mutation ratchet (cargo-mutants, advisory)": {
        "workflow": "ci.yml",
        "job_id": "ratchet",
        "owner_bead": "sq-qcnn.11",
        "promotion_criteria": "sq-qcnn.27",
        "registered": "2026-01-01",
        # No gate_script_waiver
    }
}

# C3 registry entry WITH a gate_script_waiver → clean.
_FIXTURE_C3_REGISTRY_WITH_WAIVER = {
    "mutation ratchet (cargo-mutants, advisory)": {
        "workflow": "ci.yml",
        "job_id": "ratchet",
        "owner_bead": "sq-qcnn.11",
        "promotion_criteria": "sq-qcnn.27",
        "registered": "2026-01-01",
        "gate_script_waiver": "intentional; the ratchet runs advisory while seeding",
    }
}

# A non-advisory job running a gate script must NOT be flagged.
_FIXTURE_NON_ADVISORY = """\
name: test-workflow
on: [pull_request]
jobs:
  hard-gate:
    name: coverage ratchet (HARD)
    runs-on: ubuntu-latest
    steps:
      - run: python3 scripts/coverage-gate.py --check
"""

# A job with `informational` in the name must also be caught.
_FIXTURE_INFORMATIONAL_WORKFLOW = """\
name: test-workflow
on: [pull_request]
jobs:
  proofs:
    name: kani proofs (bounded, informational)
    runs-on: ubuntu-latest
    steps:
      - run: cargo kani
"""

# An advisory job that runs a non-gate script must NOT trigger C3.
_FIXTURE_C3_CLEAN_NO_GATE = """\
name: test-workflow
on: [pull_request]
jobs:
  lint:
    name: markdownlint (advisory)
    runs-on: ubuntu-latest
    steps:
      - run: npx markdownlint-cli2
"""

# C2 — missing required field in registry entry.
_FIXTURE_C2_MISSING_FIELD_REGISTRY = {
    "some check (advisory)": {
        "workflow": "test.yml",
        "job_id": "my-check",
        # owner_bead missing
        "promotion_criteria": "once clean",
        "registered": "2026-01-01",
    }
}


# [OPUS-5] #3773 — C3 SHELL/NPM fixtures: the coverage hole that let the site
# determinism grep-gate and the gui no-sleep-gate sit inside advisory jobs unnoticed.
_FIXTURE_C3_SHELL_WORKFLOW = """\
name: test-workflow
on: [pull_request]
jobs:
  smoke:
    name: determinism gate + foundation smoke (advisory)
    runs-on: ubuntu-latest
    steps:
      - run: bash gui/e2e/support/no-sleep-gate.sh
"""

# An npm ALIAS whose own name says nothing: only resolving it through package.json
# reveals it runs a gate. This is the "derive from what the workflows actually run" leg.
_FIXTURE_C3_NPM_ALIAS_WORKFLOW = """\
name: test-workflow
on: [pull_request]
jobs:
  smoke:
    name: determinism gate + foundation smoke (advisory)
    runs-on: ubuntu-latest
    steps:
      - run: npm run verify:site
"""
_FIXTURE_NPM_SCRIPTS = {
    "verify:site": ["bash e2e/support/no-timeout-gate.sh"],
    "test:e2e:grep-gate": ["bash e2e/support/no-timeout-gate.sh"],
    "lint": ["next lint"],
}

# A gate-ish path mentioned only in a COMMENT is not an invocation.
_FIXTURE_C3_COMMENT_ONLY_WORKFLOW = """\
name: test-workflow
on: [pull_request]
jobs:
  smoke:
    name: determinism gate + foundation smoke (advisory)
    runs-on: ubuntu-latest
    steps:
      # validated by support/no-sleep-gate.sh (in another job)
      - run: |
          # bash support/no-sleep-gate.sh
          npm run lint
"""

_FIXTURE_C3_REGISTRY_SMOKE = {
    "determinism gate + foundation smoke (advisory)": {
        "workflow": "test.yml",
        "job_id": "smoke",
        "owner_bead": "sq-test",
        "promotion_criteria": "once clean",
        "registered": "2026-01-01",
    }
}


def _run_check(workflow_text: str, registry: dict,
               npm_scripts: dict[str, list[str]] | None = None) -> list[str]:
    """Run the C2/C3 check on synthetic workflow text + a registry dict.

    Used only in self-test. Returns offences list.
    """
    offences: list[str] = []
    for job in parse_jobs(workflow_text):
        job_name = job["name"]
        if not ADVISORY_RE.search(job_name):
            continue
        entry = registry.get(job_name)
        if entry is None:
            offences.append(f"C2: job {job_name!r} unregistered")
        else:
            missing = [f for f in REQUIRED_FIELDS if not entry.get(f)]
            if missing:
                offences.append(
                    f"C2: job {job_name!r} missing fields: {missing}"
                )
        gate_scripts = find_gate_scripts_in_block(job["block_text"], npm_scripts)
        if gate_scripts:
            has_waiver = bool(
                entry.get("gate_script_waiver") if entry is not None else False
            )
            if not has_waiver:
                offences.append(
                    f"C3: advisory job {job_name!r} runs gate scripts "
                    f"{gate_scripts} without waiver"
                )
    return offences


def _c4_cases() -> int:
    """[OPUS-5] #3773 — C4: a declaration must bind to a live job whose CURRENT name
    is exactly the key. Renaming the job therefore cannot silently flip its gating
    status: the gate stops matching it (it GATES, fail-closed) and this REDs."""
    entry = {
        "workflow": "site-e2e-foundation.yml",
        "job_id": "foundation-smoke",
        "owner_bead": "sq-ymr2e.1",
        "promotion_criteria": "probation",
        "registered": "2026-01-01",
    }
    cases = [
        (
            "C4-clean: key equals the live job name",
            {"foundation smoke (advisory)": entry},
            {("site-e2e-foundation.yml", "foundation-smoke"): "foundation smoke (advisory)"},
            0,
        ),
        (
            "C4-negative: job RENAMED, declaration not updated",
            {"foundation smoke (advisory)": entry},
            {("site-e2e-foundation.yml", "foundation-smoke"): "foundation smoke"},
            1,
        ),
        (
            "C4-negative: job renamed to a DIFFERENT advisory name",
            {"foundation smoke (advisory)": entry},
            {("site-e2e-foundation.yml", "foundation-smoke"):
             "foundation smoke, take two (advisory)"},
            1,
        ),
        (
            "C4-negative: STALE entry (job deleted)",
            {"foundation smoke (advisory)": entry},
            {},
            1,
        ),
        # [OPUS-5] #3774 review, finding 2(a). These four used to be a single
        # "C4-skip: identity incomplete (C2 already reports it)" case asserting ZERO
        # offences. C2 does NOT report an identity-less entry whose key carries no
        # advisory/informational token — that is the hole the reviewer walked through
        # end-to-end (`clippy (gate) + fmt (non-blocking)`, 3 fields, checker
        # `all clear`, real clippy gate excluded). C4 must REPORT, never skip.
        (
            "C4-negative(#3774): identity incomplete (no job_id) is REPORTED",
            {"foundation smoke (advisory)": {k: v for k, v in entry.items()
                                             if k != "job_id"}},
            {},
            1,
        ),
        (
            "C4-negative(#3774): identity incomplete (no workflow) is REPORTED",
            {"foundation smoke (advisory)": {k: v for k, v in entry.items()
                                             if k != "workflow"}},
            {},
            1,
        ),
        (
            "C4-negative(#3774): the reviewer's NON-token 3-field entry is REPORTED "
            "(C2 never sees it — its key carries no advisory/informational token)",
            {"clippy (gate) + fmt (non-blocking)": {
                "owner_bead": "x", "promotion_criteria": "y",
                "registered": "2026-07-25"}},
            {("ci.yml", "clippy"): "clippy (gate) + fmt (non-blocking)"},
            1,
        ),
        (
            "C4-negative(#3774): expression-ONLY key has no literal anchor",
            {"${{ matrix.label }}": {**entry, "workflow": "gui.yml",
                                     "job_id": "build"}},
            {("gui.yml", "build"): "${{ matrix.label }}"},
            1,
        ),
        (
            "C4-clean(#3774): a FRAMED expression key is fine",
            {"GUI build + clippy (${{ matrix.label }}, advisory)": {
                **entry, "workflow": "gui.yml", "job_id": "build"}},
            {("gui.yml", "build"): "GUI build + clippy (${{ matrix.label }}, advisory)"},
            0,
        ),
    ]
    failures = 0
    for label, registry, live, expected in cases:
        offences = check_registry_bindings(registry, live)
        ok = len(offences) == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}: "
              f"{len(offences)} offence(s) (want {expected})")
        if not ok:
            for o in offences:
                print(f"         offence: {o}")
            failures += 1
    return failures


def _run_command_extraction_cases() -> int:
    """[OPUS-5] #3773 — the run:-block extractor underpinning C3 precision."""
    cases = [
        ("inline run", "    steps:\n      - run: bash a-gate.sh\n", ["bash a-gate.sh"]),
        ("block scalar", "      - run: |\n          bash a-gate.sh\n          echo hi\n",
         ["          bash a-gate.sh\n          echo hi"]),
        ("comment-only line dropped", "      - run: |\n          # bash a-gate.sh\n",
         [""]),
        ("trailing comment dropped", "      - run: echo x  # bash a-gate.sh\n",
         ["echo x"]),
        ("yaml comment outside run ignored", "      # bash a-gate.sh\n      - run: echo x\n",
         ["echo x"]),
    ]
    failures = 0
    for label, text, expected in cases:
        got = extract_run_commands(text)
        ok = got == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] run-extract {label}")
        if not ok:
            print(f"         got={got!r} want={expected!r}")
            failures += 1
    return failures


def self_test() -> int:
    """Hermetic self-test exercising both violation classes and their negatives.

    Returns 0 if all cases pass, 1 otherwise.
    """
    cases: list[tuple[str, str, dict, int, str]] = [
        # (label, workflow_text, registry, expected_offence_count, description)
        (
            "C2-negative: unregistered advisory job",
            _FIXTURE_C2_WORKFLOW,
            {},
            1,
            "an advisory job with no registry entry must produce a C2 offence",
        ),
        (
            "C2-clean: registered advisory job",
            _FIXTURE_C2_WORKFLOW,
            _FIXTURE_C2_REGISTRY_CLEAN,
            0,
            "a properly registered advisory job must produce no offence",
        ),
        (
            "C2-negative: registry entry missing required field",
            _FIXTURE_C2_WORKFLOW,
            _FIXTURE_C2_MISSING_FIELD_REGISTRY,
            1,
            "a registry entry missing owner_bead must produce a C2 offence",
        ),
        (
            "C3-negative: gate script in advisory job, no waiver",
            _FIXTURE_C3_WORKFLOW,
            _FIXTURE_C3_REGISTRY_NO_WAIVER,
            1,
            "a gate script inside an advisory job without waiver must produce a C3 offence",
        ),
        (
            "C3-clean: gate script in advisory job, with waiver",
            _FIXTURE_C3_WORKFLOW,
            _FIXTURE_C3_REGISTRY_WITH_WAIVER,
            0,
            "a gate script inside an advisory job WITH waiver must produce no offence",
        ),
        (
            "non-advisory HARD job with gate script: no offence",
            _FIXTURE_NON_ADVISORY,
            {},
            0,
            "a HARD (non-advisory) job running a gate script must never be flagged",
        ),
        (
            "informational job: also caught by C2",
            _FIXTURE_INFORMATIONAL_WORKFLOW,
            {},
            1,
            "an unregistered job with 'informational' in the name must produce a C2 offence",
        ),
        (
            "C3-clean: advisory job runs non-gate script",
            _FIXTURE_C3_CLEAN_NO_GATE,
            {
                "markdownlint (advisory)": {
                    "workflow": "test.yml",
                    "job_id": "lint",
                    "owner_bead": "sq-test",
                    "promotion_criteria": "once clean",
                    "registered": "2026-01-01",
                }
            },
            0,
            "an advisory job running a non-gate script must not trigger C3",
        ),
        # --- [OPUS-5] #3773: the shell/npm coverage hole -----------------------
        (
            "C3-negative(#3773): SHELL gate in an advisory job, no waiver",
            _FIXTURE_C3_SHELL_WORKFLOW,
            _FIXTURE_C3_REGISTRY_SMOKE,
            1,
            "a *.sh gate inside an advisory job must produce a C3 offence — the old "
            "scripts/*gate*.py-only rule missed this entirely (site no-timeout-gate.sh, "
            "gui no-sleep-gate.sh)",
        ),
        (
            "C3-negative(#3773): npm ALIAS resolving to a shell gate, no waiver",
            _FIXTURE_C3_NPM_ALIAS_WORKFLOW,
            _FIXTURE_C3_REGISTRY_SMOKE,
            1,
            "`npm run verify:site` says nothing about gating; resolving it through "
            "package.json to `bash e2e/support/no-timeout-gate.sh` must still trip C3",
        ),
        (
            "C3-clean(#3773): a gate path mentioned only in COMMENTS is not an invocation",
            _FIXTURE_C3_COMMENT_ONLY_WORKFLOW,
            _FIXTURE_C3_REGISTRY_SMOKE,
            0,
            "prose/commented-out mentions must not be attributed to a job (gui.yml's "
            "tauri-e2e header comment names no-sleep-gate.sh; the old whole-block scan "
            "charged it to the PRECEDING job)",
        ),
    ]

    failures = 0
    for label, wf_text, registry, expected_count, description in cases:
        offences = _run_check(wf_text, registry, _FIXTURE_NPM_SCRIPTS)
        got = len(offences)
        ok = got == expected_count
        status = "PASS" if ok else "FAIL"
        print(f"  [{status}] {label}: {got} offence(s) (want {expected_count})")
        if not ok:
            print(f"         [{description}]")
            for o in offences:
                print(f"         offence: {o}")
            failures += 1

    failures += _c4_cases()
    failures += _run_command_extraction_cases()

    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "sq-qcnn.32: advisory-job registry + gate-intent placement check (C2/C3)"
        )
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=REPO_ROOT,
        help="repo root (default: inferred from script location)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run hermetic negative fixtures and exit",
    )
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    offences = check_workflows(args.root)
    if not offences:
        print("check-advisory-registry: all clear (C2 + C3 + C4)")
        return 0

    print(f"check-advisory-registry: {len(offences)} offence(s):\n")
    for o in offences:
        print(f"  {o}")
    print()
    print(
        "  C2: add the job to .github/advisory-registry.json with "
        "{owner_bead, promotion_criteria, registered, workflow, job_id} — or drop the "
        "misleading advisory/informational token from its name (since #3773 the token "
        "alone does NOT make a job non-gating)."
    )
    print(
        "  C3: add a gate_script_waiver to its registry entry, or move the "
        "gate-classified command to a HARD (undeclared) job. Applies to python, shell, "
        "node AND npm scripts."
    )
    print(
        "  C4: the entry's key must equal the CURRENT `name:` of workflow:job_id it "
        "declares, must carry BOTH identity halves, and must contain literal text "
        "outside any ${{ … }} expression. Re-key the entry after a rename, delete a "
        "stale entry, add the missing workflow/job_id, or frame the expression."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
