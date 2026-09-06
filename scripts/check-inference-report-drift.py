#!/usr/bin/env python3
# [OPUS-5] sq-t099c + sq-1d528 (issue #3171) — anti-drift guard binding the COMMITTED
# inference conformance report to the ci.yml extractor that gates it, to the ratchet
# ci.yml ADVERTISES in its job name, and to the central scoreboard floor.
#
# MERGE NOTE: two beads landed this gate independently and are unified here.
#   • sq-t099c contributed C1–C6 — the emitter/ci.yml/report/registry bindings and the
#     hermetic mutation table that proves each one has teeth.
#   • sq-1d528 (#3171) contributed C7 — the ci.yml JOB NAME advertisement — plus the
#     stale-vs-ahead direction split in C5's remediation text.
# One earlier sq-1d528 behaviour is deliberately NOT carried over: it matched the
# report heading loosely (`## Overall (all …)`, either spelling) to tolerate a
# hand-annotated `## Overall (all suites)` heading in the committed artefact. C2 below
# is the stricter binding — the committed heading must be exactly what the emitter
# writes — and the tracked report now carries that exact heading, so the tolerance is
# both unnecessary and, per the #1425 finding, the very drift worth failing on.
#
# WHY THIS GATE EXISTS — the gap `tests/scoreboard_floors.rs` does NOT cover:
#   `scoreboard_floors.rs` guards floor `const`s against the central registry
#   (`scoreboard::SUITES`) for the CRATE-TEST runners. It says nothing about the two
#   BINARY-runner suites (SPARQL + inference), whose floors are hand-typed shell
#   literals (`RATCHET=<n>`) inside `.github/workflows/ci.yml`, nor about the report
#   the inference job parses to produce the number it compares against that literal.
#   So four artefacts must agree and nothing checked that they did:
#
#     1. `crates/sparq-conformance/src/inference/report.rs`  — EMITS the grand-total
#        `## Overall (…)` heading and the `<N> pass / <M> fail / <D> documented
#        divergence / <K> out-of-scope` line.
#     2. `.github/workflows/ci.yml` job `inference-conformance` — greps for that
#        heading TEXTUALLY (`grep -qE '^## Overall \(all inference suites\)'`, then
#        `grep -A2 … | grep -E '\*\*[0-9]+ pass /'`), compares against `RATCHET=`, and
#        ADVERTISES that same number in its job `name:`.
#     3. `crates/sparq-conformance/src/scoreboard.rs` (mirrored byte-for-byte into
#        `bench/conformance-scoreboard.generated.json`) — the CENTRAL `ratchet_floor`
#        and the doc-comment floor list that names each `RATCHET=` literal.
#     4. `inference-conformance-report.md` — INTENTIONALLY tracked (see .gitignore,
#        sq-l8imt): the published evidence artefact readers and the compliance estate
#        cite.
#
#   THE #1425-CLASS FAILURE, observed live on main when this gate was written: the
#   emitter says `## Overall (all inference suites)` while the committed report said
#   `## Overall (all suites)` and carried a hand-added scope paragraph between the
#   heading and the totals. ci.yml's own extractor — `grep -A2 <heading>` — therefore
#   could not find either the heading or the counts in the committed artefact. It went
#   unnoticed because CI REGENERATES the report before parsing it, so the freshly
#   written file always matched and the tracked one was never parsed by anything. The
#   drift is invisible by construction: no run reads the artefact readers do.
#
#   THE #1428-CLASS FAILURE, the same silence from the other end (sq-pbz04.1.3): the
#   tracked report sat a full suite behind the enforced snapshot — a 1637-era report
#   while CI enforced a much higher floor — with every gate green. A stale committed
#   report is a published-honesty problem: it is what a reader (and
#   research/inference-completeness-audit.md) takes as the current conformance state.
#
#   The same silence hides a count/floor split — a regenerated-and-committed report
#   whose totals no longer equal the floor the registry publishes, or a `RATCHET=`
#   literal edited in ci.yml without the registry (or vice versa). Each is a silent
#   honesty defect: the published number stops meaning what the ratchet enforces.
#
# WHAT THIS CHECKS (deterministic, in-repo, NO network / NO cargo / stdlib only):
#   C1  every `## Overall (…)` heading ci.yml's `inference-conformance` job names is
#       the heading report.rs actually emits, AND the job still carries both
#       load-bearing commands — the `grep -qE` existence probe and the `grep -A2`
#       count-source probe — each exactly once, over the report path, spelling that
#       heading. (Counting mentions is NOT sufficient: the real job also names the
#       heading in an `echo ::error::` diagnostic and in a comment, so a block that
#       has lost a command outright still carries two or more mentions. Nor is
#       matching the command TEXT sufficient: commenting a step out with a leading
#       `#` leaves that text byte-for-byte intact while the runner stops executing
#       it, so a match inside a shell comment does not count.)
#   C2  the committed report contains that exact heading, exactly once.
#   C3  the emitter still spells the four buckets ci.yml greps for, AND the committed
#       report's totals line is inside ci.yml's own `grep -A2` window after the
#       heading — i.e. the tracked artefact is parseable by the gate that guards it.
#   C4  for every BINARY-runner suite in the central registry, the `RATCHET=<n>`
#       literal in its `ci_job` equals the registry's `ratchet_floor`.
#   C5  the committed report's pass+divergence equals the inference `ratchet_floor`
#       (the registry documents this floor as "the count parsed from the report's
#       Overall line", so equality is the contract, not `>=`). EQUALITY in BOTH
#       directions on purpose: below the floor means the committed report is STALE
#       (#1428); above it means conformance improved but the ratchet was never
#       raised, so CI still guards a number the engine has already beaten and a
#       regression back to the old level would pass. The ratchet itself may still
#       only ever RISE — that direction is the scoreboard's own rule; this gate only
#       pins the copies together.
#   C6  scoreboard.rs's doc-comment floor list covers every binary-runner suite and
#       its numbers agree with both the registry row and the ci.yml literal.
#   C7  for every binary-runner suite, the job-level `name: … (ratchet >= <n>
#       pass+divergence)` ci.yml ADVERTISES equals that same floor. The job name is
#       the number a reader sees on the checks list and in branch protection, so an
#       advertisement that outruns (or lags) the enforced `RATCHET=` is a published
#       claim CI is not making.
#
# NOT CHECKED — deliberately: we never regenerate the report (that needs the pinned
#   upstream suites + a release build) and never byte-compare it against a fresh run.
#   The committed artefact legitimately LAGS between regeneration commits; what must
#   never drift is its heading, its parseability, and its headline number vs the floor.
#
# EXIT: 0 when every binding holds; 1 with a per-offence message otherwise. A missing
# or unparseable input is an offence too — fail-closed, never a silent pass.
# `--self-test` runs the hermetic mutation table (baseline PASS + one mutation per
# check, each of which MUST go red) in a temp dir; it never writes into the repo.

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

CI_WORKFLOW = ".github/workflows/ci.yml"
EMITTER = "crates/sparq-conformance/src/inference/report.rs"
SCOREBOARD_RS = "crates/sparq-conformance/src/scoreboard.rs"
SCOREBOARD_JSON = "bench/conformance-scoreboard.generated.json"
REPORT = "inference-conformance-report.md"

# Registry runner kinds whose floor is a hand-typed `RATCHET=` shell literal in
# ci.yml rather than a Rust `const` (those are covered by scoreboard_floors.rs).
BINARY_RUNNERS = ("sparql-binary", "inference-binary")
INFERENCE_RUNNER = "inference-binary"

# `let _ = writeln!(md, "## Overall (all inference suites)\n");` in report.rs.
HEADING_EMIT_RE = re.compile(r'writeln!\(\s*md,\s*"(##\s+Overall[^"]*?)\\n"')
# The four buckets ci.yml's `grep -oE '[0-9]+ pass'` / `'[0-9]+ documented
# divergence'` depend on, as spelled in the emitter's format string.
EMITTER_BUCKETS = "pass / {} fail / {} documented divergence / {} out-of-scope"
# A `## Overall (…)` mention in workflow text (after un-escaping the grep regex).
# NOTE: a mention is only evidence of SPELLING — comments and `echo ::error::`
# diagnostics mention the heading too. The two grep COMMANDS are matched below.
HEADING_MENTION_RE = re.compile(r"##\s+Overall\s+\([^)\n]*\)")
# ci.yml's existence probe — `grep -qE '^## Overall \(…\)' <report>` (ERE, so the
# parens arrive backslash-escaped; `_unescape_grep` normalises them).
EXISTENCE_GREP_RE = re.compile(r"grep\s+-qE\s+'(?P<pattern>[^'\n]*)'\s+(?P<path>[^\s;|&)]+)")
# ci.yml's count-source probe — `grep -A2 '^## Overall (…)' <report>`, piped into
# the `**<N> pass /` filter (BRE, so the parens are literal and unescaped).
COUNT_GREP_RE = re.compile(
    r"grep\s+-A(?P<ctx>\d+)\s+'(?P<pattern>[^'\n]*)'\s+(?P<path>[^\s;|&)]+)"
)
COUNT_LINE_RE = re.compile(
    r"^\*\*(\d+) pass / (\d+) fail / (\d+) documented divergence / (\d+) out-of-scope"
)
RATCHET_RE = re.compile(r"^\s*RATCHET=(\d+)\s*$", re.M)
# The ratchet a job ADVERTISES in its own `name:` — `name: Inference conformance
# (ratchet >= 1967 pass+divergence)`. Anchored on a JOB-level key: a step's
# `- name:` carries the leading dash and so cannot match.
JOB_NAME_RATCHET_RE = re.compile(
    r"^[ \t]*name:[^\n]*\(ratchet >= (\d+) pass\+divergence\)[ \t]*$", re.M
)
# `/// * inference 1967 — `ci.yml` job `inference-conformance` (`RATCHET=1967`).`
DOC_FLOOR_RE = re.compile(
    r"^\s*///\s*\*\s*(?P<name>\S+)\s+(?P<floor>\d+)\s+\S+\s+`ci\.yml`\s+job\s+"
    r"`(?P<job>[^`]+)`\s+\(`RATCHET=(?P<ratchet>\d+)`\)",
    re.M,
)
# ci.yml's `grep -A2 <heading>`: the matched line plus the two after it.
GREP_AFTER_CONTEXT = 2


def _job_blocks(workflow_text: str) -> dict[str, str]:
    """Split a workflow's `jobs:` mapping into {job-id: block text}.

    Hand-rolled (no PyYAML): this gate runs in the dependency-free docs-quality
    `ci-scripts` job, like check-dashboard-publish-wiring.py / coverage-gate.py.
    Only keys at indent 2 AFTER the top-level `jobs:` line count, so an `on:` or
    `env:` mapping earlier in the file cannot be mistaken for a job.
    """
    lines = workflow_text.splitlines()
    try:
        first = next(i for i, ln in enumerate(lines) if re.match(r"^jobs:\s*$", ln))
    except StopIteration:
        return {}
    starts: list[tuple[int, str]] = []
    for i in range(first + 1, len(lines)):
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", lines[i])
        if m:
            starts.append((i, m.group(1)))
    blocks: dict[str, str] = {}
    for idx, (i, name) in enumerate(starts):
        end = starts[idx + 1][0] if idx + 1 < len(starts) else len(lines)
        blocks[name] = "\n".join(lines[i:end])
    return blocks


def _unescape_grep(text: str) -> str:
    """Drop the backslashes a shell grep ERE uses to escape literal parentheses."""
    return text.replace("\\(", "(").replace("\\)", ")")


def _comment_start(line: str) -> int:
    """Index where a shell comment begins on `line`, or `len(line)` if none.

    Quote-aware on purpose: the commands this gate matches CONTAIN `#` characters
    (`grep -qE '^## Overall (…)'`), so a naive "is there a `#`?" scan would read
    every command as commented out. In shell a `#` opens a comment only outside
    quotes and at the start of a word.
    """
    quote: str | None = None
    for i, ch in enumerate(line):
        if quote is not None:
            if ch == quote:
                quote = None
        elif ch in "'\"":
            quote = ch
        elif ch == "#" and (i == 0 or line[i - 1].isspace()):
            return i
    return len(line)


def _active_matches(cmd_re: re.Pattern[str], block: str) -> list[re.Match[str]]:
    """`cmd_re` matches in `block`, minus any that sit inside a shell comment.

    Commenting a step out with a leading `#` leaves the command's text byte-for-byte
    intact, so a plain `finditer` would still count a command the runner no longer
    executes — the whole point of C1 is that the command RUNS.
    """
    active: list[re.Match[str]] = []
    for m in cmd_re.finditer(block):
        line_start = block.rfind("\n", 0, m.start()) + 1
        line_end = block.find("\n", m.start())
        line = block[line_start : line_end if line_end != -1 else len(block)]
        if m.start() - line_start < _comment_start(line):
            active.append(m)
    return active


def emitted_heading(emitter_src: str) -> tuple[str | None, list[str]]:
    """The one `## Overall …` heading report.rs writes for the grand total."""
    found = HEADING_EMIT_RE.findall(emitter_src)
    if len(found) != 1:
        return None, [
            f"{EMITTER}: expected exactly ONE `writeln!(md, \"## Overall …\\n\")` "
            f"grand-total heading, found {len(found)}: {found!r}. This gate binds "
            "that heading to ci.yml's grep; teach it the new shape if the emitter "
            "deliberately changed."
        ]
    return found[0].strip(), []


def heading_grep_offences(block: str, job_id: str, heading: str) -> list[str]:
    """C1's load-bearing half: ci.yml's TWO heading greps, checked independently.

    Counting `## Overall (…)` mentions across the job block is NOT enough. The real
    `inference-conformance` job names the heading four times — the `grep -qE`
    existence probe, the `echo ::error::` diagnostic beneath it, the comment above
    the extractor, and the `grep -A2` count-source probe. Deleting either COMMAND
    therefore leaves the other command plus two pieces of prose, so a mention count
    of >= 2 survives while the documented two-grep contract is broken. Each command
    is matched by its own shape instead, required to appear EXACTLY ONCE over the
    report path, and required to spell the heading the emitter writes.

    Matches inside a shell comment do NOT count: commenting a step out with a
    leading `#` preserves the command text exactly, so a text-only match would keep
    C1 green for a command the runner never executes.
    """
    offences: list[str] = []
    specs = (
        (
            "existence",
            EXISTENCE_GREP_RE,
            f"grep -qE '^{heading}' {REPORT} — with the parens BACKSLASH-escaped, "
            "since -E makes bare parens a group",
            "it is what distinguishes a missing/empty report (an INFRA flake) from a "
            "genuine regression, so without it an aborted run reports a phantom "
            "'regressed below the ratchet (0 < N)'",
        ),
        (
            "count-source",
            COUNT_GREP_RE,
            f"grep -A{GREP_AFTER_CONTEXT} '^{heading}' {REPORT}",
            "it is the only step that turns the report into the number compared "
            "against `RATCHET=`",
        ),
    )
    for kind, cmd_re, shape, why in specs:
        found = [m for m in _active_matches(cmd_re, block) if m.group("path") == REPORT]
        if len(found) != 1:
            offences.append(
                f"{CI_WORKFLOW} job `{job_id}`: expected exactly ONE {kind} grep over "
                f"`{REPORT}` (`{shape}`), found {len(found)} that the runner actually "
                f"executes. Naming the heading in a comment or an `echo ::error::` "
                f"diagnostic — or leaving the command itself commented out — is not a "
                f"substitute: {why}. Did a step get rewritten?"
            )
            continue
        m = found[0]
        pattern = _unescape_grep(m.group("pattern"))
        if heading not in pattern:
            offences.append(
                f"{CI_WORKFLOW} job `{job_id}`: the {kind} grep matches `{pattern}`, "
                f"which does not name the heading {EMITTER} emits (`{heading}`). The "
                "ratchet step would report a phantom INFRA failure instead of parsing "
                "the report."
            )
        if kind == "count-source" and int(m.group("ctx")) != GREP_AFTER_CONTEXT:
            offences.append(
                f"{CI_WORKFLOW} job `{job_id}`: the count-source grep reads "
                f"`-A{m.group('ctx')}` lines of context, but C3 below models ci.yml's "
                f"window as `-A{GREP_AFTER_CONTEXT}`. Move GREP_AFTER_CONTEXT here in "
                "lock-step with ci.yml, or C3 checks a window the gate no longer uses."
            )
    return offences


def binary_suites(scoreboard: dict) -> list[dict]:
    return [
        s
        for s in scoreboard.get("suites", [])
        if s.get("runner", {}).get("kind") in BINARY_RUNNERS
    ]


def check(root: Path, bound: list[str] | None = None) -> list[str]:
    """Return the list of offences (empty == PASS).

    `bound`, when given, collects a one-line description of each binding actually
    verified, so a PASS can state what it covered rather than claiming a clean
    bill of health it never computed.
    """
    offences: list[str] = []

    missing = [p for p in (CI_WORKFLOW, EMITTER, SCOREBOARD_RS, SCOREBOARD_JSON, REPORT)
               if not (root / p).is_file()]
    if missing:
        return [f"missing input file: {p}" for p in missing]

    workflow_text = (root / CI_WORKFLOW).read_text(encoding="utf-8", errors="ignore")
    emitter_src = (root / EMITTER).read_text(encoding="utf-8", errors="ignore")
    scoreboard_rs = (root / SCOREBOARD_RS).read_text(encoding="utf-8", errors="ignore")
    report_text = (root / REPORT).read_text(encoding="utf-8", errors="ignore")
    try:
        scoreboard = json.loads((root / SCOREBOARD_JSON).read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return [f"{SCOREBOARD_JSON}: not valid JSON ({exc})"]

    heading, heading_offences = emitted_heading(emitter_src)
    offences += heading_offences

    jobs = _job_blocks(workflow_text)
    suites = binary_suites(scoreboard)
    if not suites:
        offences.append(
            f"{SCOREBOARD_JSON}: no suite with a binary runner "
            f"({'/'.join(BINARY_RUNNERS)}). This gate would be vacuous — if the "
            "registry's runner kinds were renamed, update BINARY_RUNNERS here."
        )
    inference = next(
        (s for s in suites if s.get("runner", {}).get("kind") == INFERENCE_RUNNER), None
    )

    # C1 — ci.yml's heading literals == the heading report.rs emits.
    if heading and inference:
        job_id = inference.get("ci_job", "")
        block = jobs.get(job_id)
        if block is None:
            offences.append(
                f"{CI_WORKFLOW}: no job `{job_id}` — the registry's inference row "
                f"names it as its gating job ({SCOREBOARD_JSON}). Renaming the job "
                "without updating the registry silently unbinds the floor."
            )
        else:
            # Spelling: no stale heading anywhere in the job — commands, comments
            # and `::error::` diagnostics alike (a stale diagnostic misleads too).
            mentions = HEADING_MENTION_RE.findall(_unescape_grep(block))
            wrong = sorted({m for m in mentions if m != heading})
            if wrong:
                offences.append(
                    f"{CI_WORKFLOW} job `{job_id}` names heading(s) "
                    f"{wrong!r} but {EMITTER} emits `{heading}`. The ratchet step "
                    "would report a phantom INFRA failure instead of parsing the "
                    "report."
                )
            # Presence: both load-bearing commands, independently. Prose mentions
            # cannot stand in for a deleted grep.
            offences += heading_grep_offences(block, job_id, heading)

    # C2 — the committed report carries that exact heading, once.
    report_lines = report_text.splitlines()
    heading_idx: list[int] = []
    if heading:
        heading_idx = [i for i, ln in enumerate(report_lines) if ln.rstrip() == heading]
        if len(heading_idx) != 1:
            offences.append(
                f"{REPORT}: found {len(heading_idx)} line(s) equal to the emitted "
                f"grand-total heading `{heading}` (expected exactly 1). The tracked "
                "report has drifted from the emitter — regenerate it, or fix the "
                "heading by hand if only the heading is stale."
            )

    # C3 — bucket spelling in the emitter, and `grep -A2` parseability of the report.
    if EMITTER_BUCKETS not in emitter_src:
        offences.append(
            f"{EMITTER}: the grand-total line no longer spells "
            f"`{EMITTER_BUCKETS}`. {CI_WORKFLOW} extracts the ratchet number with "
            "`grep -oE '[0-9]+ pass'` and `grep -oE '[0-9]+ documented divergence'` "
            "over exactly that shape; renaming a bucket makes the ratchet parse 0."
        )
    counts: tuple[int, int, int, int] | None = None
    if len(heading_idx) == 1:
        start = heading_idx[0]
        window = report_lines[start : start + GREP_AFTER_CONTEXT + 1]
        match = next((COUNT_LINE_RE.match(ln) for ln in window if COUNT_LINE_RE.match(ln)), None)
        if match is None:
            offences.append(
                f"{REPORT}: no `**<N> pass / <M> fail / <D> documented divergence / "
                f"<K> out-of-scope` totals line within ci.yml's own `grep -A"
                f"{GREP_AFTER_CONTEXT}` window after `{heading}`. The gate that "
                "guards this report cannot parse it — the emitter writes the totals "
                "immediately after the heading, so anything wedged in between is "
                "hand-added drift a regeneration would erase."
            )
        else:
            counts = tuple(int(g) for g in match.groups())  # type: ignore[assignment]

    # C4 — ci.yml `RATCHET=` literal == the central registry floor.
    # C7 — and the ratchet the job ADVERTISES in its own `name:` == that same floor.
    for suite in suites:
        job_id = suite.get("ci_job", "")
        floor = suite.get("ratchet_floor")
        block = jobs.get(job_id)
        if block is None:
            offences.append(
                f"{CI_WORKFLOW}: no job `{job_id}` for registry suite "
                f"`{suite.get('label', '?')}` — the registry names it as the "
                "enforcing job, so the floor is enforced by nothing."
            )
            continue
        literals = [int(n) for n in RATCHET_RE.findall(block)]
        if len(literals) != 1:
            offences.append(
                f"{CI_WORKFLOW} job `{job_id}`: expected exactly one `RATCHET=<n>` "
                f"literal, found {len(literals)} ({literals}). This gate binds that "
                "literal to the registry floor; it cannot do so ambiguously."
            )
        elif literals[0] != floor:
            offences.append(
                f"floor drift: {CI_WORKFLOW} job `{job_id}` enforces "
                f"RATCHET={literals[0]} but the central registry publishes "
                f"ratchet_floor={floor} for `{suite.get('label', '?')}`. Raise both "
                f"together ({SCOREBOARD_RS} + its generated mirror + ci.yml)."
            )
        elif bound is not None:
            bound.append(f"ci.yml job `{job_id}` RATCHET={floor} == registry floor")

        advertised = [int(n) for n in JOB_NAME_RATCHET_RE.findall(block)]
        if len(advertised) != 1:
            offences.append(
                f"{CI_WORKFLOW} job `{job_id}`: expected exactly one job-level "
                f"`name: … (ratchet >= <n> pass+divergence)`, found {len(advertised)} "
                f"({advertised}). The job name is the number a reader sees on the "
                "checks list and in branch protection, so it must state the floor it "
                "enforces."
            )
        elif advertised[0] != floor:
            offences.append(
                f"job-name drift: {CI_WORKFLOW} job `{job_id}` advertises "
                f"`ratchet >= {advertised[0]}` in its name but the central registry "
                f"publishes ratchet_floor={floor}. The checks list would publish a "
                "claim CI is not making; update the name with the floor."
            )
        elif bound is not None:
            bound.append(
                f"ci.yml job `{job_id}` name advertises `ratchet >= {floor}` "
                "== registry floor"
            )

    # C5 — the committed report's headline number == the inference floor.
    if counts is not None and inference is not None:
        total = counts[0] + counts[2]
        floor = inference.get("ratchet_floor")
        if total == floor and bound is not None:
            bound.append(
                f"{REPORT} headline {counts[0]}+{counts[2]}={total} == inference floor, "
                f"under heading `{heading}`"
            )
        if total != floor:
            direction = (
                "the committed report is STALE (behind the floor) — it predates the "
                "enforced snapshot (#1428). Re-run `scripts/fetch-inference-suites.sh` "
                "then `cargo run --release -p sparq-conformance --bin "
                "sparq-inference-conformance` and commit the regenerated report"
                if isinstance(floor, int) and total < floor
                else "the report is AHEAD of the floor — conformance improved but the "
                "ratchet was never raised, so CI still guards a number the engine has "
                "already beaten and a regression back to the old level would pass"
            )
            offences.append(
                f"count drift: {REPORT} publishes {counts[0]} pass + {counts[2]} "
                f"documented divergence = {total}, but the central registry "
                f"publishes ratchet_floor={floor}. The registry documents this floor "
                "as the count parsed from the report's Overall line, so the two must "
                f"be equal: {direction}. Whichever way it moved, keep the four copies "
                f"together — the floor in {SCOREBOARD_RS}, its regenerated mirror "
                f"{SCOREBOARD_JSON}, and `RATCHET=` + the job `name:` in "
                f"{CI_WORKFLOW}."
            )

    # C6 — scoreboard.rs's doc-comment floor list agrees with rows and ci.yml.
    doc_rows = {m.group("job"): m for m in DOC_FLOOR_RE.finditer(scoreboard_rs)}
    for suite in suites:
        job_id = suite.get("ci_job", "")
        floor = suite.get("ratchet_floor")
        m = doc_rows.get(job_id)
        if m is None:
            offences.append(
                f"{SCOREBOARD_RS}: the FLOORS doc-comment list does not name job "
                f"`{job_id}`. Every binary-runner floor lives as a ci.yml shell "
                "literal, so that list is the only place a reader can see it; keep "
                "it complete."
            )
            continue
        doc_floor, doc_ratchet = int(m.group("floor")), int(m.group("ratchet"))
        if doc_floor != floor or doc_ratchet != floor:
            offences.append(
                f"doc drift: {SCOREBOARD_RS}'s floor list says `{m.group('name')} "
                f"{doc_floor} … RATCHET={doc_ratchet}` for job `{job_id}`, but the "
                f"registry row publishes ratchet_floor={floor}."
            )
    return offences


# --------------------------------------------------------------------------
# Hermetic self-test: baseline PASS + one mutation per check, each MUST go red.
# --------------------------------------------------------------------------

_FIX_CI = """\
name: CI
on:
  push:
    branches: [main]
jobs:
  conformance:
    name: W3C SPARQL conformance (ratchet >= 11 pass+divergence)
    runs-on: ubuntu-latest
    steps:
      - name: Enforce ratchet
        run: |
          RATCHET=11
          if ! grep -qE '^## Overall' conformance-report.md; then exit 1; fi
  inference-conformance:
    name: Inference conformance (ratchet >= 7 pass+divergence)
    runs-on: ubuntu-latest
    steps:
      - name: Enforce ratchet
        run: |
          RATCHET=7
          if [ ! -s inference-conformance-report.md ] || ! grep -qE '^## Overall \\(all inference suites\\)' inference-conformance-report.md; then
            echo "::error::report missing/empty — no '## Overall (all inference suites)' section. INFRA failure, not a regression."
            exit 1
          fi
          # The Overall section emits:
          # **<N> pass / <M> fail / <D> documented divergence / <K> out-of-scope — ...**
          # (suite subtotals use the same shape, so parse the line after the
          # final "## Overall (all inference suites)")
          OVERALL=$(grep -A2 '^## Overall (all inference suites)' inference-conformance-report.md | grep -E '\\*\\*[0-9]+ pass /')
"""
# The fixture deliberately mirrors the real job's PROSE — the `::error::`
# diagnostic and the four-line comment — because that prose is what let a
# mention-COUNT check pass with a load-bearing grep deleted. The two removal
# mutations below still go red without it, but only WITH it do they reproduce the
# masking condition and so demonstrate the per-command check earns its keep.

_FIX_EMITTER = r'''
pub fn render() -> String {
    let mut md = String::new();
    let _ = writeln!(md, "## Overall (all inference suites)\n");
    let _ = writeln!(
        md,
        "**{} pass / {} fail / {} documented divergence / {} out-of-scope - \
         pass+divergence {} of run.**",
    );
    md
}
'''

_FIX_SCOREBOARD_RS = """\
/// FLOORS — keep each in lock-step with its enforcer:
/// * SPARQL  11  — `ci.yml` job `conformance` (`RATCHET=11`).
/// * inference 7 — `ci.yml` job `inference-conformance` (`RATCHET=7`).
pub const SUITES: &[Suite] = &[];
"""

_FIX_JSON = json.dumps(
    {
        "schema": "sparq.conformance-scoreboard/v1",
        "suites": [
            {
                "ci_job": "conformance",
                "label": "W3C SPARQL",
                "ratchet_floor": 11,
                "runner": {"kind": "sparql-binary"},
            },
            {
                "ci_job": "inference-conformance",
                "label": "Inference",
                "ratchet_floor": 7,
                "runner": {"kind": "inference-binary"},
            },
            {
                "ci_job": "shacl-conformance",
                "label": "W3C SHACL core",
                "ratchet_floor": 98,
                "runner": {"kind": "crate-test", "crate": "sparq-shacl"},
            },
        ],
    },
    indent=2,
)

_FIX_REPORT = """\
# sparq inference conformance report

## Overall (all inference suites)

**5 pass / 0 fail / 2 documented divergence / 3 out-of-scope - pass+divergence 100.0% of run.**
"""

_FIXTURE = {
    CI_WORKFLOW: _FIX_CI,
    EMITTER: _FIX_EMITTER,
    SCOREBOARD_RS: _FIX_SCOREBOARD_RS,
    SCOREBOARD_JSON: _FIX_JSON,
    REPORT: _FIX_REPORT,
}

# (case name, path, old, new) — each mutation must make the gate go RED.
_MUTATIONS = [
    (
        "C2 committed report heading drifts from the emitter (the live #1425 bug)",
        REPORT,
        "## Overall (all inference suites)",
        "## Overall (all suites)",
    ),
    (
        "C3 hand-added prose pushes the totals out of ci.yml's grep -A2 window",
        REPORT,
        "\n**5 pass",
        "\nCovers the four inference regime suites plus rdf-turtle.\n\n**5 pass",
    ),
    (
        "C5 report totals no longer equal the central ratchet_floor",
        REPORT,
        "**5 pass / 0 fail / 2 documented",
        "**6 pass / 0 fail / 2 documented",
    ),
    (
        "C5 report totals fall BEHIND the floor (the stale-artefact #1428 bug)",
        REPORT,
        "**5 pass / 0 fail / 2 documented",
        "**4 pass / 0 fail / 2 documented",
    ),
    (
        "C4 ci.yml RATCHET literal raised without the registry",
        CI_WORKFLOW,
        "RATCHET=7",
        "RATCHET=8",
    ),
    (
        "C4 registry floor raised without ci.yml",
        SCOREBOARD_JSON,
        '"ratchet_floor": 7',
        '"ratchet_floor": 9',
    ),
    (
        "C7 job name advertises a ratchet the registry no longer publishes",
        CI_WORKFLOW,
        "name: Inference conformance (ratchet >= 7 pass+divergence)",
        "name: Inference conformance (ratchet >= 6 pass+divergence)",
    ),
    (
        "C7 job name drops its ratchet advertisement entirely",
        CI_WORKFLOW,
        "    name: Inference conformance (ratchet >= 7 pass+divergence)\n",
        "    name: Inference conformance\n",
    ),
    (
        "C7 the SPARQL job's name drifts from its own floor",
        CI_WORKFLOW,
        "name: W3C SPARQL conformance (ratchet >= 11 pass+divergence)",
        "name: W3C SPARQL conformance (ratchet >= 10 pass+divergence)",
    ),
    (
        "C1/C2 emitter heading renamed without ci.yml or the report",
        EMITTER,
        "## Overall (all inference suites)",
        "## Overall (every inference suite)",
    ),
    (
        "C3 emitter renames a bucket ci.yml greps for",
        EMITTER,
        "documented divergence",
        "documented divergences",
    ),
    (
        "C6 scoreboard doc-comment floor list drifts from the row",
        SCOREBOARD_RS,
        "/// * inference 7 —",
        "/// * inference 8 —",
    ),
    (
        "C1 ci.yml greps for a heading the emitter does not write",
        CI_WORKFLOW,
        "grep -qE '^## Overall \\(all inference suites\\)'",
        "grep -qE '^## Overall \\(all suites\\)'",
    ),
    (
        "C1 existence grep DELETED (comment + ::error:: diagnostic still mention it)",
        CI_WORKFLOW,
        " || ! grep -qE '^## Overall \\(all inference suites\\)' inference-conformance-report.md",
        "",
    ),
    (
        "C1 count-source grep DELETED (comment + ::error:: diagnostic still mention it)",
        CI_WORKFLOW,
        "grep -A2 '^## Overall (all inference suites)' inference-conformance-report.md | ",
        "",
    ),
    (
        "C1 existence grep COMMENTED OUT (its full text survives verbatim)",
        CI_WORKFLOW,
        "          if [ ! -s inference-conformance-report.md ]",
        "          # if [ ! -s inference-conformance-report.md ]",
    ),
    (
        "C1 count-source grep COMMENTED OUT (its full text survives verbatim)",
        CI_WORKFLOW,
        "          OVERALL=$(grep -A2",
        "          # OVERALL=$(grep -A2",
    ),
    (
        "C1 count-source grep widened past the -A2 window C3 models",
        CI_WORKFLOW,
        "grep -A2 '^## Overall (all inference suites)'",
        "grep -A5 '^## Overall (all inference suites)'",
    ),
    (
        "job rename unbinds the registry's ci_job from ci.yml",
        CI_WORKFLOW,
        "  inference-conformance:",
        "  inference-conf:",
    ),
]


def _materialize(root: Path, files: dict[str, str]) -> None:
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


def self_test() -> int:
    failures = 0
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        _materialize(root, _FIXTURE)
        baseline = check(root)
        if baseline:
            failures += 1
            print("FAIL baseline: clean fixture should PASS, got:")
            for off in baseline:
                print(f"       - {off}")
        else:
            print("PASS baseline: consistent fixture is green")

        for name, rel, old, new in _MUTATIONS:
            mutated = dict(_FIXTURE)
            if old not in mutated[rel]:
                failures += 1
                print(f"FAIL {name}: mutation anchor {old!r} not in the {rel} fixture")
                continue
            mutated[rel] = mutated[rel].replace(old, new)
            with tempfile.TemporaryDirectory() as tmp2:
                root2 = Path(tmp2)
                _materialize(root2, mutated)
                offences = check(root2)
            if offences:
                print(f"PASS {name}: caught ({offences[0].splitlines()[0][:90]}…)")
            else:
                failures += 1
                print(f"FAIL {name}: mutation NOT caught — the check is vacuous")

    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print(f"\nself-test: all {len(_MUTATIONS) + 1} cases PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Fail if the committed inference conformance report, the ci.yml "
        "ratchet extractor + job name, and the central scoreboard floor have drifted "
        "apart (beads sq-t099c + sq-1d528 / issue #3171)."
    )
    ap.add_argument("--root", default=str(REPO_ROOT), help="repo root (default: this repo).")
    ap.add_argument("--self-test", action="store_true", help="run the hermetic mutation table and exit.")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    bound: list[str] = []
    offences = check(Path(args.root), bound)
    if not offences:
        print(
            "inference report/scoreboard drift gate: PASS — the committed report's "
            "heading and headline count are in lock-step with the emitter, ci.yml's "
            "ratchet extractor + advertised job name, and the central scoreboard "
            "floor. Bindings verified:"
        )
        for line in bound:
            print(f"    - {line}")
        return 0

    print("inference report/scoreboard drift gate: FAIL\n")
    print(
        "The committed inference conformance report, the ci.yml ratchet step and the\n"
        "central conformance registry must agree. CI regenerates the report before\n"
        "parsing it, so drift in the TRACKED artefact is invisible to every other\n"
        "check — that is why this gate exists (beads sq-t099c + sq-1d528). Offences:\n"
    )
    for off in offences:
        print(f"    - {off}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
