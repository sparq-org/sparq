#!/usr/bin/env python3
"""The review lane's VISIBILITY PREDICATE — ONE definition, shared by every sparq-side
component that writes or censuses review state.

[OPUS-5] 🤖 SPARQ agent. Issue #4677, measured on live sparq: four green, ready,
not-moving PRs carried ``review:unreviewed`` — a label applied IN THIS REPOSITORY — while
the only review producer that exists lives in ``jeswr/agent-account-registry`` and
structurally cannot enumerate any of them. Among them was the FIRST crates.io release PR,
unreviewed for ~22h. The string ``review:unreviewed`` appears nowhere in the registry
codebase: the labeller is sparq-side, the reviewer is registry-side, and the two
components disagreed about which PRs are in the lane.

WHY THIS MODULE EXISTS — THE DEFECT IS THE DISAGREEMENT, NOT EITHER PREDICATE
============================================================================
A labeller with WIDER vision than its worker manufactures invisible backlog. The label
makes a PR look *enrolled*, so it is neither reviewed nor noticed, and every per-run
success rate the lane reports is computed over a population that excludes those PRs
entirely — the lane can report fully healthy while this class grows without bound. A PR
that gets labelled but never reviewed is therefore WORSE than one never labelled.

So the predicate gets exactly ONE definition, imported by both sides:

* ``scripts/verdict-bridge.py``   — the WRITER. Applies ``review:unreviewed``; must not
  claim enrolment for a PR the lane cannot see, and withdraws the label where it already
  did.
* ``scripts/review_lane_alarm.py`` — the CENSUS. Counts the blind spot, and counts any
  surviving labeller/reviewer disagreement as its own row.

Re-stating the predicate a third time would re-commit the bug it fixes.

THE PREDICATE ITSELF (replicated from the registry, deliberately byte-identical)
===============================================================================
The registry's ``enumerate_review_items`` (registry ``scripts/dispatch-claim.py``) admits
a PR only through three AUTHOR-SIDE gates that all fail closed::

    if not HEAD_REF_RE.match(ref):        continue   # ^sparq-agent/issue-([1-9][0-9]*)-
    if head_repo != repo:                 continue
    if not login.endswith("[bot]") ...:   continue   # must be the worker App bot

followed by a mandatory registry-side provenance record that only a host-side worker run
can write. Only the three STRUCTURAL gates are replicated here: the provenance record is a
live registry read neither sparq component holds a token for, and it can only ever narrow
admission further — so ``lane_reachable`` returning True is the CONSERVATIVE direction for
the census (it under-reports the blind spot) and the STRICT direction for the writer (it
labels less, never more).

THE ``[bot]`` GATE IS NOT A SUFFIX TEST HERE, AND THAT IS LOAD-BEARING. GitHub reports one
App under three spellings depending on the surface: ``app/sparq-orchestrator`` (`gh`),
``sparq-orchestrator[bot]`` (REST), and a bare ``sparq-orchestrator`` (GraphQL). The
registry runs against the REST listing, where the suffix is present. verdict-bridge reads
GraphQL, where it is NOT — so a copied ``endswith("[bot]")`` there would read as a guard
while never matching, and every worker PR would be classed unreachable. Callers therefore
pass an explicit ``author_is_machine`` boolean derived from THEIR OWN surface
(``user.type == "Bot"`` / ``author.__typename == "Bot"`` / the login suffix), and
``machine_actor`` below is the shared way to derive it.

DISPOSITION: WHAT AN UNREACHABLE PR IS *FOR*
============================================
Registry #916 enrols the ORCHESTRATOR-AUTHORED class. It deliberately does not cover
release-plz or dependabot, and the reason is structural rather than a policy choice:
reviewer selection INVERTS ``impl_provider`` to guarantee cross-provider review, and those
classes have no implementing model. Admitting one would require FABRICATING a provider,
which makes the cross-provider assertion vacuous rather than merely weaker.

Those classes therefore need a different disposition, not a wider allowlist — and sparq
already encodes one, it just was not written down:

* RELEASE PRs — ``scripts/release_pr_guard.py`` blocks every automated arming path
  (``auto-arm.py``, ``rearm-sweeper.py``, ``check-pr-arm-base.py``,
  ``batch-merge.py``), keyed on branch/author/title and never on a label. The Release PR
  can only merge by a maintainer's own hand, so the maintainer IS its reviewer of record.
* DEPENDABOT PRs — ``batch-merge.py`` excludes ``dependabot/*`` heads and no producer
  enumerates them; the risk surface (pinned action SHAs, dependency versions) is reviewed
  by the maintainer alongside the supply-chain gates that already run on the PR.

``unreachable_disposition`` names that decision on every row the census prints, so the
population is *stated* rather than silently accumulated. This module decides NOTHING about
merging and grants NO arming authority: it produces a boolean and a sentence.

Usage::

    import review_lane_reach
    reachable = review_lane_reach.lane_reachable(
        head_ref=..., head_repo=..., author_is_machine=..., repo="sparq-org/sparq")
    klass, why = review_lane_reach.unreachable_disposition(
        head_ref=..., author_login=..., title=...)

    python3 scripts/review_lane_reach.py --self-test   # hermetic, stdlib-only, no network

stdlib-only (imported by scripts that run under a sparse checkout with no dependencies).
"""

from __future__ import annotations

import argparse
import re
import sys

import release_pr_guard

# The registry review lane's own admission regex, replicated VERBATIM from
# `scripts/dispatch-claim.py` (registry, master @ 2026-07-26). Kept byte-identical on
# purpose: the whole claim of the components importing this is "the registry lane can /
# cannot see this PR", so the test must be the registry's test and not a paraphrase of it.
REGISTRY_HEAD_REF_RE = re.compile(r"^sparq-agent/issue-([1-9][0-9]*)-")

# Disposition classes for a PR the lane cannot reach. Stable strings: they are printed in
# the census and pinned by tests, so renaming one is a visible change, not a silent one.
CLASS_RELEASE = "release-pr"
CLASS_DEPENDABOT = "dependabot"
CLASS_NOT_ENROLLED = "not-enrolled"

# Heads dependabot opens PRs on, and the login it opens them under.
DEPENDABOT_BRANCH_PREFIX = "dependabot/"
DEPENDABOT_LOGINS = frozenset({"dependabot", "dependabot-preview"})


def normalize_login(login: object) -> str:
    """Case-folded login with an ``app/`` prefix and a ``[bot]`` suffix stripped.

    One App is spelled three ways depending on which surface reports it:
    ``app/sparq-orchestrator`` (`gh`), ``sparq-orchestrator[bot]`` (REST),
    ``sparq-orchestrator`` (GraphQL). Same rule as
    ``scripts/release_pr_guard.py::normalize_login``.
    """
    return release_pr_guard.normalize_login(login)


def machine_actor(who: object) -> bool:
    """True iff a GitHub user/actor OBJECT denotes a machine account.

    Both signals, because they are populated inconsistently across the surfaces sparq
    reads: the ``[bot]`` login suffix (REST, `gh`) and a ``Bot`` type/typename (REST
    ``user.type``, GraphQL ``__typename``). GraphQL reports an App's login WITHOUT the
    suffix, so the suffix test alone is not a predicate there — see the module header.
    """
    if not isinstance(who, dict):
        return False
    kind = str(who.get("type") or who.get("__typename") or "")
    return kind == "Bot" or str(who.get("login") or "").endswith("[bot]")


def lane_reachable(
    *,
    head_ref: object,
    head_repo: object,
    author_is_machine: bool,
    repo: str,
) -> bool:
    """True iff the registry review lane can EVER enumerate this PR.

    False => no review producer in existence can see it, so nothing sparq-side may label
    it as though it were awaiting one.

    ``author_is_machine`` is passed in rather than derived from a login, because the
    registry's own ``endswith("[bot]")`` test is surface-specific and does not survive the
    move to GraphQL (module header). Use ``machine_actor`` to derive it.
    """
    ref = str(head_ref or "")
    if not REGISTRY_HEAD_REF_RE.match(ref):
        return False
    if str(head_repo or "") != str(repo or ""):
        return False
    return bool(author_is_machine)


def unreachable_disposition(
    *, head_ref: object, author_login: object = None, title: object = None
) -> tuple[str, str]:
    """-> ``(class, disposition)`` for a PR the review lane cannot reach.

    ``class`` is one of ``CLASS_RELEASE`` / ``CLASS_DEPENDABOT`` / ``CLASS_NOT_ENROLLED``.
    ``disposition`` is the one-line statement of what happens to it INSTEAD of an
    automated review — the thing #4677 asked to be decided explicitly rather than left to
    silent accumulation.

    Callers must only ask this of a PR ``lane_reachable`` already returned False for; it
    describes a disposition, it does not decide reachability.

    NOT A SAFETY DECISION. The authoritative refusal to arm a release PR is
    ``release_pr_guard.arm_block_reason``, which is strictly WIDER than the positive tests
    used here (it also blocks on an indeterminate branch or title). This function only
    picks the sentence to print, so a miss degrades to the generic class, never to an arm.
    """
    ref = str(head_ref or "").strip()
    login = normalize_login(author_login)

    if (
        release_pr_guard.is_release_branch(ref)
        or login in release_pr_guard.RELEASE_BOT_LOGINS
        or release_pr_guard.is_release_title(title)
    ):
        return (
            CLASS_RELEASE,
            "MAINTAINER-REVIEWED release PR: every automated arming path already refuses "
            "it (scripts/release_pr_guard.py, #1135) because merging it tags and "
            "publishes to crates.io irreversibly, so a maintainer is its reviewer of "
            "record — no automated producer will ever review it",
        )

    if ref.lower().startswith(DEPENDABOT_BRANCH_PREFIX) or login in DEPENDABOT_LOGINS:
        return (
            CLASS_DEPENDABOT,
            "MAINTAINER-REVIEWED dependency PR: batch-merge excludes `dependabot/*` heads "
            "and the registry lane cannot enrol it (no implementing model to invert for "
            "cross-provider review), so the maintainer reviews it alongside the "
            "supply-chain gates that already run on the head",
        )

    return (
        CLASS_NOT_ENROLLED,
        "NOT ENROLLED in any review lane: the head ref does not match "
        "`^sparq-agent/issue-([1-9][0-9]*)-`, the head repo is not this repo, and/or the "
        "author is not the worker App — so the registry's `enumerate_review_items` can "
        "never dispatch a review for it",
    )


# --------------------------------------------------------------------------- self-test
def self_test() -> int:  # noqa: C901 - a flat table of named assertions reads best flat
    failures: list[str] = []
    repo = "sparq-org/sparq"

    def check(label: str, condition: bool) -> None:
        if not condition:
            failures.append(label)
        print(f"  [{'PASS' if condition else 'FAIL'}] {label}")

    # --- the regex is the registry's, byte for byte.
    check(
        "head-ref regex is byte-identical to the registry gate",
        REGISTRY_HEAD_REF_RE.pattern == r"^sparq-agent/issue-([1-9][0-9]*)-",
    )
    check("issue-0 is not a worker branch", not REGISTRY_HEAD_REF_RE.match("sparq-agent/issue-0-1-1"))

    # --- reachability, on REAL branch names measured on live sparq.
    check(
        "worker branch + machine author is reachable",
        lane_reachable(
            head_ref="sparq-agent/issue-2908-30221671021-1",
            head_repo=repo,
            author_is_machine=True,
            repo=repo,
        ),
    )
    for ref in (
        "ci/auto-arm-workflows-permission",        # #3798
        "research/knowledge-management-strategy",  # #4193
        "release-plz-2026-07-27T02-19-35Z",        # #4460 — the first crates.io release PR
        "dependabot/github_actions/actions-minor",  # #4488
    ):
        check(
            f"the #4677 population is unreachable: {ref}",
            not lane_reachable(
                head_ref=ref, head_repo=repo, author_is_machine=True, repo=repo
            ),
        )
    check(
        "a worker branch from a HUMAN author is unreachable",
        not lane_reachable(
            head_ref="sparq-agent/issue-1-2-1",
            head_repo=repo,
            author_is_machine=False,
            repo=repo,
        ),
    )
    check(
        "a fork head is unreachable",
        not lane_reachable(
            head_ref="sparq-agent/issue-1-2-1",
            head_repo="attacker/sparq",
            author_is_machine=True,
            repo=repo,
        ),
    )
    check(
        "an absent head ref is unreachable (never labelled as enrolled)",
        not lane_reachable(
            head_ref=None, head_repo=repo, author_is_machine=True, repo=repo
        ),
    )

    # --- the `[bot]` spelling trap: a GraphQL login carries NO suffix, so a suffix test
    # would class every worker PR unreachable. `machine_actor` must catch all three.
    check("REST spelling is a machine actor", machine_actor({"login": "sparq-orchestrator[bot]"}))
    check("REST type field is a machine actor", machine_actor({"login": "x", "type": "Bot"}))
    check(
        "GraphQL __typename is a machine actor",
        machine_actor({"login": "sparq-orchestrator", "__typename": "Bot"}),
    )
    check("a human is not a machine actor", not machine_actor({"login": "jeswr", "type": "User"}))
    check("a non-object is not a machine actor", not machine_actor("sparq-orchestrator[bot]"))

    # --- disposition: the explicit decision for each unreachable class.
    release, release_why = unreachable_disposition(
        head_ref="release-plz-2026-07-27T02-19-35Z",
        author_login="app/sparq-orchestrator",
        title="chore: release v0.2.0",
    )
    check("the release PR is classed as a release PR", release == CLASS_RELEASE)
    check("the release disposition names the maintainer", "MAINTAINER-REVIEWED" in release_why)
    check(
        "a release TITLE alone classifies (branch renamed)",
        unreachable_disposition(head_ref="rel/x", title="chore: release v1.2.3")[0]
        == CLASS_RELEASE,
    )
    check(
        "the release-plz login alone classifies",
        unreachable_disposition(head_ref="rel/x", author_login="release-plz[bot]")[0]
        == CLASS_RELEASE,
    )
    dependabot, dependabot_why = unreachable_disposition(
        head_ref="dependabot/github_actions/actions-minor-1a2b",
        author_login="dependabot[bot]",
        title="chore(deps): bump actions/checkout",
    )
    check("the dependabot PR is classed as dependabot", dependabot == CLASS_DEPENDABOT)
    check("the dependabot disposition names the maintainer", "MAINTAINER-REVIEWED" in dependabot_why)
    check(
        "the dependabot LOGIN alone classifies (branch renamed)",
        unreachable_disposition(head_ref="deps/bump", author_login="dependabot[bot]")[0]
        == CLASS_DEPENDABOT,
    )
    for ref in ("ci/auto-arm-workflows-permission", "research/knowledge-management-strategy"):
        check(
            f"an agent-session branch is not enrolled: {ref}",
            unreachable_disposition(head_ref=ref, author_login="jeswr", title="ci: fix")[0]
            == CLASS_NOT_ENROLLED,
        )
    check(
        "every class states a disposition",
        all(
            why.strip()
            for _, why in (
                unreachable_disposition(head_ref=ref)
                for ref in ("release-plz-main", "dependabot/x", "research/y", "")
            )
        ),
    )

    if failures:
        print(f"::error::review_lane_reach self-test: {len(failures)} failure(s)")
        return 1
    print("review_lane_reach self-test: all checks passed")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="The review lane's visibility predicate")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if not args.self_test:
        parser.error("this module is a library; run it with --self-test")
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
