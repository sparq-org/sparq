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
                           ("live_area_labels", lambda: {"area:sparq-reason-dl"}),
                           # #5448: main() paces its writes; keep this suite hermetic
                           # (the pacing itself is asserted in TestWriteBudget).
                           ("_sleep", lambda _s: None)):
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


class TestWriteBudget(unittest.TestCase):
    """[OPUS-5] (#5448) The per-run write budget: bounded, paced, and NEVER SILENT.

    #5003 widened the queue from the `needs:area` park to every area-less open issue.
    That did not widen what is written per ISSUE, but it widened what is written per
    RUN: one `gh issue edit` per classified issue, against a queue #3816 counted at 832.
    GitHub's secondary limit on content-mutating requests (~80/min, ~500/hour) sits in
    that range, so the first ticks would 403 — self-healing (the sweep is idempotent and
    the next tick resumes) but red every half hour for hours, which is alert fatigue on
    the one lane whose value is that a human reads its summary.

    The three properties that make bounding it safe, each invisible from outside:

      1. BOUNDED — at most `--max-writes` mutating calls leave a run.
      2. NOT SILENT — the deferred tail is printed, counted in the tally line, and kept
         SEPARATE from the `LEFT` residue. A budget that quietly dropped its tail would
         look exactly like a tick with less work to do, and would break the one property
         #3816 added this lane's report for.
      3. CONVERGENT — the deferral is a deterministic prefix split of the
         issue-number-ordered plan, and every write removes its issue from the queue, so
         consecutive ticks drain the backlog monotonically instead of re-deciding it.

    Plus the arithmetic behind the two defaults, which is only checkable if the limits it
    is derived from are written down (they are, as named constants).
    """

    WORKFLOW = REPO_ROOT / ".github" / "workflows" / "triage-area.yml"

    @staticmethod
    def _classifiable(count, first=101):
        """`count` issues that all classify to one known area, numbered consecutively
        so the prefix split is assertable by number."""
        return [{"number": first + i,
                 "title": "DL L4: narrow the RL disjointWith guard",
                 "body": "", "labels": [{"name": TA.PARK_LABEL}]}
                for i in range(count)]

    def _run_main(self, issues, argv):
        """Drive `main()` over `issues` with gh, the fetch, the label list and the
        CLOCK stubbed out. Returns (gh argvs, slept durations, stdout)."""
        calls, sleeps = [], []
        for name, stub in (("_gh", lambda a: (calls.append(list(a)), "")[1]),
                           ("_sleep", sleeps.append),
                           ("candidate_issues", lambda: issues),
                           ("live_area_labels", lambda: {"area:sparq-reason-dl"})):
            real = getattr(TA, name)
            setattr(TA, name, stub)
            self.addCleanup(lambda n=name, r=real: setattr(TA, n, r))
        real_argv = sys.argv
        sys.argv = ["triage-area.py", *argv]
        self.addCleanup(lambda: setattr(sys, "argv", real_argv))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(TA.main(), 0)
        return calls, sleeps, out.getvalue()

    @staticmethod
    def _edited(calls):
        return [c[2] for c in calls if c[:2] == ["issue", "edit"]]

    def test_the_budget_bounds_the_writes_a_single_run_can_spend(self):
        calls, _, _ = self._run_main(
            self._classifiable(5), ["--apply", "--max-writes", "2", "--write-pace", "0"])
        self.assertEqual(self._edited(calls), ["101", "102"],
                         "the per-run write budget did not bound the mutating calls — "
                         "the first widened ticks will run into GitHub's secondary limit")

    def test_the_deferred_tail_is_reported_not_silently_dropped(self):
        """The headline property. A cap the report does not name is indistinguishable
        from a tick that simply had less to do."""
        _, _, out = self._run_main(
            self._classifiable(5), ["--apply", "--max-writes", "2", "--write-pace", "0"])
        self.assertEqual(re.findall(r"^   DEFERRED #(\d+)", out, re.M),
                         ["103", "104", "105"],
                         f"the budget dropped its tail without reporting it:\n{out}")
        # The tally line is what .github/workflows/triage-area.yml lifts into the run
        # summary (`sed -n 's/^-- //p'`), so the counts must be IN it, not merely printed.
        self.assertRegex(out, r"(?m)^-- 5 classified \(2 writable this run, 3 deferred")
        # ...and DEFERRED must never be folded into LEFT: LEFT means "no rule could
        # attribute this, a human is needed", DEFERRED means "classified, next tick".
        self.assertEqual(re.findall(r"^   LEFT #(\d+)", out, re.M), [],
                         "a deferred issue was reported as unattributable residue")

    def test_a_dry_run_shows_the_deferral_the_next_apply_tick_will_make(self):
        # Otherwise the budget is invisible to a maintainer until the moment it bites.
        calls, _, out = self._run_main(
            self._classifiable(5), ["--max-writes", "2", "--write-pace", "0"])
        self.assertEqual(self._edited(calls), [], "a dry run wrote labels")
        self.assertEqual(re.findall(r"^   DEFERRED #(\d+)", out, re.M),
                         ["103", "104", "105"])

    def test_the_deferred_tail_is_written_by_the_next_tick(self):
        """CONVERGENCE. The budget defers, it does not drop. Simulate the next tick:
        the issues written above now carry an `area:` label, so `candidate_issues()`
        no longer returns them, and the queue the budget sees is strictly smaller."""
        issues = self._classifiable(5)
        calls, _, _ = self._run_main(
            issues, ["--apply", "--max-writes", "2", "--write-pace", "0"])
        written = set(self._edited(calls))
        remaining = [it for it in issues if str(it["number"]) not in written]
        calls2, _, _ = self._run_main(
            remaining, ["--apply", "--max-writes", "2", "--write-pace", "0"])
        self.assertEqual(self._edited(calls2), ["103", "104"],
                         "the next tick did not resume where the budget stopped")

    def test_writes_are_paced_between_each_other_and_not_before_the_first(self):
        calls, sleeps, _ = self._run_main(
            self._classifiable(3), ["--apply", "--write-pace", "0.25"])
        self.assertEqual(len(self._edited(calls)), 3)
        self.assertEqual(sleeps, [0.25, 0.25],
                         "the mutating calls are not paced — nothing keeps this lane "
                         "under GitHub's per-minute secondary limit")
        # A lane with a single write must not pay a pace interval for nothing.
        _, single, _ = self._run_main(
            self._classifiable(1), ["--apply", "--write-pace", "0.25"])
        self.assertEqual(single, [])

    def test_the_defaults_stay_under_the_documented_secondary_limits(self):
        """The two constants are only defensible as arithmetic on the published limits,
        so do the arithmetic here rather than trusting the comment next to them."""
        self.assertLessEqual(
            TA.MAX_WRITES_PER_RUN * TA.TICKS_PER_HOUR, TA.SECONDARY_WRITES_PER_HOUR,
            "the per-run budget times the ticks in an hour exceeds GitHub's documented "
            "hourly limit for content-mutating requests")
        self.assertGreater(TA.WRITE_PACE_SECONDS, 0,
                           "a zero default pace leaves the per-minute limit unguarded")
        self.assertLessEqual(60 / TA.WRITE_PACE_SECONDS, TA.SECONDARY_WRITES_PER_MINUTE,
                             "the default pace admits more writes per minute than "
                             "GitHub's documented secondary limit allows")

    def test_ticks_per_hour_matches_the_cron_that_actually_drives_the_lane(self):
        """The budget is the hourly allowance divided across the ticks in an hour, so
        widening the cron without re-deriving it silently doubles the hourly spend.
        Read from the workflow, not asserted as a literal."""
        source = self.WORKFLOW.read_text(encoding="utf-8")
        crons = re.findall(r"-\s*cron:\s*'([^']+)'", source)
        self.assertEqual(len(crons), 1, f"expected exactly one cron: {crons}")
        minute, hour = crons[0].split()[0], crons[0].split()[1]
        self.assertEqual(hour, "*", "the hour field moved; re-derive TICKS_PER_HOUR")
        self.assertRegex(minute, r"^\d+(,\d+)*$",
                         "the minute field is no longer a plain list, so the tick count "
                         "below cannot be counted from it — re-derive TICKS_PER_HOUR")
        self.assertEqual(TA.TICKS_PER_HOUR, len(minute.split(",")),
                         f"cron {crons[0]!r} fires {len(minute.split(','))} times an "
                         f"hour but TICKS_PER_HOUR says {TA.TICKS_PER_HOUR}")

    def test_there_is_no_unlimited_write_mode(self):
        # `--max-writes 0` reading as "no budget" is the footgun this flag exists to
        # remove; argparse must refuse it rather than restore the unbounded tick.
        for bad in ("0", "-1"):
            real_argv = sys.argv
            sys.argv = ["triage-area.py", "--apply", "--max-writes", bad]
            try:
                with contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as caught:
                        TA.main()
                self.assertEqual(caught.exception.code, 2, f"--max-writes {bad}")
            finally:
                sys.argv = real_argv


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


if __name__ == "__main__":
    unittest.main(verbosity=2)
