#!/usr/bin/env python3
# [OPUS-5] Tests for the review-lane blind-spot alarm (scripts/review_lane_alarm.py +
# .github/workflows/review-alarm.yml). 🤖 SPARQ agent.
#
# TWO HALVES, and the second is the load-bearing one.
#
# 1. BEHAVIOUR — the four obligations a verdict detector must meet, each as a NAMED test
#    that goes red on deletion OR inversion of the guard it covers:
#      * a PR with no verdict never reads as reviewed (so it can never be armed),
#      * a verdict bound to a SUPERSEDED head does not count,
#      * an author cannot verdict their own PR,
#      * the review brief's own quoted instruction text does not register as a pass.
#
# 2. THE YAML SEAM — measured in this repo, every uncaught mutant of an 18-mutant run
#    lived in a workflow `if:` / step / call-site, not in the Python. YAML `if:`/`on:`/
#    `permissions:` cannot be unit-tested by execution, so this suite pins their SHAPE:
#      * the workflow EXISTS and is scheduled (delete the trigger => red),
#      * the self-test step runs BEFORE the live step (reorder => red),
#      * the live step actually invokes the script (break the call site => red),
#      * the job holds `issues: write` and NOT `pull-requests: write` / `contents: write`
#        (an arming-authority escalation => red),
#      * docs-quality.yml wires THIS test file (drop the call site => red).
#
# The last one is the anti-vacuity anchor: without it, this whole file could be deleted
# from CI's reachable set and nothing would notice.
#
# Stdlib + PyYAML (already a docs-quality dependency). Run:
#   python3 scripts/tests/test_review_lane_alarm.py

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "review_lane_alarm.py"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "review-alarm.yml"
DOCS_QUALITY = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"

# The alarm imports `review_lane_reach` — the reachability predicate it now SHARES with
# the sparq-side labeller (sparq#4677) — as a sibling module. Under
# `python3 scripts/review_lane_alarm.py` that resolves through sys.path[0]; loading the
# module by path from here does not, so put `scripts/` on the path explicitly.
sys.path.insert(0, str(SCRIPT.parent))

_spec = importlib.util.spec_from_file_location("review_lane_alarm", SCRIPT)
alarm = importlib.util.module_from_spec(_spec)
assert _spec.loader is not None
_spec.loader.exec_module(alarm)
reach = sys.modules["review_lane_reach"]

SHA_A = "a" * 40
SHA_B = "b" * 40
REPO = "sparq-org/sparq"

# Verbatim from the standing review brief. Substring-grepping for `VERDICT: pass` has
# already scored this quotation as a real pass in this repository.
BRIEF_INSTRUCTION = (
    "Post ONE comment per PR, opening with the `> 🤖 SPARQ agent` blockquote, ending "
    "with a line-anchored final line, exactly `VERDICT: pass` or `VERDICT: fail` "
    "(nothing after it).\nState the head SHA you reviewed.\n"
)


def pr(number=1, *, ref="research/x", login="jeswr", sha=SHA_A, draft=False, labels=(),
       created="2026-07-01T00:00:00Z", repo=REPO):
    return {
        "number": number,
        "draft": draft,
        "created_at": created,
        "title": f"pr {number}",
        "user": {"login": login},
        "labels": [{"name": name} for name in labels],
        "head": {"ref": ref, "sha": sha, "repo": {"full_name": repo}},
    }


def comment(body, login="reviewer-bot", created="2026-07-02T00:00:00Z"):
    return {"body": body, "user": {"login": login}, "created_at": created}


# The account that applied 41/41 of the live "human-owned" holds sparq#4911 measured.
BOT = "sparq-orchestrator[bot]"


def labeled(label="needs:user", login="jeswr", created="2026-07-20T00:00:00Z", kind=None):
    """A `labeled` issue event, in the shape `GET /repos/{r}/issues/{n}/events` returns."""
    actor = {"login": login}
    if kind:
        actor["type"] = kind
    return {"event": "labeled", "label": {"name": label}, "actor": actor, "created_at": created}


class TestNoVerdictIsNotArmable(unittest.TestCase):
    """OBLIGATION 1: a PR with no verdict must never read as reviewed."""

    def test_no_comments_at_all_is_a_blind_spot(self):
        self.assertEqual(alarm.classify_pr(pr(), [], REPO), "blind-spot")

    def test_chatter_without_a_verdict_line_is_a_blind_spot(self):
        chatter = [comment(f"looks good to me, head is {SHA_A}"), comment("bumping")]
        self.assertEqual(alarm.classify_pr(pr(), chatter, REPO), "blind-spot")

    def test_no_verdict_never_yields_a_polarity(self):
        self.assertIsNone(alarm.countable_verdict(pr(), [])[0])

    def test_an_unreviewed_pr_past_the_threshold_alarms(self):
        now = alarm._parse_iso("2026-07-26T00:00:00Z")
        findings, _ = alarm.find_blind_spot(
            [pr(number=42, created="2026-07-01T00:00:00Z")], {}, REPO, now, 24)
        self.assertEqual([f["number"] for f in findings], [42])


class TestSupersededHeadDoesNotCount(unittest.TestCase):
    """OBLIGATION 2: a force-push must invalidate a verdict."""

    def test_verdict_naming_the_previous_head_does_not_count(self):
        stale = [comment(f"reviewed {SHA_B}\n\nVERDICT: pass")]
        self.assertIsNone(alarm.countable_verdict(pr(sha=SHA_A), stale)[0])

    def test_superseded_verdict_still_leaves_the_pr_in_the_blind_spot(self):
        stale = [comment(f"reviewed {SHA_B}\n\nVERDICT: pass")]
        self.assertEqual(alarm.classify_pr(pr(sha=SHA_A), stale, REPO), "blind-spot")

    def test_the_superseded_reason_is_reported_not_dropped(self):
        stale = [comment(f"reviewed {SHA_B}\n\nVERDICT: pass")]
        _, why = alarm.countable_verdict(pr(sha=SHA_A), stale)
        self.assertTrue(any("superseded" in reason for reason in why), why)

    def test_a_truncated_sha_does_not_bind(self):
        self.assertFalse(alarm.comment_binds_head(f"reviewed {SHA_A[:12]}", SHA_A))

    def test_a_longer_hex_run_does_not_bind(self):
        # A 41-hex token must not satisfy a 40-hex binding by prefix.
        self.assertFalse(alarm.comment_binds_head(SHA_A + "c", SHA_A))

    def test_the_same_verdict_counts_once_the_comment_names_the_new_head(self):
        fresh = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass")]
        self.assertEqual(alarm.countable_verdict(pr(sha=SHA_A), fresh)[0], "pass")


class TestAuthorCannotVerdictOwnPr(unittest.TestCase):
    """OBLIGATION 3: author XOR reviewer, structurally.

    This is not hypothetical here. On 2026-07-26 the live repository carried three open
    PRs (#3765, #3435, #2278) with a line-anchored, HEAD-BOUND `VERDICT:` comment posted
    by the PR's own author at author_association MEMBER — the exact shape the arming
    bridge admits. They were all `fail`, so nothing self-armed; a `pass` in the same shape
    would have."""

    def test_self_posted_pass_does_not_count(self):
        selfrev = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass", login="jeswr")]
        self.assertIsNone(alarm.countable_verdict(pr(login="jeswr"), selfrev)[0])

    def test_self_posted_pass_leaves_the_pr_in_the_blind_spot(self):
        selfrev = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass", login="jeswr")]
        self.assertEqual(alarm.classify_pr(pr(login="jeswr"), selfrev, REPO), "blind-spot")

    def test_the_self_review_is_named_in_the_report(self):
        selfrev = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass", login="jeswr")]
        _, why = alarm.countable_verdict(pr(login="jeswr"), selfrev)
        self.assertTrue(any("self-review" in reason for reason in why), why)

    def test_self_review_is_reported_as_self_not_merely_stale(self):
        # Checked BEFORE the head test on purpose: a self-review's problem is not its age,
        # and a human told "stale" would re-review the head and re-arm the same hole.
        selfrev = [comment(f"reviewed {SHA_B}\n\nVERDICT: pass", login="jeswr")]
        _, why = alarm.countable_verdict(pr(login="jeswr", sha=SHA_A), selfrev)
        self.assertTrue(any("self-review" in reason for reason in why), why)

    def test_a_different_account_on_the_same_head_does_count(self):
        # The guard must not be so broad that no verdict can ever pass.
        other = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass", login="someone-else")]
        self.assertEqual(alarm.countable_verdict(pr(login="jeswr"), other)[0], "pass")

    def test_a_self_pass_cannot_override_a_cross_author_fail(self):
        comments = [
            comment(f"{SHA_A}\n\nVERDICT: fail", login="r1", created="2026-07-02T00:00:00Z"),
            comment(f"{SHA_A}\n\nVERDICT: pass", login="jeswr", created="2026-07-03T00:00:00Z"),
        ]
        self.assertEqual(alarm.countable_verdict(pr(login="jeswr"), comments)[0], "fail")


class TestInstructionTextIsNotAPass(unittest.TestCase):
    """OBLIGATION 4: never substring-grep. The brief quotes the literal at every
    reviewer, and a substring match on that quotation has produced a false pass here."""

    def test_the_brief_instruction_is_not_a_verdict(self):
        self.assertIsNone(alarm.verdict_polarity(BRIEF_INSTRUCTION))

    def test_a_comment_quoting_the_brief_stays_in_the_blind_spot(self):
        quoted = [comment(f"{SHA_A}\n{BRIEF_INSTRUCTION}")]
        self.assertEqual(alarm.classify_pr(pr(), quoted, REPO), "blind-spot")

    def test_backticked_literal_is_not_a_verdict(self):
        self.assertIsNone(alarm.verdict_polarity("`VERDICT: pass`"))

    def test_verdict_line_followed_by_prose_is_not_a_verdict(self):
        self.assertIsNone(
            alarm.verdict_polarity("VERDICT: pass\n\nActually, one more thought:")
        )

    def test_suffixed_verdict_line_is_not_a_verdict(self):
        self.assertIsNone(alarm.verdict_polarity("VERDICT: pass (modulo the nit above)"))

    def test_wrong_case_is_not_a_verdict(self):
        self.assertIsNone(alarm.verdict_polarity("VERDICT: PASS"))

    def test_a_genuine_trailing_verdict_line_does_parse(self):
        self.assertEqual(alarm.verdict_polarity(f"{SHA_A}\n\nVERDICT: pass\n\n"), "pass")

    # FOUND BY MUTATION: dropping the `^` anchor from VERDICT_LINE_RE (leaving the `$`)
    # survived the whole suite. Every existing negative case put text AFTER the literal,
    # so nothing covered text BEFORE it. That gap matters more here than almost anywhere:
    # every comment in this repo opens with the `> 🤖 SPARQ agent` blockquote convention,
    # so QUOTING another agent's verdict — `> VERDICT: pass` — is the natural shape of a
    # false positive, and the unanchored regex scores it as a real pass.
    def test_a_BLOCKQUOTED_verdict_is_not_a_verdict(self):
        self.assertIsNone(alarm.verdict_polarity("> VERDICT: pass"))

    def test_a_verdict_with_ANY_prefix_text_is_not_a_verdict(self):
        for line in ("Final answer: VERDICT: pass", "- VERDICT: pass", "1. VERDICT: fail",
                     "not a VERDICT: pass", "* VERDICT: pass"):
            with self.subTest(line=line):
                self.assertIsNone(alarm.verdict_polarity(line))

    def test_quoting_someone_elses_verdict_leaves_the_pr_unreviewed(self):
        quoted = [comment(f"Copying the earlier review of {SHA_A}:\n\n> VERDICT: pass")]
        self.assertEqual(alarm.classify_pr(pr(), quoted, REPO), "blind-spot")

    def test_leading_whitespace_is_still_tolerated(self):
        # The guard must reject PREFIX TEXT, not indentation — the line is stripped first.
        self.assertEqual(alarm.verdict_polarity("   VERDICT: pass"), "pass")


class TestRegistryLaneReachability(unittest.TestCase):
    """The reachability predicate is the claim this whole alarm rests on: it must be the
    registry's own admission test, pinned to REAL branch names measured on 2026-07-26."""

    def test_real_worker_branch_is_reachable(self):
        self.assertTrue(alarm.registry_lane_reachable(
            pr(ref="sparq-agent/issue-2908-30221671021-1",
               login="sparq-orchestrator[bot]"), REPO))

    def test_real_local_agent_branches_are_unreachable(self):
        for ref in ("ci/pr-freshness-auto-update-branch", "research/agent-context-sharing",
                    "fix/readiness-visibility-opus5", "feat/start-here-renderer"):
            with self.subTest(ref=ref):
                self.assertFalse(alarm.registry_lane_reachable(pr(ref=ref, login="jeswr"), REPO))

    def test_non_bot_author_on_a_worker_branch_is_unreachable(self):
        self.assertFalse(
            alarm.registry_lane_reachable(pr(ref="sparq-agent/issue-1-2-1", login="jeswr"), REPO))

    def test_fork_head_is_unreachable(self):
        self.assertFalse(alarm.registry_lane_reachable(
            pr(ref="sparq-agent/issue-1-2-1", login="sparq-orchestrator[bot]",
               repo="attacker/sparq"), REPO))

    def test_the_regex_is_byte_identical_to_the_registry_gate(self):
        # If the registry ever loosens its head-ref gate, this pin must be updated in the
        # same breath — a paraphrased predicate would silently mis-scope the alarm.
        self.assertEqual(alarm.REGISTRY_HEAD_REF_RE.pattern,
                         r"^sparq-agent/issue-([1-9][0-9]*)-")

    def test_the_predicate_is_the_SAME_OBJECT_the_labeller_uses(self):
        """sparq#4677. The bug was never one predicate being wrong — it was two
        components in different repositories holding DIFFERENT ones, so a PR could be
        labelled `review:unreviewed` here and be invisible to the reviewer there. A
        second sparq-side copy would re-commit exactly that, so pin identity (same module
        object), not equality of source."""
        self.assertIs(alarm.registry_lane_reachable.__globals__["review_lane_reach"], reach)
        self.assertIs(alarm.REGISTRY_HEAD_REF_RE, reach.REGISTRY_HEAD_REF_RE)


class TestLabelledButUnreviewable(unittest.TestCase):
    """sparq#4677: the DRIFT ROW between the sparq-side labeller and the review lane.

    `review:unreviewed` is written by scripts/verdict-bridge.py in THIS repository and
    means "awaiting review"; the only reviewer lives in `jeswr/agent-account-registry`
    and never sees the label at all. When the two visibility predicates disagree the
    labelled-but-unreviewable class grows silently — it reached four PRs, including the
    first crates.io release PR (~22h unreviewed), and was found by accident. The census
    row makes that population countable on every tick.
    """

    NOW = "2026-07-26T00:00:00Z"
    AGED = "2026-07-18T00:00:00Z"

    # The four PRs the issue measured, by their real head refs and authors.
    MEASURED = (
        (3798, "ci/auto-arm-workflows-permission", "jeswr", reach.CLASS_NOT_ENROLLED),
        (4193, "research/knowledge-management-strategy", "jeswr", reach.CLASS_NOT_ENROLLED),
        (4460, "release-plz-2026-07-27T02-19-35Z", BOT, reach.CLASS_RELEASE),
        (4488, "dependabot/github_actions/actions-minor", "dependabot[bot]",
         reach.CLASS_DEPENDABOT),
    )

    def census(self, prs):
        return alarm.find_blind_spot(prs, {}, REPO, alarm._parse_iso(self.NOW), 24)

    def test_the_row_is_emitted_at_ZERO_on_a_clean_repository(self):
        """A counter that materialises only when it fires cannot report agreement: its
        absence would be indistinguishable from the check having been deleted."""
        _, census = self.census([pr(number=1, ref="sparq-agent/issue-5-1-1", login=BOT)])
        self.assertEqual(census[alarm.LABELLED_UNREVIEWABLE], 0)

    def test_every_measured_pr_is_counted(self):
        prs = [
            pr(number=n, ref=ref, login=login,
               labels=(alarm.UNREVIEWED_LABEL,), created=self.AGED)
            for n, ref, login, _ in self.MEASURED
        ]
        _, census = self.census(prs)
        self.assertEqual(census[alarm.LABELLED_UNREVIEWABLE], len(self.MEASURED))

    def test_a_worker_pr_carrying_the_flag_is_NOT_drift(self):
        """Non-vacuity: the row must count DISAGREEMENT, not merely the label."""
        _, census = self.census([
            pr(number=1, ref="sparq-agent/issue-5-1-1", login=BOT,
               labels=(alarm.UNREVIEWED_LABEL,), created=self.AGED)
        ])
        self.assertEqual(census[alarm.LABELLED_UNREVIEWABLE], 0)

    def test_the_flag_never_buys_the_lane_labelled_exit(self):
        """`review:unreviewed` enrols a PR in nothing, so it must not silence the alarm
        the way a real lane label does — that would hide the very population #4677 is
        about behind the label that created it."""
        self.assertNotIn(alarm.UNREVIEWED_LABEL, alarm.LANE_LABELS)
        self.assertEqual(
            alarm.classify_pr(pr(labels=(alarm.UNREVIEWED_LABEL,)), [], REPO),
            "blind-spot",
        )

    def test_a_flagged_DRAFT_is_still_counted_as_drift(self):
        """The label is wrong on a draft too — drafts just do not additionally alarm."""
        _, census = self.census([
            pr(number=1, ref="research/x", draft=True, labels=(alarm.UNREVIEWED_LABEL,))
        ])
        self.assertEqual(census[alarm.LABELLED_UNREVIEWABLE], 1)
        self.assertEqual(census.get("draft"), 1)

    def test_each_unreachable_class_carries_an_explicit_disposition(self):
        """#4677 asked for release and dependabot PRs to be DECIDED rather than left to
        silent accumulation: they cannot be enrolled (reviewer selection inverts
        `impl_provider` and neither class has an implementing model), so the row must say
        what happens to them instead."""
        prs = [
            pr(number=n, ref=ref, login=login, created=self.AGED)
            for n, ref, login, _ in self.MEASURED
        ]
        findings, _ = self.census(prs)
        by_number = {f["number"]: f for f in findings}
        self.assertEqual(len(by_number), len(self.MEASURED))
        for number, _, _, expected in self.MEASURED:
            with self.subTest(pr=number):
                self.assertEqual(by_number[number]["class"], expected)
                self.assertTrue(by_number[number]["disposition"].strip())
        for number in (4460, 4488):
            self.assertIn("MAINTAINER-REVIEWED", by_number[number]["disposition"])

    def test_the_issue_body_reports_the_row_and_the_dispositions(self):
        prs = [
            pr(number=n, ref=ref, login=login,
               labels=(alarm.UNREVIEWED_LABEL,), created=self.AGED)
            for n, ref, login, _ in self.MEASURED
        ]
        findings, census = self.census(prs)
        body = alarm.render_issue_body(findings, census, REPO, 24)
        self.assertIn(alarm.LABELLED_UNREVIEWABLE, body)
        self.assertIn(f"{alarm.LABELLED_UNREVIEWABLE}: 4", body)
        self.assertEqual(body.count("MAINTAINER-REVIEWED"), 2, body)
        for _, _, _, klass in self.MEASURED:
            self.assertIn(klass, body)


class TestNonAlarmingStates(unittest.TestCase):
    """The alarm must not be so eager that a human hold or a real review re-fires it."""

    def test_draft_is_not_alarmed(self):
        self.assertEqual(alarm.classify_pr(pr(draft=True), [], REPO), "draft")

    def test_human_holds_are_terminal(self):
        for label in ("needs:user", "review:needs-user"):
            with self.subTest(label=label):
                self.assertEqual(alarm.classify_pr(pr(labels=(label,)), [], REPO), "human-held")

    def test_lane_labelled_prs_are_not_double_reported(self):
        self.assertEqual(alarm.classify_pr(pr(labels=("review:pass",)), [], REPO),
                         "lane-labelled")

    def test_a_genuinely_reviewed_pr_raises_no_finding(self):
        now = alarm._parse_iso("2026-07-26T00:00:00Z")
        good = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass")]
        findings, _ = alarm.find_blind_spot(
            [pr(number=7, created="2026-07-01T00:00:00Z")], {7: good}, REPO, now, 24)
        self.assertEqual(findings, [])

    def test_a_fresh_blind_spot_pr_is_counted_but_not_alarmed(self):
        now = alarm._parse_iso("2026-07-26T00:00:00Z")
        findings, census = alarm.find_blind_spot(
            [pr(number=9, created="2026-07-25T23:00:00Z")], {}, REPO, now, 24)
        self.assertEqual(findings, [])
        self.assertEqual(census.get("blind-spot-fresh"), 1)

    def test_the_census_counts_every_state_exit(self):
        now = alarm._parse_iso("2026-07-26T00:00:00Z")
        _, census = alarm.find_blind_spot(
            [
                pr(number=1, created="2026-07-01T00:00:00Z"),
                pr(number=2, draft=True),
                pr(number=3, labels=("needs:user",)),
                pr(number=4, ref="sparq-agent/issue-5-1-1", login="sparq-orchestrator[bot]"),
            ],
            {}, REPO, now, 24)
        self.assertEqual(
            census,
            # `hold-owner-unknown` rides alongside the state exits: #3's hold is exempt,
            # but with no event log supplied nothing PROVED a human applied it, and an
            # unproved exemption has to be countable or it is invisible (sparq#4911).
            #
            # `labelled-unreviewable` is present at ZERO, and that is the point of the row
            # (sparq#4677): a counter that only materialises when it fires cannot say "the
            # labeller and the reviewer still agree" — its absence would be
            # indistinguishable from the check having been deleted.
            {"blind-spot": 1, "draft": 1, "human-held": 1, "registry-lane": 1,
             "hold-owner-unknown": 1, alarm.LABELLED_UNREVIEWABLE: 0},
        )


class TestAMachineWrittenHoldBuysNoExemption(unittest.TestCase):
    """sparq#4911 — the human-hold exit was keyed on the label's NAME, and the name does
    not carry the fact. Census over the open sparq PRs: 41/41 live "human-owned" holds
    were machine-applied (23/23 `needs:user` and 18/18 `review:needs-user` by
    `sparq-orchestrator[bot]`), and a further population of machine PARKS was written
    under the maintainer's own alias. So this alarm's QUIETEST exit — "a human is
    deciding, stand down" — was being taken on machine writes: the loop escalates to a
    human, signs the escalation as one, and the detector reads a decision that never
    happened.

    Ownership is now resolved from the `labeled` timeline event. Each test below goes red
    on deletion OR inversion of one clause of `hold_ownership`."""

    def setUp(self):
        self.held = pr(number=4804, labels=("needs:user",), created="2026-07-01T00:00:00Z")
        self.now = alarm._parse_iso("2026-07-26T00:00:00Z")

    def test_a_bot_applied_hold_does_not_exempt_the_pr(self):
        self.assertEqual(alarm.hold_ownership(self.held, [labeled(login=BOT)], []), "machine")
        self.assertEqual(
            alarm.classify_pr(self.held, [], REPO, [labeled(login=BOT)]), "blind-spot")

    def test_a_real_human_hold_is_still_terminal(self):
        self.assertEqual(alarm.hold_ownership(self.held, [labeled(login="jeswr")], []), "human")
        self.assertEqual(
            alarm.classify_pr(self.held, [], REPO, [labeled(login="jeswr")]), "human-held")

    def test_a_type_bot_actor_without_the_suffix_is_machine(self):
        self.assertEqual(
            alarm.hold_ownership(self.held, [labeled(login="orchestrator", kind="Bot")], []),
            "machine")

    def test_an_alias_written_hold_with_a_bot_receipt_is_machine_owned(self):
        """PR #3620's real shape: `jeswr` writes the label at 08:54:39 and the
        orchestrator posts its `class=capacity cause=partition` receipt 86s later —
        mixed credentials inside ONE logical operation. The actor test alone reads that
        as a human decision; the receipt is what the loop cannot fake away."""
        alias = [labeled(login="jeswr", created="2026-07-28T08:54:39Z")]
        receipt = [comment("park: class=capacity cause=partition", login=BOT,
                           created="2026-07-28T08:56:05Z")]
        self.assertEqual(alarm.hold_ownership(self.held, alias, receipt), "machine")
        self.assertEqual(alarm.classify_pr(self.held, receipt, REPO, alias), "blind-spot")

    def test_the_receipt_window_is_a_window(self):
        alias = [labeled(login="jeswr", created="2026-07-28T08:54:39Z")]
        late = [comment("park: class=capacity", login=BOT, created="2026-07-29T08:56:05Z")]
        self.assertEqual(alarm.hold_ownership(self.held, alias, late), "human")

    def test_a_human_comment_quoting_a_class_marker_proves_nothing(self):
        """The receipt clause trusts the loop's INTENT, and only the loop emits receipts.
        A human pasting `class=capacity` into the thread must not machine-own their own
        hold — that would invert the fix into a way to silence a real human decision."""
        alias = [labeled(login="jeswr", created="2026-07-28T08:54:39Z")]
        quoted = [comment("why is this class=capacity?", login="jeswr",
                          created="2026-07-28T08:55:00Z")]
        self.assertEqual(alarm.hold_ownership(self.held, alias, quoted), "human")

    def test_the_latest_application_decides(self):
        """Mirrors the park policy's own rule — the LATEST application decides — so a
        machine re-park over an older human hold is machine-owned."""
        self.assertEqual(
            alarm.hold_ownership(
                self.held,
                [labeled(login="jeswr", created="2026-07-20T00:00:00Z"),
                 labeled(login=BOT, created="2026-07-21T00:00:00Z")],
                []),
            "machine")

    def test_an_event_for_a_label_the_pr_no_longer_carries_owns_nothing(self):
        self.assertIsNone(
            alarm.latest_hold_application(self.held, [labeled(label="review:parked", login=BOT)]))

    def test_an_unresolvable_hold_stays_exempt_and_is_counted(self):
        """The under-reporting direction, on purpose — this file never alarms on a guess.
        But the exemption is counted, so a population of unproved holds is visible."""
        self.assertEqual(alarm.hold_ownership(self.held, [], []), "unknown")
        _, census = alarm.find_blind_spot([self.held], {}, REPO, self.now, 24, {})
        self.assertEqual(census.get("human-held"), 1)
        self.assertEqual(census.get("hold-owner-unknown"), 1)

    def test_the_machine_held_pr_reaches_the_findings_with_its_reason(self):
        findings, census = alarm.find_blind_spot(
            [self.held], {}, REPO, self.now, 24, {4804: [labeled(login=BOT)]})
        self.assertEqual([f["number"] for f in findings], [4804])
        self.assertEqual(census.get("hold-owner-machine"), 1)
        note = alarm.finding_note(findings[0])
        self.assertIn("MACHINE write", note)
        self.assertIn(BOT, note)
        self.assertIn(note, alarm.render_issue_body(findings, census, REPO, 24))

    def test_the_live_run_reads_the_event_log_for_held_prs(self):
        """The predicate is only worth anything if `run()` actually FETCHES the events.
        Without this, every live hold resolves to `unknown` and the fix is a no-op with a
        fully green suite — the shape of the measured no-op sparq#4819 shipped."""
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("fetch_label_events(repo, number)", source)
        self.assertIn("events_by_pr", source)


class TestExitCodeCarriesTheVerdict(unittest.TestCase):
    """FOUND BY MUTATION: rewriting `return 1` to `return 0` in run() survived the whole
    suite. Nothing asserted that a detected blind spot actually REDS the run — the alarm
    would have printed its findings into a green log forever, which is the exact
    exit-zero-swallowing class this repository has been bitten by three times in a day.

    These tests pin the exit code as the alarm's real output. `--dry-run` keeps them
    hermetic (no gh writes); `--prs-file` supplies the population."""

    def _run(self, pulls, comments=None, *, max_age="24"):
        import json
        import tempfile
        payload = {"pulls": pulls, "comments": comments or {}}
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
            json.dump(payload, fh)
            path = fh.name
        return alarm.main([
            "--repo", REPO, "--prs-file", path, "--now", "2026-07-26T00:00:00Z",
            "--max-age-hours", max_age, "--dry-run",
        ])

    def test_an_aged_blind_spot_pr_REDS_the_run(self):
        self.assertEqual(self._run([pr(number=42, created="2026-07-01T00:00:00Z")]), 1)

    def test_a_clean_population_exits_zero(self):
        self.assertEqual(
            self._run([pr(number=5, ref="sparq-agent/issue-5-1-1",
                          login="sparq-orchestrator[bot]")]),
            0,
        )

    def test_a_reviewed_pr_exits_zero(self):
        good = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass")]
        self.assertEqual(
            self._run([pr(number=7, created="2026-07-01T00:00:00Z")], {"7": good}), 0)

    def test_a_SELF_reviewed_pr_still_REDS_the_run(self):
        selfrev = [comment(f"reviewed {SHA_A}\n\nVERDICT: pass", login="jeswr")]
        self.assertEqual(
            self._run([pr(number=8, created="2026-07-01T00:00:00Z")], {"8": selfrev}), 1)

    def test_a_STALE_headed_verdict_still_REDS_the_run(self):
        stale = [comment(f"reviewed {SHA_B}\n\nVERDICT: pass")]
        self.assertEqual(
            self._run([pr(number=9, created="2026-07-01T00:00:00Z")], {"9": stale}), 1)

    def test_an_empty_population_exits_zero(self):
        self.assertEqual(self._run([]), 0)

    def test_the_three_exit_codes_are_distinct(self):
        # 0 = clean, 1 = blind spot found, 2 = the detector itself is broken. Collapsing
        # any pair would make a broken detector indistinguishable from a healthy repo.
        clean = self._run([])
        alarmed = self._run([pr(number=1, created="2026-07-01T00:00:00Z")])
        broken = alarm.main(["--repo", "not-a-slug", "--dry-run"])
        self.assertEqual((clean, alarmed, broken), (0, 1, 2))


class TestFailLoud(unittest.TestCase):
    """A broken detector must never look like a quiet green."""

    def test_malformed_pull_entry_raises(self):
        with self.assertRaises(alarm.AlarmError):
            alarm.registry_lane_reachable("not-a-dict", REPO)

    def test_invalid_pr_number_raises(self):
        now = alarm._parse_iso("2026-07-26T00:00:00Z")
        with self.assertRaises(alarm.AlarmError):
            alarm.find_blind_spot([{"number": 0}], {}, REPO, now, 24)

    def test_unparseable_timestamp_raises(self):
        now = alarm._parse_iso("2026-07-26T00:00:00Z")
        with self.assertRaises(alarm.AlarmError):
            alarm.find_blind_spot([pr(created="whenever")], {}, REPO, now, 24)

    def test_bad_repo_slug_raises(self):
        import argparse
        args = argparse.Namespace(repo="not-a-slug", now="", prs_file="", max_age_hours=24,
                                  dry_run=True, self_test=False)
        with self.assertRaises(alarm.AlarmError):
            alarm.run(args)

    def test_main_maps_infrastructure_failure_to_exit_two(self):
        self.assertEqual(alarm.main(["--repo", "not-a-slug", "--dry-run"]), 2)

    def test_the_embedded_self_test_passes(self):
        self.assertEqual(alarm.main(["--self-test"]), 0)


# --------------------------------------------------------------------------- #
# THE YAML SEAM
# --------------------------------------------------------------------------- #
def _load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _on_block(wf: dict) -> dict:
    """Bare `on` is the YAML boolean True, so PyYAML keys it under `True`."""
    return wf.get("on", wf.get(True, {})) or {}


class TestWorkflowWiring(unittest.TestCase):
    """Mutate the workflow and one of these must go red."""

    @classmethod
    def setUpClass(cls):
        cls.wf = _load(WORKFLOW)
        cls.job = cls.wf["jobs"]["alarm"]
        cls.steps = cls.job["steps"]

    def test_the_workflow_exists(self):
        self.assertTrue(WORKFLOW.is_file(), WORKFLOW)

    def test_it_is_scheduled(self):
        # Delete the trigger => the detector never runs => red here.
        schedule = _on_block(self.wf).get("schedule")
        self.assertTrue(schedule, "review-alarm must keep a schedule trigger")
        self.assertTrue(all("cron" in entry for entry in schedule))

    def test_the_cron_is_explicit_not_shorthand(self):
        for entry in _on_block(self.wf)["schedule"]:
            self.assertNotIn("*/", entry["cron"], "explicit minutes keep the cadence auditable")

    def test_it_is_manually_dispatchable(self):
        self.assertIn("workflow_dispatch", _on_block(self.wf))

    def test_it_never_triggers_on_a_pr_or_merge_group(self):
        # A check-run on a PR head would drag this non-gate into the ci-summary poll set.
        for trigger in ("pull_request", "pull_request_target", "merge_group", "push"):
            self.assertNotIn(trigger, _on_block(self.wf), trigger)

    def test_the_job_has_no_if_guard(self):
        # Invert-the-`if:` is the classic vacuity mutant; the job simply has none to invert.
        self.assertNotIn("if", self.job)

    def _step_indices(self):
        """(self-test index, live index) or None for either when absent. Deliberately NOT
        `next(...)`: a bare next() raises StopIteration on a broken call site, which is a
        CRASH-kill — it reds, but it reports a traceback instead of naming the defect. A
        mutation run caught exactly that here."""
        commands = [str(step.get("run", "")) for step in self.steps]
        selftest = next(
            (i for i, c in enumerate(commands) if "review_lane_alarm.py --self-test" in c), None)
        live = next(
            (i for i, c in enumerate(commands)
             if "review_lane_alarm.py" in c and "--self-test" not in c), None)
        return selftest, live

    def test_the_LIVE_step_actually_invokes_the_script(self):
        # A whole-workflow substring check is VACUOUS here: the self-test step also names
        # the script, so typoing ONLY the live call site would satisfy it. Assert against
        # the live step specifically.
        _, live = self._step_indices()
        self.assertIsNotNone(live, "no step invokes the alarm outside --self-test mode")
        self.assertIn("scripts/review_lane_alarm.py", str(self.steps[live]["run"]))

    def test_the_self_test_step_actually_invokes_the_script(self):
        selftest, _ = self._step_indices()
        self.assertIsNotNone(selftest, "no step runs the alarm's --self-test")

    def test_the_self_test_runs_before_the_live_step(self):
        selftest, live = self._step_indices()
        self.assertIsNotNone(selftest)
        self.assertIsNotNone(live)
        self.assertLess(selftest, live, "a detector proves itself before it is trusted")

    def test_the_self_test_step_is_unconditional(self):
        for step in self.steps:
            if "--self-test" in str(step.get("run", "")):
                self.assertNotIn("if", step, "the self-test must never be skippable")

    def test_the_live_step_is_unconditional(self):
        for step in self.steps:
            run = str(step.get("run", ""))
            if "review_lane_alarm.py" in run and "--self-test" not in run:
                self.assertNotIn("if", step)

    def test_no_continue_on_error_anywhere(self):
        # Exit-zero swallowing: a later transient must never discard the earned red.
        self.assertNotIn("continue-on-error", self.job)
        for step in self.steps:
            self.assertNotIn("continue-on-error", step)

    def test_the_job_holds_issues_write_only(self):
        perms = self.job["permissions"]
        self.assertEqual(perms.get("issues"), "write")
        self.assertEqual(perms.get("contents"), "read")

    def test_the_job_has_NO_arming_authority(self):
        # THE escalation guard. Granting pull-requests:write would let this detector
        # write review:pass and therefore arm — the one thing it must never be able to do.
        perms = self.job["permissions"]
        self.assertNotIn("pull-requests", perms)
        self.assertNotEqual(perms.get("contents"), "write")
        for forbidden in ("actions", "checks", "id-token"):
            self.assertNotIn(forbidden, perms, forbidden)

    def test_the_workflow_level_permissions_are_read_only(self):
        self.assertEqual(self.wf["permissions"], {"contents": "read"})

    def test_it_does_not_act_on_the_transport_workflow(self):
        # verdict-bridge.yml owns the comment -> label TRANSPORT hop; this workflow owns
        # noticing that no verdict was ever PRODUCED. They must not both write the same
        # label or drive each other. Prose may name it (the header does); no executable
        # step may touch it, and no step may write a PR label at all.
        for step in self.steps:
            run = str(step.get("run", ""))
            self.assertNotIn("verdict-bridge", run)
            self.assertNotIn("workflow run", run)
            for label_write in ("gh pr edit", "gh issue edit", "--add-label", "/labels"):
                self.assertNotIn(label_write, run, label_write)

    def test_actions_are_sha_pinned(self):
        import re
        for step in self.steps:
            uses = step.get("uses")
            if uses:
                self.assertRegex(uses, r"@[0-9a-f]{40}$", uses)

    def test_the_job_name_carries_no_advisory_token(self):
        import re
        self.assertIsNone(re.search(r"\b(advisory|informational)\b", self.job["name"]))

    def test_the_checkout_does_not_persist_credentials(self):
        for step in self.steps:
            if str(step.get("uses", "")).startswith("actions/checkout@"):
                self.assertIs(step["with"]["persist-credentials"], False)

    def test_the_sparse_checkout_includes_the_script(self):
        # A sparse-checkout that misses the script would fail the run at HEAD, but silently
        # drift if the script is ever renamed; pin the pairing.
        for step in self.steps:
            if str(step.get("uses", "")).startswith("actions/checkout@"):
                self.assertIn("scripts/review_lane_alarm.py", step["with"]["sparse-checkout"])


class TestGateCallSite(unittest.TestCase):
    """The anti-vacuity anchor: without this, the whole suite could leave CI unnoticed."""

    def test_docs_quality_runs_this_suite(self):
        text = DOCS_QUALITY.read_text(encoding="utf-8")
        self.assertIn("scripts/tests/test_review_lane_alarm.py", text)

    def test_the_call_site_step_is_unconditional(self):
        wf = _load(DOCS_QUALITY)
        found = False
        for job in wf["jobs"].values():
            for step in job.get("steps") or []:
                if "test_review_lane_alarm.py" in str(step.get("run", "")):
                    found = True
                    self.assertNotIn("if", step, "the suite must not be conditionally skipped")
                    self.assertNotIn("continue-on-error", step)
        self.assertTrue(found, "docs-quality.yml must invoke test_review_lane_alarm.py")


if __name__ == "__main__":
    unittest.main(verbosity=2, argv=[sys.argv[0], "-v"])
