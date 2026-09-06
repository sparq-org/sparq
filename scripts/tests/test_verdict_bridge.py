#!/usr/bin/env python3
"""Hermetic tests for the VERDICT-comment -> review-label bridge.

Two halves, because every uncaught mutant this project has measured lived at the YAML
seam rather than in the Python:

* ``TestDecisionCore`` / ``TestVerdictParsing`` — the pure predicate. Every guard in
  ``decide()`` has a NAMED test here that reds when the guard is deleted or inverted.
* ``TestWorkflowWiring`` — STRUCTURAL PyYAML inspection of
  ``.github/workflows/verdict-bridge.yml``. Substring and ``count(...) == N`` assertions
  do not catch ``if: false`` or a deleted step, so the workflow document is parsed and
  the job/step graph asserted against directly.
* ``TestCrossPolicyConsistency`` — imports the LIVE ``auto-arm.py`` /
  ``rearm-sweeper.py`` and pins the two invariants that make the bridge safe by
  construction: it never attests a PR the arming policy would refuse, and the
  informational label it writes is not a hold anywhere.

Stdlib unittest + PyYAML. No network, no gh, no git.
"""

# [OPUS-5] sparq-org/sparq — green-but-unqueued PR investigation, 2026-07-26.

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import re
import sys
import unittest
from pathlib import Path
from types import ModuleType

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = REPO_ROOT / "scripts"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "verdict-bridge.yml"
AUTO_ARM_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "auto-arm.yml"


def load(path: Path, name: str) -> ModuleType:
    """Import a hyphenated script as a module."""
    sys.path.insert(0, str(SCRIPTS))
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader, path
    module = importlib.util.module_from_spec(spec)
    # Register BEFORE exec: dataclasses resolves annotations via sys.modules[__module__].
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


vb = load(SCRIPTS / "verdict-bridge.py", "verdict_bridge")
auto_arm = load(SCRIPTS / "auto-arm.py", "auto_arm_policy")
rearm = load(SCRIPTS / "rearm-sweeper.py", "rearm_sweeper_policy")

HEAD = "a" * 40
OTHER = "b" * 40

# The live sweep step's `run:` is a FOLDED (`>-`) scalar, so YAML joins its argv onto one
# line. Anchoring on `--repo` is what distinguishes it from the `--self-test` step.
LIVE_RUN_NEEDLE = "python3 scripts/verdict-bridge.py --repo"


def comment(body: str, **kw) -> dict:
    return vb.comment(body, **kw)


def pr(**kw):
    return vb.pr_fixture(**kw)


class TestVerdictParsing(unittest.TestCase):
    """The line-anchored + head-bound + provenance predicate."""

    def test_trailing_verdict_reads_the_last_non_blank_line(self):
        self.assertEqual(vb.trailing_verdict("prose\n\nVERDICT: pass\n\n"), "pass")
        self.assertEqual(vb.trailing_verdict("**VERDICT: fail**"), "fail")

    def test_verdict_must_be_the_final_line(self):
        """A quoted INSTRUCTION mentioning the phrase is not a verdict."""
        self.assertIsNone(
            vb.trailing_verdict("end with `VERDICT: pass`\n\nstill reviewing")
        )
        self.assertIsNone(vb.trailing_verdict("VERDICT: pass\nbut actually no"))

    def test_verdict_line_must_not_carry_trailing_content(self):
        """It is never read as a PASS — it degrades to AMBIGUOUS, not to a verdict."""
        for line in ("VERDICT: pass (with caveats)", "VERDICT: pass, mostly"):
            with self.subTest(line=line):
                self.assertEqual(vb.trailing_verdict(line), vb.AMBIGUOUS)

    def test_a_verdict_shaped_line_that_does_not_parse_is_ambiguous_not_absent(self):
        """Returning None here is the composition fail-open: an older, well-formed pass
        would survive the retraction and stay ``found[-1]``."""
        for line in (
            "VERDICT: FAIL",
            "VERDICT: fail (retracting my pass)",
            "**VERDICT: Fail**",
            "VERDICT - fail",
            "verdict: unclear",
            "VERDICT:",
        ):
            with self.subTest(line=line):
                self.assertEqual(vb.trailing_verdict(line), vb.AMBIGUOUS)

    def test_a_mention_of_the_phrase_is_still_not_verdict_shaped(self):
        """Quoted / blockquoted / fenced / bulleted mentions stay 'no verdict' — the
        instruction that tells reviewers what to write must influence nothing."""
        for line in (
            "> VERDICT: pass",
            "- VERDICT: pass",
            "* VERDICT: fail",
            "`VERDICT: pass`",
            "the brief says to end with VERDICT: pass",
            "```",
        ):
            with self.subTest(line=line):
                self.assertIsNone(vb.trailing_verdict(line))

    def test_an_unparseable_retraction_defeats_an_earlier_pass_at_the_same_head(self):
        """THE composition hole. A reviewer's only retraction channel is a comment, so a
        slightly-misformatted retraction must never leave the superseded pass standing."""
        stale_pass = comment(
            f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1
        )
        for line in ("VERDICT: FAIL", "VERDICT: fail (retracting my pass)"):
            with self.subTest(line=line):
                retraction = comment(
                    f"{HEAD}\n\n{line}", created_at="2026-02-01T00:00:00Z", cid=2
                )
                self.assertEqual(
                    vb.head_bound_verdict([stale_pass, retraction], HEAD).value,
                    vb.AMBIGUOUS,
                )
                self.assertNotEqual(decision(pr(), [stale_pass, retraction]), "promote")
                self.assertEqual(
                    decision(
                        pr(labels={vb.REVIEW_ATTESTATION}), [stale_pass, retraction]
                    ),
                    "retract",
                    "an unreadable retraction must not leave review:pass standing",
                )

    def test_an_untrusted_ambiguous_line_cannot_suppress_a_trusted_pass(self):
        """Ambiguity is fail-closed, but only for REVIEWERS — otherwise any drive-by
        commenter could deny arming to every PR in the repo."""
        trusted_pass = comment(
            f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1
        )
        for association in ("NONE", "CONTRIBUTOR", "BOT", ""):
            with self.subTest(association=association):
                drive_by = comment(
                    f"{HEAD}\n\nVERDICT: FAIL",
                    association=association,
                    created_at="2026-02-01T00:00:00Z",
                    cid=2,
                )
                self.assertEqual(decision(pr(), [trusted_pass, drive_by]), "promote")

    def test_an_ambiguous_line_bound_to_another_head_is_ignored(self):
        """Head binding still gates ambiguity — a retraction of a SUPERSEDED head must
        not suppress a fresh pass."""
        fresh = comment(
            f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1
        )
        old = comment(
            f"{OTHER}\n\nVERDICT: FAIL", created_at="2026-02-01T00:00:00Z", cid=2
        )
        self.assertEqual(decision(pr(), [fresh, old]), "promote")

    def test_a_review_body_can_WITHHOLD_a_pass(self):
        """The composition hole is channel-independent: a reviewer who posts a pass as a
        comment and then withdraws it in a PR REVIEW body must not leave it standing."""
        old_pass = comment(
            f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1
        )
        for line in ("VERDICT: fail", "VERDICT: FAIL"):
            with self.subTest(line=line):
                withheld = dict(
                    comment(f"{HEAD}\n\n{line}", created_at="2026-02-01T00:00:00Z", cid=2),
                    _channel="review",
                )
                self.assertNotEqual(decision(pr(), [old_pass, withheld]), "promote")
                self.assertEqual(
                    decision(pr(labels={vb.REVIEW_ATTESTATION}), [old_pass, withheld]),
                    "retract",
                )

    def test_a_review_body_can_NEVER_grant_a_pass(self):
        """Reading a review body as a pass would extend arming authority into a channel
        the standing review brief does not mandate. The promote-set may only shrink."""
        review_pass = dict(
            comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-03-01T00:00:00Z", cid=3),
            _channel="review",
        )
        self.assertIsNone(vb.head_bound_verdict([review_pass], HEAD))
        self.assertEqual(decision(pr(), [review_pass]), "flag")
        # ... not even as the NEWEST evidence over an older review fail.
        review_fail = dict(
            comment(f"{HEAD}\n\nVERDICT: fail", created_at="2026-02-01T00:00:00Z", cid=2),
            _channel="review",
        )
        self.assertEqual(
            vb.head_bound_verdict([review_fail, review_pass], HEAD).value, "fail"
        )

    def test_a_review_withholding_still_obeys_head_binding_and_provenance(self):
        good_pass = comment(
            f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1
        )
        for kwargs, why in (
            ({"association": "NONE"}, "untrusted author"),
            ({}, "bound to another head"),
        ):
            body = f"{HEAD}\n\nVERDICT: fail" if kwargs else f"{OTHER}\n\nVERDICT: fail"
            with self.subTest(why=why):
                withheld = dict(
                    comment(body, created_at="2026-02-01T00:00:00Z", cid=2, **kwargs),
                    _channel="review",
                )
                self.assertEqual(decision(pr(), [good_pass, withheld]), "promote")

    def test_binds_head_requires_the_full_forty_hex_sha(self):
        self.assertTrue(vb.binds_head(f"reviewed {HEAD}", HEAD))
        self.assertFalse(vb.binds_head(f"reviewed {HEAD[:12]}", HEAD))
        self.assertFalse(vb.binds_head(f"reviewed {OTHER}", HEAD))

    def test_binds_head_rejects_a_sha_glued_into_a_longer_hex_run(self):
        self.assertFalse(vb.binds_head(f"reviewed {HEAD}cafe", HEAD))
        self.assertFalse(vb.binds_head(f"reviewed cafe{HEAD}", HEAD))

    def test_untrusted_association_is_not_a_reviewer(self):
        for association in ("CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR", "NONE", "", "BOT"):
            with self.subTest(association=association):
                self.assertIsNone(
                    vb.head_bound_verdict(
                        [comment(f"{HEAD}\n\nVERDICT: pass", association=association)],
                        HEAD,
                    )
                )

    def test_malformed_comment_payloads_never_read_as_a_pass(self):
        self.assertIsNone(
            vb.head_bound_verdict([None, 7, {}, {"body": None}, {"body": ""}], HEAD)
        )

    def test_latest_by_created_at_wins_regardless_of_input_order(self):
        early = comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1)
        late = comment(f"{HEAD}\n\nVERDICT: fail", created_at="2026-01-02T00:00:00Z", cid=2)
        for ordering in ([early, late], [late, early]):
            with self.subTest(ordering=[c["id"] for c in ordering]):
                self.assertEqual(vb.head_bound_verdict(ordering, HEAD).value, "fail")

    def test_recency_is_created_at_not_comment_id(self):
        """Comment ids are not monotone across a PR's timeline (reviews, transfers,
        cross-posted bodies), so ordering by id can silently resurrect a stale pass.
        Here the RETRACTION carries the LOWER id but the LATER created_at."""
        stale_pass = comment(
            f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=900
        )
        retraction = comment(
            f"{HEAD}\n\nVERDICT: fail", created_at="2026-01-02T00:00:00Z", cid=100
        )
        self.assertEqual(
            vb.head_bound_verdict([stale_pass, retraction], HEAD).value, "fail"
        )
        self.assertEqual(
            decision(pr(labels={vb.REVIEW_ATTESTATION}), [stale_pass, retraction]),
            "retract",
        )

    def test_editing_an_older_pass_cannot_reorder_it_ahead_of_a_newer_fail(self):
        """Ordering uses immutable created_at — an edit must not defeat a retraction."""
        early = dict(
            comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1),
            updated_at="2099-01-01T00:00:00Z",
        )
        late = comment(f"{HEAD}\n\nVERDICT: fail", created_at="2026-01-02T00:00:00Z", cid=2)
        self.assertEqual(vb.head_bound_verdict([early, late], HEAD).value, "fail")


class TestDecisionCore(unittest.TestCase):
    """One named test per guard in ``decide()``."""

    def setUp(self):
        self.passing = comment(f"reviewed {HEAD}\n\nVERDICT: pass")
        self.failing = comment(f"reviewed {HEAD}\n\nVERDICT: fail", cid=2)

    def test_head_bound_pass_on_a_clean_pr_promotes(self):
        self.assertEqual(decision(pr(), [self.passing]), "promote")

    def test_pass_bound_to_a_superseded_head_never_promotes(self):
        stale = comment(f"reviewed {OTHER}\n\nVERDICT: pass")
        self.assertNotEqual(decision(pr(), [stale]), "promote")

    def test_hold_labels_are_fail_closed(self):
        for hold in sorted(vb.HOLD_LABELS) + ["needs:design", "needs:whatever"]:
            with self.subTest(hold=hold):
                self.assertEqual(decision(pr(labels={hold}), [self.passing]), "none")

    def test_draft_is_never_attested(self):
        self.assertEqual(decision(pr(is_draft=True), [self.passing]), "none")

    def test_closed_pr_is_never_attested(self):
        for state in ("MERGED", "CLOSED", ""):
            with self.subTest(state=state):
                self.assertEqual(decision(pr(state=state), [self.passing]), "none")

    def test_promotion_is_idempotent(self):
        self.assertEqual(
            decision(pr(labels={vb.REVIEW_ATTESTATION}), [self.passing]), "none"
        )

    def test_head_bound_fail_retracts_an_existing_attestation(self):
        self.assertEqual(
            decision(pr(labels={vb.REVIEW_ATTESTATION}), [self.failing]), "retract"
        )

    def test_absence_of_evidence_never_retracts(self):
        """A hand-applied review:pass with no comment behind it must survive."""
        self.assertEqual(decision(pr(labels={vb.REVIEW_ATTESTATION}), []), "none")
        stale_fail = comment(f"reviewed {OTHER}\n\nVERDICT: fail")
        self.assertEqual(
            decision(pr(labels={vb.REVIEW_ATTESTATION}), [stale_fail]), "none"
        )

    def test_green_mergeable_unreviewed_pr_is_flagged_visible(self):
        self.assertEqual(decision(pr(), []), "flag")

    def test_flag_requires_green_and_mergeable_and_non_draft(self):
        for kwargs in (
            {"is_draft": True},
            {"mergeable": "CONFLICTING"},
            {"mergeable": "UNKNOWN"},
            {"gate_conclusion": "failure"},
            {"gate_conclusion": None},
        ):
            with self.subTest(**kwargs):
                self.assertEqual(decision(pr(**kwargs), []), "none")

    def test_flag_skips_a_pr_already_in_a_review_lane(self):
        for lane in ("review:needs", "review:changes", "review:needs-user", "review:pass"):
            with self.subTest(lane=lane):
                self.assertEqual(decision(pr(labels={lane}), []), "none")

    def test_flag_is_cleared_once_any_verdict_binds(self):
        flagged = pr(labels={vb.UNREVIEWED_LABEL})
        self.assertEqual(decision(flagged, [self.passing]), "unflag")
        self.assertEqual(decision(flagged, [self.failing]), "unflag")
        self.assertEqual(decision(flagged, []), "none")


def decision(pull, comments) -> str:
    return vb.decide(pull, comments).action


# ------------------------------------------------------------------- driver (write path)
#
# `decide()` only produces a WORD. The layer that turns that word into arming authority is
# `VerdictBridge` — the dispatch table, the dry-run short-circuit, the write cap, the
# base-branch filter and the paged reads. A guard with no red test on THIS layer is the
# expensive kind: mis-wiring `"flag"` to `review:pass` would attest every never-reviewed
# green PR in the repo, and every decide()-level and YAML-level test would still pass.

REPO = "sparq-org/sparq"


def commits_connection(logins=("claude",), *, messages=(), total=None) -> dict:
    """A GraphQL ``commits`` connection carrying ONE commit per author login.

    ``total`` defaults to the node count (a complete read). Passing a LARGER ``total``
    models a TRUNCATED connection, which must refuse every grant.
    """
    nodes = [
        {
            "commit": {
                "message": message,
                "authors": {"nodes": [{"user": {"login": login}}]},
                "committer": {"user": {"login": "web-flow"}},
            }
        }
        for login, message in zip(
            logins, list(messages) + [""] * len(logins), strict=False
        )
    ]
    return {"totalCount": len(nodes) if total is None else total, "nodes": nodes}


def node(number: int, **overrides) -> dict:
    """A GraphQL pullRequest node as `list_open` returns it.

    Modelled on the REAL population (censused 2026-07-26): the PR is opened by the
    orchestrator App and its commits are authored by `claude`, while reviewers comment
    under a DIFFERENT login. A fixture whose author fields were absent would make every
    grant fail closed for the wrong reason and hide real regressions.
    """
    labels = overrides.pop("labels", ())
    reviews = overrides.pop("reviews", ())
    base = {
        "author": {"login": overrides.pop("author", "sparq-orchestrator")},
        "commits": overrides.pop("commits", commits_connection()),
        "reviews": {
            "nodes": [
                {
                    "databaseId": r.get("id", 1),
                    "body": r.get("body"),
                    "authorAssociation": r.get("association", "MEMBER"),
                    "submittedAt": r.get("submitted_at", "2026-02-01T00:00:00Z"),
                    "author": {"login": r.get("user", "reviewer")},
                }
                for r in reviews
            ]
        },
        "number": number,
        "state": "OPEN",
        "isDraft": False,
        "baseRefName": "main",
        "headRefOid": HEAD,
        "mergeable": "MERGEABLE",
        "labels": {
            "nodes": [{"name": name} for name in labels],
            "pageInfo": {"hasNextPage": overrides.pop("labels_overflow", False)},
        },
    }
    base.update(overrides)
    return base


def check_run(name: str = "gate", *, conclusion="success", status="completed", rid=1, started="2026-01-01T00:00:00Z") -> dict:
    return {
        "name": name,
        "status": status,
        "conclusion": conclusion,
        "id": rid,
        "started_at": started,
    }


class FakeGitHub:
    """Hermetic stand-in for the `gh` CLI: serves the driver's three reads, RECORDS every
    write. No network, no subprocess."""

    def __init__(self, nodes, *, comments=None, runs=None, pr_page=50, run_page=100):
        self.nodes = list(nodes)
        self._comments = dict(comments or {})
        self._runs = dict(runs or {})
        self.pr_page = pr_page
        self.run_page = run_page
        self.writes: list[list[str]] = []
        self.reads: list[str] = []
        self.total_count_override: int | None = None
        # Fires ONCE, immediately before the first single-PR (reconfirm) read is served.
        # This is what lets a test interpose a concurrent actor's change into the exact
        # window between the sweep's snapshot and its write — the double-fire race.
        self.before_confirm = None

    # -- writes --
    def write(self, argv: list[str]) -> str:
        self.writes.append(list(argv))
        self._apply_label_edit(argv)
        return ""

    def _apply_label_edit(self, argv: list[str]) -> None:
        """Mutate the served node so a SECOND read sees the FIRST write.

        A fake whose state never changes cannot distinguish an idempotent write path
        from one that double-writes: every run would re-read the original labels and
        re-decide the same way. Double-fire idempotence is unprovable without this.
        """
        if argv[:2] != ["pr", "edit"]:
            return
        number = int(argv[2])
        for node in self.nodes:
            if node.get("number") != number:
                continue
            names = [n["name"] for n in node["labels"]["nodes"]]
            if "--add-label" in argv:
                add = argv[argv.index("--add-label") + 1]
                if add not in names:
                    names.append(add)
            if "--remove-label" in argv:
                drop = argv[argv.index("--remove-label") + 1]
                names = [n for n in names if n != drop]
            node["labels"] = dict(node["labels"], nodes=[{"name": n} for n in names])

    def labels_of(self, number: int) -> set:
        for node in self.nodes:
            if node.get("number") == number:
                return {n["name"] for n in node["labels"]["nodes"]}
        raise AssertionError(f"no such node: {number}")

    # -- reads --
    def read(self, argv: list[str]) -> str:
        self.reads.append(" ".join(argv))
        if argv[:2] == ["api", "graphql"]:
            query = next((a for a in argv if a.startswith("query=")), "")
            if "pullRequest(number:" in query:
                return json.dumps(self._pr_one(argv))
            return json.dumps(self._pr_page(argv))
        path = argv[1]
        if "/check-runs" in path:
            return json.dumps(self._check_runs(path))
        if "/comments" in path:
            return json.dumps(self._comment_page(path))
        raise AssertionError(f"unexpected read: {argv}")

    def _pr_one(self, argv: list[str]) -> dict:
        if self.before_confirm is not None:
            hook, self.before_confirm = self.before_confirm, None
            hook(self)
        number = int(next(a for a in argv if a.startswith("number=")).split("=", 1)[1])
        node = next((n for n in self.nodes if n.get("number") == number), None)
        return {"data": {"repository": {"pullRequest": node}}}

    def _pr_page(self, argv: list[str]) -> dict:
        cursor = next(
            (a.split("=", 1)[1] for a in argv if a.startswith("cursor=")), "0"
        )
        start = int(cursor)
        page = self.nodes[start : start + self.pr_page]
        end = start + len(page)
        total = (
            self.total_count_override
            if self.total_count_override is not None
            else len(self.nodes)
        )
        return {
            "data": {
                "repository": {
                    "pullRequests": {
                        "pageInfo": {
                            "hasNextPage": end < len(self.nodes),
                            "endCursor": str(end),
                        },
                        "totalCount": total,
                        "nodes": page,
                    }
                }
            }
        }

    def _check_runs(self, path: str) -> dict:
        sha = path.split("/commits/", 1)[1].split("/", 1)[0]
        page = int(path.rsplit("&page=", 1)[1])
        runs = self._runs.get(sha, [check_run()])
        start = (page - 1) * self.run_page
        return {
            "total_count": len(runs),
            "check_runs": runs[start : start + self.run_page],
        }

    def _comment_page(self, path: str) -> list:
        number = int(path.split("/issues/", 1)[1].split("/", 1)[0])
        page = int(path.rsplit("&page=", 1)[1])
        body = self._comments.get(number, [])
        return body[(page - 1) * 100 : page * 100]


def bridge(fake: FakeGitHub, **kw) -> "vb.VerdictBridge":
    logs: list[str] = []
    b = vb.VerdictBridge(
        REPO,
        kw.pop("default_branch", "main"),
        gh=fake.write,
        gh_read=fake.read,
        log=logs.append,
        **kw,
    )
    b.logs = logs  # type: ignore[attr-defined]
    return b


PASS_COMMENT = [vb.comment(f"reviewed {HEAD}\n\nVERDICT: pass")]
FAIL_COMMENT = [vb.comment(f"reviewed {HEAD}\n\nVERDICT: fail")]


class TestWritePathDispatch(unittest.TestCase):
    """Every decision maps to the RIGHT label edit — the mutation that matters most is
    silent: a dispatch-table entry pointing `flag` at the attestation label."""

    def edit(self, fake: FakeGitHub) -> list[str]:
        self.assertEqual(len(fake.writes), 1, f"expected exactly one write: {fake.writes}")
        return fake.writes[0]

    def test_promote_adds_the_attestation_label_and_removes_nothing_else(self):
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        self.assertEqual(bridge(fake).run(), 0)
        argv = self.edit(fake)
        self.assertEqual(
            argv, ["pr", "edit", "4200", "--repo", REPO, "--add-label", vb.REVIEW_ATTESTATION]
        )
        self.assertNotIn("--remove-label", argv)

    def test_a_flagged_pr_is_unflagged_first_then_promoted_on_the_next_sweep(self):
        """The informational label is cleared before the attestation is written, so the
        two labels are never both live. Cycle 1 unflags; cycle 2 (labels updated by the
        first write) promotes. Neither cycle may write the attestation AND leave the
        flag, which would leave the PR in two lanes at once."""
        first = FakeGitHub(
            [node(4200, labels=[vb.UNREVIEWED_LABEL])], comments={4200: PASS_COMMENT}
        )
        bridge(first).run()
        self.assertEqual(
            self.edit(first),
            ["pr", "edit", "4200", "--repo", REPO, "--remove-label", vb.UNREVIEWED_LABEL],
        )
        second = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        bridge(second).run()
        self.assertEqual(
            self.edit(second),
            ["pr", "edit", "4200", "--repo", REPO, "--add-label", vb.REVIEW_ATTESTATION],
        )

    def test_retract_removes_the_attestation_and_adds_nothing(self):
        fake = FakeGitHub(
            [node(4200, labels=[vb.REVIEW_ATTESTATION])], comments={4200: FAIL_COMMENT}
        )
        bridge(fake).run()
        argv = self.edit(fake)
        self.assertEqual(
            argv,
            ["pr", "edit", "4200", "--repo", REPO, "--remove-label", vb.REVIEW_ATTESTATION],
        )
        self.assertNotIn("--add-label", argv)

    def test_an_unreadable_retraction_also_removes_the_attestation(self):
        """End-to-end proof of the composition fix, through the real write path."""
        comments = [
            vb.comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1),
            vb.comment(
                f"{HEAD}\n\nVERDICT: fail (retracting my pass)",
                created_at="2026-02-01T00:00:00Z",
                cid=2,
            ),
        ]
        fake = FakeGitHub([node(4200, labels=[vb.REVIEW_ATTESTATION])], comments={4200: comments})
        bridge(fake).run()
        argv = self.edit(fake)
        self.assertEqual(argv[argv.index("--remove-label") + 1], vb.REVIEW_ATTESTATION)
        self.assertNotIn("--add-label", argv)

    def test_flag_writes_ONLY_the_informational_label(self):
        """If this ever wrote review:pass, every green never-reviewed PR in the repo
        would be attested and armed. The decide()-level suite cannot see that."""
        fake = FakeGitHub([node(4200)], comments={4200: []})
        bridge(fake).run()
        argv = self.edit(fake)
        self.assertEqual(
            argv, ["pr", "edit", "4200", "--repo", REPO, "--add-label", vb.UNREVIEWED_LABEL]
        )
        self.assertNotIn(vb.REVIEW_ATTESTATION, argv)

    def test_unflag_removes_only_the_informational_label(self):
        fake = FakeGitHub(
            [node(4200, labels=[vb.UNREVIEWED_LABEL])], comments={4200: FAIL_COMMENT}
        )
        bridge(fake).run()
        argv = self.edit(fake)
        self.assertEqual(
            argv,
            ["pr", "edit", "4200", "--repo", REPO, "--remove-label", vb.UNREVIEWED_LABEL],
        )
        self.assertNotIn(vb.REVIEW_ATTESTATION, argv)

    def test_a_review_body_retraction_reaches_the_write_path(self):
        """Reviews ride the SAME GraphQL page as the PR, so this costs no extra API call
        — and a withdrawal posted as a review must remove the attestation."""
        fake = FakeGitHub(
            [
                node(
                    4200,
                    labels=[vb.REVIEW_ATTESTATION],
                    reviews=[
                        {
                            "body": f"{HEAD}\n\nVERDICT: FAIL",
                            "id": 7,
                            "submitted_at": "2099-01-01T00:00:00Z",
                        }
                    ],
                )
            ],
            comments={4200: PASS_COMMENT},
        )
        bridge(fake).run()
        argv = self.edit(fake)
        self.assertEqual(argv[argv.index("--remove-label") + 1], vb.REVIEW_ATTESTATION)
        self.assertNotIn("--add-label", argv)

    def test_a_review_body_pass_never_reaches_the_write_path_as_an_attestation(self):
        fake = FakeGitHub(
            [node(4200, reviews=[{"body": f"{HEAD}\n\nVERDICT: pass", "id": 7}])],
            comments={4200: []},
        )
        bridge(fake).run()
        argv = self.edit(fake)
        self.assertNotIn(vb.REVIEW_ATTESTATION, argv)
        self.assertIn(vb.UNREVIEWED_LABEL, argv)

    def test_the_pr_list_query_actually_requests_the_review_bodies(self):
        """Reviews ride the PR-list query. If the connection (or any field the
        normaliser reads) is dropped, every review-body retraction silently vanishes and
        the fake in these tests would happily keep serving them."""
        query = vb.PR_LIST_QUERY
        self.assertIn("reviews(", query, "the reviews connection is not requested")
        for gql_field in ("databaseId", "body", "authorAssociation", "submittedAt", "author"):
            with self.subTest(field=gql_field):
                self.assertIn(gql_field, query.split("reviews(", 1)[1].split("}}", 1)[0])

    def test_a_PENDING_review_is_not_evidence(self):
        """An unsubmitted review body is a draft; it must not suppress anything."""
        pending = node(
            4200,
            reviews=[{"body": f"{HEAD}\n\nVERDICT: FAIL", "id": 7, "submitted_at": None}],
        )
        self.assertEqual(
            vb.VerdictBridge.review_withholdings(pending),
            [],
            "an unsubmitted review body must not be normalised into evidence at all",
        )
        fake = FakeGitHub(
            [
                node(
                    4200,
                    reviews=[
                        {"body": f"{HEAD}\n\nVERDICT: FAIL", "id": 7, "submitted_at": None}
                    ],
                )
            ],
            comments={4200: PASS_COMMENT},
        )
        bridge(fake).run()
        self.assertEqual(
            self.edit(fake),
            ["pr", "edit", "4200", "--repo", REPO, "--add-label", vb.REVIEW_ATTESTATION],
        )

    def test_a_none_decision_writes_nothing(self):
        fake = FakeGitHub(
            [node(4200, labels=["needs:user"])], comments={4200: PASS_COMMENT}
        )
        self.assertEqual(bridge(fake).run(), 0)
        self.assertEqual(fake.writes, [])


class TestWritePathSafetyRails(unittest.TestCase):
    """dry-run, the write cap, the stacked-PR filter and the fail-closed label read."""

    def test_dry_run_performs_no_write_at_all(self):
        """--dry-run is this PR's central evidence; if it wrote, the evidence would be
        a live mutation of the repo."""
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        b = bridge(fake, dry_run=True)
        self.assertEqual(b.run(), 0)
        self.assertEqual(fake.writes, [], "a dry run must not touch a single label")
        self.assertTrue(any("PROMOTE" in line for line in b.logs), b.logs)

    def test_the_per_run_write_cap_is_enforced(self):
        fake = FakeGitHub(
            [node(n) for n in (1, 2, 3)],
            comments={n: PASS_COMMENT for n in (1, 2, 3)},
        )
        b = bridge(fake, max_writes=2)
        self.assertEqual(b.run(), 0)
        self.assertEqual(len(fake.writes), 2, fake.writes)
        self.assertTrue(any("write cap" in line for line in b.logs), b.logs)

    def test_a_stacked_pr_is_never_touched(self):
        """base != the default branch: auto-arm refuses these outright (the stacked-PR
        auto-merge trap), so attesting one would paint a misleading label."""
        fake = FakeGitHub(
            [node(4200, baseRefName="feature/stack-base")], comments={4200: PASS_COMMENT}
        )
        self.assertEqual(bridge(fake).run(), 0)
        self.assertEqual(fake.writes, [])

    def test_a_label_set_that_exceeds_one_page_fails_CLOSED(self):
        """An unseen label could be a hold, so the PR must be SKIPPED, not promoted."""
        fake = FakeGitHub(
            [node(4200, labels_overflow=True)], comments={4200: PASS_COMMENT}
        )
        b = bridge(fake)
        self.assertEqual(b.run(), 1, "an unreadable label set must be reported as an error")
        self.assertEqual(fake.writes, [], "a partially-read label set must never promote")
        self.assertTrue(any("SKIP inspect-failed" in line for line in b.logs), b.logs)

    def test_a_failed_label_edit_is_counted_as_an_error(self):
        def explode(argv):
            raise vb.GhError("403 label edit denied")

        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        b = vb.VerdictBridge(REPO, "main", gh=explode, gh_read=fake.read, log=lambda _: None)
        self.assertEqual(b.run(), 1)

    def test_run_bridge_reds_the_workflow_on_a_hard_error(self):
        """The exit code is the only signal the cron surfaces."""

        class Boom:
            def run(self):
                return 3

        self.assertEqual(vb.run_bridge(Boom(), log=lambda _: None), 1)

    def test_run_bridge_survives_exhausted_TRANSIENT_reads_with_a_warning(self):
        class Transient:
            def run(self):
                raise vb.gh_retry.GhTransientExhausted("502 x5")

        logs: list[str] = []
        self.assertEqual(vb.run_bridge(Transient(), log=logs.append), 0)
        self.assertTrue(any("::warning" in line for line in logs), logs)


class TestPagedReads(unittest.TestCase):
    """PRs here carry up to ~1181 check-runs; a page-1 read silently truncates."""

    def test_check_runs_are_paged_and_cross_checked_against_total_count(self):
        runs = [check_run(f"filler-{i}", rid=i) for i in range(150)]
        runs.append(check_run("gate", rid=999, started="2026-01-01T05:00:00Z"))
        fake = FakeGitHub([node(4200)], runs={HEAD: runs}, run_page=100)
        b = bridge(fake)
        self.assertEqual(len(b.check_runs(HEAD)), 151)
        self.assertEqual(
            b.gate_conclusion(HEAD), "success", "the gate run lives beyond page 1"
        )

    def test_a_TRUNCATED_check_run_read_raises(self):
        """Fewer runs than total_count means the `gate` may be in the part never read."""
        fake = FakeGitHub([node(4200)], runs={HEAD: [check_run()]})
        original = fake._check_runs
        fake._check_runs = lambda path: dict(original(path), total_count=99)  # type: ignore[assignment]
        with self.assertRaises(vb.GhError):
            bridge(fake).check_runs(HEAD)

    def test_check_runs_CREATED_mid_pagination_do_not_red_the_sweep(self):
        """MEASURED on sparq#4074: `paged 479 check-runs, total_count=473` — runs were
        being created between page reads. Reading MORE than total_count is benign; a
        two-sided equality check skipped the PR and red the workflow every cycle. Only
        TRUNCATION (fewer) is dangerous. Growth also repeats entries across page
        boundaries, so the read de-duplicates by run id."""
        runs = [check_run(f"job-{i}", rid=i) for i in range(150)]
        runs.append(check_run("gate", rid=999, started="2026-01-01T05:00:00Z"))
        fake = FakeGitHub([node(4200)], runs={HEAD: runs}, run_page=100)
        original = fake._check_runs
        # total_count as first read, BEFORE the last 51 runs existed.
        fake._check_runs = lambda path: dict(original(path), total_count=100)  # type: ignore[assignment]
        b = bridge(fake)
        got = b.check_runs(HEAD)
        self.assertEqual(len(got), 151, "growth must not truncate the read")
        self.assertEqual(
            len({r["id"] for r in got}), len(got), "pages that overlap must de-duplicate"
        )
        self.assertEqual(b.gate_conclusion(HEAD), "success")

    def test_a_duplicated_check_run_across_pages_is_counted_once(self):
        """A window that shifts under pagination repeats entries; the server counts them
        once, so double-counting them would mask a real truncation."""
        dupe = check_run("gate", rid=7)
        fake = FakeGitHub([node(4200)], runs={HEAD: [dupe, dict(dupe)]})
        original = fake._check_runs
        fake._check_runs = lambda path: dict(original(path), total_count=1)  # type: ignore[assignment]
        self.assertEqual(len(bridge(fake).check_runs(HEAD)), 1)

    def test_the_check_run_read_is_page_bounded(self):
        """A pathological or looping response must FAIL the read, never spin forever.

        The fixture serves MAX+5 full pages then stops, so the assertion reds cleanly
        when the bound is removed instead of hanging the suite — a test that hangs on a
        mutant is a test that times CI out rather than reporting a defect."""
        pages = vb.MAX_CHECK_RUN_PAGES + 5
        runs = [check_run(f"j-{i}", rid=i) for i in range(pages * 100)]
        fake = FakeGitHub([node(4200)], runs={HEAD: runs}, run_page=100)
        with self.assertRaises(vb.GhError):
            bridge(fake).check_runs(HEAD)

    def test_comments_are_paged_so_a_verdict_beyond_page_one_is_seen(self):
        chatter = [
            vb.comment(f"noise {i}", cid=i, created_at="2026-01-01T00:00:00Z")
            for i in range(100)
        ]
        verdict = vb.comment(
            f"reviewed {HEAD}\n\nVERDICT: pass", cid=5000, created_at="2026-02-01T00:00:00Z"
        )
        fake = FakeGitHub([node(4200)], comments={4200: chatter + [verdict]})
        b = bridge(fake)
        self.assertEqual(len(b.comments(4200)), 101)
        b.run()
        self.assertEqual(len(fake.writes), 1, "the verdict on page 2 must be honoured")
        self.assertIn(vb.REVIEW_ATTESTATION, fake.writes[0])

    def test_open_prs_are_paged_and_cross_checked_against_total_count(self):
        fake = FakeGitHub([node(n) for n in range(1, 121)], pr_page=50)
        self.assertEqual(len(bridge(fake).list_open()), 120)
        fake.total_count_override = 200
        with self.assertRaises(vb.GhError):
            bridge(fake).list_open()


class TestGateResolution(unittest.TestCase):
    """`gate` is resolved NEWEST-run-per-name; a cancelled twin must not read as live."""

    def test_the_newest_gate_run_wins_over_an_older_success(self):
        old = check_run("gate", conclusion="success", rid=1, started="2026-01-01T00:00:00Z")
        new = check_run("gate", conclusion="failure", rid=2, started="2026-01-01T09:00:00Z")
        for ordering in ([old, new], [new, old]):
            with self.subTest(order=[r["id"] for r in ordering]):
                fake = FakeGitHub([node(4200)], runs={HEAD: ordering})
                self.assertEqual(bridge(fake).gate_conclusion(HEAD), "failure")

    def test_an_incomplete_newest_run_reads_as_NO_gate_not_as_the_older_result(self):
        """Only `status: completed` may be believed. A queued or in-progress re-run can
        still carry the PREVIOUS attempt's `conclusion` field; reading it would report a
        superseded green as the live result — the cancelled-twin failure this repo has
        already been bitten by (sparq #3677)."""
        old = check_run("gate", conclusion="success", rid=1, started="2026-01-01T00:00:00Z")
        for status, leftover in (
            ("in_progress", None),
            ("in_progress", "success"),
            ("queued", "success"),
            ("waiting", "failure"),
        ):
            with self.subTest(status=status, conclusion=leftover):
                rerun = check_run(
                    "gate",
                    conclusion=leftover,
                    status=status,
                    rid=2,
                    started="2026-01-01T09:00:00Z",
                )
                fake = FakeGitHub([node(4200)], runs={HEAD: [old, rerun]})
                self.assertIsNone(bridge(fake).gate_conclusion(HEAD))

    def test_a_red_gate_still_never_flags_but_a_verdict_is_still_honoured(self):
        red = [check_run("gate", conclusion="failure")]
        fake = FakeGitHub([node(4200)], runs={HEAD: red}, comments={4200: []})
        self.assertEqual(bridge(fake).run(), 0)
        self.assertEqual(fake.writes, [], "a red PR is not the invisible population")


class TestAuthorXorReviewer(unittest.TestCase):
    """The self-approval guard. One named test per obligation; each reds on deletion.

    NON-VACUITY IS THE POINT OF THIS CLASS. The measured population (all 119 open sparq
    PRs, paginated and totalCount-cross-checked, 2026-07-26) is:

    * 16 head-bound verdict comments from a trusted association;
    * 11 of the 16 posted by the PR's own author, SAME LOGIN (`jeswr`/`jeswr`);
    * only 3 of the 16 would be caught by commit authorship alone, because Claude Code
      commits as `claude` while the reviewing session comments as `jeswr`.

    So a fixture whose author and reviewer differ CANNOT exercise this guard. Every test
    below that claims to cover self-review uses ONE login for both roles, and
    ``test_the_same_login_case_is_not_vacuous`` proves the suite as a whole would notice
    if the guard were removed.
    """

    def contributor_pr(self, **kw):
        """The real shape: the PR's author is (always) in its own contributor set."""
        kw.setdefault("contributors", frozenset({"jeswr", "claude"}))
        return pr(**kw)

    # -- OBLIGATION 0: the OPENER carve-out. Refusing on it stalls the real lane. ------

    def test_a_pass_from_the_PR_OPENER_who_wrote_no_commit_still_PROMOTES(self):
        """Regression pin for a FALSE POSITIVE this guard originally had.

        sparq#4331 and #4386 are `jeswr`-opened with a `jeswr` head-bound pass, and both
        are genuine independent cross-agent reviews (confirmed from dispatch records).
        On the wire: `COMMIT authorship=['claude'], jeswr_committed=False`. An earlier
        draft of this guard folded the opener into the contributor set and called both
        self-approvals. Since 29 of 34 open non-draft PRs cannot be reached by the
        automated review lane at all, refusing these is a total arming stall.
        """
        opener_only = pr(opener="jeswr", contributors=frozenset({"claude"}))
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
        self.assertEqual(decision(opener_only, [own]), "promote")

    def test_the_opener_pass_is_reported_as_COUNTED_not_refused(self):
        opener_only = pr(opener="jeswr", contributors=frozenset({"claude"}))
        got = vb.decide(opener_only, [comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")])
        self.assertTrue(got.opener_verdict, got)
        self.assertFalse(got.self_review, "counted, not refused")
        self.assertIn("COUNTED", got.reason)

    def test_the_opener_advisory_reaches_the_log_without_blocking_the_write(self):
        fake = FakeGitHub(
            [node(4331, author="jeswr", commits=commits_connection(("claude",)))],
            comments={4331: [vb.comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")]},
        )
        b = bridge(fake)
        b.run()
        self.assertEqual(
            fake.writes,
            [["pr", "edit", "4331", "--repo", REPO, "--add-label", vb.REVIEW_ATTESTATION]],
            "the legitimate review must still promote",
        )
        self.assertTrue([ln for ln in b.logs if ln.startswith("::notice")], b.logs)
        self.assertFalse(
            [ln for ln in b.logs if ln.startswith("::warning")],
            "an opener verdict is not a refusal and must not be reported as one",
        )
        self.assertTrue(any("opener_verdicts=1" in ln for ln in b.logs), b.logs)
        self.assertTrue(any("self_reviews_refused=0" in ln for ln in b.logs), b.logs)

    def test_every_opener_pass_is_also_COUNTABLE(self):
        """The invariant that keeps the withdrawal branch unreachable for openers.

        The withdrawal only runs when there is NO countable verdict. An opener pass is
        appended to `countable` as well as `opener_passes`, so an opener-only PR always
        has a verdict and can never reach the withdrawal. Mutating the withdrawal to key
        on `opener_passes` is therefore an EQUIVALENT mutant (measured: M41 SURVIVED) —
        but only while this subset invariant holds. If a future change makes an opener
        pass non-countable, the widening becomes live and would strip review:pass off a
        legitimate same-login agent review. Pin the invariant, not the symptom.
        """
        opener_only = pr(opener="jeswr", contributors=frozenset({"claude"}))
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr", cid=7)
        countable, self_passes, opener_passes = vb.classify_verdicts(
            [own], HEAD, opener_only
        )
        self.assertEqual([v.comment_id for v in opener_passes], [7])
        self.assertEqual(self_passes, [])
        countable_ids = {v.comment_id for v in countable}
        self.assertTrue(
            {v.comment_id for v in opener_passes} <= countable_ids,
            "an opener pass must remain COUNTABLE or the withdrawal becomes reachable",
        )

    def test_the_opener_is_never_in_the_commit_contributor_set(self):
        contributors, complete = vb.commit_contributors(
            {"author": {"login": "jeswr"}, "commits": commits_connection(("claude",))}
        )
        self.assertTrue(complete)
        self.assertNotIn("jeswr", contributors)
        self.assertIn("claude", contributors)

    def test_an_opener_pass_never_causes_a_WITHDRAWAL(self):
        """The withdrawal is scoped to commit-authorship evidence only.

        Stripping review:pass off a PR whose only pass came from its opener would un-arm
        a legitimate review — the exact damage the carve-out exists to prevent.
        """
        opener_held = pr(
            opener="jeswr",
            contributors=frozenset({"claude"}),
            labels={vb.REVIEW_ATTESTATION},
        )
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
        self.assertEqual(decision(opener_held, [own]), "none")

    def test_a_BOT_AUTHORED_pr_does_not_make_this_guard_fire(self):
        """DOCUMENTED LIMITATION, pinned so it is not mistaken for a bug and "fixed".

        Under a scheme where agent PRs are opened by the orchestrator App on conforming
        branches, the PR author is a bot, the commits are `claude`'s and the reviewer
        comments as the operator. That configuration satisfies this guard REGARDLESS of
        whether the commenter is an independent reviewer or the authoring session itself
        — moving the author field does not create a reviewer signal. Closing that gap
        needs a distinct reviewing principal, not a different PR author.
        """
        contributors, _ = vb.commit_contributors(
            {
                "author": {"login": "sparq-orchestrator"},
                "commits": commits_connection(("claude",)),
            }
        )
        bot_pr = pr(opener="sparq-orchestrator", contributors=contributors)
        verdict = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
        got = vb.decide(bot_pr, [verdict])
        self.assertEqual(got.action, "promote")
        self.assertFalse(got.self_review)
        self.assertFalse(
            got.opener_verdict,
            "not even the advisory fires — the guard is blind here, by construction",
        )

    # -- OBLIGATION 1: a verdict by the PR's own author does not promote or arm --------

    def test_a_pass_from_the_prs_own_author_does_not_promote(self):
        own = comment(f"reviewed {HEAD}\n\nVERDICT: pass", user="jeswr")
        self.assertNotEqual(decision(self.contributor_pr(), [own]), "promote")

    def test_the_same_login_case_is_not_vacuous(self):
        """The guard must be what refuses it — not the association, head or shape checks.

        Identical comment body, IDENTICAL PR fixture, ONE difference: the commenter's
        login. If the guard is deleted BOTH arms read `promote` and this reds on the
        second assertion; if the guard over-fires the FIRST assertion reds. Both arms are
        load-bearing.
        """
        body = f"reviewed {HEAD}\n\nVERDICT: pass"
        own = comment(body, user="jeswr")
        independent = comment(body, user="reviewer")
        self.assertEqual(
            decision(self.contributor_pr(), [independent]),
            "promote",
            "the control arm must PASS or the test proves nothing",
        )
        self.assertNotEqual(decision(self.contributor_pr(), [own]), "promote")

    def test_the_live_shape_that_the_bridge_promoted_is_refused(self):
        """Regression-pin of the measured hole, at its real numbers.

        sparq#4331: author `jeswr`, head-bound `VERDICT: pass` by `jeswr` at MEMBER,
        non-draft, MERGEABLE, no review:* label. `decide()` on origin/main returned
        `promote`.
        """
        live = comment(
            f"Reviewed head {HEAD}.\n\nVERDICT: pass",
            user="jeswr",
            association="MEMBER",
        )
        got = vb.decide(
            pr(labels={"area:ci"}, contributors=frozenset({"jeswr", "claude"})), [live]
        )
        self.assertNotEqual(got.action, "promote", got)
        self.assertTrue(got.self_review, got)

    def test_a_self_pass_never_reaches_the_WRITE_path_as_an_attestation(self):
        """decide() is not the arming layer — pin the label the driver actually writes."""
        fake = FakeGitHub(
            [node(4200, author="jeswr", commits=commits_connection(("jeswr",)))],
            comments={4200: [vb.comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")]},
        )
        bridge(fake).run()
        for argv in fake.writes:
            self.assertNotIn(vb.REVIEW_ATTESTATION, argv, fake.writes)

    def test_a_self_pass_cannot_be_laundered_through_a_bot_suffix(self):
        """GraphQL says `sparq-orchestrator`; REST says `sparq-orchestrator[bot]`.

        Without login normalisation an App's self-review is permanently invisible to the
        guard — a check that reads as present and never fires.
        """
        contributors, complete = vb.commit_contributors(
            {
                "author": {"login": "someone-else"},
                # The App COMMITTED here, spelled with the [bot] suffix by GitActor.
                "commits": commits_connection(("sparq-orchestrator[bot]",)),
            }
        )
        self.assertTrue(complete)
        self.assertIn("sparq-orchestrator", contributors)
        app_self = comment(f"{HEAD}\n\nVERDICT: pass", user="sparq-orchestrator[bot]")
        self.assertNotEqual(
            decision(pr(contributors=contributors), [app_self]), "promote"
        )

    def test_a_commit_author_who_did_not_open_the_pr_still_cannot_grant(self):
        """The axis is the CONTRIBUTOR SET, not `pr.user.login`.

        Measured: 86 of 119 open PRs are opened by the App and never commented on by it,
        so a bare `comment.user != pr.user` comparison is unconditionally true there —
        vacuous for the majority population. A commit author must still be refused.
        """
        contributors, _ = vb.commit_contributors(
            {
                "author": {"login": "sparq-orchestrator"},
                "commits": commits_connection(("codex", "jeswr")),
            }
        )
        self.assertIn("jeswr", contributors)
        wrote_it = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
        self.assertNotEqual(
            decision(pr(contributors=contributors), [wrote_it]), "promote"
        )

    def test_an_unresolved_co_author_trailer_still_counts_as_a_contributor(self):
        contributors, _ = vb.commit_contributors(
            {
                "author": {"login": "sparq-orchestrator"},
                "commits": commits_connection(
                    ("claude",),
                    messages=(
                        "feat: thing\n\nCo-Authored-By: A Name "
                        "<12345+ghost@users.noreply.github.com>",
                    ),
                ),
            }
        )
        self.assertIn("ghost", contributors)
        ghost = comment(f"{HEAD}\n\nVERDICT: pass", user="ghost")
        self.assertNotEqual(decision(pr(contributors=contributors), [ghost]), "promote")

    # -- the guard is WITHHOLD-ONLY: it must not become a denial-of-service ------------

    def test_a_self_FAIL_still_retracts_an_existing_attestation(self):
        """Refusing a contributor's own retraction would be fail-OPEN, not fail-closed.

        The REASON is asserted, not just the action. Both the fail path and the
        self-only-withdrawal path retract, so `action == "retract"` alone passes even
        when the guard has been mutated to swallow the fail — measured: that mutant (M14)
        SURVIVED until this assertion named which path produced the retraction.
        """
        own_fail = comment(f"{HEAD}\n\nVERDICT: fail", user="jeswr")
        got = vb.decide(self.contributor_pr(labels={vb.REVIEW_ATTESTATION}), [own_fail])
        self.assertEqual(got.action, "retract", got)
        self.assertIn("head-bound fail by jeswr", got.reason)
        self.assertFalse(got.self_review, "a FAIL is not a suppressed self-pass")

    def test_a_contributors_FAIL_stays_COUNTABLE_and_never_becomes_a_self_pass(self):
        """The guard is one-directional. Routing a contributor's fail into `self_passes`
        would silently disarm every retraction they can make."""
        own_fail = comment(f"{HEAD}\n\nVERDICT: fail", user="jeswr")
        own_ambiguous = comment(f"{HEAD}\n\nVERDICT: FAIL", user="jeswr", cid=2)
        countable, self_passes, _ = vb.classify_verdicts(
            [own_fail, own_ambiguous], HEAD, self.contributor_pr()
        )
        self.assertEqual(
            sorted(v.value for v in countable), sorted(["fail", vb.AMBIGUOUS])
        )
        self.assertEqual(self_passes, [], "only a PASS may ever be suppressed")

    def test_self_passes_only_ever_holds_passes(self):
        mixed = [
            comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr", cid=1),
            comment(f"{HEAD}\n\nVERDICT: fail", user="jeswr", cid=2),
            comment(f"{HEAD}\n\nVERDICT: pass", user="reviewer", cid=3),
        ]
        _, self_passes, _ = vb.classify_verdicts(mixed, HEAD, self.contributor_pr())
        self.assertEqual([(v.author, v.value) for v in self_passes], [("jeswr", "pass")])

    def test_a_self_pass_cannot_override_an_independent_FAIL(self):
        indep_fail = comment(
            f"{HEAD}\n\nVERDICT: fail", user="reviewer", created_at="2026-07-26T10:00:00Z", cid=1
        )
        self_pass = comment(
            f"{HEAD}\n\nVERDICT: pass", user="jeswr", created_at="2026-07-26T23:00:00Z", cid=2
        )
        self.assertNotEqual(
            decision(self.contributor_pr(), [indep_fail, self_pass]), "promote"
        )

    def test_an_independent_pass_still_promotes_alongside_a_self_pass(self):
        """Fail-closed must not mean wedged: the guard removes ONE comment, not the lane."""
        indep = comment(f"{HEAD}\n\nVERDICT: pass", user="reviewer", cid=1)
        self_pass = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr", cid=2)
        self.assertEqual(
            decision(self.contributor_pr(), [indep, self_pass]), "promote"
        )

    # -- OBLIGATION 4: the refusal terminates VISIBLY, not silently --------------------

    def test_a_refused_self_review_is_reported_not_silent(self):
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
        got = vb.decide(self.contributor_pr(), [own])
        self.assertTrue(got.self_review, got)
        self.assertIn("SELF-REVIEW", got.reason)
        self.assertIn("@jeswr", got.reason)

    def test_the_refusal_reaches_the_workflow_log_as_a_warning(self):
        fake = FakeGitHub(
            [node(4200, author="jeswr", commits=commits_connection(("jeswr",)))],
            comments={4200: [vb.comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")]},
        )
        b = bridge(fake)
        b.run()
        warnings = [line for line in b.logs if line.startswith("::warning")]
        self.assertTrue(warnings, b.logs)
        self.assertIn("self-review", warnings[0])
        self.assertIn("#4200", warnings[0])
        self.assertTrue(
            any("self_reviews_refused=1" in line for line in b.logs), b.logs
        )

    def test_the_warning_fires_even_when_no_label_changes(self):
        """The dangerous silence: a refused self-review on an already-labelled PR writes
        NOTHING, so the log line is the ONLY evidence anything was inspected."""
        fake = FakeGitHub(
            [
                node(
                    4200,
                    author="jeswr",
                    commits=commits_connection(("jeswr",)),
                    labels=("review:needs",),
                )
            ],
            comments={4200: [vb.comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")]},
        )
        b = bridge(fake)
        b.run()
        self.assertEqual(fake.writes, [], "no label change is exactly the silent case")
        self.assertTrue([line for line in b.logs if line.startswith("::warning")], b.logs)

    def test_a_refused_self_review_leaves_the_pr_visibly_unreviewed(self):
        """The label half of the terminal state: review-lane-alarm censuses this."""
        fake = FakeGitHub(
            [node(4200, author="jeswr", commits=commits_connection(("jeswr",)))],
            comments={4200: [vb.comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")]},
        )
        bridge(fake).run()
        self.assertEqual(
            fake.writes,
            [["pr", "edit", "4200", "--repo", REPO, "--add-label", vb.UNREVIEWED_LABEL]],
        )

    def test_an_attestation_resting_only_on_a_self_review_is_WITHDRAWN(self):
        """Remediates the promotion that already landed, not just the next one.

        MEASURED: verdict-bridge run 30223748218 labelled sparq#4331 `review:pass` at
        2026-07-26T22:53:30Z from a `VERDICT: pass` its own author posted at 22:11:41Z.
        A self-review is POSITIVE evidence, so this is not a retract-on-absence.
        """
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
        got = vb.decide(
            self.contributor_pr(labels={vb.REVIEW_ATTESTATION, "area:ci"}), [own]
        )
        self.assertEqual(got.action, "retract", got)
        self.assertTrue(got.self_review, got)

    def test_the_withdrawal_reaches_the_write_path(self):
        fake = FakeGitHub(
            [
                node(
                    4331,
                    author="jeswr",
                    commits=commits_connection(("jeswr",)),
                    labels=(vb.REVIEW_ATTESTATION,),
                )
            ],
            comments={4331: [vb.comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")]},
        )
        bridge(fake).run()
        self.assertEqual(
            fake.writes,
            [["pr", "edit", "4331", "--repo", REPO, "--remove-label", vb.REVIEW_ATTESTATION]],
        )

    def test_an_attestation_with_NO_comment_behind_it_is_still_never_retracted(self):
        """The boundary of the rule above: absence must still not retract, or the bridge
        starts fighting every label an orchestrator applied by hand."""
        self.assertEqual(
            decision(self.contributor_pr(labels={vb.REVIEW_ATTESTATION}), []), "none"
        )
        stale = comment(f"reviewed {OTHER}\n\nVERDICT: pass", user="jeswr")
        self.assertEqual(
            decision(self.contributor_pr(labels={vb.REVIEW_ATTESTATION}), [stale]), "none"
        )

    def test_an_independent_pass_alongside_a_self_pass_keeps_the_attestation(self):
        indep = comment(f"{HEAD}\n\nVERDICT: pass", user="reviewer", cid=1)
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr", cid=2)
        self.assertEqual(
            decision(self.contributor_pr(labels={vb.REVIEW_ATTESTATION}), [indep, own]),
            "none",
            "a real review stands; only a self-ONLY attestation is withdrawn",
        )

    def test_a_clean_pr_reports_no_self_review(self):
        """The flag must not be stuck on — otherwise the warning means nothing."""
        indep = comment(f"{HEAD}\n\nVERDICT: pass", user="reviewer")
        self.assertFalse(vb.decide(pr(), [indep]).self_review)

    # -- fail closed on an UNREADABLE contributor set ---------------------------------

    def test_a_truncated_commit_list_refuses_every_grant(self):
        contributors, complete = vb.commit_contributors(
            {
                "author": {"login": "sparq-orchestrator"},
                "commits": commits_connection(("claude",), total=900),
            }
        )
        self.assertFalse(complete, "900 commits cannot fit in a 1-node connection")
        indep = comment(f"{HEAD}\n\nVERDICT: pass", user="reviewer")
        got = vb.decide(
            pr(contributors=contributors, contributors_complete=False), [indep]
        )
        self.assertNotEqual(got.action, "promote", got)
        self.assertIn("unreadable", got.reason)

    def test_a_truncated_commit_list_still_permits_a_retraction(self):
        fail = comment(f"{HEAD}\n\nVERDICT: fail", user="reviewer")
        self.assertEqual(
            decision(
                pr(labels={vb.REVIEW_ATTESTATION}, contributors_complete=False), [fail]
            ),
            "retract",
        )

    def test_an_unattributable_comment_cannot_grant(self):
        anon = comment(f"{HEAD}\n\nVERDICT: pass")
        anon["user"] = {}
        self.assertNotEqual(decision(pr(), [anon]), "promote")

    def test_a_missing_commits_connection_reads_as_INCOMPLETE(self):
        """An absent field must not read as 'nobody contributed'."""
        _, complete = vb.commit_contributors({"author": {"login": "jeswr"}})
        self.assertFalse(complete)

    # -- the guard composes with the axes that already existed ------------------------

    def test_a_blockquoted_self_pass_is_still_not_a_verdict(self):
        """Regression-pin of #4391's finding: every comment here opens `> 🤖 SPARQ agent`,
        so a dropped `^` anchor makes a blockquoted pass the natural false positive."""
        self.assertIsNone(vb.trailing_verdict("> VERDICT: pass"))
        quoted = comment(f"{HEAD}\n\n> VERDICT: pass", user="reviewer")
        self.assertNotEqual(decision(pr(), [quoted]), "promote")

    def test_a_self_pass_bound_to_a_SUPERSEDED_head_is_not_even_seen(self):
        stale = comment(f"reviewed {OTHER}\n\nVERDICT: pass", user="jeswr")
        got = vb.decide(self.contributor_pr(), [stale])
        self.assertNotEqual(got.action, "promote", got)
        self.assertFalse(got.self_review, "a stale verdict is not a self-review refusal")

    def test_classify_verdicts_partitions_rather_than_dropping(self):
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr", cid=1)
        indep = comment(f"{HEAD}\n\nVERDICT: fail", user="reviewer", cid=2)
        countable, self_passes, _ = vb.classify_verdicts(
            [own, indep], HEAD, self.contributor_pr()
        )
        self.assertEqual([v.author for v in countable], ["reviewer"])
        self.assertEqual([v.author for v in self_passes], ["jeswr"])

    def test_classify_without_a_pr_leaves_the_contributor_axis_off(self):
        """The default must not silently enforce a half-configured guard."""
        own = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
        countable, self_passes, _ = vb.classify_verdicts([own], HEAD)
        self.assertEqual(len(countable), 1)
        self.assertEqual(self_passes, [])

    def test_normalize_login_is_case_and_bot_suffix_insensitive(self):
        self.assertEqual(vb.normalize_login("JesWr"), "jeswr")
        self.assertEqual(vb.normalize_login("Sparq-Orchestrator[bot]"), "sparq-orchestrator")
        self.assertEqual(vb.normalize_login(None), "")

    def test_the_pr_list_query_actually_requests_the_authorship_fields(self):
        """A guard fed a field the query never asks for is silently always-empty.

        Deliberately NOT a substring search. `author{login}` and `totalCount` each occur
        TWICE in this query — once where the guard needs them and once inside the
        `reviews` / `pullRequests` connections. A bare `assertIn` passes with the
        PR-level field deleted; measured as a real mutation SURVIVOR (M20) before this
        assertion was tightened. Assert the PR-node line itself, and assert the commits
        fields inside the commits block.
        """
        for name in ("PR_LIST_QUERY", "PR_ONE_QUERY"):
            with self.subTest(query=name):
                query = getattr(vb, name)
                lines = [line.strip() for line in query.splitlines()]
                self.assertIn(
                    "author{login}",
                    lines,
                    "the PR-node-level author field is gone; the `reviews` one does "
                    "not feed the guard",
                )
                start = query.find("commits(last:")
                self.assertGreater(start, 0, "the commits connection is gone")
                block = query[start : start + 260]
                for field_name in ("totalCount", "authors(", "committer", "message"):
                    self.assertIn(
                        field_name, block, f"{field_name} missing from the commits block"
                    )

    def test_the_verdict_regexes_are_anchored_AND_matched_from_the_start(self):
        """Belt and braces, because either one alone is silently sufficient.

        `re.match` already anchors, so deleting the `^` is an EQUIVALENT mutation today —
        it only becomes live the moment someone swaps `.match` for `.search`. Pin the
        pattern text so the anchor cannot be "cleaned up" as redundant, and pin the
        behaviour so a `.search` refactor reds. In this repo EVERY comment opens with
        `> 🤖 SPARQ agent`, so a blockquoted line is the natural false-positive shape.
        """
        self.assertTrue(vb.VERDICT_RE.pattern.startswith("^"), vb.VERDICT_RE.pattern)
        self.assertTrue(vb.VERDICT_SHAPE_RE.pattern.startswith("^"), vb.VERDICT_SHAPE_RE.pattern)
        for shape in ("> VERDICT: pass", ">VERDICT: pass", "> **VERDICT: pass**", "  > VERDICT: FAIL"):
            with self.subTest(shape=shape):
                self.assertIsNone(vb.VERDICT_RE.search(shape), shape)
                self.assertIsNone(vb.VERDICT_SHAPE_RE.search(shape), shape)
                self.assertIsNone(vb.trailing_verdict(shape), shape)

    def test_the_EVENT_DRIVEN_path_enforces_the_guard_too(self):
        """#4386 added a single-PR fetch + reconfirm. A guard wired only into the sweep
        would be silently always-empty there — the contributor set would come back empty
        and every verdict would grant. Both queries share PR_NODE_FIELDS; pin the
        behaviour, not just the string."""
        n = node(4200, author="jeswr", commits=commits_connection(("jeswr",)))
        fake = FakeGitHub(
            [n], comments={4200: [vb.comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")]}
        )
        b = bridge(fake)
        parsed = b.parse_node(n, "success")
        self.assertIn("jeswr", parsed.contributors)
        fresh, decision_ = b.reconfirm(parsed, vb.Decision("promote", "stale"))
        self.assertNotEqual(
            decision_.action, "promote", "the event path must refuse a commit-author pass"
        )
        self.assertTrue(decision_.self_review, decision_)

    def test_the_commits_page_is_pinned_to_the_documented_maximum(self):
        """Narrowing this silently WEDGES the lane instead of opening a hole.

        With a page smaller than the PR's commit count, `totalCount` exceeds the nodes
        read, `contributors_complete` goes False and EVERY grant is refused — fail-closed,
        but a total arming stall. Measured as a mutation survivor (M37, `last:1`) before
        this test existed. 100 is GitHub's documented per-connection maximum, so it is
        also the ceiling.
        """
        found = re.search(r"commits\(last:(\d+)\)", vb.PR_LIST_QUERY)
        self.assertIsNotNone(found, "the commits connection lost its page size")
        self.assertEqual(
            int(found.group(1)),
            100,
            "100 is both the documented GitHub maximum and 10x the live commit ceiling",
        )

    def test_parse_node_populates_the_contributor_set_from_the_live_shape(self):
        b = bridge(FakeGitHub([]))
        parsed = b.parse_node(
            node(4200, author="jeswr", commits=commits_connection(("claude", "codex"))),
            "success",
        )
        self.assertTrue(parsed.contributors_complete)
        self.assertEqual(parsed.contributors, frozenset({"claude", "codex", "web-flow"}))
        self.assertNotIn(
            "jeswr", parsed.contributors, "the OPENER must not be a refusal signal"
        )
        self.assertEqual(parsed.opener, "jeswr", "but it must still be reported")

class TestScopedRunAuthority(unittest.TestCase):
    """The EVENT path must not be a laxer route to the same label than the CRON path.

    The two paths differ in exactly one thing — WHICH PRs enter the loop. Everything that
    confers authority (the node field selection, ``decide``, provenance, head binding,
    holds, the dispatch table, the base-branch filter, the fail-closed label read) is
    shared. These tests assert that by DIFFERENTIAL EXECUTION rather than by inspection:
    the same fixture is run both ways and the emitted writes must be byte-identical.
    """

    # Every interesting shape the policy distinguishes, exercised through both paths.
    CASES = {
        "promote": (dict(), PASS_COMMENT),
        "flag": (dict(), []),
        "retract-on-fail": (dict(labels=[vb.REVIEW_ATTESTATION]), FAIL_COMMENT),
        "unflag": (dict(labels=[vb.UNREVIEWED_LABEL]), FAIL_COMMENT),
        "hold-needs-user": (dict(labels=["needs:user"]), PASS_COMMENT),
        "hold-zk": (dict(labels=["area:sparq-zk"]), PASS_COMMENT),
        "draft": (dict(isDraft=True), PASS_COMMENT),
        "conflicting": (dict(mergeable="CONFLICTING"), []),
        "stacked-base": (dict(baseRefName="feature/stack"), PASS_COMMENT),
        "labels-overflow": (dict(labels_overflow=True), PASS_COMMENT),
        "untrusted-pass": (
            dict(),
            [vb.comment(f"{HEAD}\n\nVERDICT: pass", association="NONE")],
        ),
        "stale-head-pass": (dict(), [vb.comment(f"{OTHER}\n\nVERDICT: pass")]),
        "ambiguous-retraction": (
            dict(labels=[vb.REVIEW_ATTESTATION]),
            [
                vb.comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1),
                vb.comment(f"{HEAD}\n\nVERDICT: FAIL", created_at="2026-02-01T00:00:00Z", cid=2),
            ],
        ),
    }

    def both_paths(self, overrides, comments):
        sweep_fake = FakeGitHub([node(4200, **overrides)], comments={4200: list(comments)})
        sweep = bridge(sweep_fake)
        sweep_rc = sweep.run()
        event_fake = FakeGitHub([node(4200, **overrides)], comments={4200: list(comments)})
        event = bridge(event_fake, only_pr=4200)
        event_rc = event.run()
        return (sweep_fake, sweep_rc), (event_fake, event_rc)

    def test_the_event_path_emits_the_IDENTICAL_writes_as_the_cron_path(self):
        for name, (overrides, comments) in self.CASES.items():
            with self.subTest(case=name):
                (sweep, sweep_rc), (event, event_rc) = self.both_paths(overrides, comments)
                self.assertEqual(
                    event.writes, sweep.writes, f"{name}: event path diverged from cron"
                )
                self.assertEqual(event_rc, sweep_rc, f"{name}: exit code diverged")

    def test_the_event_path_can_never_write_the_attestation_where_the_cron_would_not(self):
        """The one-directional statement of the same property: for EVERY fixture, the set
        of PRs the event path attests is a SUBSET of the set the cron path attests."""
        for name, (overrides, comments) in self.CASES.items():
            with self.subTest(case=name):
                (sweep, _), (event, _) = self.both_paths(overrides, comments)
                attests = lambda fake: any(  # noqa: E731
                    vb.REVIEW_ATTESTATION in w and "--add-label" in w for w in fake.writes
                )
                if attests(event):
                    self.assertTrue(
                        attests(sweep),
                        f"{name}: the event path attested a PR the cron path refused",
                    )

    def test_both_paths_request_the_SAME_node_fields(self):
        """A field present only in the sweep query would make the event path decide on
        LESS information — e.g. dropping the labels connection reads every hold as
        absent, and every held PR would be attested through the event path alone."""
        self.assertIn(vb.PR_NODE_FIELDS, vb.PR_LIST_QUERY)
        self.assertIn(vb.PR_NODE_FIELDS, vb.PR_ONE_QUERY)
        for field in ("labels(", "reviews(", "headRefOid", "baseRefName", "mergeable",
                      "isDraft", "state", "authorAssociation", "submittedAt"):
            with self.subTest(field=field):
                self.assertIn(field, vb.PR_NODE_FIELDS)

    def test_a_scoped_run_reads_ONLY_the_named_pr(self):
        """The whole point: an event must not trigger the ~9-minute all-PR sweep."""
        fake = FakeGitHub(
            [node(n) for n in (4200, 4201, 4202)],
            comments={n: PASS_COMMENT for n in (4200, 4201, 4202)},
        )
        bridge(fake, only_pr=4201).run()
        self.assertEqual([w[2] for w in fake.writes], ["4201"])
        self.assertFalse(
            any("pullRequests(states:OPEN" in r for r in fake.reads),
            "a scoped run must never issue the whole-repo listing query",
        )

    def test_an_event_naming_a_vanished_pr_is_a_clean_no_op(self):
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        b = bridge(fake, only_pr=999999)
        self.assertEqual(b.run(), 0)
        self.assertEqual(fake.writes, [])

    def test_a_broken_single_pr_read_RAISES_rather_than_reading_as_no_work(self):
        """Fail-closed at the new read. Returning [] on an error would make every API
        blip look like 'this PR needs nothing'."""
        fake = FakeGitHub([node(4200)])
        b = bridge(fake, only_pr=4200)
        for broken in (
            {"errors": [{"message": "boom"}]},   # GraphQL reported an error
            {"data": {}},                        # no repository connection at all
            {"data": {"repository": None}},      # null repository (permission loss)
        ):
            with self.subTest(payload=broken):
                b.gh_read = lambda argv, _p=broken: json.dumps(_p)
                with self.assertRaises(vb.GhError):
                    b.fetch_one(4200)

    def test_a_non_positive_or_non_numeric_pr_scope_is_refused(self):
        for bad in (0, -1):
            with self.subTest(pr=bad):
                with self.assertRaises(ValueError):
                    vb.VerdictBridge(REPO, "main", only_pr=bad)

    def test_the_pr_argument_parser_never_degrades_garbage_to_a_full_sweep(self):
        """`--pr` carries webhook-supplied data. Coercing an unparseable value to None
        (= sweep) would let a malformed payload start a repository-wide run."""
        self.assertIsNone(vb.parse_pr_argument(""))
        self.assertIsNone(vb.parse_pr_argument(None))
        self.assertEqual(vb.parse_pr_argument(" 4324 "), 4324)
        for bad in ("0", "-3", "12x", "4324 && curl evil", "1e3", "٤٣", "null", "*"):
            with self.subTest(value=bad):
                with self.assertRaises(ValueError):
                    vb.parse_pr_argument(bad)

    def test_arming_relevant_decisions_never_depend_on_the_gate_read(self):
        """Load-bearing for reconfirm(), which carries the gate conclusion forward rather
        than paying up to 7 paginated pages again. If promote/retract/unflag ever started
        consulting the gate, that carry-forward would become a stale input on the ARMING
        path — this test is what makes the shortcut sound."""
        for name, (overrides, comments) in (
            ("promote", (dict(), PASS_COMMENT)),
            ("retract", (dict(labels=[vb.REVIEW_ATTESTATION]), FAIL_COMMENT)),
            ("unflag", (dict(labels=[vb.UNREVIEWED_LABEL]), FAIL_COMMENT)),
        ):
            with self.subTest(action=name):
                reader = bridge(FakeGitHub([node(4200, **overrides)]))
                actions = {
                    vb.decide(
                        reader.parse_node(node(4200, **overrides), gate), comments
                    ).action
                    for gate in ("success", "failure", "cancelled", None)
                }
                self.assertEqual(
                    actions, {name}, f"{name} changed with the gate conclusion: {actions}"
                )


class TestDoubleFireIdempotence(unittest.TestCase):
    """CONSTRAINT: event and cron now race. Running twice on one head must converge.

    These are EXECUTIONS, not assertions about the design. The fake mutates its label
    state on write (FakeGitHub._apply_label_edit), so a second run genuinely observes what
    the first one did.
    """

    def test_two_sequential_runs_over_one_head_write_exactly_once(self):
        for name, (overrides, comments) in (
            ("promote", (dict(), PASS_COMMENT)),
            ("flag", (dict(), [])),
            ("retract", (dict(labels=[vb.REVIEW_ATTESTATION]), FAIL_COMMENT)),
            ("unflag", (dict(labels=[vb.UNREVIEWED_LABEL]), FAIL_COMMENT)),
        ):
            with self.subTest(action=name):
                fake = FakeGitHub([node(4200, **overrides)], comments={4200: comments})
                bridge(fake).run()
                first = len(fake.writes)
                self.assertEqual(first, 1, fake.writes)
                bridge(fake).run()
                self.assertEqual(
                    len(fake.writes), 1, f"{name} re-wrote on the second run: {fake.writes}"
                )

    def test_the_event_run_and_the_cron_run_over_one_head_converge(self):
        """The literal double-fire: the SAME state, one scoped run and one sweep."""
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        bridge(fake, only_pr=4200).run()
        bridge(fake).run()
        self.assertEqual(len(fake.writes), 1, fake.writes)
        self.assertEqual(fake.labels_of(4200), {vb.REVIEW_ATTESTATION})

    def test_a_STALE_sweep_decision_cannot_resurrect_a_label_an_event_just_removed(self):
        """THE race the conversion creates, executed through the REAL driver loop.

        Interleaving, produced by the fake rather than described in prose: the sweep
        reads PR #4200 at T0 (verdict = pass, decision = promote) and is still working
        through its other ~100 PRs. At T1 — inside the window, injected by
        ``before_confirm`` — a reviewer posts a retraction and the event run removes the
        attestation. The sweep then reaches its write with a T0 snapshot that says
        `promote`, and would arm a PR whose review was just withdrawn.

        The pre-write re-read is the only thing that stops it. Mutant P01 (delete the
        `confirmed.action != decision.action` skip) reds HERE.
        """
        fake = FakeGitHub([node(4200)], comments={4200: list(PASS_COMMENT)})

        def a_retraction_lands_mid_sweep(f):
            f._comments[4200] = list(PASS_COMMENT) + [
                vb.comment(
                    f"{HEAD}\n\nVERDICT: fail", created_at="2099-01-01T00:00:00Z", cid=99
                )
            ]

        fake.before_confirm = a_retraction_lands_mid_sweep
        sweep = bridge(fake)
        self.assertEqual(sweep.run(), 0)
        self.assertEqual(
            fake.writes, [], "a stale sweep resurrected a retracted attestation"
        )
        self.assertNotIn(vb.REVIEW_ATTESTATION, fake.labels_of(4200))
        self.assertTrue(
            any("SKIP superseded" in line for line in sweep.logs), sweep.logs
        )

    def test_promote_and_the_unreviewed_flag_are_mutually_exclusive_by_construction(self):
        """Why the promote dispatch entry removes NOTHING.

        `decide()` returns `unflag` before it can ever return `promote` when the
        informational label is present, so no confirmed `promote` can coexist with it.
        The removal that used to be paired with `promote` was therefore dead — and dead
        code on a write path is exactly where a stale-snapshot read hides. This pins the
        precondition so the pairing cannot be reintroduced on a false premise.
        """
        for extra in ([], ["area:sparq-core"], ["kind:task"]):
            with self.subTest(extra=extra):
                flagged = pr(labels=set(extra) | {vb.UNREVIEWED_LABEL})
                self.assertNotEqual(
                    vb.decide(flagged, PASS_COMMENT).action,
                    "promote",
                    "a flagged PR must unflag first; promote is a later cycle",
                )
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        bridge(fake).run()
        self.assertEqual(
            fake.writes,
            [["pr", "edit", "4200", "--repo", REPO, "--add-label", vb.REVIEW_ATTESTATION]],
        )

    def test_main_refuses_an_event_run_whose_payload_named_no_pull_request(self):
        """`--mode event --pr ''` must be a no-op, NOT a fall-through to the whole-repo
        sweep: that is the difference between an unroutable webhook and letting any
        commenter start a ~9-minute repository-wide run. Mutant P11 reds here."""
        started: list = []
        original_run_bridge, original_argv = vb.run_bridge, sys.argv
        try:
            vb.run_bridge = lambda b, **kw: started.append(b) or 0

            def main_with(*args) -> int:
                sys.argv = ["verdict-bridge.py", "--repo", REPO, *args]
                with contextlib.redirect_stdout(io.StringIO()):
                    return vb.main()

            self.assertEqual(main_with("--mode", "event", "--pr", ""), 0)
            self.assertEqual(started, [], "an unroutable event started a full sweep")

            self.assertEqual(main_with("--mode", "event", "--pr", "42"), 0)
            self.assertEqual([b.only_pr for b in started], [42])

            self.assertEqual(main_with("--mode", "sweep", "--pr", ""), 0)
            self.assertEqual(
                [b.only_pr for b in started], [42, None], "the cron sweep must still run"
            )
        finally:
            vb.run_bridge, sys.argv = original_run_bridge, original_argv

    def test_a_HEAD_CHANGE_between_read_and_write_abandons_the_write(self):
        """A force-push invalidates every head-bound verdict. The pre-write re-read is a
        genuine compare-and-set on the head SHA."""
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        b = bridge(fake)
        stale_pr = b.parse_node(node(4200), "success")
        # The PR is force-pushed to a new head after the snapshot was taken.
        fake.nodes[0]["headRefOid"] = "c" * 40
        _fresh, confirmed = b.reconfirm(stale_pr, vb.Decision("promote", "stale"))
        self.assertNotEqual(confirmed.action, "promote")
        b.run()
        self.assertNotIn(vb.REVIEW_ATTESTATION, fake.labels_of(4200))

    def test_reconfirm_refuses_when_the_pr_vanished_or_its_base_moved(self):
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        b = bridge(fake)
        pull = b.parse_node(node(4200), "success")
        fake.nodes = []
        self.assertEqual(b.reconfirm(pull, vb.Decision("promote", ""))[1].action, "none")
        fake.nodes = [node(4200, baseRefName="feature/stack")]
        self.assertEqual(b.reconfirm(pull, vb.Decision("promote", ""))[1].action, "none")

    def test_a_DROPPED_webhook_is_still_reconciled_by_the_sweep(self):
        """CONSTRAINT: the cron is the backstop for events GitHub never delivers.

        Modelled by simply not running the event path at all. The sweep must reach the
        same terminal state — otherwise the conversion has traded a latency bug for a
        lost-work bug, which is strictly worse.
        """
        for name, (overrides, comments, expected) in {
            "promote": (dict(), PASS_COMMENT, {vb.REVIEW_ATTESTATION}),
            "flag": (dict(), [], {vb.UNREVIEWED_LABEL}),
            "retract": (dict(labels=[vb.REVIEW_ATTESTATION]), FAIL_COMMENT, set()),
            "unflag": (dict(labels=[vb.UNREVIEWED_LABEL]), FAIL_COMMENT, set()),
        }.items():
            with self.subTest(action=name):
                delivered = FakeGitHub(
                    [node(4200, **overrides)], comments={4200: comments}
                )
                bridge(delivered, only_pr=4200).run()
                dropped = FakeGitHub([node(4200, **overrides)], comments={4200: comments})
                bridge(dropped).run()  # cron only — the webhook never arrived
                self.assertEqual(dropped.labels_of(4200), expected)
                self.assertEqual(dropped.labels_of(4200), delivered.labels_of(4200))

    def test_a_write_still_happens_when_nothing_changed_under_the_run(self):
        """The guard must not be a blanket refusal — that would be a silent kill switch
        indistinguishable from 'the bridge is safe now'."""
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        self.assertEqual(bridge(fake).run(), 0)
        self.assertEqual(len(fake.writes), 1, fake.writes)
        self.assertEqual(fake.labels_of(4200), {vb.REVIEW_ATTESTATION})

    def test_a_reconfirm_read_failure_is_an_ERROR_and_writes_nothing(self):
        fake = FakeGitHub([node(4200)], comments={4200: PASS_COMMENT})
        b = bridge(fake)
        real = fake.read

        def flaky(argv):
            if "pullRequest(number:" in " ".join(argv):
                raise vb.GhError("502 on the reconfirm read")
            return real(argv)

        fake.read = flaky
        b.gh_read = flaky
        self.assertEqual(b.run(), 1, "an unverifiable write must be reported")
        self.assertEqual(fake.writes, [], "never write on an unconfirmed decision")


class TestEventModeFailureSemantics(unittest.TestCase):
    """A dropped EVENT has no useful backstop: the cron's MEASURED real cadence on this
    repository is 53-75 minutes (11.1% of scheduled ticks fired in the 24h to
    2026-07-26T22:10Z), not the nominal 10."""

    class Transient:
        def run(self):
            raise vb.gh_retry.GhTransientExhausted("502 x5")

    def test_event_mode_reds_on_exhausted_transients(self):
        logs: list[str] = []
        self.assertEqual(
            vb.run_bridge(self.Transient(), log=logs.append, mode="event"), 1
        )
        self.assertTrue(any("::error" in line for line in logs), logs)

    def test_sweep_mode_still_fails_soft(self):
        logs: list[str] = []
        self.assertEqual(
            vb.run_bridge(self.Transient(), log=logs.append, mode="sweep"), 0
        )
        self.assertTrue(any("::warning" in line for line in logs), logs)

    def test_sweep_is_the_default_mode(self):
        self.assertEqual(vb.run_bridge(self.Transient(), log=lambda _: None), 0)


class TestCrossPolicyConsistency(unittest.TestCase):
    """The bridge must be safe against the LIVE arming policies, not a copy of them."""

    def test_bridge_holds_cover_every_auto_arm_exclusion(self):
        """Never attest a PR auto-arm would refuse — the label would be a lie."""
        required = auto_arm.HUMAN_OR_TRUST_LABELS | auto_arm.REVIEW_CHANGES_LABELS
        missing = required - vb.HOLD_LABELS
        self.assertEqual(missing, set(), f"auto-arm excludes these, the bridge does not: {missing}")

    def test_bridge_holds_cover_every_rearm_sweeper_exclusion(self):
        missing = rearm.EXCLUDED_LABELS - vb.HOLD_LABELS
        self.assertEqual(missing, set(), f"rearm-sweeper excludes these: {missing}")

    def test_the_informational_label_is_not_a_hold_in_any_policy(self):
        """review:unreviewed must never block an arm, or flagging would deadlock it."""
        label = vb.UNREVIEWED_LABEL
        self.assertNotIn(label, auto_arm.HUMAN_OR_TRUST_LABELS)
        self.assertNotIn(label, auto_arm.REVIEW_CHANGES_LABELS)
        self.assertNotIn(label, rearm.EXCLUDED_LABELS)
        self.assertEqual(rearm.exclusion_labels(frozenset({label})), [])
        self.assertEqual(vb.hold_labels({label}), [])

    def test_the_attestation_label_matches_the_arming_predicate(self):
        self.assertEqual(vb.REVIEW_ATTESTATION, auto_arm.REVIEW_LABEL)
        self.assertEqual(vb.REVIEW_ATTESTATION, rearm.REVIEW_ATTESTATION)


# --------------------------------------------------------------------------- YAML seam


def load_workflow(path: Path) -> dict:
    doc = yaml.safe_load(path.read_text(encoding="utf-8"))
    assert isinstance(doc, dict), path
    return doc


def triggers(doc: dict) -> dict:
    # PyYAML resolves the bare key `on` to the boolean True (YAML 1.1).
    return doc.get("on", doc.get(True)) or {}


def cron_minutes(doc: dict) -> set[int]:
    out: set[int] = set()
    for entry in triggers(doc).get("schedule") or []:
        minute_field = str(entry["cron"]).split()[0]
        out.update(int(m) for m in minute_field.split(","))
    return out


# --------------------------------------------------- GitHub-expression evaluator
#
# The measured lesson on this repository is that every uncaught mutant in an 18-mutant run
# lived at the YAML seam — a workflow `if:`, a step, a call site — not in the Python. A
# substring assertion over an `if:` is exactly the vacuous shape that lets an INVERTED
# clause survive. So the job condition is PARSED and EVALUATED here against synthetic
# webhook payloads, and the admit/skip matrix is asserted. Inverting `!=` to `==`,
# deleting a clause, or dropping the schedule admission all change that matrix.
#
# Supported subset (all this workflow uses): || && == != ( ) null 'literal'
# and dotted context paths with [n] indexing.

_TOKEN = re.compile(
    r"\s*(\(|\)|,|\|\||&&|==|!=|'[^']*'|[A-Za-z_][A-Za-z0-9_.\[\]]*)"
)


def _tokenize(expression: str) -> list[str]:
    out, pos = [], 0
    while pos < len(expression):
        m = _TOKEN.match(expression, pos)
        if not m:
            if expression[pos].isspace():
                pos += 1
                continue
            raise AssertionError(f"unlexable at {pos}: {expression[pos:pos + 30]!r}")
        out.append(m.group(1))
        pos = m.end()
    return out


class _ExprParser:
    """Recursive descent over the tokenized expression. || binds loosest, then &&."""

    def __init__(self, tokens: list[str], context: dict):
        self.tokens, self.pos, self.context = tokens, 0, context

    def peek(self):
        return self.tokens[self.pos] if self.pos < len(self.tokens) else None

    def take(self):
        tok = self.peek()
        self.pos += 1
        return tok

    def parse(self):
        value = self.or_expr()
        assert self.peek() is None, f"trailing tokens from {self.pos}: {self.tokens}"
        return value

    def or_expr(self):
        value = self.and_expr()
        while self.peek() == "||":
            self.take()
            right = self.and_expr()
            value = value if _truthy(value) else right
        return value

    def and_expr(self):
        value = self.cmp_expr()
        while self.peek() == "&&":
            self.take()
            right = self.cmp_expr()
            value = right if _truthy(value) else value
        return value

    def cmp_expr(self):
        left = self.primary()
        if self.peek() in ("==", "!="):
            op = self.take()
            right = self.primary()
            return (left == right) if op == "==" else (left != right)
        return left

    def primary(self):
        tok = self.take()
        if tok == "(":
            value = self.or_expr()
            assert self.take() == ")", "unbalanced parentheses"
            return value
        if tok == "contains" and self.peek() == "(":
            self.take()
            haystack = self.or_expr()
            assert self.take() == ",", "contains() takes two arguments"
            needle = self.or_expr()
            assert self.take() == ")", "unbalanced parentheses"
            # GitHub's contains() is CASE-INSENSITIVE for strings — load-bearing here,
            # because verdict-bridge.py's VERDICT_SHAPE_RE is re.IGNORECASE.
            return str(needle).lower() in str(haystack or "").lower()
        if tok.startswith("'"):
            return tok[1:-1]
        if tok == "null":
            return None
        if tok in ("true", "false"):
            return tok == "true"
        return _lookup(self.context, tok)


def _truthy(value) -> bool:
    return bool(value) and value != ""


def _lookup(context: dict, path: str):
    """`github.event.workflow_run.pull_requests[0].number` against a dict tree."""
    node = context
    for part in path.split("."):
        while part.endswith("]"):
            part, _, index = part[:-1].partition("[")
            if part:
                node = node.get(part) if isinstance(node, dict) else None
                part = ""
            if not isinstance(node, list) or int(index) >= len(node):
                return None
            node = node[int(index)]
        if part:
            node = node.get(part) if isinstance(node, dict) else None
        if node is None:
            return None
    return node


def evaluate_if(expression: str, context: dict) -> bool:
    return _truthy(_ExprParser(_tokenize(expression), context).parse())


def payload(event_name: str, **event) -> dict:
    return {"github": {"event_name": event_name, "event": event}}


class TestExpressionEvaluatorItself(unittest.TestCase):
    """A broken evaluator would make TestJobConditionAdmission pass vacuously."""

    def test_boolean_precedence_and_short_circuit(self):
        ctx = payload("issue_comment", issue={"pull_request": {"url": "u"}})
        self.assertTrue(evaluate_if("github.event_name == 'issue_comment'", ctx))
        self.assertFalse(evaluate_if("github.event_name == 'schedule'", ctx))
        self.assertTrue(evaluate_if("github.event.issue.pull_request != null", ctx))
        self.assertFalse(evaluate_if("github.event.missing.thing != null", ctx))
        # && binds tighter than ||
        self.assertTrue(
            evaluate_if(
                "github.event_name == 'schedule' "
                "|| github.event_name == 'issue_comment' "
                "&& github.event.issue.pull_request != null",
                ctx,
            )
        )
        self.assertFalse(
            evaluate_if(
                "(github.event_name == 'schedule' "
                "|| github.event_name == 'issue_comment') "
                "&& github.event.issue.pull_request == null",
                ctx,
            )
        )

    def test_contains_is_case_insensitive_and_absent_safe(self):
        ctx = payload("issue_comment", comment={"body": "reviewed\n\nverdict: fail"})
        self.assertTrue(evaluate_if("contains(github.event.comment.body, 'VERDICT')", ctx))
        self.assertFalse(
            evaluate_if(
                "contains(github.event.comment.body, 'VERDICT')",
                payload("issue_comment", comment={"body": "lgtm"}),
            )
        )
        self.assertFalse(
            evaluate_if(
                "contains(github.event.comment.body, 'VERDICT')",
                payload("issue_comment", comment={}),
            )
        )

    def test_array_indexing_resolves_and_fails_soft_when_absent(self):
        with_pr = payload("workflow_run", workflow_run={"pull_requests": [{"number": 7}]})
        without = payload("workflow_run", workflow_run={"pull_requests": []})
        path = "github.event.workflow_run.pull_requests[0].number"
        self.assertEqual(_lookup(with_pr, path), 7)
        self.assertIsNone(_lookup(without, path))
        self.assertTrue(evaluate_if(f"{path} != null", with_pr))
        self.assertFalse(evaluate_if(f"{path} != null", without))


class TestWorkflowWiring(unittest.TestCase):
    """STRUCTURAL inspection: `if: false`, a deleted step, or a wrong input must red."""

    @classmethod
    def setUpClass(cls):
        cls.doc = load_workflow(WORKFLOW)
        cls.job = cls.doc["jobs"]["bridge"]
        cls.steps = cls.job["steps"]

    def step_by_run_needle(self, needle: str) -> dict:
        matches = [
            s for s in self.steps if needle in str(s.get("run") or "")
        ]
        self.assertEqual(
            len(matches), 1, f"expected exactly one step whose run contains {needle!r}"
        )
        return matches[0]

    def test_the_job_is_not_disabled_by_a_job_level_condition(self):
        """An `if: false` on the job silently turns the whole bridge off.

        The condition is no longer absent (the event triggers need a payload guard), so
        the property asserted is the one that matters: it may never be a constant, and
        the SCHEDULED backstop must be admitted unconditionally. The full admit/skip
        matrix is evaluated in TestJobConditionAdmission.
        """
        self.assertIn("if", self.job, "the event payload guard must be present")
        self.assertNotIn(
            str(self.job["if"]).strip().lower(),
            ("false", "true"),
            "a constant job condition is either a kill switch or an unguarded event path",
        )
        self.assertNotIn("continue-on-error", self.job)

    def test_no_step_is_disabled_or_swallowed(self):
        """Every step except the fail-soft App-token mint runs unconditionally."""
        for step in self.steps:
            name = step.get("name") or step.get("uses") or "<step>"
            with self.subTest(step=name):
                self.assertNotEqual(
                    str(step.get("if", "")).strip().lower(),
                    "false",
                    "a literal `if: false` disables this step",
                )
                self.assertNotIn("continue-on-error", step, name)
        conditional = [s for s in self.steps if "if" in s]
        self.assertEqual(
            [s.get("id") for s in conditional],
            ["app-token"],
            "only the App-token mint may be conditional",
        )

    def test_the_self_test_step_exists_and_runs_the_policy_self_test(self):
        step = self.step_by_run_needle("scripts/verdict-bridge.py --self-test")
        self.assertNotIn("if", step)
        self.assertIn("scripts/gh_retry.py --self-test", step["run"])

    def test_the_live_run_step_invokes_the_bridge_with_a_repo(self):
        step = self.step_by_run_needle(LIVE_RUN_NEEDLE)
        self.assertIn("--repo", step["run"])
        self.assertIn("--default-branch", step["run"])
        self.assertEqual(step["env"]["REPO"], "${{ github.repository }}")

    def test_the_scheduled_sweep_is_never_a_hard_coded_dry_run(self):
        """`DRY_RUN: --dry-run` (or a literal `--dry-run` in the `run:`) turns the whole
        workflow into a permanent no-op while EVERY other structural test still passes.
        The flag may only come from the workflow_dispatch input."""
        step = self.step_by_run_needle(LIVE_RUN_NEEDLE)
        self.assertNotIn(
            "--dry-run", step["run"], "the sweep's argv must not hard-code --dry-run"
        )
        self.assertIn("$DRY_RUN", step["run"], "the flag must come from the env")
        dry_run = str(step["env"]["DRY_RUN"])
        self.assertIn(
            "github.event.inputs.dry_run",
            dry_run,
            "DRY_RUN must be gated on the manual dispatch input, nothing else",
        )
        self.assertTrue(
            dry_run.startswith("${{") and dry_run.endswith("}}"),
            f"DRY_RUN must be an expression, not the literal {dry_run!r}",
        )
        inputs = (triggers(self.doc).get("workflow_dispatch") or {}).get("inputs") or {}
        self.assertIn("dry_run", inputs, "the expression references an undeclared input")
        self.assertFalse(
            inputs["dry_run"].get("default"), "a scheduled sweep must default to writing"
        )

    def test_the_informational_label_is_reified_before_use(self):
        """Without `gh label create`, every flag decision fails its label edit."""
        step = self.step_by_run_needle("gh label create")
        self.assertIn(vb.UNREVIEWED_LABEL, step["run"])
        self.assertIn("--force", step["run"], "creation must be idempotent")
        order = [self.steps.index(step), self.steps.index(
            self.step_by_run_needle(LIVE_RUN_NEEDLE)
        )]
        self.assertLess(order[0], order[1], "reify the label before the sweep uses it")

    def test_the_checkout_materializes_every_script_the_run_imports(self):
        checkouts = [s for s in self.steps if "actions/checkout" in str(s.get("uses"))]
        self.assertEqual(len(checkouts), 1)
        sparse = checkouts[0]["with"]["sparse-checkout"]
        for needed in ("scripts/verdict-bridge.py", "scripts/gh_retry.py"):
            self.assertIn(needed, sparse, f"{needed} would be missing at runtime")
        self.assertEqual(
            checkouts[0]["with"]["ref"],
            "${{ github.event.repository.default_branch }}",
            "the policy must come from the trusted default branch, never a PR head",
        )

    def test_it_never_triggers_on_a_candidate_controlled_ref(self):
        """The forbidden set is exactly the triggers that resolve the WORKFLOW FILE from
        the PR's own ref (so a candidate branch could rewrite this policy and have it run
        with write permissions), plus `pull_request_target`, which additionally hands a
        privileged token to a run about untrusted head code.

        `issue_comment`, `pull_request_review` and `workflow_run` are NOT in this set:
        GitHub resolves all three from the DEFAULT BRANCH, and the checkout step below is
        separately pinned to the default branch, so no candidate code or candidate
        workflow definition can execute here.
        """
        on = triggers(self.doc)
        for forbidden in ("pull_request", "pull_request_target", "push", "merge_group"):
            self.assertNotIn(
                forbidden, on, f"{forbidden} resolves the workflow file from candidate ref"
            )
        self.assertIn("schedule", on)
        self.assertIn("workflow_dispatch", on)

    def test_the_cron_backstop_survives_the_event_conversion(self):
        """CONSTRAINT: the event trigger is ADDITIVE. Deleting the cron converts a
        latency bug into a lost-work bug — a webhook GitHub never delivers, or a run its
        concurrency group cancelled, would then never be reconciled at all."""
        schedule = triggers(self.doc).get("schedule")
        self.assertTrue(schedule, "the reconciliation cron must not be removed")
        self.assertEqual(
            sorted(cron_minutes(self.doc)),
            [1, 11, 21, 31, 41, 51],
            "the backstop cadence must not be silently widened",
        )

    def test_every_event_trigger_is_wired_to_a_state_change_the_policy_reads(self):
        on = triggers(self.doc)
        self.assertEqual(
            sorted(on["issue_comment"]["types"]),
            ["created", "edited"],
            "an EDITED comment can add or retract a verdict just as a new one can",
        )
        self.assertEqual(
            sorted(on["pull_request_review"]["types"]),
            ["dismissed", "edited", "submitted"],
        )
        self.assertEqual(on["workflow_run"]["workflows"], ["ci-summary"])
        self.assertEqual(on["workflow_run"]["types"], ["completed"])

    def test_the_gate_producing_workflow_named_in_workflow_run_actually_exists(self):
        """`workflows: [ci-summary]` matches by workflow NAME, not filename. A rename
        upstream would silently make the green/flag event path dead — and every other
        assertion in this class would still pass."""
        named = triggers(self.doc)["workflow_run"]["workflows"]
        ci_summary = load_workflow(REPO_ROOT / ".github" / "workflows" / "ci-summary.yml")
        self.assertIn(
            ci_summary["name"], named, "the referenced workflow name no longer exists"
        )
        gate_jobs = [
            job for job in ci_summary["jobs"].values()
            if str(job.get("name", "")).startswith("gate")
        ]
        self.assertTrue(
            gate_jobs, "ci-summary no longer publishes the `gate` check this path waits on"
        )

    def test_the_run_step_scopes_the_event_to_one_pr_and_sets_the_mode(self):
        """Without --pr, every issue comment in the repository would start the ~9-minute
        all-PR sweep (MEASURED 8m30s over 103 open PRs, run 30221614764) — strictly worse
        than the cron this replaces."""
        step = self.step_by_run_needle(LIVE_RUN_NEEDLE)
        self.assertIn('--pr "$BRIDGE_PR"', step["run"])
        self.assertIn('--mode "$BRIDGE_MODE"', step["run"])
        bridge_pr = " ".join(str(step["env"]["BRIDGE_PR"]).split())
        for source in (
            "github.event.issue.number",
            "github.event.pull_request.number",
            "github.event.workflow_run.pull_requests[0].number",
        ):
            self.assertIn(source, bridge_pr, f"{source} would never scope its event")
        self.assertTrue(bridge_pr.startswith("${{") and bridge_pr.endswith("}}"))

    def test_the_mode_expression_makes_exactly_the_cron_paths_fail_soft(self):
        """Inverting this makes a dropped EVENT silently disappear (the measured real
        cron cadence is 53-75 minutes, so 'the cron will get it' is not true), or makes
        every transient blip red a routine sweep."""
        step = self.step_by_run_needle(LIVE_RUN_NEEDLE)
        expression = " ".join(str(step["env"]["BRIDGE_MODE"]).split())
        inner = expression[len("${{"):-len("}}")]
        for event_name, expected in (
            ("schedule", "sweep"),
            ("workflow_dispatch", "sweep"),
            ("issue_comment", "event"),
            ("pull_request_review", "event"),
            ("workflow_run", "event"),
        ):
            with self.subTest(event=event_name):
                self.assertEqual(
                    _ExprParser(_tokenize(inner), payload(event_name)).parse(), expected
                )

    UNTRUSTED_PAYLOAD_PATHS = (
        "github.event.comment.body",
        "github.event.issue.title",
        "github.event.issue.body",
        "github.event.review.body",
        "github.event.pull_request.title",
        "github.event.pull_request.head.ref",
        "github.event.workflow_run.head_branch",
    )

    def test_no_untrusted_payload_text_is_interpolated_into_a_shell(self):
        """The trust rail. A comment body / title / branch name reaching a `run:`, a step
        `env:` or a step `with:` is the classic script-injection sink; only the integer PR
        number may cross into the execution environment.

        A job-level `if:` is deliberately NOT a sink — GitHub evaluates it itself and the
        value never reaches bash. The comment-body volume filter lives there, and only
        there; test_the_comment_body_is_read_ONLY_by_the_job_condition pins that.
        """
        sinks = json.dumps(
            [
                {k: step.get(k) for k in ("run", "env", "with")}
                for step in self.steps
            ]
            + [self.job.get("env"), self.doc.get("env"), self.doc.get("concurrency")]
        )
        for path in self.UNTRUSTED_PAYLOAD_PATHS:
            with self.subTest(sink=path):
                self.assertNotIn(path, sinks)

    def test_the_comment_body_is_read_ONLY_by_the_job_condition(self):
        blob = json.dumps(self.doc)
        occurrences = blob.count("github.event.comment.body")
        self.assertEqual(
            occurrences,
            str(self.job["if"]).count("github.event.comment.body"),
            "the comment body appears somewhere other than the job `if:`",
        )
        self.assertLessEqual(occurrences, 1)

    def test_the_comment_body_filter_is_a_SUPERSET_of_the_evidence_the_policy_reads(self):
        """The volume filter must not be able to drop a comment the policy would act on.

        Driven off the LIVE ``VERDICT_SHAPE_RE`` / ``trailing_verdict``: for any body,
        if the policy reads a verdict out of it, the workflow condition must admit it.
        """
        condition = str(self.job["if"])
        bodies = [
            f"{'a' * 40}\n\nVERDICT: pass",
            f"{'a' * 40}\n\nVERDICT: fail",
            f"{'a' * 40}\n\n**VERDICT: Fail**",
            f"{'a' * 40}\n\nverdict: unclear",
            "VERDICT: FAIL",
            "VERDICT - pass",
            "VERDICT:",
            "lgtm, merging",              # no verdict -> policy decides nothing
            "> VERDICT: pass",            # a MENTION -> policy decides nothing
            "",
        ]
        for body in bodies:
            with self.subTest(body=body[:32]):
                policy_acts = vb.trailing_verdict(body) is not None
                admitted = evaluate_if(
                    condition,
                    payload(
                        "issue_comment",
                        issue={"number": 4324, "pull_request": {"url": "u"}},
                        comment={"body": body},
                    ),
                )
                if policy_acts:
                    self.assertTrue(
                        admitted, "the filter dropped a comment the policy reads"
                    )

    def test_only_a_SUCCESSFUL_gate_can_change_a_decision_the_ci_event_uniquely_enables(self):
        """Justifies `workflow_run.conclusion == 'success'`.

        The gate conclusion reaches `decide()` only through `is_green_and_ready`, which
        guards `flag`. So over the population the CI event uniquely covers — a PR with NO
        head-bound verdict, where no comment or review event will fire — a non-success
        conclusion can only ever produce `none`. Dropping those events loses nothing, and
        the cron still reconciles.
        """
        reader = bridge(FakeGitHub([node(4200)]))
        for gate in ("failure", "cancelled", "timed_out", "action_required", None):
            with self.subTest(conclusion=gate):
                self.assertEqual(
                    vb.decide(reader.parse_node(node(4200), gate), []).action,
                    "none",
                    "a non-success gate produced an actionable decision after all",
                )
        self.assertEqual(
            vb.decide(reader.parse_node(node(4200), "success"), []).action, "flag"
        )

    def test_it_has_no_contents_write_authority(self):
        perms = self.doc["permissions"]
        self.assertEqual(perms.get("contents"), "read")
        self.assertEqual(perms.get("pull-requests"), "write")
        self.assertEqual(perms.get("issues"), "write")

    def test_the_cron_fires_before_each_auto_arm_sweep(self):
        """A wrong cron input makes every promotion wait a full extra arming cycle."""
        bridge_minutes = sorted(cron_minutes(self.doc))
        arm_minutes = sorted(cron_minutes(load_workflow(AUTO_ARM_WORKFLOW)))
        self.assertTrue(bridge_minutes, "the bridge must be scheduled at all")
        self.assertTrue(arm_minutes, "auto-arm's cron is the backstop this pairs with")
        self.assertEqual(
            set(bridge_minutes) & set(arm_minutes), set(), "must not collide with auto-arm"
        )
        for minute in bridge_minutes:
            following = [a for a in arm_minutes if a > minute]
            self.assertTrue(following, f"no auto-arm sweep follows minute {minute}")
            self.assertLessEqual(
                following[0] - minute, 10, f"minute {minute} waits too long to be armed"
            )

    def test_every_action_is_sha_pinned(self):
        for step in self.steps:
            uses = step.get("uses")
            if not uses:
                continue
            with self.subTest(uses=uses):
                ref = uses.split("@", 1)[1].split(" ")[0]
                self.assertEqual(len(ref), 40, f"{uses} is not SHA-pinned")
                int(ref, 16)


class TestJobConditionAdmission(unittest.TestCase):
    """EVALUATE the job `if:` against synthetic payloads — the admit/skip matrix.

    Every row here reds on a different single-line mutation of the condition.
    """

    @classmethod
    def setUpClass(cls):
        cls.condition = load_workflow(WORKFLOW)["jobs"]["bridge"]["if"]

    def admits(self, ctx) -> bool:
        return evaluate_if(self.condition, ctx)

    def test_the_scheduled_backstop_is_admitted_unconditionally(self):
        self.assertTrue(self.admits(payload("schedule")))
        self.assertTrue(self.admits(payload("workflow_dispatch")))

    def comment_event(self, body, *, is_pr=True):
        issue = {"number": 4324}
        if is_pr:
            issue["pull_request"] = {"url": "https://x"}
        return payload("issue_comment", issue=issue, comment={"body": body})

    def test_a_VERDICT_comment_on_a_PULL_REQUEST_is_admitted(self):
        self.assertTrue(self.admits(self.comment_event("reviewed\n\nVERDICT: pass")))

    def test_a_comment_on_a_PLAIN_ISSUE_is_refused(self):
        """Otherwise any commenter on any of the ~1300 open issues could start a
        full-repository sweep."""
        self.assertFalse(self.admits(self.comment_event("VERDICT: pass", is_pr=False)))

    def test_ordinary_PR_chatter_is_refused(self):
        """MEASURED 1286 comments on PRs in 24h. Admitting all of them would add >1300
        jobs/day into a repo with a known congestion-collapse mode. The filter is a
        provable superset of the evidence set — see
        TestWorkflowWiring::test_the_comment_body_filter_is_a_SUPERSET_of_...
        """
        for body in ("lgtm", "rebased onto main", "", "ping @jeswr"):
            with self.subTest(body=body):
                self.assertFalse(self.admits(self.comment_event(body)))

    def test_a_pull_request_review_is_admitted(self):
        self.assertTrue(
            self.admits(payload("pull_request_review", pull_request={"number": 4324}))
        )

    def ci_event(self, conclusion="success", pulls=({"number": 4324},)):
        return payload(
            "workflow_run",
            workflow_run={"conclusion": conclusion, "pull_requests": list(pulls)},
        )

    def test_a_SUCCESSFUL_ci_summary_run_WITH_an_associated_pr_is_admitted(self):
        self.assertTrue(self.admits(self.ci_event()))

    def test_a_ci_summary_run_with_NO_associated_pr_is_refused(self):
        """merge_group refs and fork heads report an empty pull_requests array. The cron
        reconciles them; an unscoped sweep per merge-queue batch would not."""
        for runs in ([], [{}]):
            with self.subTest(pull_requests=runs):
                self.assertFalse(self.admits(self.ci_event(pulls=runs)))

    def test_a_non_successful_ci_summary_run_is_refused(self):
        for conclusion in ("failure", "cancelled", "skipped", "timed_out", None):
            with self.subTest(conclusion=conclusion):
                self.assertFalse(self.admits(self.ci_event(conclusion=conclusion)))

    def test_an_unknown_event_is_refused(self):
        """Adding a trigger without extending this condition must not fall through to an
        unscoped sweep."""
        for event_name in ("pull_request", "push", "issues", "check_suite"):
            with self.subTest(event=event_name):
                self.assertFalse(self.admits(payload(event_name)))


class TestConcurrencyGrouping(unittest.TestCase):
    """The group EXPRESSION, evaluated — not grepped."""

    @classmethod
    def setUpClass(cls):
        cls.doc = load_workflow(WORKFLOW)
        raw = " ".join(str(cls.doc["concurrency"]["group"]).split())
        if "${{" in raw:
            cls.prefix, _, rest = raw.partition("${{")
            cls.inner = rest[: rest.rindex("}}")]
        else:
            # A CONSTANT group. Handled rather than crashed on: a setUpClass exception
            # is an error-kill that names no guard, and "the harness blew up" reads the
            # same as "the property is checked" in a mutation report. Let the matrix
            # tests below report the actual defect instead.
            cls.prefix, cls.inner = raw, "''"

    def group_for(self, ctx) -> str:
        return self.prefix + str(_ExprParser(_tokenize(self.inner), ctx).parse())

    def test_events_about_DIFFERENT_prs_never_share_a_group(self):
        """A single shared group would serialise every PR's event behind every other's —
        reintroducing exactly the queueing the conversion removes."""
        a = self.group_for(payload("issue_comment", issue={"number": 4324}))
        b = self.group_for(payload("issue_comment", issue={"number": 4325}))
        self.assertNotEqual(a, b)

    def test_events_about_the_SAME_pr_share_a_group_across_channels(self):
        comment = self.group_for(payload("issue_comment", issue={"number": 4324}))
        review = self.group_for(
            payload("pull_request_review", pull_request={"number": 4324})
        )
        ci = self.group_for(
            payload("workflow_run", workflow_run={"pull_requests": [{"number": 4324}]})
        )
        self.assertEqual({comment, review, ci}, {comment})

    def test_schedule_and_dispatch_share_the_single_sweep_group(self):
        self.assertEqual(
            self.group_for(payload("schedule")),
            self.group_for(payload("workflow_dispatch")),
            "two concurrent whole-repo sweeps would double every write decision",
        )

    def test_the_sweep_group_is_DISJOINT_from_every_per_pr_group(self):
        """Documented honestly rather than wished away: concurrency CANNOT prevent an
        event run and the sweep from racing on one PR. That is why the write path
        re-reads (reconfirm) — see TestDoubleFireIdempotence."""
        self.assertNotEqual(
            self.group_for(payload("schedule")),
            self.group_for(payload("issue_comment", issue={"number": 4324})),
        )

    def test_no_group_cancels_in_progress(self):
        self.assertFalse(self.doc["concurrency"]["cancel-in-progress"])

if __name__ == "__main__":
    unittest.main(verbosity=2)
