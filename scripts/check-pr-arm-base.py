#!/usr/bin/env python3
# [OPUS-4.8] PreToolUse arm guard (bead sq-u59rq): NEVER auto-merge a PR whose BASE
# is another PR's branch (a "stacked" PR). Authored by Opus 4.8 (Fable unavailable;
# flag for re-review when Fable returns).
#
# WHY THIS GUARD EXISTS — the stacked-PR auto-merge trap (observed 2026-06-21, #1023):
# When you chain PR B onto PR A's branch to avoid a re-conflict (B's base = A's head
# branch, not `main`) and then arm BOTH for auto-merge, GitHub squash-merges B into
# its STACKED BASE (A's branch), not `main`, the moment A merges first. GitHub marks
# B "merged" and auto-deletes B's head branch (unreopenable) — but B's content never
# reaches `main`. The fix that day was to re-land identical content as a fresh PR
# (#1028). The standing rule (AGENTS.md, *Arming model* / *Stacked PRs*): do NOT arm
# auto-merge on a PR whose base is not `main`. Stack strictly sequentially, or retarget
# the upper PR's base back to `main` (`gh pr edit <n> --base main`) once the lower PR
# lands, THEN arm it.
#
# WHAT THIS DOES: it is a deterministic, network-light **PreToolUse hook** on `Bash`
# (sibling to the sparq-perf-reviewer agent-hook in .claude/settings.json) that fires
# on the arming step. It:
#   1. reads the PreToolUse hook JSON from stdin (tool_input.command),
#   2. ALLOWs untouched anything that is NOT a `gh pr merge … --auto` arming command,
#   3. for an arming command, parses the PR number, asks `gh pr view <n> --json
#      baseRefName,headRefName,number`, and
#   4. DENIES the arm (permissionDecision: "deny") when baseRefName != the trunk
#      (default `main`) — i.e. the PR is stacked on another branch — with a 🤖 SPARQ
#      reason telling the operator to retarget the base to `main` (or land sequentially)
#      before arming; otherwise ALLOWs.
# It is cheap (no model spend) and complements the perf-reviewer: both run on the same
# matcher and BOTH must allow for the arm to proceed (any deny blocks the tool call).
#
# [OPUS-5] SECOND GUARD, issue #1135 — NEVER let an agent arm the release-plz RELEASE PR.
# This hook is the choke point for AGENT-typed arms (`.claude/workflows/*.js`, hand-written
# `gh pr merge … --auto`), which bypass scripts/auto-arm.py and scripts/rearm-sweeper.py
# entirely. Merging the Release PR cuts a `v*` tag and — once `publish = true` lands in
# release-plz.toml — `cargo publish`es 37 crates. A crates.io version can NEVER be
# unpublished, so this branch is FAIL-CLOSED, keyed on branch/author/title
# (scripts/release_pr_guard.py) and never on a label.
#
# FAILURE DISPOSITION — DIFFERENT PER AXIS, deliberately:
#   * STACKED-BASE axis: unchanged, FAIL-OPEN on this guard's own errors. A wrongly-merged
#     stacked PR is recoverable (#1023 was re-landed as #1028), and the perf-reviewer +
#     ci-summary + branch protection still gate the merge.
#   * RELEASE-PR axis: FAIL-CLOSED. [OPUS-5] #1135 CHANGES THE PRE-EXISTING BEHAVIOUR: a
#     failed `gh pr view` now DENIES instead of allowing. Rationale: the outcome it
#     protects against is irreversible, and the cost of the deny is near zero — if `gh pr
#     view` cannot answer, the very next `gh pr merge` almost certainly cannot either, so
#     the deny forfeits an arm that would have failed anyway. The operator retries, or a
#     maintainer merges by hand.
#
# WHAT THIS HOOK DOES **NOT** COVER — stated so the coverage is not over-read (PR #4192):
# `is_arm_command` matches `gh pr merge` + `--auto`, so this hook sees exactly the
# `--auto` phrasings. Executed against a fake `gh`, these reach the tool unblocked:
#   * `gh pr merge <n> --squash` (no `--auto`) and `--admin` — a DIRECT merge, which is
#     the more dangerous operation; the deliberate scope choice is that this hook governs
#     ARMING, and the self-test pins it,
#   * `gh api graphql … enablePullRequestAutoMerge(…)` — the shape scripts/auto-arm.py
#     itself uses — and `gh api -X PUT repos/:o/:r/pulls/<n>/merge`,
#   * backslash line-continuation between `gh` and `pr merge`, and shell-variable
#     indirection (`M="pr merge"; gh $M <n> --auto`).
# What actually bounds all of those is the BELT, not this hook:
# scripts/release-interval-guard.py runs inside release-plz.yml itself, so however the
# Release PR is merged, no release can be cut inside MIN_RELEASE_INTERVAL.
#
# THE WRAPPER MUST NOT INVERT THIS. `.claude/settings.json` invokes this script from a
# shell one-liner. That wrapper used to emit `allow` whenever the script could not RUN
# (missing/unreadable/crashing, or `CLAUDE_PROJECT_DIR` unset), inverting the fail-closed
# disposition above at the exact moment there is no guard. It now DENIES an unrunnable
# guard for any `gh pr merge` and allows everything else (this hook matches every Bash
# call, so a blanket deny would brick the fleet).
# scripts/tests/test_release_publish_guard.py::TestSettingsJsonWrapperDoesNotInvertTheGuard
# executes the wrapper string out of settings.json and reds if it reverts to `allow`.
#
# Usage:
#   check-pr-arm-base.py                 # read hook JSON on stdin, emit decision JSON
#   check-pr-arm-base.py --trunk main    # override the trunk branch name (default main)
#   check-pr-arm-base.py --self-test     # hermetic logic self-test (no network)
#
# stdlib-only.

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# [OPUS-5] #1135. A missing guard module must NOT degrade to "allow": the stub denies
# every arm. This hook is imported from an ordinary (non-sparse) checkout, so the stub
# is a last-resort safety net, not an expected path.
try:
    import release_pr_guard

    RELEASE_GUARD_DEGRADED = False
except ImportError:  # pragma: no cover - see the self-test

    class _FailClosedReleaseGuard:
        _REASON = (
            "release-pr-guard: scripts/release_pr_guard.py is not importable, so this "
            "arm cannot be proven safe against the release-plz Release PR — denying "
            "(fail-closed, #1135)"
        )

        @staticmethod
        def arm_block_reason(**_kwargs) -> str:
            return _FailClosedReleaseGuard._REASON

    release_pr_guard = _FailClosedReleaseGuard  # type: ignore[assignment]
    RELEASE_GUARD_DEGRADED = True

DEFAULT_TRUNK = "main"

SELF_ID = (
    "🤖 SPARQ stacked-PR arm guard"
)

# `gh pr merge` with `--auto` (the arming step). We also accept the future `--merge-when`
# style only via `--auto`, which is the canonical arming flag the orchestrator uses.
_GH_PR_MERGE_RE = re.compile(r"\bgh\s+pr\s+merge\b")
_AUTO_RE = re.compile(r"(?<![\w-])--auto(?![\w-])")


def _allow(reason: str) -> dict:
    return {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": reason,
        }
    }


def _deny(reason: str) -> dict:
    return {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    }


def is_arm_command(command: str) -> bool:
    """True iff `command` is a `gh pr merge … --auto` arming invocation."""
    if not command:
        return False
    return bool(_GH_PR_MERGE_RE.search(command) and _AUTO_RE.search(command))


def parse_pr_number(command: str) -> str | None:
    """Extract the PR number/URL argument from a `gh pr merge` command.

    `gh pr merge` takes the PR as its first positional (`<number> | <url> | <branch>`).
    We tokenise with shlex and return the first token after `merge` that is NOT an
    option (doesn't start with `-`) and is NOT the value of a preceding value-taking
    flag. We accept a bare number, a PR URL (…/pull/<n>), or a branch name — `gh`
    resolves all three; we only need to hand the SAME token back to `gh pr view`.
    """
    try:
        toks = shlex.split(command)
    except ValueError:
        return None
    # Find the `merge` subcommand position (after `gh pr`).
    try:
        i = toks.index("merge")
    except ValueError:
        return None
    # Flags that consume the following token as their value (so that value is NOT the
    # positional PR arg). Conservative list covering gh pr merge's value-flags.
    value_flags = {
        "-b",
        "--body",
        "-F",
        "--body-file",
        "-s",
        "--subject",
        "-t",
        "--subject",
        "--match-head-commit",
        "--author-email",
        "-R",
        "--repo",
    }
    j = i + 1
    n = len(toks)
    while j < n:
        tok = toks[j]
        if tok.startswith("-"):
            # `--flag=value` consumes nothing extra; a bare value-flag eats the next.
            if "=" not in tok and tok in value_flags:
                j += 2
            else:
                j += 1
            continue
        return tok
    return None


def pr_facts(pr: str, trunk: str) -> tuple[dict | None, str | None]:
    """Return (facts, error) for a PR.

    ``facts`` carries every field both axes need:
    ``{"base": str, "head_ref": str|None, "author_login": str|None, "title": str|None}``.
    ``error`` is set (and facts None) when gh cannot answer. Callers FAIL OPEN on error
    for the stacked-base axis and FAIL CLOSED for the release-PR axis (see the header).
    """
    try:
        proc = subprocess.run(
            [
                "gh",
                "pr",
                "view",
                pr,
                "--json",
                # [OPUS-5] #1135: headRefName/author/title are the release-PR guard's
                # inputs. Dropping one makes the guard fail CLOSED, never open.
                "baseRefName,headRefName,number,author,title",
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )
    except FileNotFoundError:
        return None, "gh CLI not found"
    except subprocess.TimeoutExpired:
        return None, "gh pr view timed out"
    except OSError as e:  # pragma: no cover - defensive
        return None, f"gh invocation failed: {e}"
    if proc.returncode != 0:
        return None, (proc.stderr or proc.stdout or "gh pr view failed").strip()
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return None, "gh pr view returned non-JSON"
    base = data.get("baseRefName")
    if not isinstance(base, str) or not base:
        return None, "gh pr view returned no baseRefName"
    author = data.get("author")
    return (
        {
            "base": base,
            "head_ref": data.get("headRefName"),
            "author_login": (
                author.get("login") if isinstance(author, dict) else None
            ),
            "title": data.get("title"),
        },
        None,
    )


def decide(command: str, trunk: str, base_lookup) -> dict:
    """Pure decision logic. `base_lookup(pr) -> (facts, error)` is injected so the
    self-test can exercise this without a network/gh dependency. `facts` is the dict
    documented on :func:`pr_facts`; a bare ``str`` is accepted as a base-only shorthand."""
    if not is_arm_command(command):
        return _allow("not a `gh pr merge … --auto` arming command; base guard N/A")
    pr = parse_pr_number(command)
    if pr is None:
        # [OPUS-5] #1135: an UNIDENTIFIABLE PR cannot be proven not to be the Release PR,
        # and this is an arm command, so the release axis fails CLOSED here — a change
        # from the previous allow. Denying costs one retry; admitting can publish.
        return _deny(
            f"{SELF_ID}: could not parse a PR argument from a `gh pr merge … --auto` "
            "command, so it cannot be proven not to be the release-plz Release PR "
            "(#1135). Re-issue the arm with an explicit PR number."
        )
    facts, error = base_lookup(pr)
    if error is not None:
        # [OPUS-5] #1135: the release axis fails CLOSED on a lookup failure. See the
        # header — a `gh pr view` that cannot answer means the following `gh pr merge`
        # almost certainly cannot either, so the deny forfeits little and the outcome it
        # prevents (an irreversible crates.io publish) cannot be undone at all.
        return _deny(
            f"{SELF_ID}: could not resolve PR #{pr} ({error}), so it cannot be proven "
            "not to be the release-plz Release PR — arming is refused (fail-closed, "
            "#1135). Retry once `gh` responds, or have a maintainer merge by hand."
        )
    if isinstance(facts, str):  # base-only shorthand
        facts = {"base": facts, "head_ref": None, "author_login": None, "title": None}
    base = facts.get("base")

    # [OPUS-5] #1135 — the RELEASE-PR axis, checked BEFORE the stacked-base axis because
    # the Release PR's base IS the trunk (so the stacked check would happily allow it).
    release_reason = release_pr_guard.arm_block_reason(
        head_ref=facts.get("head_ref"),
        author_login=facts.get("author_login"),
        title=facts.get("title"),
    )
    if release_reason:
        return _deny(
            f"{SELF_ID}: refusing to arm PR #{pr} — {release_reason}. Merging the "
            "release-plz Release PR cuts a `v*` tag and, once `publish = true` in "
            "release-plz.toml, publishes every crate in the version_group to crates.io. "
            "A crates.io version can NEVER be unpublished. The Release PR is merged by a "
            "MAINTAINER, by hand, deliberately — never armed. This decision is keyed on "
            "the branch/author/title, so relabelling the PR cannot change it."
        )

    if base == trunk:
        return _allow(
            f"{SELF_ID}: PR #{pr} base is `{trunk}` (not stacked) — arm OK"
        )
    return _deny(
        f"{SELF_ID}: PR #{pr} base is `{base}`, NOT `{trunk}` — this is a STACKED PR. "
        "Arming auto-merge on a PR stacked on another branch squash-merges it into that "
        "base (not `main`) the moment the lower PR lands, so its content never reaches "
        "`main` and its head branch auto-deletes (unreopenable; bead sq-u59rq, #1023). "
        f"Retarget the base to `{trunk}` first (`gh pr edit {pr} --base {trunk}`) once "
        "the lower PR has landed, THEN arm — or land the stack strictly sequentially."
    )


# --------------------------------------------------------------------------- tests
def self_test() -> int:
    trunk = "main"

    def _facts(base, *, head_ref="sparq-agent/issue-1-worker", author="jeswr",
               title="fix(engine): a change"):
        return {
            "base": base,
            "head_ref": head_ref,
            "author_login": author,
            "title": title,
        }

    def base_main(_pr):
        return _facts("main"), None

    def base_stacked(_pr):
        return _facts("site/sq-1022-explain"), None

    def base_error(_pr):
        return None, "gh CLI not found"

    # [OPUS-5] #1135: the release-plz Release PR — base IS `main`, so ONLY the release
    # axis can catch it. Its head branch, author, and title are all release-plz's.
    def base_release_pr(_pr):
        return (
            _facts(
                "main",
                head_ref="release-plz-main",
                author="app/github-actions",
                title="chore: release v0.2.0",
            ),
            None,
        )

    # A Release PR whose head branch gh did not report — must still be refused.
    def base_release_pr_unknown_head(_pr):
        return (
            _facts("main", head_ref=None, author="app/github-actions", title=None),
            None,
        )

    cases = [
        # (label, command, base_lookup, expected_decision)
        (
            "not an arm command (no --auto)",
            "gh pr merge 1044 --squash",
            base_main,
            "allow",
        ),
        (
            "not a gh pr merge at all",
            "gh pr create --fill",
            base_main,
            "allow",
        ),
        (
            "arm, base=main → allow",
            "gh pr merge 1044 --auto --squash",
            base_main,
            "allow",
        ),
        (
            "arm, base=stacked branch → DENY",
            "gh pr merge 1023 --auto --squash",
            base_stacked,
            "deny",
        ),
        (
            "arm by URL, base=stacked → DENY",
            "gh pr merge https://github.com/sparq-org/sparq/pull/1023 --auto --squash",
            base_stacked,
            "deny",
        ),
        (
            "arm with --body value flag before PR num, base=stacked → DENY",
            'gh pr merge --body "ship it" 1023 --auto --squash',
            base_stacked,
            "deny",
        ),
        (
            # [OPUS-5] #1135 BEHAVIOUR CHANGE (was: fail OPEN / allow). An arm whose PR
            # cannot be resolved cannot be proven not to be the Release PR, and the
            # outcome that admits — a crates.io publish — is irreversible.
            "arm, gh lookup error → fail CLOSED (deny, #1135)",
            "gh pr merge 1023 --auto --squash",
            base_error,
            "deny",
        ),
        (
            # Same reason: no parseable PR argument, so nothing can be proven.
            "arm with no PR argument → fail CLOSED (deny, #1135)",
            "gh pr merge --auto --squash",
            base_main,
            "deny",
        ),
        (
            "arm, --auto as --auto=… style still detected (base=stacked) → DENY",
            "gh pr merge 1023 --squash --auto",
            base_stacked,
            "deny",
        ),
        (
            "substring 'autograph' must NOT count as --auto",
            "gh pr merge 1044 --autograph",
            base_stacked,
            "allow",
        ),
        # ------------------------------------------------------- #1135 Release-PR axis
        # THE CASE THE STACKED-BASE GUARD CANNOT CATCH: the Release PR's base IS `main`,
        # so `base == trunk` allows it. Only the release axis denies. `base_main` above
        # ALLOWS on the same command shape, which makes this a discriminating pair.
        (
            "arm the release-plz Release PR (base IS main) → DENY (#1135)",
            "gh pr merge 900 --auto --squash",
            base_release_pr,
            "deny",
        ),
        (
            "arm a Release PR whose head branch gh did not report → DENY (#1135)",
            "gh pr merge 900 --auto --squash",
            base_release_pr_unknown_head,
            "deny",
        ),
        (
            # NOT an arm command: the release axis must not police non-arming calls.
            "`gh pr merge` on the Release PR WITHOUT --auto is not this hook's business",
            "gh pr merge 900 --squash",
            base_release_pr,
            "allow",
        ),
    ]
    failures = 0
    for label, cmd, lookup, expected in cases:
        out = decide(cmd, trunk, lookup)
        got = out["hookSpecificOutput"]["permissionDecision"]
        ok = got == expected
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}: {got} (want {expected})")
        if not ok:
            failures += 1

    # parse_pr_number unit checks
    pn_cases = [
        ("gh pr merge 1023 --auto --squash", "1023"),
        ("gh pr merge --repo sparq-org/sparq 1023 --auto", "1023"),
        ("gh pr merge https://github.com/sparq-org/sparq/pull/9 --auto",
         "https://github.com/sparq-org/sparq/pull/9"),
        ("gh pr merge --auto --squash", None),
    ]
    for cmd, want in pn_cases:
        got = parse_pr_number(cmd)
        ok = got == want
        print(f"  [{'PASS' if ok else 'FAIL'}] parse_pr_number({cmd!r}): "
              f"{got!r} (want {want!r})")
        if not ok:
            failures += 1

    # [GPT-5.6] #3605: exercise the real hook command with fixture input from both
    # the repository root and an unrelated cwd. Claude Code deliberately runs hooks
    # in the shell's current cwd, so the settings entry MUST anchor the guard through
    # CLAUDE_PROJECT_DIR. The launcher's own missing/unreadable/script-error path is
    # fail-open, matching this guard's existing failure disposition: only a positively
    # confirmed non-trunk base may deny an arm.
    repo_root = Path(__file__).resolve().parent.parent
    guard_script = Path(__file__).resolve()
    settings_path = repo_root / ".claude" / "settings.json"
    try:
        settings = json.loads(settings_path.read_text(encoding="utf-8"))
        hook_command = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
    except (OSError, KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
        print(f"  [FAIL] load arm-guard hook command: {error}")
        failures += 1
        hook_command = ""

    with tempfile.TemporaryDirectory(prefix="sparq-arm-guard-test-") as temp_dir:
        fixture_root = Path(temp_dir)
        off_root_cwd = fixture_root / "off-root-cwd"
        fake_bin = fixture_root / "bin"
        off_root_cwd.mkdir()
        fake_bin.mkdir()
        fake_gh = fake_bin / "gh"
        fake_gh.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' "
            "'{\"baseRefName\":\"feature/stacked\","
            "\"headRefName\":\"fixture\",\"number\":1023}'\n",
            encoding="utf-8",
        )
        fake_gh.chmod(0o755)

        # [OPUS-5] #1135 END-TO-END: a second fake `gh` that answers with the LIVE Release
        # PR shape — base `main` (so the stacked axis allows it), head branch
        # `release-plz-main`, author github-actions, title `chore: release …`. Running the
        # REAL settings hook command against it proves the deny travels the whole wired
        # path, not just `decide()`. The YAML/JSON seam is where vacuity lives.
        release_bin = fixture_root / "release-bin"
        release_bin.mkdir()
        release_gh = release_bin / "gh"
        release_gh.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' "
            "'{\"baseRefName\":\"main\",\"headRefName\":\"release-plz-main\","
            "\"number\":900,\"author\":{\"login\":\"app/github-actions\"},"
            "\"title\":\"chore: release v0.2.0\"}'\n",
            encoding="utf-8",
        )
        release_gh.chmod(0o755)

        fixture_env = os.environ.copy()
        fixture_env["CLAUDE_PROJECT_DIR"] = str(repo_root)
        fixture_env["PATH"] = f"{fake_bin}{os.pathsep}{fixture_env.get('PATH', '')}"

        release_env = fixture_env.copy()
        release_env["PATH"] = f"{release_bin}{os.pathsep}{os.environ.get('PATH', '')}"
        for release_label, release_command, release_expected in (
            # base == main: ONLY the release axis can deny this. If the release axis were
            # deleted, this same fixture would ALLOW — that is what makes it discriminate.
            ("arm the Release PR", "gh pr merge 900 --auto --squash", "deny"),
            # …and an ordinary non-arm command against the same PR is untouched.
            ("non-arm command", "printf harmless", "allow"),
        ):
            for release_invocation, release_argv, release_cwd in (
                ("direct", [sys.executable, str(guard_script)], repo_root),
                ("settings hook", ["bash", "-c", hook_command], off_root_cwd),
            ):
                release_proc = subprocess.run(
                    release_argv,
                    cwd=release_cwd,
                    env=release_env,
                    input=json.dumps({"tool_input": {"command": release_command}}),
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                try:
                    release_got = json.loads(release_proc.stdout)[
                        "hookSpecificOutput"
                    ]["permissionDecision"]
                except (KeyError, TypeError, json.JSONDecodeError):
                    release_got = None
                release_ok = (
                    release_proc.returncode == 0 and release_got == release_expected
                )
                print(
                    f"  [{'PASS' if release_ok else 'FAIL'}] #1135 end-to-end "
                    f"{release_label}, {release_invocation}: {release_got} "
                    f"(want {release_expected})"
                )
                if not release_ok:
                    if release_proc.stderr:
                        print(f"    stderr: {release_proc.stderr.strip()}")
                    failures += 1

        fixture_cases = [
            ("non-arm fixture", "printf harmless", "allow"),
            ("stacked arm fixture", "gh pr merge 1023 --auto", "deny"),
        ]
        invocations = [
            ("direct/root cwd", [sys.executable, str(guard_script)], repo_root),
            ("direct/off-root cwd", [sys.executable, str(guard_script)], off_root_cwd),
            ("settings/root cwd", ["bash", "-c", hook_command], repo_root),
            ("settings/off-root cwd", ["bash", "-c", hook_command], off_root_cwd),
        ]
        for case_label, fixture_command, expected in fixture_cases:
            payload = json.dumps({"tool_input": {"command": fixture_command}})
            reference = None
            for invocation_label, argv, cwd in invocations:
                proc = subprocess.run(
                    argv,
                    cwd=cwd,
                    env=fixture_env,
                    input=payload,
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                try:
                    output = json.loads(proc.stdout)
                    got = output["hookSpecificOutput"]["permissionDecision"]
                except (KeyError, TypeError, json.JSONDecodeError):
                    output = None
                    got = None
                if reference is None:
                    reference = output
                ok = proc.returncode == 0 and got == expected and output == reference
                print(
                    f"  [{'PASS' if ok else 'FAIL'}] {case_label}, "
                    f"{invocation_label}: {got} (want {expected}, root-identical)"
                )
                if not ok:
                    if proc.stderr:
                        print(f"    stderr: {proc.stderr.strip()}")
                    failures += 1

        # AN UNRUNNABLE GUARD. Two obligations pull in opposite directions and BOTH are
        # pinned here, against the REAL settings.json wrapper:
        #
        #  (a) EXIT ZERO WITH AN EXPLICIT DECISION, always. A non-zero PreToolUse hook
        #      exit is fail-closed in Claude Code across the whole `Bash` matcher and
        #      caused a live all-Bash deadlock; that is what the exit-0 half protects.
        #  (b) DENY THE ARM. [OPUS-5] PR #4192 review: the wrapper used to answer `allow`
        #      here, which inverted this script's deliberate fail-closed release axis at
        #      the exact moment there is no guard at all — an agent in a checkout where
        #      the script is missing could arm the Release PR. It now denies.
        #
        # The non-arm row is what keeps (b) from re-creating (a): this hook matches EVERY
        # Bash tool call, so the deny must be scoped to `gh pr merge` and nothing else.
        unavailable_projects = [
            ("missing guard script", fixture_root / "missing-project"),
            ("unreadable guard script", fixture_root / "unreadable-project"),
        ]
        unavailable_projects[0][1].mkdir()
        unreadable_scripts = unavailable_projects[1][1] / "scripts"
        unreadable_scripts.mkdir(parents=True)
        unreadable_guard = unreadable_scripts / "check-pr-arm-base.py"
        unreadable_guard.write_text("raise SystemExit(1)\n", encoding="utf-8")
        unreadable_guard.chmod(0o000)
        unavailable_commands = [
            # armed, and the more dangerous UNarmed direct merge
            ("arm", "gh pr merge 1023 --auto", "deny"),
            ("direct merge", "gh pr merge 1023 --squash", "deny"),
            # the discriminating row: ordinary work must not be bricked
            ("ordinary command", "printf harmless", "allow"),
        ]
        for unavailable_label, project_dir in unavailable_projects:
            unavailable_env = fixture_env.copy()
            unavailable_env["CLAUDE_PROJECT_DIR"] = str(project_dir)
            for command_label, command, expected in unavailable_commands:
                unavailable_proc = subprocess.run(
                    ["bash", "-c", hook_command],
                    cwd=off_root_cwd,
                    env=unavailable_env,
                    input=json.dumps({"tool_input": {"command": command}}),
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                try:
                    unavailable_output = json.loads(unavailable_proc.stdout)
                    unavailable_decision = unavailable_output["hookSpecificOutput"][
                        "permissionDecision"
                    ]
                except (KeyError, TypeError, json.JSONDecodeError):
                    unavailable_decision = None
                unavailable_ok = (
                    unavailable_proc.returncode == 0
                    and unavailable_decision == expected
                )
                print(
                    f"  [{'PASS' if unavailable_ok else 'FAIL'}] {unavailable_label} + "
                    f"{command_label}, off-root cwd: {unavailable_decision} "
                    f"(want {expected} + exit 0)"
                )
                if not unavailable_ok:
                    if unavailable_proc.stderr:
                        print(f"    stderr: {unavailable_proc.stderr.strip()}")
                    failures += 1

    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="PreToolUse guard: deny arming auto-merge on a STACKED PR "
        "(base != trunk). Bead sq-u59rq."
    )
    ap.add_argument(
        "--trunk",
        default=DEFAULT_TRUNK,
        help=f"trunk branch a PR must target to be armable (default: {DEFAULT_TRUNK}).",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="run the hermetic logic self-test (no network) and exit.",
    )
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    raw = sys.stdin.read()
    command = ""
    if raw.strip():
        try:
            payload = json.loads(raw)
            command = (payload.get("tool_input") or {}).get("command", "") or ""
        except json.JSONDecodeError:
            command = ""

    out = decide(command, args.trunk, lambda pr: pr_facts(pr, args.trunk))
    print(json.dumps(out))
    # A PreToolUse command hook signals its decision via the JSON above (exit 0). We
    # always exit 0 and let `permissionDecision` carry allow/deny — exit-code semantics
    # would be ambiguous next to the structured decision.
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
