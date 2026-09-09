#!/usr/bin/env python3
# [OPUS-5] Hermetic tests for scripts/triage-area.py — the `needs:area` backlog
# classifier that clears the migration's 257-issue dispatch block (#1135).
#
# These are DELIBERATELY not a copy of the script's own `--self-test`. That one
# proves the rules do what the author meant; this one proves the two properties
# that make the tool SAFE to point at 257 live issues:
#
#   1. FAIL CLOSED — no evidence must yield NO label. A wrong `area:` routes a
#      worker at the wrong crate and can put two workers on one conflict
#      partition; the park is maintainer-visible and self-clearing.
#   2. NO DRIFT between the two writes — `needs:area` is removed ONLY together
#      with >=1 real `area:` label. Either half alone re-breaks dispatch (a
#      still-parked issue stays invisible; an unparked no-area issue silently
#      reserves the serializing `__global__` partition).
#
# and the invariant that stops the rule table rotting:
#
#   3. EVERY area the rule table can emit must be a label the repo ALREADY has.
#      Enumerated statically from RULES — no network.
#   4. SCOPE DISCIPLINE — a `title`-scoped rule must never fire off a BODY mention.
#      60 of the 70 rules are title-scoped, and body-matching is the documented
#      root cause of the mislabels this tool exists to clean up. Asserted PER RULE
#      (see TestScopeDiscipline), because the realistic edit changes one rule.
#
# and — since #5003 widened the queue from "the parked issues" to "every AREA-LESS
# open issue" — the two properties that make that widening safe:
#
#   5. REACH — the fetch must SEE the never-parked area-less class, and must not
#      silently truncate. Both defects are invisible from outside: a sweep that
#      never fetched an issue looks exactly like a sweep with nothing to do
#      (TestFetchReach).
#   6. WIDER REACH, IDENTICAL WRITES — an issue that was never parked may only ever
#      GAIN `area:` labels; the unpark is decided per issue from its own labels
#      (TestUnparkDiscipline).
#
# Run:  python3 scripts/tests/test_triage_area.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import re
import sys
import unittest
from pathlib import Path

try:  # the parsed-regex API moved in 3.11
    from re import _parser as sre_parser
except ImportError:  # pragma: no cover - Python < 3.11
    import sre_parse as sre_parser  # type: ignore[no-redef]

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


TA = _load("triage_area", "triage-area.py")
CRATES = TA.crate_names()


def areas(title: str, body: str = "") -> list:
    return TA.classify(title, body, CRATES)[0]


def evidence(title: str, body: str = "") -> str:
    return TA.classify(title, body, CRATES)[1]


# --- a MATCHING witness string for an arbitrary rule regex ----------------------
# The scope property has to be asserted per rule (the realistic edit tweaks ONE
# rule), and 60 hand-written fixtures would rot the moment a rule's regex moves.
# So derive the fixture FROM the regex: walk the parsed pattern and emit the
# shortest string it accepts. Every witness is then re-checked against the live
# regex in test_every_title_rule_has_a_witness_that_actually_matches, so a
# generator that silently produced a non-matching string fails LOUDLY instead of
# turning the whole property into a vacuous pass.
#
# Unsupported node kinds raise: a new regex construct must be taught to the
# generator, never silently skipped (a skipped rule is an unguarded rule).
_CATEGORY_SAMPLE = {"CATEGORY_WORD": "x", "CATEGORY_DIGIT": "1", "CATEGORY_SPACE": " "}


def _sample_from_set(items) -> str:
    for op, arg in items:
        name = str(op)
        if name == "LITERAL":
            return chr(arg)
        if name == "RANGE":
            return chr(arg[0])
        if name == "CATEGORY":
            return _CATEGORY_SAMPLE[str(arg)]
    raise ValueError(f"unsupported character set {items!r}")


def _emit(parsed) -> tuple[str, bool]:
    """(shortest accepted string, is_anchored_at_string_start) for a parsed regex.

    A `^`-anchored alternative can only ever match at offset 0 — i.e. the start of
    the TITLE, since classify() searches `title + "\\n" + body`. Such a rule is
    scope-immune by construction, so BRANCH prefers an unanchored alternative and
    reports back when every alternative is anchored."""
    out: list[str] = []
    anchored = False
    for op, arg in parsed:
        name = str(op)
        if name == "LITERAL":
            out.append(chr(arg))
        elif name == "ANY":
            out.append("x")
        elif name == "IN":
            out.append(_sample_from_set(arg))
        elif name == "AT":
            if str(arg) in ("AT_BEGINNING", "AT_BEGINNING_STRING") and not out:
                anchored = True
        elif name == "SUBPATTERN":
            text, sub_anchored = _emit(arg[3])
            anchored = anchored or (sub_anchored and not out)
            out.append(text)
        elif name in ("MAX_REPEAT", "MIN_REPEAT"):
            low, _high, sub = arg
            text, _ = _emit(sub)
            out.append(text * low if low else "")
        elif name == "BRANCH":
            alternatives = [_emit(alt) for alt in arg[1]]
            unanchored = [t for t, a in alternatives if not a]
            if unanchored:
                out.append(unanchored[0])
            else:
                out.append(alternatives[0][0])
                anchored = True
        else:
            raise ValueError(f"unsupported regex node {name} in {parsed!r}")
    return "".join(out), anchored


def rule_witness(regex: str) -> tuple[str, bool]:
    return _emit(sre_parser.parse(regex, flags=0))


def title_rules():
    return [r for r in TA.RULES if r[1] == "title"]


def text_rules():
    return [r for r in TA.RULES if r[1] == "text"]


#: The ONLY rules permitted to match an issue BODY. Body-matching is what let
#: `derive_areas` label a zkSPARQL spec bead `area:site` off one incidental word,
#: so the list is short, hand-reviewed, and pinned by name — see
#: TestRuleTableHygiene.test_only_the_pinned_rules_are_allowed_to_read_the_body.
#: Each of these matches a repo PATH or a code SYMBOL, which a body cites
#: deliberately and a neighbouring bead does not name in passing.
TEXT_SCOPED_RULES = frozenset({
    "site-specs", "site-papers", "zk-xpath", "zk-compose", "reason-dl-floor",
    "js", "gui-tauri", "site-app", "bench", "ci",
})

# A title with no evidence of its own — pinned by the assertion in
# TestScopeDiscipline.setUp so it can never quietly acquire an area and turn the
# per-rule property into a tautology.
NEUTRAL_TITLE = "Recurring chore: worktree disk-hygiene sweep"


class TestFailClosed(unittest.TestCase):
    """No evidence => no label. This is the property the whole design rests on."""

    def test_no_evidence_stays_parked(self):
        for title in ("do the thing",
                      "make sure the pinned overview issue gets maintained",
                      "Recurring chore: worktree disk-hygiene sweep",
                      "LinkedIn advert post: enumerate ALL implemented specs"):
            self.assertEqual(areas(title), [], title)

    def test_declaration_naming_no_real_crate_stays_parked(self):
        # sq-lsp7k.12 declares "NEW opt-in crate" — a package that does not exist.
        # Inventing one here would be the exact misroute the park exists to avoid.
        self.assertEqual(
            areas("BI/SQL wire-protocol facade",
                  "crate_or_surface: NEW opt-in crate (pgwire-class) | effort:XL"), [])

    def test_a_body_name_drop_is_not_evidence(self):
        # A bead that merely cites a neighbouring crate in prose must NOT be
        # partitioned into it — the failure mode that produced these 257 parks.
        self.assertEqual(
            areas("formalize the codex-EC2 worker tooling",
                  "the harness has been used against sparq-core and sparq-jsonld"), [])


class TestNoDrift(unittest.TestCase):
    """`needs:area` and the `area:` labels are written in ONE call, or not at all."""

    def test_plan_emits_labels_only_for_classifiable_issues(self):
        rows = TA.plan([
            {"number": 1, "title": "DL L4: narrow the RL disjointWith guard",
             "body": "", "labels": [{"name": TA.PARK_LABEL}]},
            {"number": 2, "title": "do the thing", "body": "",
             "labels": [{"name": TA.PARK_LABEL}]},
        ], CRATES)
        self.assertEqual(rows[0][1], ["area:sparq-reason-dl"])
        self.assertEqual(rows[1][1], [])

    def _capture_gh(self):
        """Record every argv apply_row/main would hand to `gh`, and restore the
        real _gh afterwards so one test can never leak a stub into the next."""
        calls = []
        real = TA._gh
        TA._gh = lambda a: (calls.append(list(a)), "")[1]
        self.addCleanup(lambda: setattr(TA, "_gh", real))
        return calls

    def test_the_unpark_and_the_areas_travel_in_one_call(self):
        # apply_row is the ONLY writer. Assert on the argv it builds rather than
        # trusting the prose above it. (This case has AREAS — the empty-add case
        # is test_unpark_is_never_emitted_without_an_area below, and the two are
        # separate because a call carrying two areas can never exhibit the bug
        # that a call carrying none does.)
        calls = self._capture_gh()
        TA.apply_row(7, ["area:sparq-core", "area:sparq-vectors"])
        argv = calls[0]
        self.assertIn("--remove-label", argv)
        self.assertEqual(argv[argv.index("--remove-label") + 1], TA.PARK_LABEL)
        self.assertEqual(argv.count("--add-label"), 2)

    def test_unpark_is_never_emitted_without_an_area(self):
        """The named case: apply_row called with NO areas must not reach `gh` at
        all. A bare `--remove-label needs:area` is the worst output this tool can
        produce — the issue looks triaged, retriage.py promotes it, and it then
        reserves the serializing `__global__` partition, collapsing the dispatch
        frontier to a single worker."""
        calls = self._capture_gh()
        with self.assertRaises(ValueError):
            TA.apply_row(4242, [])
        self.assertEqual(calls, [], f"a bare unpark reached gh: {calls}")

    def test_main_never_applies_an_unclassified_row(self):
        """End-to-end over `main() --apply`: the filter that decides WHICH rows are
        written is exercised, not merely described. Feed it one classifiable and
        two unclassifiable issues and assert only the classifiable one is written,
        and that no emitted argv removes the park without adding an area."""
        calls = self._capture_gh()
        issues = [
            {"number": 11, "title": "DL L4: narrow the RL disjointWith guard",
             "body": "", "labels": [{"name": TA.PARK_LABEL}]},
            {"number": 12, "title": "do the thing", "body": "it would be nice",
             "labels": [{"name": TA.PARK_LABEL}]},
            {"number": 13, "title": "make sure the pinned overview issue is kept",
             "body": "", "labels": [{"name": TA.PARK_LABEL}]},
        ]
        for name, stub in (("candidate_issues", lambda: issues),
                           ("live_area_labels", lambda: {"area:sparq-reason-dl"})):
            real = getattr(TA, name)
            setattr(TA, name, stub)
            self.addCleanup(lambda n=name, r=real: setattr(TA, n, r))
        real_argv = sys.argv
        sys.argv = ["triage-area.py", "--apply"]
        self.addCleanup(lambda: setattr(sys, "argv", real_argv))

        try:
            with contextlib.redirect_stdout(io.StringIO()):  # main() prints the plan
                self.assertEqual(TA.main(), 0)
        except ValueError as exc:
            # apply_row's own fail-closed guard caught it one layer down. That is
            # the SECOND line of defence firing; report it as this test's failure
            # rather than as an unhandled error, so the diagnosis names the filter.
            self.fail(f"main() handed an unclassified row to apply_row: {exc}")

        edits = [c for c in calls if c[:2] == ["issue", "edit"]]
        self.assertEqual([c[2] for c in edits], ["11"],
                         f"main() wrote issues it did not classify: {edits}")
        for argv in edits:
            self.assertGreaterEqual(argv.count("--add-label"), 1,
                                    f"bare unpark emitted by main(): {argv}")

    def test_already_classified_issue_is_a_no_op(self):
        # Idempotence: a re-run must never re-decide settled work.
        rows = TA.plan([{"number": 3, "title": "DL L4: whatever", "body": "",
                         "labels": [{"name": "area:sparq-core"},
                                    {"name": TA.PARK_LABEL}]}], CRATES)
        self.assertEqual(rows[0][1], [])
        self.assertTrue(rows[0][2].startswith("SKIP"))


class TestFetchReach(unittest.TestCase):
    """[OPUS-5] (#5003) The candidate FETCH must SEE the whole area-less population.

    Two independent reach defects, both invisible from the outside — a sweep that
    never fetched an issue looks exactly like a sweep with nothing to do:

      1. REACH. The fetch was `--label needs:area`, i.e. only the issues triage.py /
         bd-to-issues.py PARKED. The class that actually blocks dispatch is "no
         `area:` label", and an issue the event triager never ran on carries neither
         a status nor a park. #3816 measured 832 open no-area issues against a park
         an order of magnitude smaller. A label query cannot express the absence of
         a label, so widening any downstream predicate could not have helped: an
         issue that is never fetched is never classified.
      2. TRUNCATION. `gh issue list --limit 1000` stops at the limit and reports
         nothing. Latent while the queue is under the limit, and it re-arms silently
         the moment it is not — on a half-hourly cron, where nobody is watching.

    The stub below emulates BOTH `gh` shapes faithfully — `gh api --paginate`
    exhausts the page chain, `gh api` without it returns one page, `gh issue list
    --label L --limit N` returns only labelled rows truncated newest-first — so
    every reverted implementation is EXECUTABLE here and fails on missing ROWS
    rather than on an unrecognised command line. That is what makes these
    behavioural guards and not spelling tests.
    """

    #: One page is 100, so a fetch that forgets `--paginate` returns 100 of these;
    #: the old `--limit 1000` returns the newest 1000 and drops #1..#200.
    PARKED = 1200
    #: Area-less and NEVER PARKED: no `needs:area`, no `status:*`, nothing a label
    #: query can select on. This is the population the widening exists to reach.
    NEVER_PARKED = (90001, 90002, 90003)
    #: Already attributed — a maintainer's decision the sweep must not re-open.
    HAS_AREA = 90101
    #: The issues endpoint returns PRs too.
    PULL_REQUEST = 90201

    def setUp(self):
        self.corpus = [
            {"number": n, "title": "DL L4: narrow the RL disjointWith guard",
             "body": "", "labels": [{"name": TA.PARK_LABEL}]}
            for n in range(1, self.PARKED + 1)
        ]
        self.corpus += [
            {"number": n, "title": "DL L4: narrow the RL disjointWith guard",
             "body": "", "labels": [{"name": "priority:P2"}]}
            for n in self.NEVER_PARKED
        ]
        self.corpus.append({"number": self.HAS_AREA, "title": "DL L4: whatever",
                            "body": "", "labels": [{"name": "area:sparq-core"}]})
        self.corpus.append({"number": self.PULL_REQUEST, "title": "DL L4: a PR",
                            "body": "", "labels": [{"name": TA.PARK_LABEL}],
                            "pull_request": {"url": "x"}})
        self.calls = []
        real = TA._gh
        TA._gh = self._stub_gh
        self.addCleanup(lambda: setattr(TA, "_gh", real))

    def _stub_gh(self, args):
        """Faithful emulator of both CLI shapes — see the class docstring."""
        self.calls.append(list(args))
        if args[0] == "api":
            rows = list(self.corpus)
            pages = [rows[i:i + 100] for i in range(0, len(rows), 100)] or [[]]
            return json.dumps(pages if "--paginate" in args else pages[:1])
        if args[0:2] == ["issue", "list"]:
            # `gh issue list` really does exclude PRs, unlike the REST issues
            # endpoint — model that, so the PR assertion below is testing THIS
            # implementation's filter and not an artifact of the stub.
            label = args[args.index("--label") + 1]
            rows = [r for r in self.corpus if "pull_request" not in r
                    and label in {lb["name"] for lb in r["labels"]}]
            limit = int(args[args.index("--limit") + 1]) if "--limit" in args else len(rows)
            return json.dumps(sorted(rows, key=lambda r: -r["number"])[:limit])
        raise AssertionError(f"unexpected gh invocation: {args}")

    def test_a_never_parked_area_less_issue_reaches_the_queue(self):
        """THE REACH GUARD. A `--label needs:area` fetch cannot return an issue that
        carries no park, so reverting the fetch drops all of NEVER_PARKED here."""
        nums = {it["number"] for it in TA.candidate_issues()}
        self.assertEqual([n for n in self.NEVER_PARKED if n in nums],
                         list(self.NEVER_PARKED),
                         "issues that were never parked are unreachable — the sweep "
                         "still only sees the park, not the area-less class")

    def test_the_fetch_does_not_silently_truncate(self):
        """THE TRUNCATION GUARD. `--limit 1000` keeps the newest 1000 and reports
        nothing about the rest; a single page keeps 100. Both lose the OLDEST rows,
        which on a work-queue sweep are the ones that have waited longest."""
        nums = sorted(it["number"] for it in TA.candidate_issues())
        self.assertEqual(len(nums), self.PARKED + len(self.NEVER_PARKED))
        self.assertEqual(nums[:3], [1, 2, 3],
                         "the oldest issues were dropped by a limited fetch")
        self.assertEqual(len(set(nums)), len(nums), "an issue was fetched twice")

    def test_the_fetch_uses_cursor_pagination(self):
        TA.candidate_issues()
        self.assertTrue(self.calls, "candidate_issues() made no gh call at all")
        self.assertTrue(all(c[0] == "api" and "--paginate" in c for c in self.calls),
                        f"the fetch used a fixed-limit call: {self.calls}")

    def test_an_already_attributed_issue_is_not_re_fetched(self):
        """Widening the reach must not re-open settled work: an issue that already
        carries an `area:` is the maintainer's/author's decision, not ours."""
        self.assertNotIn(self.HAS_AREA, {it["number"] for it in TA.candidate_issues()})

    def test_pull_requests_are_not_candidates(self):
        """The issues endpoint returns PRs; a PR has no dispatch partition, so
        labelling one would be pure noise on the board."""
        self.assertNotIn(self.PULL_REQUEST,
                         {it["number"] for it in TA.candidate_issues()})

    def test_a_runaway_snapshot_fails_closed(self):
        """The ceiling is the other half of dropping the limit: pagination to
        exhaustion must still refuse to half-report an implausible snapshot rather
        than quietly editing thousands of live issues."""
        with self.assertRaises(SystemExit):
            TA.open_issues(ceiling=10)


class TestUnparkDiscipline(unittest.TestCase):
    """[OPUS-5] (#5003) Wider reach, IDENTICAL writes.

    Most rows in the widened queue were never parked. Such an issue must only ever
    GAIN `area:` labels: emitting `--remove-label needs:area` for it is a write the
    sweep has no business making, and it reports a park-clearing that never
    happened. The park half of the no-drift invariant (TestNoDrift) is unchanged for
    the issues that ARE parked."""

    def _capture_gh(self):
        calls = []
        real = TA._gh
        TA._gh = lambda a: (calls.append(list(a)), "")[1]
        self.addCleanup(lambda: setattr(TA, "_gh", real))
        return calls

    def test_a_never_parked_issue_is_not_unparked(self):
        calls = self._capture_gh()
        TA.apply_row(31, ["area:sparq-core"], unpark=False)
        argv = calls[0]
        self.assertNotIn("--remove-label", argv,
                         f"the sweep removed a park the issue never had: {argv}")
        self.assertEqual(argv.count("--add-label"), 1)

    def test_a_parked_issue_still_travels_with_its_unpark(self):
        calls = self._capture_gh()
        TA.apply_row(32, ["area:sparq-core"], unpark=True)
        argv = calls[0]
        self.assertEqual(argv[argv.index("--remove-label") + 1], TA.PARK_LABEL)

    def test_the_empty_add_guard_holds_in_both_modes(self):
        """A bare unpark is refused with `unpark=True`; with `unpark=False` there is
        nothing legitimate to write either, so the same guard must stop it reaching
        `gh` as a contentless edit."""
        calls = self._capture_gh()
        for unpark in (True, False):
            with self.assertRaises(ValueError):
                TA.apply_row(4242, [], unpark=unpark)
        self.assertEqual(calls, [], f"a contentless edit reached gh: {calls}")

    def test_main_decides_the_unpark_per_issue(self):
        """End-to-end through `main() --apply`: the decision is read from each
        issue's OWN labels, not from a single flag for the whole sweep."""
        calls = []
        real = TA._gh
        TA._gh = lambda a: (calls.append(list(a)), "")[1]
        self.addCleanup(lambda: setattr(TA, "_gh", real))
        issues = [
            {"number": 41, "title": "DL L4: narrow the RL disjointWith guard",
             "body": "", "labels": [{"name": TA.PARK_LABEL}]},
            {"number": 42, "title": "DL L4: narrow the RL disjointWith guard",
             "body": "", "labels": [{"name": "priority:P2"}]},
        ]
        for name, stub in (("candidate_issues", lambda: issues),
                           ("live_area_labels", lambda: {"area:sparq-reason-dl"})):
            prev = getattr(TA, name)
            setattr(TA, name, stub)
            self.addCleanup(lambda n=name, r=prev: setattr(TA, n, r))
        real_argv = sys.argv
        sys.argv = ["triage-area.py", "--apply"]
        self.addCleanup(lambda: setattr(sys, "argv", real_argv))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(TA.main(), 0)

        edits = {c[2]: c for c in calls if c[:2] == ["issue", "edit"]}
        self.assertEqual(sorted(edits), ["41", "42"])
        self.assertIn("--remove-label", edits["41"])
        self.assertNotIn("--remove-label", edits["42"],
                         f"main() unparked a never-parked issue: {edits['42']}")
        for argv in edits.values():
            self.assertIn("area:sparq-reason-dl", argv)


class TestRuleTableHygiene(unittest.TestCase):
    def test_every_emitted_area_is_a_real_crate_or_a_known_surface(self):
        """Never invent a label. An `area:` the repo does not have is invisible to
        ready-issues.py and push-frontier.sh, so the issue stays undispatchable
        while LOOKING triaged — strictly worse than the park."""
        # The non-crate surfaces the repo really carries (checked against
        # `gh label list` when this test was written, 2026-07-26).
        surfaces = {"site", "site-specs", "site-papers", "gui", "js", "bench", "ci",
                    "docs", "deps", "release", "workspace", "upstream", "e2ee",
                    "zk", "zk-xpath", "knowledge-graph", "deploy-demo"}
        for rid, _scope, _rx, emitted, _why in TA.RULES:
            for a in emitted:
                self.assertTrue(a in CRATES or a in surfaces,
                                f"rule {rid} emits unknown area {a!r}")

    def test_rule_order_survives_the_known_name_drop_collisions(self):
        """Three orderings were each MEASURED wrong on the live backlog. Pin them:
        reordering the table silently re-breaks these and nothing else would."""
        # A .typ spec that names zk/ieee754 + zk/xpath is SPEC work.
        self.assertEqual(
            areas("zksparql.typ 7.3 stale estate sentence: still names zk/ieee754 "
                  "+ zk/xpath as in-tree"), ["site-specs"])
        # An ieee754 bead whose body literally reads "FILE: zk/xpath NO -- zk/ieee754/..."
        self.assertEqual(
            areas("ieee754 OPT: gate-neutral kernels.nr cleanups",
                  "FILE: zk/xpath NO -- zk/ieee754/src/ops/kernels.nr"), ["zk"])
        # An ODRL bead whose paper recorded it is ODRL work, not paper work.
        self.assertEqual(
            areas("odrl-bridge: materialise rule-level provenance",
                  "recorded as limitation #5 in site/papers/odrl-policy-bridge.typ"),
            ["sparq-policy"])

    def test_cross_cutting_issues_keep_every_area(self):
        """The partitioner maps multi-area to __global__ deliberately. Collapsing a
        genuinely cross-crate issue to one crate to make it look dispatchable is a
        lie that lands two workers in one partition."""
        self.assertEqual(
            areas("MPC M4-v1: attestation GATE assembly",
                  "Crate: sparq-mpc (pipeline.rs/proof.rs) + sparq-zk-compose "
                  "(federated reconstruct_public_inputs reuse). The buildable M4 v1:"),
            ["sparq-mpc", "sparq-zk-compose"])
        self.assertEqual(
            areas("[epic] Proof-of-correctness program for sparq_ieee754 & noir_XPath"),
            ["zk", "zk-xpath"])

    def test_declaration_span_stops_at_the_sentence_break(self):
        """sq-p4zci's field is followed by prose containing 'docs/SKILL examples';
        running the span to end-of-line derived a spurious area:docs."""
        self.assertEqual(
            areas("Datalog: surface wiring",
                  "crates: sparq-reason + sparq-cli. CLI flag + a handle for datalog "
                  "programs; docs/SKILL examples beyond the API reference."),
            ["sparq-reason", "sparq-cli"])

    def test_scope_is_declared_and_only_ever_title_or_text(self):
        """`scope` is a two-valued dispatch in classify(); a typo'd third value
        would silently fall through to the body-matching branch."""
        self.assertEqual({r[1] for r in TA.RULES}, {"title", "text"})

    def test_only_the_pinned_rules_are_allowed_to_read_the_body(self):
        """The scope of a rule is a POLICY decision, and widening one from `title`
        to `text` is the commonest rule-table edit there is. It cannot be caught
        behaviourally — a widened rule behaves exactly like a rule that was always
        text-scoped — so the allow-list is pinned by NAME here. A flip fails with
        the rule id in the diff, which is the review prompt: does this rule's
        evidence really live in bodies, or is it about to inherit `derive_areas`'s
        mislabels?"""
        self.assertEqual({r[0] for r in TA.RULES if r[1] == "text"}, TEXT_SCOPED_RULES)


class TestScopeDiscipline(unittest.TestCase):
    """A `title`-scoped rule must NEVER fire off a BODY mention — asserted PER RULE.

    This is the anti-mislabel mechanism the whole rule table leans on: 60 of the 70
    rules are title-scoped, and matching bodies is precisely how the migration-time
    `derive_areas` sent a zkSPARQL spec bead to `area:site` off one incidental word.

    Asserted per rule rather than with a single fixture because the failure mode is
    per rule: the realistic edit is not "delete the scope dispatch", it is "flip one
    rule's `\"title\"` to `\"text\"` while tuning it" — the exact shape of a rule-table
    change. A single fixture pins one rule and leaves the other 59 unguarded.

    Division of labour with TestRuleTableHygiene: the tests here prove the DISPATCH
    still works as declared (a mechanism check, which is what the one-line
    `hay = text` collapse breaks); the pinned TEXT_SCOPED_RULES allow-list there
    proves the DECLARATION has not moved (a policy check). A widened rule behaves
    exactly like a rule that was always text-scoped, so only the allow-list can
    catch it — which is why both exist."""

    #: Rules whose every alternative is `^`-anchored. classify() searches
    #: `title + "\n" + body`, so `^` can only match at the start of the TITLE and
    #: the scope flag is behaviourally inert for them. Pinned rather than skipped:
    #: dropping a `^` moves a rule OUT of this set, which reds this test and
    #: simultaneously brings the rule under the per-rule assertion below.
    ANCHORED_ONLY = {"difftest-normaliser", "difftest-harness", "kani-harness",
                     "site-page", "deploy-demo"}

    def setUp(self):
        # Anti-tautology: the carrier title must itself classify to nothing, or
        # "the rule did not fire" would be true for uninteresting reasons.
        self.assertEqual(areas(NEUTRAL_TITLE), [], NEUTRAL_TITLE)

    def test_every_rule_has_a_witness_that_actually_matches(self):
        """The generated fixtures are the substrate of both properties below; if one
        stopped matching its own regex they would pass vacuously. Two checks per
        rule: the witness matches the regex, AND it reaches THAT rule through the
        real classify() path (a witness shadowed by an earlier rule could never
        demonstrate anything about this one)."""
        checked = 0
        for rid, _scope, rx, _areas, _why in TA.RULES:
            witness, _anchored = rule_witness(rx)
            self.assertTrue(re.search(rx, witness, re.I),
                            f"rule {rid}: generated witness {witness!r} does not match {rx!r}")
            self.assertTrue(evidence(witness).startswith(f"T1 {rid}:"),
                            f"rule {rid}: witness {witness!r} is claimed by "
                            f"{evidence(witness)!r} — the rule is unreachable")
            checked += 1
        self.assertEqual(checked, len(TA.RULES))
        self.assertGreater(checked, 0)

    def test_a_title_scoped_rule_never_fires_from_the_body_alone(self):
        """THE property. Each title rule's own witness, moved into the body of an
        issue whose title carries no evidence, must not produce that rule's areas.
        Ranges over the DECLARED title rules, so it stays true as the table grows —
        the pinned allow-list in TestRuleTableHygiene is what makes a rule leaving
        this set a reviewed act rather than a silent one."""
        leaked = []
        for rid, _scope, rx, _rule_areas, _why in title_rules():
            witness, _anchored = rule_witness(rx)
            got = TA.classify(NEUTRAL_TITLE, witness, CRATES)
            if got[1].startswith(f"T1 {rid}:"):
                leaked.append(f"{rid} fired from a body-only {witness!r} -> {got[0]}")
        self.assertEqual(leaked, [], "title-scoped rules matched the body:\n  "
                                     + "\n  ".join(leaked))

    def test_a_text_scoped_rule_does_fire_from_the_body(self):
        """The other direction of the same dispatch. The `text`-scoped rules exist
        precisely BECAUSE their evidence (a repo path, a symbol name) lives in
        bodies; collapsing the dispatch the other way — `hay = low_title` — would
        silently stop them finding it and quietly shrink the tool's yield. Without
        this, only one of the two branches of the scope dispatch is pinned."""
        missed = []
        for rid, _scope, rx, _areas, _why in text_rules():
            witness, _anchored = rule_witness(rx)
            got = TA.classify(NEUTRAL_TITLE, witness, CRATES)
            if not got[1].startswith(f"T1 {rid}:"):
                missed.append(f"{rid} did not fire on body-only {witness!r} "
                              f"(got {got[1]!r})")
        self.assertEqual(missed, [], "text-scoped rules ignored the body:\n  "
                                     + "\n  ".join(missed))

    def test_the_scope_immune_rules_are_exactly_the_fully_anchored_ones(self):
        """Keeps the exemption above honest: a rule is exempt only because every
        alternative is `^`-anchored, and that fact is re-derived from the regex,
        never assumed. Derived over the WHOLE table so it is independent of the
        scope flags — dropping a `^` reds here whatever the rule's scope says."""
        anchored = {rid for rid, _s, rx, _a, _w in TA.RULES if rule_witness(rx)[1]}
        self.assertEqual(anchored, self.ANCHORED_ONLY)

    def test_zk_ieee754_scope_is_pinned_not_only_its_order(self):
        """The rule's own comment gives TWO reasons it is safe — "TITLE-scoped and
        ahead of zk-xpath". test_rule_order_survives_the_known_name_drop_collisions
        pins only the "ahead of" half; sq-3x7dl.10's body reads "FILE: zk/xpath NO
        -- zk/ieee754/...", so if `ieee754` could match a body then every issue that
        merely discusses float semantics would be routed at zk/ieee754."""
        self.assertEqual(
            areas("Recurring chore: worktree disk-hygiene sweep",
                  "background: the failure only shows up under ieee754 rounding"), [])
        # ...while the same token in the TITLE still routes there (the rule works).
        self.assertEqual(areas("ieee754 OPT: gate-neutral kernels.nr cleanups"), ["zk"])


class TestUnattributedCensus(unittest.TestCase):
    """#5004 — the class the `--label needs:area` query cannot see.

    Per #5004 — a claim about the private registry that nothing here can check — its
    `curate-frontier.py` skips any issue whose area it cannot derive and re-skips it every
    tick forever. The half that IS checkable is the board: sparq's residue report (#3816)
    is built on the PARK query, so it counts only issues that already carry the park, while
    `triage.py` withholds that park from epics, from externally-gated issues and from
    anything still missing a role or a priority, and `retriage.py` computes the park delta
    and discards it (PROMOTING deltas only). A no-area issue that was never parked
    therefore appears in NEITHER report — which is the whole finding.

    These pin the census predicate rather than the prose: the split is what makes the
    previously-invisible half a number, so each half is asserted independently.
    """

    @staticmethod
    def _iss(number, *labels, title="whatever"):
        return {"number": number, "title": title,
                "labels": [{"name": lb} for lb in labels]}

    def test_the_unparked_half_is_counted(self):
        """THE HEADLINE. A no-area issue with no park is the class #5004 is about; if the
        census only ever returned parked issues it would restate the report that already
        exists and close nothing."""
        parked, unparked = TA.census([self._iss(1)])
        self.assertEqual([it["number"] for it in unparked], [1])
        self.assertEqual(parked, [])

    def test_the_parked_half_is_reported_separately(self):
        """The two halves must not be merged: the parked one is already covered by the
        residue report, so collapsing them would hide how much of the count is NEW."""
        parked, unparked = TA.census([self._iss(2, TA.PARK_LABEL)])
        self.assertEqual([it["number"] for it in parked], [2])
        self.assertEqual(unparked, [])

    def test_the_withheld_park_classes_are_exactly_what_shows_up_unparked(self):
        """The three shapes `triage.py` deliberately refuses to park — an epic, an
        externally-gated issue, and one missing a role/priority — are the population
        `retriage.py` hands on and nothing picks up. They are the census's reason to
        exist, so assert them by name rather than trusting one generic fixture."""
        _, unparked = TA.census([
            self._iss(31, "kind:epic", "priority:P1", "role:impl"),
            self._iss(32, "priority:P1", "role:impl", "needs:user"),
            self._iss(33),  # the zero-label class
        ])
        self.assertEqual([it["number"] for it in unparked], [31, 32, 33])

    def test_an_attributable_issue_is_not_in_the_class(self):
        """`area:` present => the curator CAN stage it => it is not the residue. Including
        the drifted shape (a human added an area but left the park on): counting it would
        inflate the number with work that is already routable."""
        parked, unparked = TA.census([
            self._iss(4, "area:sparq-core"),
            self._iss(5, "area:sparq-core", TA.PARK_LABEL),
        ])
        self.assertEqual((parked, unparked), ([], []))

    def test_the_count_is_printed_even_when_the_class_is_empty(self):
        """ALWAYS-PRINTED is the point — #5004's finding is silence, and a count emitted
        only when it is non-zero is indistinguishable from a lane that stopped running."""
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            TA.print_census([self._iss(6, "area:sparq-core")])
        tally = [ln for ln in buf.getvalue().splitlines() if ln.startswith("== ")]
        self.assertEqual(len(tally), 1, buf.getvalue())
        self.assertIn("0 open issue(s) carry no area: label", tally[0])

    def test_the_report_prefixes_cannot_be_sed_by_the_classifier_report(self):
        """triage-area.yml runs two report steps, and the classifier's extracts `^-- `
        and `^   LEFT `. They read separate logs today, so this pins the property that
        keeps them separable if that ever stops being true: were the census to reuse
        either prefix, one step would absorb the other's lines and the number would be
        WRONG rather than missing — the harder failure to notice."""
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            TA.print_census([self._iss(7), self._iss(8, TA.PARK_LABEL)])
        lines = buf.getvalue().splitlines()
        self.assertEqual([ln for ln in lines if ln.startswith(("-- ", "   LEFT "))], [])
        self.assertEqual([ln.split()[1] for ln in lines if ln.startswith("   UNPARKED ")],
                         ["#7"], buf.getvalue())

    def test_a_fetch_at_the_limit_raises_instead_of_undercounting(self):
        """The census asks for the WHOLE board, not just the park. A fetch that quietly
        stops at a limit reports FEWER unattributed issues than exist and reads exactly
        like a shrinking backlog — #5004's failure mode, reintroduced by the fix.

        Since #5003 the census shares the classifier's cursor-paginated `open_issues()`,
        so a short count is impossible by construction and the remaining risk is the
        other half: an implausible snapshot must fail CLOSED, not half-report."""
        rows = [{"number": n, "title": "x", "labels": []} for n in range(20)]
        real = TA._gh
        TA._gh = lambda a: __import__("json").dumps([rows])
        self.addCleanup(lambda: setattr(TA, "_gh", real))
        with self.assertRaises(SystemExit):
            TA.open_issues(ceiling=10)

    def test_the_census_mode_writes_nothing(self):
        """It reports on a live board from a job that holds `issues: write`. Assert on the
        argv it hands `gh`, not on the absence of an apply call: `--census` must reach the
        READ endpoint only — the shared paginated `open_issues()` fetch since #5003."""
        calls = []
        real = TA._gh
        TA._gh = lambda a: (calls.append(list(a)),
                            __import__("json").dumps(
                                [[{"number": 9, "title": "x", "labels": []}]]))[1]
        self.addCleanup(lambda: setattr(TA, "_gh", real))
        real_argv = sys.argv
        sys.argv = ["triage-area.py", "--census"]
        self.addCleanup(lambda: setattr(sys, "argv", real_argv))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(TA.main(), 0)
        self.assertEqual([c[:2] for c in calls], [["api", "--paginate"]], calls)
        for argv in calls:
            self.assertNotIn("--method", argv)
            self.assertNotIn("-X", argv)
            self.assertNotIn("--add-label", argv)
            self.assertNotIn("--remove-label", argv)


if __name__ == "__main__":
    unittest.main(verbosity=2)
