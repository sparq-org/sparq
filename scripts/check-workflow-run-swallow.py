#!/usr/bin/env python3
# [OPUS-5] #5799 / #6226 — workflow failure-swallow check (W1/W2/W3/W4).
# 🤖 SPARQ agent.
#
# THE HOLE THIS CLOSES. A GitHub Actions step can be made unable to fail in two
# different ways. The DECLARATIVE way is the `continue-on-error:` key, which is
# visible in the YAML structure. The IMPERATIVE way is inside the `run:` body —
# `cmd || true`, `set +e`, a trailing `exit 0` — where the step still reports
# success and the job stays green, so the leg keeps producing a passing
# check-run that `ci-summary / gate` happily counts. Nothing on main looks at
# run-body text for this, so an assertion could be neutralised in a gating lane
# with a two-character edit and no reviewer signal. #5799.
#
# WHY A WAIVER LIST AND NOT A BAN. The idiom has many legitimate uses, so #5799
# asked for a measured inventory FIRST. Inventory taken 2026-09-02 over the 78
# workflows on main: 79 run-body swallow sites, 57 of them inside jobs that GATE
# (29 `|| true`, 7 `set +e`, 21 bare `exit 0`). Every one of the 57 was read and
# found intentional — `grep -c … || true` (grep exits 1 on no-match, a DATA
# outcome), `git rebase --abort || true` (cleanup), `cat "$log" || true`
# (diagnostics on an optional file), `gh issue edit … || true` (best-effort forge
# writes). So the job here is not to remove them: it is to FREEZE the clean
# inventory, so that the NEXT one has to be justified.
#
# Under this checker's scope those 57 resolve to all 29 `|| true` sites in gating
# jobs, every one of them declared in the waiver file,
# plus 7 `set +e` regions, all of which capture and act on the status.
#
# The checks, over every `run:` body in a job that GATES (i.e. a job whose name
# is NOT declared in .github/advisory-registry.json — the same load-bearing
# definition ci_summary_gate.py uses since #3773):
#
#  W1: a statement-level `|| true` / `|| :` not covered by a waiver in
#      .github/run-swallow-waivers.json. There is NO exemption by command name.
#      An earlier draft exempted a set of "observational" heads (grep/cat/ls/
#      find/…) on the theory that their non-zero exit is a data outcome — but a
#      program is not intrinsically observational: `grep -q required out.log ||
#      true` and `find . -exec validate {} \; || true` are assertions with the
#      verdict deleted, `cat generated-artifact || true` hides a missing build
#      output, and a shell function named `true` is not `true(1)`. A name-based
#      exemption is exactly the two-character neutralisation this check exists
#      to stop, so every site — diagnostic ones included — is declared ONCE,
#      with a reason that argues why THAT invocation removes no verdict.
#
#  W2: a `set +e` region that does not IMMEDIATELY capture and act on the status
#      of the command it tolerates. Concretely: the first command inside the
#      region must be followed, as the VERY NEXT statement, by either `VAR=$?`
#      (with `$VAR` actually read afterwards) or a direct branch/exit on `$?`.
#      `set +e` itself is not the defect — every one of the 7 live uses captures
#      the status on the next line (`rc=$?` / `CODE=$?`) and acts on it; 6 also
#      restore `set -e` immediately, and the 7th (dependency-monitoring `audit`)
#      routes the captured code to `$GITHUB_OUTPUT` for a later step to fail the
#      job on. The defect is tolerating a failure and then never looking at it,
#      which is indistinguishable from not running the command at all — and
#      "looking at it" has to mean THAT command's status: `./gate; echo note;
#      rc=$?` captures `echo`'s status and drops the gate's, and `echo $?`
#      prints a status while enforcing nothing. Both are offences.
#      BOUND, stated honestly: a SECOND command placed after a valid capture in
#      the same region is not covered. Every live region tolerates exactly one
#      command, so tightening that would only add unenforceable noise today.
#
#  W3: a waiver that matches no live command — STALE. Mirrors C4's spirit in
#      check-advisory-registry.py: a declaration that binds to nothing must be
#      deleted, or it silently pre-authorises a future command of that shape.
#
#  W4: a step-level `continue-on-error:` value other than a literal false/null
#      in a gating job. Expressions are deliberately included: their runtime
#      value can make the step non-fatal, so treating only literal `true` as a
#      swallow would leave the dynamic form unchecked. Job-level keys are not
#      part of this rule; the gate aggregator classifies whole jobs separately.
#
# WHAT IS DELIBERATELY *NOT* CHECKED — a trailing bare `exit 0`. #5799 names it,
# and it was measured: 21 of the 57 gating-job sites. But GitHub runs a `run:`
# body under `bash -e` (and an explicit `shell: bash` keeps `-eo pipefail`), so a
# failing earlier command aborts the shell BEFORE a trailing `exit 0` is ever
# reached — the `exit 0` is an explicit success exit, not a swallow. It only
# swallows when errexit is not in force at that point, which in this repo means
# inside a `set +e` region, and that is exactly what W2 covers. The live example
# is dependency-monitoring `audit`: `set +e` … `CODE=$?` … `exit 0`, where the
# captured code goes to `$GITHUB_OUTPUT` and a later `Fail the job if advisories
# were found` step exits 1 on it — so the `exit 0` ends that STEP, not the
# verdict. Flagging bare `exit 0` would have produced 21 findings with no signal.
#
# Usage:
#   check-workflow-run-swallow.py              # check live workflows (default)
#   check-workflow-run-swallow.py --root DIR   # use DIR as repo root
#   check-workflow-run-swallow.py --inventory  # print every swallow site found
#   check-workflow-run-swallow.py --self-test  # hermetic fixtures (all classes)
#
# Exit 0 = all clean; exit 1 = one or more offences.
# stdlib-only.

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

WAIVERS_PATH = Path(".github") / "run-swallow-waivers.json"

# NOTE: there is deliberately no OBSERVATIONAL_COMMANDS set here. The head verb
# is still parsed — it makes the offence message readable and is asserted by the
# parser self-tests — but it grants NO exemption. See W1 in the header.
#
# `|| true` / `|| :`. The `:` form only counts as a swallow when `:` is the whole
# command (followed by end-of-line or a shell operator), so `|| :.foo` is not one.
SWALLOW_RE = re.compile(r"\|\|\s*(?:true(?![\w.-])|:(?=\s*(?:$|[;)&|])))")

# `set +e` / `set +o errexit` / a combined `set +eu`-style flag word containing e.
SET_PLUS_E_RE = re.compile(r"(?:^|;|&&|\|\|)\s*set\s+(?:\+[a-df-z]*e[a-z]*|\+o\s+errexit)\b")
SET_MINUS_E_RE = re.compile(r"(?:^|;|&&|\|\|)\s*set\s+(?:-[a-df-z]*e[a-z]*|-o\s+errexit)\b")
# Any read of the previous command's status: `rc=$?`, `if [ $? -ne 0 ]`, `echo $?`.
STATUS_READ_RE = re.compile(r"\$\?")
# A status read that ENFORCES something. `VAR=$?` parks the status in a variable
# (which must then actually be read — see _status_is_enforcing), and the branch
# form acts on it on the spot. A bare `echo $?` matches neither: printing a
# status is not inspecting it, so it leaves the failure just as swallowed.
STATUS_ASSIGN_RE = re.compile(
    r"(?:^|[;&|({]|&&|\|\||\bthen\b|\bdo\b)\s*"
    r"(?:local\s+|export\s+|declare\s+|readonly\s+)?([A-Za-z_][A-Za-z0-9_]*)=\$\?")
STATUS_BRANCH_RE = re.compile(r"(?:\b(?:if|test|exit|return|case|while|until)\b|\[\[?)[^;]*\$\?")
# A statement that is ONLY a `set` builtin — neither a tolerated command nor a
# status read, so it neither opens the W2 obligation nor discharges it.
SET_ONLY_RE = re.compile(r"\s*set\s+[-+][^;&|]*")

# Process substitution `<( … )` / `>( … )`. Normalised to a plain `(` BEFORE
# redirections are stripped, or `done < <(git ls-tree … | grep -E … || true)` has
# its `git` eaten as a redirection target and the head verb reads as garbage.
PROCESS_SUBST_RE = re.compile(r"[<>]\(")
# Shell redirections, stripped before the head verb is read (`ls -la x >&2` → `ls`).
REDIRECTION_RE = re.compile(r"\d*(?:>>|>&|>|<<<|<<|<)\s*&?\S*")
# Shell operators that end one command and start the next. A BACKSLASH-ESCAPED
# `;` is find(1)'s `-exec … \;` terminator, not a separator, and `{}` is its
# placeholder — a group brace is always followed by whitespace (`{ cmd; }`), so
# requiring that keeps `-exec ./validate.sh {} \;` from eating the head verb.
COMMAND_SPLIT_RE = re.compile(
    r"(?:(?<!\\);|&&|\|\||\||\bthen\b|\bdo\b|\belse\b|\{(?=\s)|\()")
# `VAR=value cmd` prefixes.
ENV_PREFIX_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=\S*")
# `${{ … }}` GitHub expressions — masked before shell parsing (they can contain
# quotes, parens and pipes that are not shell syntax at all).
GHA_EXPR_RE = re.compile(r"\$\{\{.*?\}\}", re.DOTALL)


def _load_advisory_module():
    """Import scripts/check-advisory-registry.py by path (scripts/ is not a
    package) so the workflow/job parser and the advisory-registry loader are
    SHARED. Duplicating them would let "which jobs gate" drift between the two
    checkers, which is the exact class of bug #3773 was about."""
    path = Path(__file__).resolve().parent / "check-advisory-registry.py"
    spec = importlib.util.spec_from_file_location("check_advisory_registry", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # type: ignore[union-attr]
    return module


# ---------------------------------------------------------------------------
# Shell-text normalisation
# ---------------------------------------------------------------------------

def mask_quotes_and_expressions(text: str) -> str:
    """Blank the CONTENTS of quoted strings and `${{ … }}` expressions, keeping
    length and delimiters so offsets still line up.

    Without this the operator split below trips over shell metacharacters that
    are just payload: triage-issue.yml's `gh issue comment --body "… **quarantined**
    (\\`trust:untrusted\\`) …"` contains `(` and backticks, and reading the head
    verb after splitting on those yields `the` instead of `gh`.
    """
    text = GHA_EXPR_RE.sub(lambda m: " " * len(m.group(0)), text)
    out = list(text)
    quote: str | None = None
    i = 0
    while i < len(text):
        ch = text[i]
        if quote is None:
            if ch in ("'", '"'):
                quote = ch
        elif quote == '"' and text.startswith("$(", i):
            # Double quotes do not suppress command substitution. Recursively
            # mask its shell text while retaining operators such as `|| true`.
            depth = 1
            end = i + 2
            inner_quote: str | None = None
            while end < len(text) and depth:
                current = text[end]
                if inner_quote is None:
                    if current in ("'", '"'):
                        inner_quote = current
                    elif current == "(":
                        depth += 1
                    elif current == ")":
                        depth -= 1
                elif current == "\\" and inner_quote == '"':
                    end += 1
                elif current == inner_quote:
                    inner_quote = None
                end += 1
            interior_end = end - 1 if depth == 0 else len(text)
            out[i + 2:interior_end] = mask_quotes_and_expressions(
                text[i + 2:interior_end])
            i = end
            continue
        elif ch == "\\" and quote == '"':
            out[i] = " "
            if i + 1 < len(text):
                out[i + 1] = " "
            i += 2
            continue
        elif ch == quote:
            quote = None
        else:
            out[i] = " "
        i += 1
    return "".join(out)


def strip_shell_comment(line: str) -> str:
    """Remove a shell comment without treating `#` in quotes as syntax."""
    masked = mask_quotes_and_expressions(line)
    for match in re.finditer(r"(?<!\S)#", masked):
        return line[:match.start()]
    return line


def logical_lines(body: str, folded: bool) -> list[str]:
    """Split a `run:` body into LOGICAL shell commands.

    `folded` is True for a `run: >`-style block scalar, whose newlines YAML turns
    into spaces — miri.yml's shard step is one `cargo miri nextest … || true`
    spread over six source lines, and reading it line-by-line sees a bare
    `|| true` with no command attached to it at all.
    """
    if folded:
        joined = " ".join(line.strip() for line in body.splitlines() if line.strip())
        return [joined] if joined else []
    out: list[str] = []
    pending = ""
    for raw in body.splitlines():
        line = raw.rstrip()
        if not line.strip():
            continue
        if line.endswith("\\"):
            pending += line[:-1].strip() + " "
            continue
        out.append((pending + line.strip()).strip())
        pending = ""
    if pending.strip():
        out.append(pending.strip())
    return out


def head_command(segment: str, masked: str) -> str:
    """The command word that `|| true` is swallowing.

    `segment` is the real text (the offence quotes it verbatim); `masked` is the
    SAME SPAN with quoted payload blanked, and is what gets parsed. Masking is
    length-preserving, so the two stay index-aligned and a token located in
    `masked` can be read back out of `segment` — that alignment is load-bearing
    and must not be broken by rstrip()ing one side only.
    """
    masked = PROCESS_SUBST_RE.sub(" (", masked)
    masked = REDIRECTION_RE.sub(lambda m: " " * len(m.group(0)), masked)
    start = 0
    for match in COMMAND_SPLIT_RE.finditer(masked):
        start = match.end()
    tail_masked = masked[start:]
    tail_real = segment[start:start + len(tail_masked)].ljust(len(tail_masked))
    tokens_masked = tail_masked.split()
    if not tokens_masked:
        return ""
    # Re-read each token from the REAL text so a quoted path stays readable.
    offset = 0
    tokens: list[str] = []
    for token in tokens_masked:
        idx = tail_masked.index(token, offset)
        tokens.append(tail_real[idx:idx + len(token)])
        offset = idx + len(token)
    while tokens and ENV_PREFIX_RE.fullmatch(tokens[0]):
        tokens.pop(0)
    if not tokens:
        return ""
    word = tokens[0].strip("\"'")
    if word in ("sudo", "command", "exec", "time", "env", "xargs"):
        return head_command(" ".join(tokens[1:]), " ".join(tokens_masked[1:]))
    return Path(word).name


# ---------------------------------------------------------------------------
# Workflow scanning
# ---------------------------------------------------------------------------

RUN_KEY_RE = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
STEP_START_RE = re.compile(r"^(\s*)-\s+\S")
STEP_NAME_RE = re.compile(r"^(\s*)(?:-\s+)?name:\s*(.*)$")
CONTINUE_ON_ERROR_RE = re.compile(
    r"^(\s*)(-\s+)?continue-on-error:\s*(.*?)\s*(?:#.*)?$")
FALSE_CONTINUE_VALUES = {"false", "null", "~", "0", ""}


def extract_run_blocks(block_text: str) -> list[tuple[str, bool]]:
    """Every `run:` body in a job block, as (text, is_folded).

    check-advisory-registry.extract_run_commands() drops the block-scalar STYLE,
    and this checker needs it (see logical_lines). Comments are removed only
    after quote masking so a quoted `#` cannot hide a later swallow.
    """
    blocks: list[tuple[str, bool]] = []
    lines = block_text.splitlines()
    i = 0
    while i < len(lines):
        match = RUN_KEY_RE.match(lines[i])
        if not match:
            i += 1
            continue
        indent, inline = match.group(1), match.group(2).strip()
        i += 1
        if inline and inline not in ("|", ">", "|-", ">-", "|+", ">+"):
            blocks.append((strip_shell_comment(inline), False))
            continue
        folded = inline.startswith(">")
        body: list[str] = []
        while i < len(lines):
            line = lines[i]
            if line.strip() and (len(line) - len(line.lstrip())) <= len(indent):
                break
            body.append(strip_shell_comment(line))
            i += 1
        blocks.append(("\n".join(body), folded))
    return blocks


def find_continue_on_error(block_text: str) -> list[dict]:
    """Return non-false step-level `continue-on-error` sites in one job.

    Step keys are indented beneath a YAML list item, regardless of which mapping
    key appears first.
    Comparing indentation keeps a job-level key out without needing a YAML
    dependency, and preserves expressions verbatim for waiver matching.
    """
    hits: list[dict] = []
    step_indent: int | None = None
    step_name = "<unnamed step>"
    step_hit_start = 0
    for raw in block_text.splitlines():
        start = STEP_START_RE.match(raw)
        if start:
            step_indent = len(start.group(1))
            step_name = "<unnamed step>"
            step_hit_start = len(hits)
        name_match = STEP_NAME_RE.match(raw)
        if (name_match and step_indent is not None
                and len(name_match.group(1)) in (step_indent, step_indent + 2)):
            name = name_match.group(2).strip().strip("\"'")
            step_name = name or "<unnamed step>"
            for hit in hits[step_hit_start:]:
                value = hit["statement"].split(": continue-on-error: ", 1)[1]
                hit["command"] = f"{step_name}: continue-on-error: {value}"
                hit["statement"] = hit["command"]
        match = CONTINUE_ON_ERROR_RE.match(raw)
        if not match or step_indent is None:
            continue
        match_indent = len(match.group(1))
        dash_form = match.group(2) is not None
        if ((dash_form and match_indent != step_indent)
                or (not dash_form and match_indent != step_indent + 2)):
            continue
        value = match.group(3).strip().strip("\"'")
        if value.lower() in FALSE_CONTINUE_VALUES:
            continue
        statement = f"{step_name}: continue-on-error: {match.group(3).strip()}"
        hits.append({"command": statement, "head": "continue-on-error",
                     "statement": statement, "kind": "continue-on-error"})
    return hits


def find_swallows(body: str, folded: bool) -> list[dict]:
    """Every W1 candidate in one `run:` body: {command, head, statement}."""
    hits: list[dict] = []
    for line in logical_lines(body, folded):
        masked = mask_quotes_and_expressions(line)
        for match in SWALLOW_RE.finditer(masked):
            # Slice BOTH at the same index — see head_command on why the real and
            # masked spans must stay index-aligned.
            end = match.start()
            head = head_command(line[:end], masked[:end])
            hits.append({"command": line, "head": head,
                         "statement": line[:end].rstrip()})
    return hits


def _statement_kind(masked: str) -> str:
    """`status` (reads `$?`), `set` (a bare `set` builtin), or `command`."""
    if STATUS_READ_RE.search(masked):
        return "status"
    if SET_ONLY_RE.fullmatch(masked):
        return "set"
    return "command"


def _status_is_enforcing(masked: str, real: str, later: list[str]) -> str | None:
    """None if this status read acts on the status; else why it does not.

    `VAR=$?` only counts when `$VAR` is read somewhere afterwards — a captured
    status nobody looks at is the same swallow with an extra step. `later` is
    the RAW (unmasked) remainder of the body, because the read is almost always
    inside double quotes (`if [ "$rc" -ne 0 ]`) which masking blanks out.
    """
    assign = STATUS_ASSIGN_RE.search(masked)
    if assign:
        var = assign.group(1)
        reference = re.compile(r"\$\{?" + re.escape(var) + r"\b")
        tail = [real[assign.end():], *later]
        if any(reference.search(text) for text in tail):
            return None
        return (f"captures the status into `{var}` and then never reads `${var}` "
                f"— a status nobody looks at is the swallow with an extra step")
    if STATUS_BRANCH_RE.search(masked):
        return None
    return ("reads `$?` without acting on it (no `VAR=$?` capture, no branch or "
            "`exit` on it) — printing a status is not inspecting it")


def find_unread_set_plus_e(body: str, folded: bool) -> list[str]:
    """W2: `set +e` regions that do not immediately capture and act on the status.

    A region runs from `set +e` to the next `set -e` (or the end of the body).
    The obligation is discharged only by the disciplined `set +e / cmd / rc=$? /
    set -e` idiom all 7 live sites use: the status read must be the statement
    IMMEDIATELY after the tolerated command (otherwise it is some other
    command's status), and it must enforce something. See W2 in the header for
    the one bound this does not cover.
    """
    lines = logical_lines(body, folded)
    masked_lines = [mask_quotes_and_expressions(line) for line in lines]
    offences: list[str] = []
    index = 0
    while index < len(lines):
        opener_match = SET_PLUS_E_RE.search(masked_lines[index])
        if not opener_match:
            index += 1
            continue
        opener = index
        end = opener + 1
        while end < len(lines) and not SET_MINUS_E_RE.search(masked_lines[end]):
            end += 1

        # (real, masked, index-of-the-line-it-came-from). A `set +e` can share a
        # line with the command it tolerates (`set +e; ./gate`), so the opener's
        # remainder is the region's first statement when it is non-empty.
        region: list[tuple[str, str, int]] = []
        tail_real = lines[opener][opener_match.end():]
        if tail_real.strip(" ;&|"):
            region.append((tail_real, masked_lines[opener][opener_match.end():], opener))
        region.extend((lines[i], masked_lines[i], i) for i in range(opener + 1, end))

        kinds = [_statement_kind(m) for _, m, _ in region]
        if "command" in kinds:
            first = kinds.index("command")
            statement = region[first][0].strip()
            if first + 1 >= len(region) or kinds[first + 1] != "status":
                offences.append(
                    f"opens a `set +e` region whose first tolerated command "
                    f"{statement[:120]!r} is not followed IMMEDIATELY by a status "
                    f"read — the next statement runs first, so any later `$?` is "
                    f"that statement's status, not this command's"
                )
            else:
                real, masked, line_index = region[first + 1]
                why = _status_is_enforcing(masked, real, lines[line_index + 1:])
                if why:
                    offences.append(
                        f"opens a `set +e` region whose tolerated command "
                        f"{statement[:120]!r} {why}"
                    )
        index = end + 1 if end < len(lines) else len(lines)
    return offences


def scan(root: Path) -> tuple[list[dict], list[str]]:
    """Return (W1/W4 swallow sites, W2 offences) across gating jobs."""
    registry = _ADV.load_registry(root)
    wf_dir = root / ".github" / "workflows"
    sites: list[dict] = []
    set_e_offences: list[str] = []
    if not wf_dir.is_dir():
        return sites, set_e_offences

    for path in sorted(wf_dir.iterdir()):
        if path.suffix not in (".yml", ".yaml"):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for job in _ADV.parse_jobs(text):
            # A job GATES unless it is DECLARED advisory — the same rule
            # ci_summary_gate.py applies to a check-run since #3773.
            if job["name"] in registry:
                continue
            for hit in find_continue_on_error(job["block_text"]):
                sites.append({
                    "workflow": path.name,
                    "job_id": job["id"],
                    "job_name": job["name"],
                    **hit,
                })
            for body, folded in extract_run_blocks(job["block_text"]):
                for hit in find_swallows(body, folded):
                    sites.append({
                        "workflow": path.name,
                        "job_id": job["id"],
                        "job_name": job["name"],
                        "kind": "run-body",
                        **hit,
                    })
                for detail in find_unread_set_plus_e(body, folded):
                    set_e_offences.append(
                        f"W2: {path.name}: gating job {job['name']!r} (job_id "
                        f"{job['id']}) {detail}. Tolerating a failure and never "
                        f"inspecting it is indistinguishable from not running the "
                        f"command. Capture the status on the very next statement "
                        f"(`rc=$?`) and branch on it, or drop the `set +e`."
                    )
    return sites, set_e_offences


# ---------------------------------------------------------------------------
# Waivers
# ---------------------------------------------------------------------------

WAIVER_REQUIRED_FIELDS = ("workflow", "job_id", "match", "reason", "registered")
# A `match` must pin the actual command, not rubber-stamp a job. Short substrings
# like "|| true" would waive everything the job ever grows.
MIN_MATCH_LENGTH = 12


def load_waivers(root: Path) -> tuple[list[dict], list[str]]:
    """Load .github/run-swallow-waivers.json → (waivers, malformed-entry errors)."""
    path = root / WAIVERS_PATH
    if not path.exists():
        return [], []
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as exc:
        return [], [f"W3: {WAIVERS_PATH} is unreadable: {exc}"]
    waivers = data.get("waivers", [])
    if not isinstance(waivers, list):
        return [], [f"W3: {WAIVERS_PATH}: `waivers` must be a list"]
    errors: list[str] = []
    clean: list[dict] = []
    for index, entry in enumerate(waivers):
        if not isinstance(entry, dict):
            errors.append(f"W3: {WAIVERS_PATH}: waiver #{index} is not an object")
            continue
        missing = [f for f in WAIVER_REQUIRED_FIELDS if not entry.get(f)]
        if missing:
            errors.append(
                f"W3: {WAIVERS_PATH}: waiver #{index} "
                f"({entry.get('workflow')}:{entry.get('job_id')}) is missing "
                f"required fields {missing}"
            )
            continue
        if len(str(entry["match"])) < MIN_MATCH_LENGTH:
            errors.append(
                f"W3: {WAIVERS_PATH}: waiver #{index} `match` "
                f"{entry['match']!r} is shorter than {MIN_MATCH_LENGTH} chars — "
                f"too broad. A waiver must pin the specific command it excuses, "
                f"not every swallow the job might grow."
            )
            continue
        clean.append(entry)
    return clean, errors


def waiver_matches(waiver: dict, site: dict) -> bool:
    return (
        waiver["workflow"] == site["workflow"]
        and waiver["job_id"] == site["job_id"]
        and str(waiver["match"]) in site["command"]
    )


def check(root: Path) -> list[str]:
    """Full W1 + W2 + W3 check. Returns human-readable offences (empty = clean)."""
    sites, offences = scan(root)
    waivers, waiver_errors = load_waivers(root)
    offences = list(offences)
    offences.extend(waiver_errors)

    used: set[int] = set()
    bindings: dict[int, list[dict]] = {i: [] for i in range(len(waivers))}
    for site in sites:
        matched = [i for i, w in enumerate(waivers) if waiver_matches(w, site)]
        if matched:
            used.update(matched)
            for index in matched:
                bindings[index].append(site)
            continue
        rule = "W4" if site.get("kind") == "continue-on-error" else "W1"
        mechanism = ("uses step-level `continue-on-error`" if rule == "W4" else
                     f"swallows the failure of `{site['head'] or '?'}` with `|| true`")
        offences.append(
            f"{rule}: {site['workflow']}: gating job {site['job_name']!r} (job_id "
            f"{site['job_id']}) {mechanism}: {site['statement'][:160]!r}. The step reports "
            f"success and the job stays green, so `ci-summary / gate` counts a "
            f"check-run that asserted nothing. Remove the swallow, handle the "
            f"status explicitly, or declare it in {WAIVERS_PATH}."
        )

    for index, waiver in enumerate(waivers):
        if len(bindings[index]) > 1:
            commands = ", ".join(repr(site["statement"][:80])
                                 for site in bindings[index])
            offences.append(
                f"W3: {WAIVERS_PATH}: waiver for {waiver['workflow']}:"
                f"{waiver['job_id']} matching {waiver['match']!r} binds to "
                f"{len(bindings[index])} live commands ({commands}). A waiver "
                f"must identify exactly one site; narrow its `match` and declare "
                f"each additional swallow separately."
            )
        if index not in used:
            offences.append(
                f"W3: {WAIVERS_PATH}: waiver for {waiver['workflow']}:"
                f"{waiver['job_id']} matching {waiver['match']!r} matches no live "
                f"command — STALE. Delete it: a waiver that binds to nothing "
                f"silently pre-authorises a FUTURE swallow of that shape."
            )
    return offences


# ---------------------------------------------------------------------------
# Self-test — hermetic fixtures
# ---------------------------------------------------------------------------

_WF_HEADER = "name: t\non: [pull_request]\njobs:\n"


def _wf(job_id: str, name: str, run_body: str, folded: bool = False) -> str:
    style = ">-" if folded else "|"
    indented = "\n".join(f"          {l}" for l in run_body.splitlines())
    return (
        f"{_WF_HEADER}  {job_id}:\n    name: {name}\n    runs-on: ubuntu-latest\n"
        f"    steps:\n      - run: {style}\n{indented}\n"
    )


def _scan_text(workflow_text: str, registry: dict) -> tuple[list[dict], list[str]]:
    """scan() over synthetic text — mirrors the real loop without touching disk."""
    sites: list[dict] = []
    set_e: list[str] = []
    for job in _ADV.parse_jobs(workflow_text):
        if job["name"] in registry:
            continue
        for hit in find_continue_on_error(job["block_text"]):
            sites.append({"workflow": "t.yml", "job_id": job["id"],
                          "job_name": job["name"], **hit})
        for body, folded in extract_run_blocks(job["block_text"]):
            for hit in find_swallows(body, folded):
                sites.append({"workflow": "t.yml", "job_id": job["id"],
                              "job_name": job["name"], "kind": "run-body", **hit})
            set_e.extend(find_unread_set_plus_e(body, folded))
    return sites, set_e


def _w1_cases() -> int:
    """Each case pins the HEADS of the flagged sites, so the parser tests assert
    what the parser actually resolved instead of leaning on an exemption list."""
    cases = [
        ("W1-negative: a repo script silenced in a gating job",
         _wf("g", "hard gate", "python3 scripts/coverage-gate.py --check || true"),
         {}, ["python3"]),
        ("W1-clean: the same swallow in a DECLARED advisory job",
         _wf("g", "hard gate", "python3 scripts/coverage-gate.py --check || true"),
         {"hard gate": {}}, []),
        # No head is exempt: these three are the counter-examples that killed the
        # OBSERVATIONAL_COMMANDS idea. Each is an ASSERTION whose verdict `|| true`
        # deletes, spelled with a head that "reads like" a passive probe.
        ("W1-negative: `grep` can be the assertion itself, not a data probe",
         _wf("g", "hard gate", "grep -q 'expected result' out.log || true"), {}, ["grep"]),
        ("W1-negative: `find -exec` runs an arbitrary validator",
         _wf("g", "hard gate",
             "find . -name '*.ttl' -exec ./scripts/validate.sh {} \\; || true"), {}, ["find"]),
        ("W1-negative: `cat` of a REQUIRED generated artifact is a check",
         _wf("g", "hard gate", "cat target/generated-manifest.json || true"), {}, ["cat"]),
        ("W1-negative: `|| :` is the same swallow spelled differently",
         _wf("g", "hard gate", "cargo test --all || :"), {}, ["cargo"]),
        ("W1-clean: `|| :.foo` is not a swallow (`:` is not the whole command)",
         _wf("g", "hard gate", "cargo test --all || :.foo"), {}, []),
        # The parsing bugs found while measuring main's real inventory. These assert
        # the resolved head verb — the thing that was wrong before the fix.
        ("W1-parse: FOLDED scalar — the command is on earlier source lines",
         _wf("g", "hard gate",
             "cargo miri nextest run -p sparq-core\n> shard.jsonl 2> shard.stderr\n|| true",
             folded=True), {}, ["cargo"]),
        ("W1-parse: backslash continuation joins into one command",
         _wf("g", "hard gate",
             "python3 scripts/cone_coverage.py \\\n  --mode report \\\n  --summary-file x || true"),
         {}, ["python3"]),
        ("W1-parse: shell metacharacters inside a QUOTED argument are payload",
         _wf("g", "hard gate",
             "gh issue comment 1 --body \"quarantined (see |x| and {y})\" || true"),
         {}, ["gh"]),
        ("W1-negative: swallow inside a quoted command substitution",
         _wf("g", "hard gate", 'out="$(cmd || true)"'), {}, ["cmd"]),
        ("W1-negative: quoted command substitution used as an argument",
         _wf("g", "hard gate", ': "$(cmd || true)"'), {}, ["cmd"]),
        ("W1-negative: continued pipeline inside a quoted command substitution",
         _wf("g", "hard gate", 'out="$(cmd \\\n  | head -n1 || true)"'), {}, ["head"]),
        ("W1-negative: quoted hash is not a shell comment",
         _wf("g", "hard gate", 'cmd --title "a # b" || true'), {}, ["cmd"]),
        ("W1-parse: a redirection is not the command word",
         _wf("g", "hard gate", "ls -la target/release >&2 || true"), {}, ["ls"]),
        ("W1-parse: an env-var prefix is not the command word",
         _wf("g", "hard gate", "FOO=1 BAR=2 ./scripts/thing.sh || true"),
         {}, ["thing.sh"]),
        ("W1-parse: `sudo`/`env` wrappers resolve to the wrapped command",
         _wf("g", "hard gate", "sudo cat /var/log/x || true"), {}, ["cat"]),
        # Both of these read as a `head` of junk (`'n'`, `' '`) before the mask/real
        # index alignment and the process-substitution normalisation.
        ("W1-parse: an `a || b || true` chain reads b, not a redirection target",
         _wf("g", "hard gate",
             "git fetch --unshallow origin 2>/dev/null || "
             "git fetch --deepen=200 origin 2>/dev/null || true"), {}, ["git"]),
        ("W1-parse: process substitution `< <(…)` does not eat the command word",
         _wf("g", "hard gate",
             "done < <(git ls-tree -d --name-only origin/x dev/ "
             "| grep -E '^dev/bench' || true)"), {}, ["grep"]),
        ("W1-clean: a swallow in a COMMENT is not an invocation",
         _wf("g", "hard gate", "# cargo test --all || true\necho ok"), {}, []),
    ]
    failures = 0
    for label, text, registry, expected in cases:
        got = [site["head"] for site in _scan_text(text, registry)[0]]
        ok = got == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}: {got} (want {expected})")
        if not ok:
            for site in _scan_text(text, registry)[0]:
                print(f"         site: head={site['head']!r} {site['command']!r}")
            failures += 1
    return failures


def _w2_cases() -> int:
    disciplined = (
        "set +e\n"
        "python3 scripts/perf-gate.py --update-baseline a.json b.json\n"
        "rc=$?\n"
        "set -e\n"
        "if [ \"$rc\" != \"0\" ]; then exit \"$rc\"; fi"
    )
    captured_no_restore = (
        "set +e\n"
        "REPORT=\"$(cargo deny check advisories 2>&1)\"\n"
        "CODE=$?\n"
        "echo \"exit_code=$CODE\" >> \"$GITHUB_OUTPUT\"\n"
        "exit 0"
    )
    unread = "set +e\ncargo test --all\necho done\nexit 0"
    unread_restored = "set +e\ncargo test --all\nset -e\necho done"
    # The three ways a region can read `$?` and still swallow the verdict.
    overwritten = (
        "set +e\n"
        "./scripts/gate.sh\n"
        "echo 'diagnostic'\n"
        "rc=$?\n"
        "set -e\n"
        "if [ \"$rc\" != \"0\" ]; then exit \"$rc\"; fi"
    )
    printed_only = "set +e\n./scripts/gate.sh\necho $?\nset -e"
    captured_unused = "set +e\n./scripts/gate.sh\nrc=$?\nset -e\necho done"
    branch_direct = (
        "set +e\n"
        "./scripts/gate.sh\n"
        "if [ $? -ne 0 ]; then exit 1; fi\n"
        "set -e"
    )
    inline_capture = "set +e\n./scripts/gate.sh || rc=$?\nset -e\nexit \"${rc:-0}\""
    same_line_opener = "set +e; cargo test --all\nset -e"
    cases = [
        ("W2-clean: the disciplined set +e / rc=$? / set -e idiom", disciplined, {}, 0),
        ("W2-clean: status captured and routed to an output, no set -e restore",
         captured_no_restore, {}, 0),
        ("W2-clean: the status is branched on directly (`if [ $? -ne 0 ]`)",
         branch_direct, {}, 0),
        ("W2-clean: the capture shares the command's statement (`cmd || rc=$?`)",
         inline_capture, {}, 0),
        ("W2-negative: set +e region where `$?` is never read", unread, {}, 1),
        ("W2-negative: set +e ... set -e with no `$?` read in between",
         unread_restored, {}, 1),
        ("W2-negative: an intervening command OVERWRITES the status before `rc=$?`",
         overwritten, {}, 1),
        ("W2-negative: `echo $?` prints the status but enforces nothing",
         printed_only, {}, 1),
        ("W2-negative: the captured status is never read again",
         captured_unused, {}, 1),
        ("W2-negative: `set +e; cmd` on ONE line still owes a capture",
         same_line_opener, {}, 1),
        ("W2-clean: the same unread region in a DECLARED advisory job",
         unread, {"hard gate": {}}, 0),
    ]
    failures = 0
    for label, body, registry, expected in cases:
        got = len(_scan_text(_wf("g", "hard gate", body), registry)[1])
        ok = got == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}: {got} (want {expected})")
        if not ok:
            failures += 1
    return failures


def _w3_cases() -> int:
    site = {"workflow": "ci.yml", "job_id": "test", "job_name": "test",
            "command": "./scripts/ci-free-disk.sh || true",
            "statement": "./scripts/ci-free-disk.sh", "head": "ci-free-disk.sh"}
    good = {"workflow": "ci.yml", "job_id": "test",
            "match": "./scripts/ci-free-disk.sh || true", "reason": "r",
            "registered": "2026-09-02"}
    cases = [
        ("W3-clean: waiver binds to the live command", good, site, True),
        ("W3-negative: right command, WRONG job", {**good, "job_id": "other"},
         site, False),
        ("W3-negative: right job, WRONG workflow", {**good, "workflow": "bench.yml"},
         site, False),
        ("W3-negative: match text absent from the command",
         {**good, "match": "./scripts/other-thing.sh"}, site, False),
    ]
    failures = 0
    for label, waiver, target, expected in cases:
        got = waiver_matches(waiver, target)
        ok = got == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}: {got} (want {expected})")
        if not ok:
            failures += 1
    # A too-short `match` must be REFUSED, not silently honoured.
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / ".github").mkdir()
        (root / WAIVERS_PATH).write_text(json.dumps(
            {"waivers": [{**good, "match": "|| true"}]}), encoding="utf-8")
        _, errors = load_waivers(root)
        ok = len(errors) == 1
        print(f"  [{'PASS' if ok else 'FAIL'}] W3-negative: a too-short `match` is "
              f"refused: {len(errors)} error(s) (want 1)")
        if not ok:
            failures += 1
        (root / WAIVERS_PATH).write_text(json.dumps(
            {"waivers": [{k: v for k, v in good.items() if k != "reason"}]}),
            encoding="utf-8")
        _, errors = load_waivers(root)
        ok = len(errors) == 1
        print(f"  [{'PASS' if ok else 'FAIL'}] W3-negative: a waiver with no "
              f"`reason` is refused: {len(errors)} error(s) (want 1)")
        if not ok:
            failures += 1
    return failures


def _w4_cases() -> int:
    def workflow(value: str, *, job_level: bool = False) -> str:
        if job_level:
            return (
                f"{_WF_HEADER}  g:\n    name: hard gate\n"
                f"    continue-on-error: {value}\n    runs-on: ubuntu-latest\n"
                "    steps:\n      - name: assertion\n        run: cargo test\n"
            )
        return (
            f"{_WF_HEADER}  g:\n    name: hard gate\n    runs-on: ubuntu-latest\n"
            "    steps:\n      - name: assertion\n        run: cargo test\n"
            f"        continue-on-error: {value}\n"
        )

    cases = [
        ("W4-negative: literal true", workflow("true"), {}, 1),
        ("W4-negative: quoted true", workflow("'true'"), {}, 1),
        ("W4-negative: expression form", workflow("${{ matrix.soft }}"), {}, 1),
        ("W4-clean: literal false", workflow("false"), {}, 0),
        ("W4-clean: quoted false", workflow("\"false\""), {}, 0),
        ("W4-clean: job-level key is outside this rule", workflow("true", job_level=True), {}, 0),
        ("W4-clean: declared advisory job", workflow("true"), {"hard gate": {}}, 0),
        ("W4-negative: continue-on-error is the first step key",
         f"{_WF_HEADER}  g:\n    name: hard gate\n    runs-on: ubuntu-latest\n"
         "    steps:\n      - continue-on-error: true\n        name: assertion\n"
         "        run: cargo test\n", {}, 1),
        ("W4-negative: id is the first step key",
         f"{_WF_HEADER}  g:\n    name: hard gate\n    runs-on: ubuntu-latest\n"
         "    steps:\n      - id: assertion\n        run: cargo test\n"
         "        continue-on-error: true\n", {}, 1),
        ("W4-negative: if is the first step key",
         f"{_WF_HEADER}  g:\n    name: hard gate\n    runs-on: ubuntu-latest\n"
         "    steps:\n      - if: always()\n        run: cargo test\n"
         "        continue-on-error: true\n", {}, 1),
    ]
    failures = 0
    for label, text, registry, expected in cases:
        got = len([s for s in _scan_text(text, registry)[0]
                   if s.get("kind") == "continue-on-error"])
        ok = got == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}: {got} (want {expected})")
        if not ok:
            failures += 1
    return failures


def self_test() -> int:
    failures = _w1_cases() + _w2_cases() + _w3_cases() + _w4_cases()
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
        description="#5799/#6226: workflow failure-swallow check (W1/W2/W3/W4)")
    parser.add_argument("--root", type=Path, default=REPO_ROOT,
                        help="repo root (default: inferred from script location)")
    parser.add_argument("--inventory", action="store_true",
                        help="print every swallow site in a gating job and exit 0")
    parser.add_argument("--self-test", action="store_true",
                        help="run hermetic fixtures and exit")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    if args.inventory:
        sites, set_e = scan(args.root)
        waivers, _ = load_waivers(args.root)
        undeclared = 0
        for site in sites:
            declared = any(waiver_matches(w, site) for w in waivers)
            kind = "waived" if declared else "NEEDS-WAIVER"
            undeclared += 0 if declared else 1
            print(f"{kind:14} {site['workflow']}:{site['job_id']} "
                  f"head={site['head']!r}  {site['command'][:120]}")
        for offence in set_e:
            print(offence)
        print(f"\n{len(sites)} swallow site(s), {undeclared} needing a waiver; "
              f"{len(set_e)} unguarded `set +e` region(s).")
        return 0

    offences = check(args.root)
    if not offences:
        print("check-workflow-run-swallow: all clear (W1 + W2 + W3 + W4)")
        return 0

    print(f"check-workflow-run-swallow: {len(offences)} offence(s):\n")
    for offence in offences:
        print(f"  {offence}")
    print()
    print(f"  W1: a `|| true` / `|| :` in a GATING job's run body makes the step "
          f"unable to fail. Fix the command, handle its status explicitly, or add "
          f"a {WAIVERS_PATH} entry with a reason.")
    print("  W2: `set +e` is only legitimate when the status is then READ (`rc=$?`) "
          "and acted on. A region that never reads `$?` swallows silently.")
    print(f"  W3: every {WAIVERS_PATH} entry must carry "
          f"{list(WAIVER_REQUIRED_FIELDS)} and must match a live command; delete "
          f"stale entries rather than leaving them to pre-authorise new swallows.")
    print("  W4: step-level `continue-on-error` in a GATING job makes that step "
          "non-fatal. Remove it or declare the specific step with a reason.")
    return 1


_ADV = _load_advisory_module()

if __name__ == "__main__":
    sys.exit(main())
