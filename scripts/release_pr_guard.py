#!/usr/bin/env python3
"""Shared, fail-closed identification of the release-plz **Release PR**.

[OPUS-5] 🤖 SPARQ agent. Issue #1135 (maintainer, 2026-07-26): *"Before I do this; can I
make sure that there are protections in place to prevent publishing too regularly, I don't
want to spam the registry."*

WHY THIS MODULE EXISTS — the arming gap
=======================================
sparq releases through a **Release PR**: `.github/workflows/release-plz.yml` runs on
`push: main`, opens/updates ONE version-bump PR, and a tag (and, once
`release-plz.toml`'s `publish` flips to `true`, a `cargo publish` of every crate in the
version_group) happens only when that PR **merges**. Release cadence therefore tracks
Release-PR *merges*.

`scripts/batch-merge.py` and `scripts/pr-backlog.py` already excluded that PR. Two arming
paths did NOT:

* `scripts/auto-arm.py` had **no** release-plz exclusion whatsoever — it armed any open PR
  carrying `review:pass` whose base is the default branch.
* `scripts/rearm-sweeper.py` excluded by **label** only (``EXCLUDED_LABELS`` + the
  ``needs:*`` namespace) — nothing keyed on the branch or the author.

So the moment anything (a reviewer, a workflow, a person, a mis-fired sweep) put
`review:pass` on the Release PR, it was armed and merged like any other PR. Harmless while
`publish = false`; **live and irreversible** the moment the crates.io credential is wired,
because a crates.io version can never be unpublished.

THE RULE THIS MODULE ENCODES
============================
The exclusion is keyed on **branch name, author identity, and title** — never on a label.
A label can be added or removed by anything with `pull-requests: write`; the head branch
and the author of a bot-opened PR cannot be changed by relabelling. `arm_block_reason` is
the single predicate every automated arming path consults.

FAIL-CLOSED. `arm_block_reason` blocks when it *cannot prove* the PR is not the Release PR:

* an absent/empty head branch (the caller's own query dropped the field, or the PR could
  not be read) blocks — an unknown must never mean "go ahead";
* a bot-authored PR with an unknown title blocks.

Over-blocking is cheap: a wrongly-skipped PR is merged by the next sweep or by a human.
Under-blocking publishes 37 crates to crates.io forever.

Usage:
    import release_pr_guard
    reason = release_pr_guard.arm_block_reason(
        head_ref=pr.head_ref, author_login=pr.author_login, title=pr.title
    )
    if reason:
        skip(reason)

    python3 scripts/release_pr_guard.py --self-test   # hermetic, stdlib-only, no network

stdlib-only (imported by scripts that run under a sparse checkout with no dependencies).
"""

from __future__ import annotations

import argparse
import re
import sys

# release-plz names its Release-PR branch `<pr_branch_prefix><base>` with a default
# prefix of `release-plz-` (this repo does not override `pr_branch_prefix`, so the live
# branch is `release-plz-main`). Match on the PREFIX WITHOUT the trailing separator so a
# future prefix change (`release-plz/`, `release-plz_v2-main`, a bare `release-plz`) is
# still caught. Anything a human names `release-plz…` is caught too — deliberately.
RELEASE_BRANCH_PREFIX = "release-plz"

# Logins that are release automation outright. Normalised (see `normalize_login`).
RELEASE_BOT_LOGINS = frozenset({"release-plz", "release-plz-bot", "releaseplz"})

# Bot logins that publish MANY kinds of PR, only some of which are releases. For these the
# title decides — and an UNKNOWN title on such a PR is fail-closed (blocked), because a bot
# PR we cannot describe is exactly the case we must not arm. `github-actions` is the login
# release-plz opens the Release PR under in this repo (it uses GITHUB_TOKEN).
GENERIC_BOT_LOGINS = frozenset({"github-actions", "actions-user"})

# release-plz's own default PR title is `chore: release v<version>` (multi-package:
# `chore: release`). Conventional-commit variants (`chore(release):`, a `!` breaking
# marker, a `prepare release` phrasing) and a bare `Release v1.2.3` are covered too.
_RELEASE_TITLE_RE = re.compile(
    r"""
    ^\s*
    (?:
        # chore: release …   |   chore(anything): release …   |   chore!: release …
        (?:chore|ci|build)\s*(?:\([^)]*\))?\s*!?\s*:\s*
        (?:prepare\s+(?:for\s+)?)?release\b
      |
        # chore(release): …  — the scope itself is the signal, whatever follows
        (?:chore|ci|build)\s*\(\s*release[^)]*\)\s*!?\s*:
      |
        # Release v1.2.3 / release 1.2.3
        release\s+v?\d
    )
    """,
    re.IGNORECASE | re.VERBOSE,
)


def normalize_login(login: object) -> str:
    """Normalise a GitHub actor login for comparison.

    `gh` reports app actors as ``app/github-actions``, the REST API as
    ``github-actions[bot]``, and GraphQL as ``github-actions``. Strip the ``app/``
    prefix, the ``[bot]`` suffix, and case, so one constant set matches all three.
    Returns ``""`` for an unknown/blank login.
    """
    text = str(login or "").strip().lower()
    if not text:
        return ""
    text = text.rsplit("/", 1)[-1]
    if text.endswith("[bot]"):
        text = text[: -len("[bot]")]
    return text.strip()


def is_release_title(title: object) -> bool:
    """True iff `title` reads as a release-preparation PR title."""
    text = str(title or "").strip()
    return bool(text) and bool(_RELEASE_TITLE_RE.match(text))


def is_release_branch(head_ref: object) -> bool:
    """True iff `head_ref` is (or looks like) release-plz's Release-PR branch."""
    text = str(head_ref or "").strip().lower()
    return bool(text) and text.startswith(RELEASE_BRANCH_PREFIX)


def arm_block_reason(
    *,
    head_ref: object,
    author_login: object = None,
    title: object = None,
) -> str | None:
    """Return a non-empty reason iff this PR must NEVER be armed by an automated path.

    Returns ``None`` only when the PR is POSITIVELY known not to be a release PR. Any
    indeterminacy returns a reason (fail-closed) — see the module docstring.

    Deliberately independent of LABELS: relabelling a PR cannot change this answer.
    """
    head = str(head_ref or "").strip()
    if not head:
        return (
            "release-pr-guard: indeterminate head branch — cannot prove this is not the "
            "release-plz Release PR, so the arm is refused (fail-closed, #1135)"
        )
    if is_release_branch(head):
        return (
            f"release-pr-guard: release-plz branch ({head}) — the Release PR is merged "
            "by a maintainer only; automated arming would publish to crates.io, which "
            "is irreversible (#1135)"
        )

    login = normalize_login(author_login)
    if login in RELEASE_BOT_LOGINS:
        return (
            f"release-pr-guard: release automation author ({login}) — the Release PR is "
            "merged by a maintainer only (#1135)"
        )

    if login in GENERIC_BOT_LOGINS:
        if title is None or not str(title).strip():
            return (
                f"release-pr-guard: indeterminate title on a bot-authored PR ({login}) — "
                "cannot prove this is not the Release PR (fail-closed, #1135)"
            )
        if is_release_title(title):
            return (
                f"release-pr-guard: release-preparation title from {login} "
                f"({str(title).strip()!r}) (#1135)"
            )
        return None

    if is_release_title(title):
        return (
            "release-pr-guard: release-preparation title "
            f"({str(title).strip()!r}) — refusing to arm a release PR (#1135)"
        )
    return None


# --------------------------------------------------------------------------- self-test
def self_test() -> int:
    failures: list[str] = []

    def check(label: str, condition: bool) -> None:
        if not condition:
            failures.append(label)
        print(f"  [{'PASS' if condition else 'FAIL'}] {label}")

    # --- the LIVE Release PR shape: release-plz on GITHUB_TOKEN, branch release-plz-main.
    live = dict(
        head_ref="release-plz-main",
        author_login="app/github-actions",
        title="chore: release v0.2.0",
    )
    check("live Release PR is blocked", arm_block_reason(**live) is not None)

    # --- THE BUG THIS FIXES: a label cannot unblock it, because labels are not an input.
    #     (Structural: `arm_block_reason` takes no `labels` parameter at all.)
    import inspect

    check(
        "arm_block_reason takes NO label input (relabelling cannot unblock)",
        "labels" not in inspect.signature(arm_block_reason).parameters,
    )

    # --- each signal alone is sufficient (they are OR-ed, not AND-ed).
    check(
        "branch alone blocks (human author, ordinary title)",
        arm_block_reason(
            head_ref="release-plz-main", author_login="jeswr", title="fix: something"
        )
        is not None,
    )
    check(
        "release-plz author alone blocks (ordinary branch + title)",
        arm_block_reason(
            head_ref="sparq-agent/issue-1-x",
            author_login="release-plz[bot]",
            title="fix: something",
        )
        is not None,
    )
    check(
        "release title alone blocks (human author, ordinary branch)",
        arm_block_reason(
            head_ref="sparq-agent/issue-1-x",
            author_login="jeswr",
            title="chore: release v0.2.0",
        )
        is not None,
    )

    # --- login normalisation covers all three API spellings.
    for spelling in ("release-plz", "release-plz[bot]", "app/release-plz"):
        check(
            f"login spelling {spelling!r} normalises to a blocked release bot",
            arm_block_reason(
                head_ref="sparq-agent/issue-1-x",
                author_login=spelling,
                title="fix: x",
            )
            is not None,
        )

    # --- title variants.
    for title in (
        "chore: release v0.2.0",
        "chore: release",
        "chore(release): prepare 0.2.0",
        "chore(main): release 0.2.0",
        "chore!: release v1.0.0",
        "Release v0.3.0",
        "chore: prepare release 0.2.0",
        "chore: prepare for release 0.2.0",
    ):
        check(
            f"release title {title!r} blocks",
            arm_block_reason(
                head_ref="sparq-agent/issue-1-x", author_login="jeswr", title=title
            )
            is not None,
        )

    # --- FAIL-CLOSED on indeterminacy.
    check(
        "absent head branch blocks (fail-closed)",
        arm_block_reason(head_ref=None, author_login="jeswr", title="fix: x") is not None,
    )
    check(
        "empty head branch blocks (fail-closed)",
        arm_block_reason(head_ref="   ", author_login="jeswr", title="fix: x") is not None,
    )
    check(
        "bot author with UNKNOWN title blocks (fail-closed)",
        arm_block_reason(
            head_ref="sparq-agent/issue-1-x",
            author_login="github-actions",
            title=None,
        )
        is not None,
    )

    # --- DISCRIMINATION: an ordinary worker PR must still be armable, or the guard is a
    #     merge-train brick rather than a release guard. This case is what makes every
    #     block above meaningful (a fixture that satisfies both readings proves nothing).
    check(
        "ordinary worker PR is NOT blocked",
        arm_block_reason(
            head_ref="sparq-agent/issue-3801-fix-thing",
            author_login="app/sparq-orchestrator",
            title="fix(engine): tighten the id-eq fast path",
        )
        is None,
    )
    check(
        "ordinary github-actions PR with a NON-release title is NOT blocked",
        arm_block_reason(
            head_ref="chore/bump-actions",
            author_login="app/github-actions",
            title="chore: bump actions/checkout to v7",
        )
        is None,
    )
    check(
        "a PR merely MENTIONING release-plz is NOT blocked",
        arm_block_reason(
            head_ref="sparq-agent/issue-1-release-plz-config",
            author_login="jeswr",
            title="feat: add release-plz config",
        )
        is None,
    )
    check(
        "dependabot PR is NOT blocked by this guard",
        arm_block_reason(
            head_ref="dependabot/cargo/serde-1.0.220",
            author_login="dependabot[bot]",
            title="build(deps): bump serde from 1.0.219 to 1.0.220",
        )
        is None,
    )

    if failures:
        print(f"\nrelease_pr_guard self-test: {len(failures)} case(s) FAILED")
        for label in failures:
            print(f"  - {label}")
        return 1
    print("\nrelease_pr_guard self-test: PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--self-test", action="store_true", help="run the hermetic self-test and exit"
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    parser.error("--self-test is the only mode; import this module to use the guard")
    return 2  # pragma: no cover - argparse exits


if __name__ == "__main__":
    sys.exit(main())
