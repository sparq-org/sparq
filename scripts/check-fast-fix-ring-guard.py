#!/usr/bin/env python3
# [OPUS-4.8] sparq-org/sparq#3475 — fail-closed REGRESSION guard for the fast-fix
# ring's cross-repo trust boundary. 🤖 SPARQ agent. Authored by Opus 4.8 (Fable
# routed off security surfaces by design; flag for re-review when Fable returns).
#
# =============================================================================
# THREAT MODEL — read this first; it fixes the bar this checker is judged against
# =============================================================================
# This script is a REGRESSION DETECTOR. Its job is to catch an ACCIDENTAL or an
# agent-introduced WEAKENING of the fast-fix ring's cross-repo trust guard when a
# change touches `.github/workflows/fast-fix-ring.yml` — a change that a HUMAN then
# ARMS (workflow edits are human-armed per repo policy). It is NOT, and cannot be, a
# defense against an actor who ALREADY HAS repo-write and DELIBERATELY edits the
# workflow to exfiltrate the secret: such an actor could weaken the guard AND update
# this checker's golden in the same commit, or exfiltrate REGISTRY_RING_TOKEN by many
# other means (a brand-new workflow, a different job, an exfiltrating step). Defending
# against a malicious committer is OUT OF SCOPE here and is handled elsewhere —
# branch protection, required human review of `.github/workflows/**` changes, and the
# LIVE security control itself, which is the workflow's own
#     github.event.workflow_run.head_repository.full_name == github.repository
# job guard plus the SHA/repo defense-in-depth. THIS script does not enforce security
# at runtime; the WORKFLOW does. This script only makes an accidental regression to
# that guard LOUD in review.
#
# =============================================================================
# STRATEGY — PIN THE WHOLE WORKFLOW DOCUMENT (byte-exact, semantically-inert-normalized)
# =============================================================================
# Rounds 1–5 of adversarial review proved that EVERY narrower pin loses to a moving
# target, because the attacker can always weaken the token's protection by editing a
# LEVEL of the file the narrower rule does not pin:
#
#   * A predicate-PARSING validator cannot win: for every "reject a top-level ||" rule
#     there is a `(A && B && guard || true)` paren-wrap; for every "the guard regex is
#     present" rule there is a `!(guard)` negation; for every "both needles present" G3
#     rule there is a conjunction→disjunction `repo OR SHA` swap (both needles survive).
#
#   * A FRAGMENT golden-match (rounds 1–3) leaked: it pinned only the EXTRACTED pieces
#     (the `if:`, the `select(...)`, the guard skeletons) and validated only the FIRST
#     token-consuming job, so an inline `: # select(...)` demotion, an uncalled-function
#     move of the guards, or an alt dispatch spelling lived OUTSIDE the extracted text.
#
#   * A WHOLE-JOB pin (round 4) closed the fragment/demotion/duplication holes WITHIN
#     the pinned job and added exactly-one-consumer to stop a SECOND privileged job.
#     But round 5 found that the job is not the top level of the file: an attacker can
#     reach REGISTRY_RING_TOKEN by editing the workflow ABOVE / BESIDE the pinned job —
#       (a) a second consumer written with `${{ secrets['REGISTRY_RING_TOKEN'] }}`
#           BRACKET syntax (a different string than the dotted needle the consumer scan
#           and the golden job both keyed on);
#       (b) a workflow-level `env:` that binds the token and is INHERITED by a second
#           job whose own steps never mention the secret string at all;
#       (c) a workflow-level `defaults.run.shell: bash -x {0}` custom-shell wrapper that
#           intercepts/echoes the token (or wraps every `run:` body) BEFORE the pinned
#           job's script runs — the pinned job text is byte-identical, yet the token is
#           exposed by machinery OUTSIDE it.
#     All three edits leave the pinned JOB byte-identical, so a whole-job pin passes
#     them. The root cause is structural: the job is not the top of the file, so there
#     is always a level above it to weaken.
#
# Codex round 5 gave the terminal design in its own words: "Pin the COMPLETE workflow
# document." So this checker pins the ENTIRE file:
#
#   PIN THE WHOLE FILE. Read `.github/workflows/fast-fix-ring.yml`, NORMALIZE it with
#   ONLY semantically-inert transforms (strip trailing whitespace per line + collapse to
#   a single trailing newline — NOT comment-stripping or reordering, because comments
#   and order ARE part of the pin), do the SAME normalization to the stored golden
#   `scripts/tests/fast-fix-ring.golden.yml`, and require BYTE-EQUALITY. Any difference
#   emits ONE offence tagged WORKFLOW-PIN carrying a unified diff (golden vs actual).
#
# WHY THIS IS TERMINAL (and why it SUBSUMES the whole-job pin):
#   There is NO level above the whole file. Every round-5 bypass — and every round-1..4
#   bypass, which the whole-job pin already caught — is an EDIT TO
#   `.github/workflows/fast-fix-ring.yml`: a second consumer in any syntax (dotted or
#   `secrets['...']` bracket), a workflow-level `env:` a second job inherits, a
#   workflow-level `defaults.run.shell` wrapper, a second job, a permissions change, an
#   `if:` weakening, a demoted-to-comment predicate, an uncalled-function move, an alt
#   dispatch spelling — ALL of them change the file's bytes. The whole-file pin looks at
#   every byte, so:
#
#       {edits that reach the token AND pass the checker}
#         = {edits that leave the whole file byte-identical after inert normalization}
#         = {no change to the workflow}.
#
#   A weakening is, by definition, a change to the workflow, so it cannot pass. The
#   whole-JOB pin was the special case of this that pinned only the job subtree; the
#   whole-FILE pin pins that subtree AND everything around it (workflow-level env,
#   defaults, permissions, a second job, comments, any secret-reference syntax), so it
#   strictly SUBSUMES the whole-job pin. There is no narrower/wider level left to move
#   to — this is why it is terminal, not another turn in the arms race.
#
#   The ONLY way to change the workflow AND pass is to regenerate the golden in the SAME
#   PR — which is exactly the intended human-review checkpoint (a reviewer sees the
#   golden move in the diff and re-confirms the guard is still sound), the CORRECT
#   fail-safe direction.
#
# The golden `scripts/tests/fast-fix-ring.golden.yml` is a byte-exact (inert-normalized)
# copy of the known-good workflow. It is human-readable ON PURPOSE — it IS the pinned
# content, so a reviewer reads exactly what is pinned. Regenerate after an intentional
# refactor, IN THE SAME PR as the workflow change, with:
#     python3 scripts/check-fast-fix-ring-guard.py --update-golden
# (or copy the normalized workflow into the golden by hand). The golden bump is the
# review checkpoint.
#
# NEGATIVE COVERAGE (--self-test, hermetic — no gh/git/network): the live workflow
# passes, and EACH round-1..5 bypass — the round-1..4 guard removals / OR-wraps /
# negations / demotions / uncalled-function move / alt dispatch spelling / second job,
# PLUS the round-5 bracket-syntax second consumer, workflow-level `env:` + second job,
# and workflow-level `defaults.run.shell` wrapper — is applied as a SURGICAL mutation to
# a PHYSICAL COPY of the real workflow and run through the LIVE `--root` path, and goes
# RED; the unmutated copy passes. See _self_test for the mutation→red table.
#
# Usage:
#   check-fast-fix-ring-guard.py                 # check the live workflow (default)
#   check-fast-fix-ring-guard.py --root DIR      # use DIR as repo root
#   check-fast-fix-ring-guard.py --self-test     # hermetic negative fixtures
#   check-fast-fix-ring-guard.py --update-golden # regenerate the golden (LOCAL ONLY)
#
# Exit 0 = all clean; exit 1 = one or more offences. Pure stdlib — no PyYAML needed for
# the whole-file pin (a byte compare parses nothing), which also removes any parser as
# an attack surface.

from __future__ import annotations

import argparse
import difflib
import os
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_REL = Path(".github") / "workflows" / "fast-fix-ring.yml"
WORKFLOW = REPO_ROOT / WORKFLOW_REL
GOLDEN_REL = Path("scripts") / "tests" / "fast-fix-ring.golden.yml"
GOLDEN = REPO_ROOT / GOLDEN_REL


class Offence(Exception):
    pass


def normalize(text: str) -> str:
    """Semantically-inert normalization ONLY: strip trailing whitespace from each line
    and collapse to a single trailing newline. Deliberately does NOT strip comments and
    does NOT reorder anything — comments, key order, and blank lines ARE part of the pin
    (a predicate demoted into a comment, or a reordered/added key, MUST red). Applied
    IDENTICALLY to the workflow and the golden so the compare ignores only editor-inert
    whitespace churn."""
    lines = [ln.rstrip() for ln in text.split("\n")]
    return "\n".join(lines).rstrip("\n") + "\n"


def check_doc(text: str, golden_text: str | None = None) -> list[str]:
    """Return a list of offence strings; empty == clean. WHOLE-FILE PIN: the normalized
    workflow must byte-equal the normalized golden. Any difference is ONE WORKFLOW-PIN
    offence with a unified diff. This pin subsumes the former whole-job pin — it covers
    the job AND everything around it (workflow-level env/defaults/permissions, a second
    job, comments, any secret-reference syntax), because there is no level above the
    whole file."""
    if golden_text is None:
        if not GOLDEN.exists():
            raise Offence(
                f"{GOLDEN_REL} not found — the whole-file pin has no golden to compare "
                f"against; the golden is security-load-bearing (#3475) and must be "
                f"committed alongside this checker."
            )
        golden_text = GOLDEN.read_text(encoding="utf-8")

    actual = normalize(text)
    golden = normalize(golden_text)
    if actual == golden:
        return []

    diff = "\n".join(
        difflib.unified_diff(
            golden.splitlines(),
            actual.splitlines(),
            fromfile=str(GOLDEN_REL),
            tofile=str(WORKFLOW_REL) + " (actual)",
            lineterm="",
        )
    )
    return [
        "WORKFLOW-PIN: the fast-fix ring workflow no longer matches its pinned golden "
        f"({GOLDEN_REL}). The WHOLE FILE is pinned byte-for-byte (after inert "
        "trailing-whitespace/newline normalization), so ANY edit reds — a job-level "
        "`if:` OR-wrap / negation / operand swap, a demoted-to-comment predicate, an "
        "uncalled-function move, an alt dispatch spelling, a dropped `select(...)` "
        "conjunct, an inverted `!=` guard, a changed permission, a SECOND token "
        "consumer in ANY syntax (dotted or `secrets['...']` bracket), a workflow-level "
        "`env:` a second job inherits, or a workflow-level `defaults.run.shell` "
        "wrapper. the fast-fix ring workflow changed; if intentional, regenerate "
        f"{GOLDEN_REL} IN THE SAME PR (the human-review checkpoint); this workflow is "
        "security-load-bearing (#3475).\n"
        f"       --- unified diff ({GOLDEN_REL} vs actual) ---\n{diff}"
    ]


# The SCOPE contract the workflow must carry in its own header. Keyed to the needles a
# DECOMPOSER greps for, not to prose: each is a literal string a bead spec would have to
# copy to scope the work correctly.
SCOPE_DOC_NEEDLES = (
    GOLDEN_REL.as_posix(),
    WORKFLOW_REL.as_posix(),
    "python3 scripts/check-fast-fix-ring-guard.py --update-golden",
    "TWO-FILE",
)


def check_scope_doc(text: str) -> list[str]:
    """[OPUS-5] issue #5235. The whole-file pin makes EVERY edit to the workflow a
    two-file change (workflow + golden). That contract is only useful if it is stated
    where a decomposer reads it — in the workflow itself, which is the single path a bead
    spec names. This keeps the SCOPE block from being deleted or drifting.

    The pin alone does NOT cover this: a golden bump that drops the block leaves the two
    files byte-consistent, so `check_doc` stays clean while the contract quietly vanishes.
    That is the gap this closes — and the --self-test fixture mutates BOTH files to prove
    the check is not merely shadowing the pin."""
    missing = [n for n in SCOPE_DOC_NEEDLES if n not in text]
    if not missing:
        return []
    return [
        "SCOPE-DOC: the fast-fix ring workflow no longer states its TWO-FILE scope "
        f"contract; missing {missing}. {WORKFLOW_REL.as_posix()} is byte-pinned to "
        f"{GOLDEN_REL.as_posix()}, so ANY edit to it is a two-file change — a task scoped "
        "to the workflow alone forbids the golden regeneration it requires and must "
        "decline (issue #2384's shape, swept in #5235). Restore the SCOPE block in the "
        "workflow header: it must name BOTH in-scope paths, say TWO-FILE, and give the "
        "exact regeneration command "
        "`python3 scripts/check-fast-fix-ring-guard.py --update-golden`."
    ]


def check_root(root: Path) -> list[str]:
    """Live path used by both the default run and every --self-test fixture: read the
    workflow from `root` and compare it against the golden that ALSO lives under `root`
    (so a fixture that mutates ONLY the workflow copy reds, and a fixture could also
    exercise a golden edit). The self-test mutates a PHYSICAL COPY of the real workflow
    on disk and calls THIS, so the fixtures exercise the exact code path that runs in
    CI."""
    wf = root / WORKFLOW_REL
    if not wf.exists():
        raise Offence(f"{wf} not found")
    golden_path = root / GOLDEN_REL
    if not golden_path.exists():
        raise Offence(f"{golden_path} not found")
    wf_text = wf.read_text(encoding="utf-8")
    return check_doc(
        wf_text,
        golden_path.read_text(encoding="utf-8"),
    ) + check_scope_doc(wf_text)


def _update_golden() -> int:
    """Convenience: rewrite the golden from the CURRENT live workflow (inert-normalized).
    LOCAL ONLY — guarded so CI never regenerates the pin it is meant to enforce. Run this
    IN THE SAME PR as an intentional workflow change so the golden bump is reviewed."""
    if os.environ.get("CI") or os.environ.get("GITHUB_ACTIONS"):
        print(
            "refusing to --update-golden under CI: the golden is the human-review "
            "checkpoint and must be regenerated locally + committed in the same PR.",
            file=sys.stderr,
        )
        return 1
    if not WORKFLOW.exists():
        print(f"error: {WORKFLOW} not found", file=sys.stderr)
        return 1
    GOLDEN.parent.mkdir(parents=True, exist_ok=True)
    GOLDEN.write_text(normalize(WORKFLOW.read_text(encoding="utf-8")), encoding="utf-8")
    print(f"regenerated {GOLDEN_REL} from the current {WORKFLOW_REL} (review this bump).")
    return 0


# --------------------------------------------------------------------------- #
#  --self-test: hermetic negative fixtures                                    #
# --------------------------------------------------------------------------- #

def _self_test() -> int:
    text = WORKFLOW.read_text(encoding="utf-8")
    golden_text = GOLDEN.read_text(encoding="utf-8")
    failures = 0

    def _run_on(mutated: str, mutated_golden: str | None = None) -> list[str]:
        """Write the (mutated) workflow AND a golden into a throwaway repo-root tree and
        run the LIVE --root path over it, so every fixture goes through
        check_root/check_doc exactly as CI does. The golden defaults to the pristine one
        (so a workflow-only mutation reds the pin); pass `mutated_golden` to move BOTH
        files together, which is how the #5235 SCOPE-DOC fixture isolates that check from
        the pin."""
        golden_for_run = golden_text if mutated_golden is None else mutated_golden
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            wf = root / WORKFLOW_REL
            wf.parent.mkdir(parents=True, exist_ok=True)
            wf.write_text(mutated, encoding="utf-8")
            gp = root / GOLDEN_REL
            gp.parent.mkdir(parents=True, exist_ok=True)
            gp.write_text(golden_for_run, encoding="utf-8")
            return check_root(root)

    def expect_clean(label: str, t: str) -> None:
        nonlocal failures
        offs = _run_on(t)
        if offs:
            failures += 1
            print(f"  [FAIL] {label}: expected CLEAN but got offences:")
            for o in offs:
                print(f"           - {o}")
        else:
            print(f"  [ok]   {label}: clean (as expected)")

    def expect_red(label: str, t: str, needle: str) -> None:
        nonlocal failures
        if t == text:
            failures += 1
            print(f"  [FAIL] {label}: MUTATION DID NOT APPLY (fixture == live workflow)")
            return
        offs = _run_on(t)
        if not any(needle in o for o in offs):
            failures += 1
            print(
                f"  [FAIL] {label}: expected an offence containing {needle!r} but got: {offs}"
            )
        else:
            print(f"  [ok]   {label}: correctly RED ({needle})")

    # 0) the real, live workflow (physical copy) must be clean against the golden.
    expect_clean("live workflow (physical copy via --root, golden matches)", text)

    # An inert-only edit (add trailing whitespace on some lines) must STAY clean — proves
    # the normalization is genuinely semantically inert and does not over-fire.
    inert_trailing_ws = text.replace(
        "runs-on: ubuntu-latest\n", "runs-on: ubuntu-latest   \n", 1
    )
    expect_clean("inert trailing whitespace (must stay clean)", inert_trailing_ws)

    # --- round-1 fixtures: a guard is REMOVED (whole-file pin => WORKFLOW-PIN) ---

    # 1) FORK RUN: remove the head_repository guard from the job `if:`.
    no_guard = text.replace(
        "\n      && github.event.workflow_run.head_repository.full_name == github.repository",
        "",
        1,
    )
    expect_red("R1 fork run (head_repository guard removed)", no_guard, "WORKFLOW-PIN")

    # 2) SAME-SHA collision: drop the `.head.sha ==` filter from the fallback select.
    no_sha_filter = text.replace(
        'and .head.sha == \\"${HEAD_SHA}\\" and .state', "and .state", 1
    )
    expect_red("R1 same-SHA collision (head.sha filter removed)", no_sha_filter, "WORKFLOW-PIN")

    # 3) STALE-HEAD: remove the live head_sha re-check block before dispatch.
    no_live_recheck = text.replace(
        '          if [ "$(jq -r \'.head_sha\' <<<"$state")" != "${HEAD_SHA}" ]; then\n'
        '            echo "resolved PR #${PR_NUMBER} head SHA no longer matches the failing run\'s head SHA (stale head / same-SHA collision) — skipping."\n'
        "            exit 0\n"
        "          fi\n",
        "",
        1,
    )
    expect_red("R1 stale-head (live head_sha re-check removed)", no_live_recheck, "WORKFLOW-PIN")

    # --- round-2 fixtures: predicate-parser was too loose (whole-file pin => WORKFLOW-PIN) ---

    # 4) TOP-LEVEL `|| true` appended to the if (bare, depth-0).
    or_bypass = text.replace(
        "&& github.event.workflow_run.head_repository.full_name == github.repository\n",
        "&& github.event.workflow_run.head_repository.full_name == github.repository\n"
        "      || true\n",
        1,
    )
    expect_red("R2 top-level `|| true` bypass appended", or_bypass, "WORKFLOW-PIN")

    # 5) fork-repo predicate removed from the fallback `select(...)`.
    no_repo_predicate = text.replace(
        'select(.head.repo.full_name == \\"${REPO}\\" and .head.sha == \\"${HEAD_SHA}\\" '
        'and .state == \\"open\\")',
        'select(.head.sha == \\"${HEAD_SHA}\\" and .state == \\"open\\")',
        1,
    )
    expect_red("R2 fallback fork-repo predicate removed", no_repo_predicate, "WORKFLOW-PIN")

    # 6) .head.sha compared to a CONSTANT instead of ${HEAD_SHA}.
    sha_constant = text.replace(
        '.head.sha == \\"${HEAD_SHA}\\" and .state',
        '.head.sha == \\"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\\" and .state',
        1,
    )
    expect_red("R2 fallback head.sha compared to a constant", sha_constant, "WORKFLOW-PIN")

    # 7) live `!=` guards inverted to `==` (fail-open).
    inverted_guards = text.replace(
        'if [ "$(jq -r \'.head_repo\' <<<"$state")" != "${REPO}" ]; then',
        'if [ "$(jq -r \'.head_repo\' <<<"$state")" == "${REPO}" ]; then',
        1,
    ).replace(
        'if [ "$(jq -r \'.head_sha\' <<<"$state")" != "${HEAD_SHA}" ]; then',
        'if [ "$(jq -r \'.head_sha\' <<<"$state")" == "${HEAD_SHA}" ]; then',
        1,
    )
    expect_red("R2 live `!=` guards inverted to `==` (fail-open)", inverted_guards, "WORKFLOW-PIN")

    # --- round-3 fixtures: bypasses codex demonstrated against the PARSER ---

    # 8) [R3] PARENTHESIZED OR-WRAP of the whole condition (actionlint-valid).
    paren_or_wrap = text.replace(
        "    if: >-\n"
        "      github.event.workflow_run.conclusion == 'failure'\n"
        "      && github.event.workflow_run.event == 'pull_request'\n"
        "      && github.event.workflow_run.head_repository.full_name == github.repository\n",
        "    if: >-\n"
        "      (github.event.workflow_run.conclusion == 'failure'\n"
        "      && github.event.workflow_run.event == 'pull_request'\n"
        "      && github.event.workflow_run.head_repository.full_name == github.repository\n"
        "      || true)\n",
        1,
    )
    expect_red("R3 parenthesized `(... || true)` OR-wrap of the whole if", paren_or_wrap, "WORKFLOW-PIN")

    # 9) [R3] NEGATED equality `!(head_repo == repo)`.
    negated_guard = text.replace(
        "&& github.event.workflow_run.head_repository.full_name == github.repository",
        "&& !(github.event.workflow_run.head_repository.full_name == github.repository)",
        1,
    )
    expect_red("R3 negated `!(head_repo == repo)` guard", negated_guard, "WORKFLOW-PIN")

    # 10) [R3] G3 CONJUNCTION->DISJUNCTION: `repo AND sha` -> `repo OR sha` in select.
    or_conjunction = text.replace(
        '.head.repo.full_name == \\"${REPO}\\" and .head.sha == \\"${HEAD_SHA}\\"',
        '.head.repo.full_name == \\"${REPO}\\" or .head.sha == \\"${HEAD_SHA}\\"',
        1,
    )
    expect_red("R3 fallback conjunction->disjunction (repo OR sha)", or_conjunction, "WORKFLOW-PIN")

    # 11) [R3] PREDICATE MOVED TO A COMMENT: the executable select drops the fork-repo
    #     predicate but a `#`-comment retains the needle text. The whole-file pin
    #     includes the run body verbatim (comment + weakened select), so it REDs.
    predicate_in_comment = text.replace(
        '            if ! pulls="$(gh api "repos/${REPO}/commits/${HEAD_SHA}/pulls" \\\n'
        '                --jq "[.[] | select(.head.repo.full_name == \\"${REPO}\\" and .head.sha == \\"${HEAD_SHA}\\" and .state == \\"open\\")] | .[0].number // empty")"; then\n',
        '            # select(.head.repo.full_name == \\"${REPO}\\" and .head.sha == \\"${HEAD_SHA}\\" and .state == \\"open\\")\n'
        '            if ! pulls="$(gh api "repos/${REPO}/commits/${HEAD_SHA}/pulls" \\\n'
        '                --jq "[.[] | select(.head.sha == \\"${HEAD_SHA}\\" and .state == \\"open\\")] | .[0].number // empty")"; then\n',
        1,
    )
    expect_red("R3 fork-repo predicate demoted to a comment (select drops it)", predicate_in_comment, "WORKFLOW-PIN")

    # --- round-4 fixtures: codex's residual fragment/duplication holes ---

    # 12) [R4] GUARDS MOVED TO AN UNCALLED FUNCTION: wrap both live `!=` guards in a
    #     `_dead_guards() { ... }` that is never called, so the needle text survives
    #     but no guard executes before dispatch. Whole-file pin REDs (run body changed).
    moved_to_uncalled_fn = text.replace(
        '          if [ "$(jq -r \'.head_repo\' <<<"$state")" != "${REPO}" ]; then\n'
        '            echo "resolved PR #${PR_NUMBER} head repo is not ${REPO} (fork / cross-repo collision) — skipping."\n'
        "            exit 0\n"
        "          fi\n"
        '          if [ "$(jq -r \'.head_sha\' <<<"$state")" != "${HEAD_SHA}" ]; then\n'
        '            echo "resolved PR #${PR_NUMBER} head SHA no longer matches the failing run\'s head SHA (stale head / same-SHA collision) — skipping."\n'
        "            exit 0\n"
        "          fi\n",
        "          _dead_guards() {\n"
        '          if [ "$(jq -r \'.head_repo\' <<<"$state")" != "${REPO}" ]; then\n'
        "            exit 0\n"
        "          fi\n"
        '          if [ "$(jq -r \'.head_sha\' <<<"$state")" != "${HEAD_SHA}" ]; then\n'
        "            exit 0\n"
        "          fi\n"
        "          }\n",
        1,
    )
    expect_red("R4 live guards moved into an uncalled function", moved_to_uncalled_fn, "WORKFLOW-PIN")

    # 13) [R4] ALT DISPATCH SPELLING: `gh workflow run -R <repo> dispatch.yml` (flag
    #     before the workflow file) instead of the golden `gh workflow run dispatch.yml
    #     -R <repo>`. Whole-file pin REDs on any run-body change.
    alt_dispatch_spelling = text.replace(
        "GH_TOKEN=\"${REGISTRY_RING_TOKEN}\" gh workflow run dispatch.yml -R jeswr/agent-account-registry",
        "GH_TOKEN=\"${REGISTRY_RING_TOKEN}\" gh workflow run -R jeswr/agent-account-registry dispatch.yml",
        1,
    )
    expect_red("R4 alt dispatch spelling (gh workflow run -R ... dispatch.yml)", alt_dispatch_spelling, "WORKFLOW-PIN")

    # 14) [R4] SECOND PRIVILEGED TOKEN-CONSUMING JOB (dotted `secrets.` syntax) with the
    #     SAME golden `if:` but NO SHA/repo defenses. The whole-file pin REDs on the
    #     added job (the old whole-job pin caught this via exactly-one-consumer; the
    #     whole-file pin catches it because the file changed).
    second_job = text.replace(
        "jobs:\n",
        "jobs:\n"
        "  ring_evil:\n"
        "    name: second ring (bypass)\n"
        "    if: >-\n"
        "      github.event.workflow_run.conclusion == 'failure'\n"
        "      && github.event.workflow_run.event == 'pull_request'\n"
        "      && github.event.workflow_run.head_repository.full_name == github.repository\n"
        "    runs-on: ubuntu-latest\n"
        "    steps:\n"
        "      - name: second unguarded ring\n"
        "        env:\n"
        "          REGISTRY_RING_TOKEN: ${{ secrets.REGISTRY_RING_TOKEN }}\n"
        "        run: |\n"
        "          GH_TOKEN=\"${REGISTRY_RING_TOKEN}\" gh workflow run dispatch.yml -R jeswr/agent-account-registry\n",
        1,
    )
    expect_red("R4 second privileged token-consuming job (dotted syntax)", second_job, "WORKFLOW-PIN")

    # --- round-5 fixtures: the bypasses that BEAT the whole-JOB pin (this is why the ---
    # --- pin had to widen to the whole FILE). Each leaves the pinned JOB byte-identical ---
    # --- but reaches the token by editing the file AROUND/ABOVE the job. ---

    # 15) [R5] SECOND CONSUMER via `secrets['REGISTRY_RING_TOKEN']` BRACKET SYNTAX: a
    #     second job that binds the token with the bracket spelling, a DIFFERENT string
    #     than the dotted `secrets.REGISTRY_RING_TOKEN` needle the old whole-job consumer
    #     scan keyed on — so the old exactly-one-consumer count missed it. The whole-file
    #     pin does not parse or key on any needle; it REDs because the file changed.
    bracket_second_consumer = text.replace(
        "jobs:\n",
        "jobs:\n"
        "  ring_bracket:\n"
        "    name: second ring (bracket bypass)\n"
        "    if: >-\n"
        "      github.event.workflow_run.conclusion == 'failure'\n"
        "    runs-on: ubuntu-latest\n"
        "    steps:\n"
        "      - name: second unguarded ring via bracket syntax\n"
        "        env:\n"
        "          REGISTRY_RING_TOKEN: ${{ secrets['REGISTRY_RING_TOKEN'] }}\n"
        "        run: |\n"
        "          GH_TOKEN=\"${REGISTRY_RING_TOKEN}\" gh workflow run dispatch.yml -R jeswr/agent-account-registry\n",
        1,
    )
    expect_red(
        "R5 second consumer via secrets['REGISTRY_RING_TOKEN'] bracket syntax",
        bracket_second_consumer,
        "WORKFLOW-PIN",
    )

    # 16) [R5] WORKFLOW-LEVEL `env:` INHERITED BY A SECOND JOB: bind the token in a
    #     top-level `env:` block, then add a second job whose OWN steps never mention the
    #     secret string at all — it inherits the token from the workflow-level env. A
    #     whole-JOB pin (which only looked inside the pinned job's steps) is byte-blind to
    #     both the workflow-level env and the inheriting job's step. The whole-file pin
    #     REDs on both edits.
    workflow_env_inherit = text.replace(
        "permissions:\n  contents: read\n",
        "permissions:\n  contents: read\n"
        "\nenv:\n"
        "  REGISTRY_RING_TOKEN: ${{ secrets.REGISTRY_RING_TOKEN }}\n",
        1,
    ).replace(
        "jobs:\n",
        "jobs:\n"
        "  ring_inherit:\n"
        "    name: second ring (inherits workflow env)\n"
        "    if: >-\n"
        "      github.event.workflow_run.conclusion == 'failure'\n"
        "    runs-on: ubuntu-latest\n"
        "    steps:\n"
        "      - name: exfil via inherited workflow-level token\n"
        "        run: |\n"
        "          GH_TOKEN=\"${REGISTRY_RING_TOKEN}\" gh workflow run dispatch.yml -R jeswr/agent-account-registry\n",
        1,
    )
    expect_red(
        "R5 workflow-level env: token inherited by a second job",
        workflow_env_inherit,
        "WORKFLOW-PIN",
    )

    # 17) [R5] WORKFLOW-LEVEL `defaults.run.shell` CUSTOM-SHELL WRAPPER: set
    #     `defaults.run.shell: bash -x {0}` at the workflow level. `-x` traces every
    #     command (echoing the token-bearing dispatch line to the log), and more
    #     generally a workflow-level custom shell wraps EVERY `run:` body — machinery
    #     that intercepts the token BEFORE the pinned job's script runs, while the pinned
    #     job text stays byte-identical. A whole-JOB pin is blind to it; the whole-file
    #     pin REDs.
    defaults_shell_wrapper = text.replace(
        "permissions:\n  contents: read\n",
        "permissions:\n  contents: read\n"
        "\ndefaults:\n"
        "  run:\n"
        "    shell: bash -x {0}\n",
        1,
    )
    expect_red(
        "R5 workflow-level defaults.run.shell: bash -x {0} wrapper",
        defaults_shell_wrapper,
        "WORKFLOW-PIN",
    )

    # --- #5235 fixture: the SCOPE contract must survive in the workflow header ------
    # Delete the whole SCOPE block from the workflow AND the golden TOGETHER. The two
    # files stay byte-consistent, so the whole-file pin is CLEAN and only SCOPE-DOC reds
    # — which is what makes this a real test of check_scope_doc rather than a second look
    # at check_doc. It is exactly the drift the pin cannot see: a golden bump that quietly
    # drops the two-file contract, re-opening the #2384 one-file-scope trap.
    _SCOPE_START = "# SCOPE — THE TWO-FILE RULE"
    _SCOPE_END = "# WHY THIS IS ITS OWN"

    def _strip_scope(t: str) -> str:
        head, sep, rest = t.partition(_SCOPE_START)
        if not sep:
            return t
        _, end_sep, tail = rest.partition(_SCOPE_END)
        return head + end_sep + tail if end_sep else t

    scope_deleted = _strip_scope(text)
    scope_deleted_golden = _strip_scope(golden_text)
    if scope_deleted == text or scope_deleted_golden == golden_text:
        failures += 1
        print(
            "  [FAIL] #5235 SCOPE-DOC deletion: MUTATION DID NOT APPLY — the SCOPE block "
            f"markers ({_SCOPE_START!r} / {_SCOPE_END!r}) were not found in both the "
            "workflow and the golden."
        )
    else:
        offs = _run_on(scope_deleted, mutated_golden=scope_deleted_golden)
        if any("WORKFLOW-PIN" in o for o in offs):
            failures += 1
            print(
                "  [FAIL] #5235 SCOPE-DOC deletion: the whole-file pin fired, so this "
                "fixture is not isolating check_scope_doc (workflow and golden were "
                f"meant to move together): {offs}"
            )
        elif not any("SCOPE-DOC" in o for o in offs):
            failures += 1
            print(
                "  [FAIL] #5235 SCOPE-DOC deletion: expected a SCOPE-DOC offence "
                f"(pin clean, contract gone) but got: {offs}"
            )
        else:
            print(
                "  [ok]   #5235 SCOPE-DOC deletion: correctly RED (SCOPE-DOC) with the "
                "whole-file pin CLEAN — the check is independent of the pin"
            )

    if failures:
        print(f"\nSELF-TEST FAILED: {failures} case(s) did not behave as expected.")
        return 1
    print(
        "\nSELF-TEST PASSED: live workflow clean + all round-1..5 negative fixtures RED "
        "(whole-FILE byte-pin WORKFLOW-PIN, each verified through the live --root path "
        "against a physically-mutated workflow copy). The round-5 bracket-consumer / "
        "workflow-env-inherit / defaults-shell fixtures — which BEAT the whole-JOB pin — "
        "all red because there is no level above the whole file. Plus the #5235 fixture: "
        "deleting the header SCOPE block from the workflow AND the golden together leaves "
        "the pin clean and reds SCOPE-DOC, so the two-file scope contract cannot drift "
        "out of the file the decomposer reads."
    )
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="fast-fix-ring cross-repo guard checker")
    ap.add_argument("--root", type=Path, default=None, help="repo root override")
    ap.add_argument(
        "--self-test", action="store_true", help="run hermetic negative fixtures"
    )
    ap.add_argument(
        "--update-golden",
        action="store_true",
        help="regenerate scripts/tests/fast-fix-ring.golden.yml from the current "
        "workflow (LOCAL ONLY — refuses to run under CI)",
    )
    args = ap.parse_args()

    if args.update_golden:
        return _update_golden()
    if args.self_test:
        return _self_test()

    root = args.root if args.root is not None else REPO_ROOT
    try:
        offences = check_root(root)
    except Offence as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    if offences:
        print("fast-fix-ring cross-repo guard check FAILED (#3475):", file=sys.stderr)
        for o in offences:
            print(f"  - {o}", file=sys.stderr)
        return 1
    print("fast-fix-ring cross-repo guard check: OK (#3475 whole-file pin matches golden).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
