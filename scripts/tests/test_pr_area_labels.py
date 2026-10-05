#!/usr/bin/env python3
# [OPUS-5] 🤖 SPARQ agent — the guard suite for PR `area:` attribution.
# Registry issue jeswr/agent-account-registry#677.
#
# Every guard in scripts/pr-area-labels.py + ci/area-labels.toml +
# .github/workflows/pr-area-label.yml has a NAMED test here that goes RED when the guard
# is deleted or inverted. The three families:
#
#   1. DERIVATION (pure) — a crates/x-only PR narrows to x; a workspace-spanning PR stays
#      `__global__`; an unattributable path stays `__global__`; an area with no live label
#      is never invented.
#   2. EFFECTS (fake `gh`) — the recorded call list IS the assertion: additive-only (a
#      human `area:` label survives, observed on the fake's simulated label state, not by
#      grepping the source), idempotent (a second pass writes nothing), dry-run writes
#      nothing, a fork PR and a no-pull-request ref make ZERO api calls.
#   3. THE YAML SEAM — parsed STRUCTURALLY, never by substring or `count(...) == N`, which
#      is what lets `if: false`, a deleted step or a swapped call-site argument survive.
#      Measured in this project: 18/18 Python mutants died while EVERY surviving mutant
#      lived in a workflow `if:` / step / call-site. So: the workflow must have NO `if:`
#      at any level (job or step), the labelling step must exist and pass exactly the
#      four documented arguments, checkout must be pinned to the default branch (never the
#      PR head — the job holds `pull-requests: write`), the permissions block must be
#      exactly the least-privilege set, `pull_request` must carry NO `paths:` filter, and
#      routing-self-tests.yml must actually INVOKE this file (a test nobody runs is a
#      call-site mutant that survives everything else).
#
# Needs PyYAML (same dependency the other wiring suites use); everything else is stdlib.
# Run:  python3 scripts/tests/test_pr_area_labels.py

from __future__ import annotations

import importlib.util
import json
import re
import sys
import tomllib
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
DERIVER = REPO_ROOT / "scripts" / "pr-area-labels.py"
POLICY = REPO_ROOT / "ci" / "area-labels.toml"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "pr-area-label.yml"
ROUTING_WF = REPO_ROOT / ".github" / "workflows" / "routing-self-tests.yml"
CRATES = REPO_ROOT / "crates"
SKILLS = REPO_ROOT / "skills"
REPO = "sparq-org/sparq"


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader, path
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


M = _load("pr_area_labels", DERIVER)


def _tracked_paths():
    """Every git-tracked repo-relative path, or None when git is unavailable."""
    import subprocess
    proc = subprocess.run(["git", "-C", str(REPO_ROOT), "ls-files", "-z"],
                          capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        return None
    return [p for p in proc.stdout.split("\0") if p]


def _unattributed_tracked(policy, known):
    """The tracked paths that attribute to NOTHING under `policy`. Shared by the hard
    totality gate and by its non-vacuity guard, so the guard exercises the SAME code the
    gate runs — a neutered computation reds both."""
    tracked = _tracked_paths()
    if tracked is None:
        return []
    return sorted({p for p in tracked if M.attribute(p, policy) not in known})


def _yaml(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _on_block(wf: dict) -> dict:
    """The `on:` mapping. Bare `on` is the YAML boolean True, so PyYAML keys it under
    `True` (same trick the other workflow-anchor suites use)."""
    return wf.get("on", wf.get(True, {})) or {}


class FakeGH:
    """A `gh` stand-in that RECORDS every invocation and SIMULATES label state.

    The simulated state is what makes the additive-only and idempotence assertions
    discriminating: they check the resulting label set, not the absence of a string in
    the source. A hypothetical "prune stale areas" feature would remove a label here and
    turn the additive-only test red.

    It also models the INPUT BOUNDARY faithfully, because that is where the interesting
    failure lives:
      * `changedFiles` is served from the FULL entry list — it is GitHub's authoritative
        count and is NOT affected by any page cap;
      * `api .../pulls/N/files` serves ONE PAGE unless `--paginate` is passed, exactly as
        `gh` does, so dropping the flag really loses the tail;
      * on top of that it serves at most `cap` entries, which reproduces a ceiling the
        caller cannot page past — GraphQL `files(first: 100)`, or REST's 3000-file cap;
      * an entry may be `"path"`, `("new_path", "previous_path")` or
        `("new_path", "previous_path", "status")`, so a rename carries `previous_filename`
        and a crate-creating PR carries `status: added`, exactly as the REST payload does.
        The default status is `modified` — the value that witnesses NOTHING — so a fixture
        must opt IN to being a new-crate PR.

    Label CREATION is simulated too (`POST /repos/{repo}/labels` appends to the live set),
    which is what makes the creation assertions discriminating: they check the resulting
    label set and the recorded call, not the absence of a string in the source.
    """

    def __init__(self, prs, labels):
        # prs: {number: {"labels": [..], "files": [entry..], "cap": int|None,
        #                "title": str, "isDraft": bool}}
        self.prs = {n: dict(p) for n, p in prs.items()}
        self.labels = list(labels)
        self.calls: list[list[str]] = []

    @property
    def writes(self):
        return [c for c in self.calls if c[:2] == ["pr", "edit"]]

    @property
    def creates(self):
        """The label names created via `POST .../labels`, in call order."""
        out = []
        for c in self.calls:
            if c[0] == "api" and "--method" in c and "POST" in c:
                out += [a.split("=", 1)[1] for a in c if a.startswith("name=")]
        return out

    @staticmethod
    def _entries(pr):
        out = []
        for f in pr["files"]:
            if isinstance(f, str):
                out.append((f, "", "modified"))
            else:
                out.append((f[0], f[1] or "", f[2] if len(f) > 2 else "modified"))
        return out

    def _files_payload(self, n, url, paginate):
        """What `gh api [--paginate] .../pulls/N/files --jq ...|@tsv` would print.

        Without `--paginate`, `gh` returns the FIRST PAGE only — modelled here, so
        dropping the flag really loses the tail. `cap` is the separate, unpageable
        ceiling."""
        entries = self._entries(self.prs[n])
        cap = self.prs[n].get("cap")
        if cap is not None:
            entries = entries[:cap]
        if not paginate:
            m = re.search(r"per_page=(\d+)", url)
            entries = entries[:int(m.group(1)) if m else 30]
        return "".join(f"{new}\t{prev}\t{status}\n" for new, prev, status in entries)

    def __call__(self, args):
        self.calls.append(list(args))
        if args[0] == "api":
            url = next((a for a in args if a.startswith("repos/")), "")
            m = re.search(r"/pulls/(\d+)/files", url)
            if m:
                return self._files_payload(int(m.group(1)), url, "--paginate" in args)
            if "--method" in args and "POST" in args and url.endswith("/labels"):
                name = next(a.split("=", 1)[1] for a in args if a.startswith("name="))
                if name in self.labels:            # GitHub answers 422 already_exists
                    raise RuntimeError(f"gh api: label {name} already exists")
                self.labels.append(name)
                return "{}"
            return "".join(f"{name}\n" for name in self.labels)
        if args[:2] == ["pr", "view"]:
            n = int(args[2])
            pr = self.prs[n]
            return json.dumps({"number": n,
                               "labels": [{"name": lb} for lb in pr["labels"]],
                               "changedFiles": len(self._entries(pr))})
        if args[:2] == ["pr", "list"]:
            return json.dumps([
                {"number": n, "labels": [{"name": lb} for lb in p["labels"]],
                 "changedFiles": len(self._entries(p)),
                 "title": p.get("title", ""), "isDraft": p.get("isDraft", False)}
                for n, p in sorted(self.prs.items())])
        if args[:2] == ["pr", "edit"]:
            n = int(args[2])
            it = iter(args[3:])
            for tok in it:
                if tok == "--add-label":
                    lb = next(it)
                    if lb not in self.prs[n]["labels"]:
                        self.prs[n]["labels"].append(lb)
                elif tok == "--remove-label":          # never exercised by this module
                    lb = next(it)
                    if lb in self.prs[n]["labels"]:
                        self.prs[n]["labels"].remove(lb)
            return ""
        raise AssertionError(f"unexpected gh call: {args}")


def _run(fake, argv, expect=0):
    import io
    buf = io.StringIO()
    rc = M.main(argv + ["--repo", REPO], runner=fake, out=buf)
    assert rc == expect, f"rc={rc} expected {expect}\n{buf.getvalue()}"
    return buf.getvalue()


# =========================================================================== policy


class TestPolicyTable(unittest.TestCase):
    """ci/area-labels.toml must be internally valid and TOTAL over the live tree — an
    unmapped path silently means `__global__`, i.e. the starvation reg#677 describes."""

    @classmethod
    def setUpClass(cls):
        cls.policy = M.load_policy()
        cls.members = cls.policy.members
        cls.raw = tomllib.loads(POLICY.read_text(encoding="utf-8"))

    def test_every_map_row_area_is_a_live_crate_or_declared_lane(self):
        """A typo'd `area` would be dropped at runtime and the PR would stay global —
        so it must fail HERE, hermetically, instead."""
        allowed = self.members | self.policy.non_crate
        bad = sorted({a for _, a in self.policy.rows if a not in allowed})
        self.assertEqual(bad, [], f"[[map]] areas that are neither a workspace member "
                                 f"nor a [lanes] non_crate name: {bad}")

    def test_every_map_row_pattern_root_exists_on_disk(self):
        """A dead row is a silent hole: it looks like coverage and matches nothing.

        `[policy] anticipated_roots` is the ONLY escape hatch (a fetched/generated tree,
        or a file an in-flight PR adds) and is itself checked below."""
        anticipated = set(self.raw["policy"].get("anticipated_roots") or [])
        missing = []
        for pattern, _ in self.policy.rows:
            if pattern in anticipated:
                continue
            root = pattern[:-3] if pattern.endswith("/**") else pattern
            if "*" in root or "?" in root:
                continue                      # a glob (e.g. `*.md`) has no fixed root
            if not (REPO_ROOT / root).exists():
                missing.append(pattern)
        self.assertEqual(missing, [], f"[[map]] patterns whose root does not exist: {missing}")

    def test_anticipated_roots_are_declared_rows(self):
        """Keeps the escape hatch honest: an entry must correspond to a real row, so it
        cannot exempt a pattern that no row uses.

        The companion "remove it once the path lands" rule is reported as DRIFT
        (`test_table_drift_report_is_advisory_not_gating`), not gated — see that test.
        Dropping a landed entry loses nothing: `test_every_map_row_pattern_root_exists_on_disk`
        would pass for that row anyway once the root exists."""
        anticipated = list(self.raw["policy"].get("anticipated_roots") or [])
        patterns = {p for p, _ in self.policy.rows}
        self.assertEqual([a for a in anticipated if a not in patterns], [],
                         "anticipated_roots entry with no matching [[map]] row")

    def test_every_crates_subdirectory_is_a_workspace_member(self):
        """The implicit `crates/<name>` rule is only TOTAL if every directory under
        crates/ is a member; a non-member directory resolves to nothing (fail closed)."""
        dirs = {p.name for p in CRATES.iterdir() if p.is_dir()}
        self.assertEqual(sorted(dirs - self.members), [],
                         "crates/ subdirectories that are not workspace members would "
                         "fall back to __global__")

    def test_skill_surfaces_without_a_specific_row_still_resolve(self):
        """The DEMOTED half of the old `test_every_skill_surface_has_a_map_row`.

        Requiring a per-surface `[[map]]` row was a GATING assertion about the whole live
        tree in a continuously-merging repo: every new `skills/` surface landing on `main`
        turned this suite red on every unrelated open PR. Measured on `80710d9c`:
        `skills/e2ee-ng` (#3464) and `skills/solid-lws-server` (#4181) did exactly that,
        hours after a green check.

        It is safe to demote because a surface with no row costs only a COARSER lane
        (`docs`, via the `skills/**` catch-all), never a WRONG crate — the direction that
        would put two workers on one crate. Attribution stays total either way.

        What is NOT demoted: every surface must still ATTRIBUTE, over its REAL tracked
        files. Surfaces lacking a specific row are printed as drift.

        The catch-all row that MAKES the demotion safe is pinned separately, by
        `test_a_future_skill_surface_resolves_through_the_catch_all`. MEASURED: with a
        specific row now present for every surface on `main`, deleting the catch-all does
        NOT red this test — so this test alone would be a vacuous guard on it."""
        patterns = {p for p, _ in self.policy.rows}
        known = self.members | self.policy.non_crate
        drift, unattributed = [], []
        for d in sorted(p for p in SKILLS.iterdir() if p.is_dir()):
            if f"skills/{d.name}/**" not in patterns:
                drift.append(d.name)
            for f in d.rglob("*"):
                if f.is_file():
                    rel = f.relative_to(REPO_ROOT).as_posix()
                    if M.attribute(rel, self.policy) not in known:
                        unattributed.append(rel)
        self.assertEqual(sorted(unattributed)[:20], [],
                         "skills/ path(s) with NO area attribution — the `skills/**` "
                         "catch-all is gone and these PRs fall to __global__")
        if drift:
            print(f"\n  [drift, advisory] skills/ surfaces with no specific [[map]] row "
                  f"(they take the `docs` catch-all): {drift}")

    def test_a_future_skill_surface_resolves_through_the_catch_all(self):
        """The load-bearing half of the demotion above: a skills surface that has NO
        `[[map]]` row must still attribute, or "the row is only advisory" is false and a
        new surface silently takes `__global__` again.

        This is a SYNTHETIC probe, deliberately — a surface with no row is by definition
        one that has not landed yet, so no real tracked path can exercise the catch-all
        while the table is complete. Delete the `skills/**` row and this goes red."""
        known = self.members | self.policy.non_crate
        patterns = {p for p, _ in self.policy.rows}
        surface = "surface-that-does-not-exist-yet"
        self.assertNotIn(f"skills/{surface}/**", patterns)
        for probe in (f"skills/{surface}/SKILL.md",
                      f"skills/{surface}/references/deep/nested.md"):
            self.assertIn(M.attribute(probe, self.policy), known,
                          f"{probe} attributes to nothing — the `skills/**` catch-all is "
                          f"gone, so a new surface takes __global__")

    def test_landed_anticipated_roots_are_reported_as_drift_but_still_attribute(self):
        """Same demotion, same reasoning, for `[policy] anticipated_roots`. An entry whose
        path has LANDED should be removed — but that is housekeeping with no runtime
        consequence (`test_every_map_row_pattern_root_exists_on_disk` covers the row the
        moment its root exists), so it must not red an unrelated PR. Measured on
        `80710d9c`: `rust-toolchain.toml` landing via #3803 did exactly that.

        Strict half kept: a landed path must still attribute."""
        anticipated = list(self.raw["policy"].get("anticipated_roots") or [])
        known = self.members | self.policy.non_crate
        landed = [a for a in anticipated
                  if (REPO_ROOT / (a[:-3] if a.endswith("/**") else a)).exists()]
        for a in landed:
            probe = a[:-3] + "/probe" if a.endswith("/**") else a
            self.assertIn(M.attribute(probe, self.policy), known,
                          f"anticipated_roots entry {a!r} has landed and does NOT attribute")
        if landed:
            print(f"\n  [drift, advisory] anticipated_roots entries that now EXIST "
                  f"(drop them; the strict dead-row check covers those rows): {landed}")

    def test_the_totality_check_detects_an_unmapped_path(self):
        """NON-VACUITY guard for the hard ratchet below. Two ratchets were just demoted to
        advisory; this pins that the one that stayed a GATE really discriminates. Drop the
        `[[map]]` row that claims `.github/**` and the totality computation must report
        those paths — if it reports nothing, `test_every_tracked_file_in_the_repo_resolves`
        is vacuous and the whole demotion is unsafe."""
        known = self.members | self.policy.non_crate
        pruned = M.Policy(self.policy.max_areas,
                          [(p, a) for p, a in self.policy.rows if p != ".github/**"],
                          self.policy.non_crate, self.members)
        self.assertNotEqual(_unattributed_tracked(pruned, known), [],
                            "removing the `.github/**` row left every tracked path "
                            "resolvable — the totality check cannot fail")

    def test_every_tracked_file_in_the_repo_resolves(self):
        """TOTALITY over the REAL tree — every git-tracked path must attribute to some
        area. This is the test that goes red when a new top-level directory or root file
        lands without a row, which is exactly the regression that re-opens reg#677's
        starvation for every PR that touches it. A synthetic `dir/probe.txt` probe would
        NOT discriminate (it can pass while the directory's real layout is unmapped, and
        fail on directories that hold only one known file), so this walks git's index."""
        tracked = _tracked_paths()
        if tracked is None:
            self.skipTest("git ls-files unavailable")
        self.assertGreater(len(tracked), 1000, "suspiciously small tracked-file set")
        known = self.members | self.policy.non_crate
        unresolved = _unattributed_tracked(self.policy, known)
        self.assertEqual(unresolved, [], f"{len(unresolved)} tracked path(s) with no area "
                                         f"attribution (each sends its PR to __global__): "
                                         f"{unresolved[:20]}")

    def test_malformed_policy_raises_rather_than_labelling_nothing(self):
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            bad = Path(td) / "area-labels.toml"
            bad.write_text('[policy]\nmax_areas = 0\n[[map]]\npattern = "x"\narea = "ci"\n')
            with self.assertRaises(M.PolicyError):
                M.load_policy(path=bad)
            bad.write_text('[policy]\nmax_areas = 4\n')
            with self.assertRaises(M.PolicyError):
                M.load_policy(path=bad)


# ======================================================================= derivation


class TestDerivation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.policy = M.load_policy()
        cls.known = cls.policy.members | cls.policy.non_crate

    def d(self, paths, known=None):
        return M.derive_areas(paths, self.policy, self.known if known is None else known)

    # --- the headline guard: a single-crate PR must NOT be __global__ ---------------
    def test_single_crate_pr_narrows_to_that_crate_and_not_global(self):
        """reg#677's live case: draft PR #4143 touched only crates/sparq-reason/** plus
        skills/inference/SKILL.md, seized all 54 crates, and deferred #2688/#2732/#2786."""
        areas, reason = self.d(["crates/sparq-reason/src/lib.rs",
                                "crates/sparq-reason/tests/rules.rs",
                                "skills/inference/SKILL.md"])
        self.assertEqual((sorted(areas), reason), (["sparq-reason"], "resolved"))
        self.assertNotIn(M.GLOBAL, areas)
        self.assertTrue(areas, "an empty area set IS __global__ — the bug being fixed")

    def test_crate_prefix_wins_over_map_rows(self):
        # `crates/sparq-core/README.md` must be sparq-core, NOT the root-markdown `docs`
        # row: a wrong narrow label is worse than a stalled frontier.
        self.assertEqual(M.attribute("crates/sparq-core/README.md", self.policy), "sparq-core")

    def test_crate_prefix_wins_even_when_a_map_row_also_matches(self):
        """DISCRIMINATING form of the precedence guard.

        With the live table no `[[map]]` row can match a `crates/...` path, so simply
        re-ordering the two rules is an EQUIVALENT mutant against real data — it passes
        the test above for the wrong reason and only bites later, when someone adds a
        `crates/**`-shaped row. So construct a policy where both rules match and assert
        the crate still wins."""
        overlapping = M.Policy(max_areas=4, rows=[("crates/**", "workspace")],
                               non_crate=["workspace"], members=self.policy.members)
        self.assertEqual(M.attribute("crates/sparq-core/src/lib.rs", overlapping),
                         "sparq-core")
        # ... and a NON-member path under crates/ does not fall through to that row
        # either: `crates/` is the member rule's exclusive territory, fail closed.
        self.assertIsNone(M.attribute("crates/README.md", overlapping))

    def test_glob_star_does_not_cross_a_slash(self):
        self.assertEqual(M.attribute("README.md", self.policy), "docs")
        self.assertEqual(M.attribute("skills/inference/SKILL.md", self.policy), "sparq-reason")
        self.assertEqual(M.attribute("site/src/app/page.md", self.policy), "site")

    # --- the fail-closed guards ----------------------------------------------------
    def test_genuinely_cross_cutting_pr_stays_global(self):
        wide = [f"crates/{c}/src/lib.rs"
                for c in sorted(self.policy.members)[:self.policy.max_areas + 1]]
        areas, reason = self.d(wide)
        self.assertEqual((areas, reason), (frozenset(), "cross-cutting"))

    def test_declared_max_areas_is_the_reviewed_value(self):
        """The cap is a SCHEDULER-SAFETY knob, so it is pinned ABSOLUTELY here.

        Every other cross-cutting test derives its fixture from `policy.max_areas`, which
        means raising the declared value moves those fixtures with it and nothing goes
        red — the cap could be set to 60 (i.e. "nothing is ever cross-cutting") silently.
        Changing the policy is allowed; changing it without updating this line is not."""
        self.assertEqual(self.policy.max_areas, 4)

    def test_a_five_crate_pr_stays_global_at_the_declared_cap(self):
        """The absolute-fixture companion to the test above: five NAMED crates, no
        dependence on `policy.max_areas`."""
        five = ["crates/sparq-core/src/lib.rs", "crates/sparq-engine/src/lib.rs",
                "crates/sparq-cli/src/main.rs", "crates/sparq-server/src/main.rs",
                "crates/sparq-shacl/src/lib.rs"]
        self.assertEqual(self.d(five), (frozenset(), "cross-cutting"))

    def test_max_areas_boundary_is_still_narrow(self):
        """Discriminates the cap from an off-by-one: exactly max_areas must RESOLVE."""
        ok = [f"crates/{c}/src/lib.rs"
              for c in sorted(self.policy.members)[:self.policy.max_areas]]
        areas, reason = self.d(ok)
        self.assertEqual(reason, "resolved")
        self.assertEqual(len(areas), self.policy.max_areas)

    def test_one_unattributable_path_poisons_the_whole_pr(self):
        areas, reason = self.d(["crates/sparq-core/src/lib.rs", "no/such/top/level/x.txt"])
        self.assertEqual((areas, reason), (frozenset(), "unresolved"))

    def test_unknown_crate_directory_without_a_live_label_is_unresolved(self):
        self.assertEqual(self.d(["crates/not-a-crate/src/lib.rs"]),
                         (frozenset(), "unresolved"))

    def test_a_file_directly_under_crates_is_not_a_crate_area(self):
        """DISCRIMINATING: `crates/README.md` must not become `area:README.md`. The
        live-label check would also reject that name, so assert on `attribute` directly —
        otherwise this fixture passes with the segment guard deleted."""
        self.assertIsNone(M.attribute("crates/README.md", self.policy))
        self.assertIsNone(M.attribute("crates", self.policy))

    def test_a_pr_that_ADDS_a_crate_resolves_once_its_label_exists(self):
        """The workflow reads the DEFAULT BRANCH's manifest, so a crate a PR is adding is
        not yet a workspace member. Such a PR collides with nothing and must still get a
        narrow partition. Two live examples in the backfill were exactly this shape
        (#3464, #3581). This test pins the pure `derive_areas` contract — label present
        resolves, label absent is fail-closed; the new-crate WITNESS that supplies the
        missing label is the separate `creatable` path tested below."""
        new = "sparq-brand-new"
        self.assertNotIn(new, self.policy.members)
        self.assertEqual(
            M.derive_areas([f"crates/{new}/src/lib.rs"], self.policy, self.known | {new}),
            (frozenset({new}), "resolved"))
        # ... and without the label it is still fail-closed.
        self.assertEqual(M.derive_areas([f"crates/{new}/src/lib.rs"], self.policy, self.known),
                         (frozenset(), "unresolved"))

    # --- the new-crate witness (#4582) ---------------------------------------------
    def test_a_pr_that_adds_a_crate_manifest_is_attributable_without_a_prior_label(self):
        """THE #4582 GUARD. Measured on #4578 (`crates/sparq-wrapper-shacl`): the deriver
        reported `KEEP __global__ [unresolved]` because the crate is absent from the
        DEFAULT BRANCH the job reads and no `area:` label had ever been made. The added
        root manifest is the witness that turns that into a resolved, narrow partition
        plus one label to create.

        Delete the `creatable` widening in `derive_areas` — or the `new_crates` read in
        `plan_for_pr` — and this reds with `reason == "unresolved"`."""
        new = "sparq-not-yet-a-crate"
        self.assertNotIn(new, self.policy.members)
        self.assertNotIn(new, self.known)
        pr = {"number": 4578, "labels": [], "complete": True,
              "files": [f"crates/{new}/Cargo.toml", f"crates/{new}/src/lib.rs"],
              "new_crates": [new]}
        plan = M.plan_for_pr(pr, self.policy, self.known)
        self.assertEqual(plan["reason"], "resolved")
        self.assertEqual(plan["add"], [f"area:{new}"])
        self.assertEqual(plan["create"], [f"area:{new}"])
        self.assertEqual(plan["partition"], [new])
        self.assertNotEqual(plan["partition"], [M.GLOBAL])

    def test_the_same_paths_without_the_witness_stay_fail_closed(self):
        """The DISCRIMINATING half: identical paths, no added root manifest (a typo'd
        directory, a stray file under a name that is not a crate). Widening on the path
        SHAPE instead of the witness would make this pass — which is the guessing the
        fail-closed rule exists to forbid."""
        pr = {"number": 4578, "labels": [], "complete": True,
              "files": ["crates/sparq-not-yet-a-crate/Cargo.toml",
                        "crates/sparq-not-yet-a-crate/src/lib.rs"],
              "new_crates": []}
        plan = M.plan_for_pr(pr, self.policy, self.known)
        self.assertEqual(plan["reason"], "unresolved")
        self.assertEqual(plan["create"], [])
        self.assertEqual(plan["partition"], [M.GLOBAL])

    def test_only_an_added_root_manifest_is_a_witness(self):
        """`new_crate_area` is the whole authority for creating a label, so every way of
        NOT being a new crate is pinned here: an edit to an existing manifest, a source
        file, a manifest nested below the crate root, a file directly under `crates/`,
        and a directory name that cannot be a label."""
        self.assertEqual(M.new_crate_area("crates/sparq-new/Cargo.toml", "added"),
                         "sparq-new")
        self.assertEqual(M.new_crate_area("crates/sparq-new/Cargo.toml", "renamed"),
                         "sparq-new")
        for path, status in (("crates/sparq-new/Cargo.toml", "modified"),
                             ("crates/sparq-new/Cargo.toml", "removed"),
                             ("crates/sparq-new/src/lib.rs", "added"),
                             ("crates/sparq-new/sub/Cargo.toml", "added"),
                             ("crates/Cargo.toml", "added"),
                             ("Cargo.toml", "added"),
                             ("crates/has space/Cargo.toml", "added"),
                             ("crates/../evil/Cargo.toml", "added"),
                             ("crates/a,b/Cargo.toml", "added")):
            self.assertIsNone(M.new_crate_area(path, status), f"{path} [{status}]")

    def test_a_witness_never_rescues_an_otherwise_unattributable_pr(self):
        """The widening is per-AREA, not per-PR: one unattributable path still poisons the
        whole PR, and then nothing is created — `create` is a subset of `add`, so no label
        is ever made for a PR that receives none."""
        new = "sparq-brand-new"
        plan = M.plan_for_pr({"number": 11, "labels": [], "complete": True,
                              "files": [f"crates/{new}/Cargo.toml",
                                        "no/such/top/level/x.txt"],
                              "new_crates": [new]}, self.policy, self.known)
        self.assertEqual(plan["reason"], "unresolved")
        self.assertEqual(plan["add"], [])
        self.assertEqual(plan["create"], [])

    def test_a_truncated_file_list_creates_nothing_even_with_a_witness(self):
        """`complete` is checked before anything else, so a partial enumeration cannot
        make a label either — the witness never bypasses the truncation boundary."""
        new = "sparq-brand-new"
        plan = M.plan_for_pr({"number": 12, "labels": [],
                              "files": [f"crates/{new}/Cargo.toml"],
                              "new_crates": [new]}, self.policy, self.known)
        self.assertEqual(plan["reason"], "incomplete-paths")
        self.assertEqual(plan["create"], [])

    def test_an_existing_crates_label_is_never_re_created(self):
        """Adding a manifest for a crate that already HAS a label (a resurrected crate, a
        rename back) resolves as usual and creates nothing."""
        plan = M.plan_for_pr({"number": 13, "labels": [], "complete": True,
                              "files": ["crates/sparq-core/Cargo.toml"],
                              "new_crates": ["sparq-core"]}, self.policy, self.known)
        self.assertEqual(plan["add"], ["area:sparq-core"])
        self.assertEqual(plan["create"], [])

    def test_empty_change_set_stays_global(self):
        self.assertEqual(self.d([]), (frozenset(), "no-paths"))

    def test_area_with_no_live_label_is_never_invented(self):
        """`gh pr edit --add-label` CREATES an unknown label, so an area outside the live
        label set — and outside the witnessed-new-crate set, empty here — must be treated
        as unresolved."""
        self.assertEqual(
            M.derive_areas(["crates/sparq-reason/src/lib.rs"], self.policy,
                           self.known - {"sparq-reason"}),
            (frozenset(), "unresolved"))

    # --- additive-only planning ----------------------------------------------------
    def test_plan_never_proposes_removing_a_human_area_label(self):
        plan = M.plan_for_pr({"number": 1,
                              "labels": ["area:sparq-core", "review:parked"],
                              "files": ["crates/sparq-engine/src/lib.rs"],
                              "complete": True},
                             self.policy, self.known)
        self.assertEqual(plan["add"], ["area:sparq-engine"])
        self.assertEqual(plan["partition"], ["sparq-core", "sparq-engine"])

    def test_fail_closed_pr_that_already_has_a_human_area_keeps_it(self):
        plan = M.plan_for_pr({"number": 2, "labels": ["area:sparq-zk"],
                              "files": ["no/such/top/level/x.txt"],
                              "complete": True},
                             self.policy, self.known)
        self.assertEqual(plan["add"], [])
        self.assertEqual(plan["partition"], ["sparq-zk"])
        self.assertNotEqual(plan["partition"], [M.GLOBAL])

    def test_already_labelled_pr_is_a_noop(self):
        plan = M.plan_for_pr({"number": 3, "labels": ["area:sparq-engine"],
                              "files": ["crates/sparq-engine/src/lib.rs"],
                              "complete": True},
                             self.policy, self.known)
        self.assertTrue(plan["noop"])
        self.assertEqual(plan["add"], [])


# ========================================================================== effects


class TestEffects(unittest.TestCase):
    """What actually reaches `gh`. Assertions are on the recorded call list and the
    fake's simulated label state, never on the source text."""

    def fake(self, files=("crates/sparq-reason/src/lib.rs",), labels=()):
        policy = M.load_policy()
        live = [f"area:{a}" for a in sorted(policy.members | policy.non_crate)]
        return FakeGH({7: {"labels": list(labels), "files": list(files)}}, live)

    def test_apply_adds_exactly_the_derived_label(self):
        f = self.fake()
        _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.writes, [["pr", "edit", "7", "--repo", REPO,
                                     "--add-label", "area:sparq-reason"]])
        self.assertEqual(f.prs[7]["labels"], ["area:sparq-reason"])

    def test_dry_run_is_the_default_and_writes_nothing(self):
        f = self.fake()
        out = _run(f, ["--pr", "7"])
        self.assertEqual(f.writes, [])
        self.assertEqual(f.prs[7]["labels"], [])
        self.assertIn("would label", out)
        self.assertIn("DRY RUN", out)

    # --- the new-crate label creation (#4582) ---------------------------------------
    NEW_CRATE_PR = (("crates/sparq-not-yet-a-crate/Cargo.toml", "", "added"),
                    "crates/sparq-not-yet-a-crate/src/lib.rs")

    def test_a_new_crate_pr_creates_its_label_and_then_applies_it(self):
        """END TO END through the fake `gh`: the #4578 shape reaches `POST .../labels`
        exactly once and the PR ends up narrowly partitioned instead of on `__global__`.

        Asserted on the fake's SIMULATED label state, so deleting the creation call reds
        here even if the plan still names the label."""
        f = self.fake(files=self.NEW_CRATE_PR)
        out = _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.creates, ["area:sparq-not-yet-a-crate"])
        self.assertIn("area:sparq-not-yet-a-crate", f.labels)
        self.assertEqual(f.prs[7]["labels"], ["area:sparq-not-yet-a-crate"])
        self.assertNotIn("KEEP __global__", out)

    def test_the_creation_call_sets_a_name_a_colour_and_a_description(self):
        """The reason to use the explicit endpoint rather than the add-labels endpoint's
        SILENT creation is that the created label is named and described. Pin that."""
        f = self.fake(files=self.NEW_CRATE_PR)
        _run(f, ["--pr", "7", "--apply"])
        post = next(c for c in f.calls if "--method" in c and "POST" in c)
        self.assertIn("repos/sparq-org/sparq/labels", post)
        body = dict(a.split("=", 1) for a in post if "=" in a and not a.startswith("-"))
        self.assertEqual(body["name"], "area:sparq-not-yet-a-crate")
        self.assertTrue(body["color"])
        self.assertTrue(body["description"])
        self.assertLessEqual(len(body["description"]), 100)   # GitHub's cap

    def test_a_dry_run_never_creates_a_label(self):
        """`--dry-run` is the default and this is the one call that adds PERMANENT repo
        state, so it must be gated on `--apply` like every write."""
        f = self.fake(files=self.NEW_CRATE_PR)
        out = _run(f, ["--pr", "7"])
        self.assertEqual(f.creates, [])
        self.assertNotIn("area:sparq-not-yet-a-crate", f.labels)
        self.assertIn("would create", out)

    def test_a_new_crate_dir_with_no_added_manifest_creates_nothing(self):
        """The live discriminator: same directory, no witnessing manifest entry (here it
        is `modified`, the default status). Must stay on `__global__` and write nothing."""
        f = self.fake(files=("crates/sparq-not-yet-a-crate/src/lib.rs",))
        out = _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.creates, [])
        self.assertEqual(f.writes, [])
        self.assertIn("KEEP __global__", out)

    def test_a_pr_touching_only_existing_crates_creates_nothing(self):
        """Non-regression: the ordinary case must not have grown a label write."""
        f = self.fake()
        _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.creates, [])

    def test_a_second_pass_over_a_created_label_creates_it_again_never(self):
        """IDEMPOTENCE across runs: once the label exists the next run sees it in the live
        set, so it neither re-creates nor re-applies it."""
        f = self.fake(files=self.NEW_CRATE_PR)
        _run(f, ["--pr", "7", "--apply"])
        f.calls.clear()
        _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.creates, [])
        self.assertEqual(f.writes, [])

    def test_a_backfill_creates_one_label_for_two_prs_adding_the_same_crate(self):
        """IDEMPOTENCE within a run. The live label set is read ONCE, so without keeping
        it current the second PR would re-POST and hit a 422."""
        policy = M.load_policy()
        live = [f"area:{a}" for a in sorted(policy.members | policy.non_crate)]
        f = FakeGH({7: {"labels": [], "files": list(self.NEW_CRATE_PR)},
                    8: {"labels": [], "files": list(self.NEW_CRATE_PR)}}, live)
        _run(f, ["--backfill", "--apply"])
        self.assertEqual(f.creates, ["area:sparq-not-yet-a-crate"])
        self.assertEqual(f.prs[7]["labels"], ["area:sparq-not-yet-a-crate"])
        self.assertEqual(f.prs[8]["labels"], ["area:sparq-not-yet-a-crate"])

    def test_a_concurrent_creation_race_is_tolerated_but_a_real_failure_reds(self):
        """The one tolerated creation failure is another run having just made the same
        label — re-READ from the live set, never inferred from the error text. Any other
        failure propagates and REDS the run: falling back to `__global__` here is exactly
        the warn-and-fall-back behaviour #4582 rejects."""
        f = self.fake(files=self.NEW_CRATE_PR)
        f.labels.append("area:sparq-not-yet-a-crate")        # created between the two reads
        M.ensure_area_label(REPO, "area:sparq-not-yet-a-crate", runner=f,
                            out=__import__("io").StringIO())

        def refusing(args):
            if "--method" in args and "POST" in args:
                raise RuntimeError("gh api: 403 Resource not accessible by integration")
            return f(args)

        with self.assertRaises(RuntimeError):
            M.ensure_area_label(REPO, "area:sparq-never-made", runner=refusing,
                                out=__import__("io").StringIO())

    def test_human_applied_area_label_survives_an_apply(self):
        """Outcome assertion: the fake would DROP the label if a removal were ever
        issued, so this reds on any future prune-stale-areas behaviour."""
        f = self.fake(labels=["area:sparq-zk"])
        _run(f, ["--pr", "7", "--apply"])
        self.assertIn("area:sparq-zk", f.prs[7]["labels"])
        self.assertIn("area:sparq-reason", f.prs[7]["labels"])
        flat = [tok for call in f.calls for tok in call]
        self.assertNotIn("--remove-label", flat)
        self.assertNotIn("DELETE", flat)

    def test_second_pass_is_a_pure_noop(self):
        f = self.fake()
        _run(f, ["--pr", "7", "--apply"])
        before = list(f.prs[7]["labels"])
        f.calls.clear()
        out = _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.writes, [], "idempotence broken: a second pass wrote")
        self.assertEqual(f.prs[7]["labels"], before)
        self.assertIn("no-change", out)

    def test_backfill_is_idempotent_across_the_whole_open_set(self):
        policy = M.load_policy()
        live = [f"area:{a}" for a in sorted(policy.members | policy.non_crate)]
        f = FakeGH({
            11: {"labels": [], "files": ["crates/sparq-core/src/lib.rs"]},
            12: {"labels": [], "files": ["site/src/app/page.tsx", ".github/workflows/ci.yml"]},
            13: {"labels": [], "files": ["no/such/top/level/x.txt"]},
            14: {"labels": ["area:sparq-hdt"], "files": ["crates/sparq-hdt/src/lib.rs"]},
        }, live)
        _run(f, ["--backfill", "--apply"])
        self.assertEqual(sorted(f.prs[11]["labels"]), ["area:sparq-core"])
        self.assertEqual(sorted(f.prs[12]["labels"]), ["area:ci", "area:site"])
        self.assertEqual(f.prs[13]["labels"], [], "unresolved PR must stay __global__")
        self.assertEqual(f.prs[14]["labels"], ["area:sparq-hdt"])
        snapshot = {n: sorted(p["labels"]) for n, p in f.prs.items()}
        f.calls.clear()
        out = _run(f, ["--backfill", "--apply"])
        self.assertEqual(f.writes, [], "second backfill pass wrote — not idempotent")
        self.assertEqual({n: sorted(p["labels"]) for n, p in f.prs.items()}, snapshot)
        self.assertIn("0 relabelled", out)

    def test_fork_pr_makes_zero_api_calls_and_exits_zero(self):
        """`pull_request` gives a fork a READ-ONLY token. The run must report and stop —
        not crash, and not mislabel."""
        f = self.fake()
        out = _run(f, ["--pr", "7", "--apply", "--head-repo", "someone-else/sparq"])
        self.assertEqual(f.calls, [], "a fork run must not touch the API at all")
        self.assertEqual(f.prs[7]["labels"], [])
        self.assertIn("READ-ONLY token", out)

    def test_same_repo_head_is_not_treated_as_a_fork(self):
        """Discriminates the fork guard from a blanket skip."""
        f = self.fake()
        _run(f, ["--pr", "7", "--apply", "--head-repo", REPO])
        self.assertEqual(f.prs[7]["labels"], ["area:sparq-reason"])

    def test_empty_head_repo_fails_closed(self):
        f = self.fake()
        _run(f, ["--pr", "7", "--apply", "--head-repo", ""])
        self.assertEqual(f.calls, [])

    def test_no_pull_request_context_makes_zero_api_calls(self):
        """The merge_group path: the workflow passes `--pr 0`."""
        f = self.fake()
        out = _run(f, ["--pr", "0", "--apply", "--head-repo", ""])
        self.assertEqual(f.calls, [])
        self.assertIn("no pull-request context", out)

    def test_unreadable_label_set_labels_nothing(self):
        """Fail-closed on the label read: no labels visible => never guess."""
        f = FakeGH({7: {"labels": [], "files": ["crates/sparq-reason/src/lib.rs"]}}, [])
        out = _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.writes, [])
        self.assertIn("inert", out)

    def test_transient_gh_failure_retries_but_a_persistent_one_raises(self):
        """A retry must not become exit-zero swallowing of an earned hard failure."""
        calls = {"n": 0}

        def flaky(args, **kw):
            calls["n"] += 1
            return None
        import subprocess as sp

        class R:
            def __init__(self, rc):
                self.returncode, self.stdout, self.stderr = rc, "ok\n", "boom"
        seq = [R(1), R(0)]
        orig = sp.run
        try:
            sp.run = lambda *a, **k: seq.pop(0)
            self.assertEqual(M._gh(["api", "x"], sleep=lambda _s: None), "ok\n")
            seq2 = [R(1), R(1), R(1)]
            sp.run = lambda *a, **k: seq2.pop(0)
            with self.assertRaises(RuntimeError):
                M._gh(["api", "x"], sleep=lambda _s: None)
        finally:
            sp.run = orig


# ============================================================== the INPUT boundary


class TestChangedFileEnumeration(unittest.TestCase):
    """The changed-path list is the ONE input attribution cannot check for itself.

    `attribute` is a pure function of the path string, so within a COMPLETE list two PRs
    touching one file always derive the same area and always serialize. The property
    fails only if the list is PARTIAL: a truncated list derives a proper SUBSET of the
    true areas, and a too-narrow reservation is the CORRUPTING direction (two workers on
    one crate), where a too-broad one merely delays.

    Measured on this repo (2026-07-26): PR #3581 reports `changedFiles = 646` while
    `gh pr view --json files` — GraphQL `files(first: 100)` — returns 100.

    Two independent guards, each with its own test below: enumerate with the PAGINATED
    REST endpoint, AND cross-check the count against `changedFiles`. The cross-check is
    the one that survives a future API change.
    """

    # The reviewer's demonstration case, rebuilt exactly: 115 entries in path order, so a
    # 100-entry cap drops the alphabetically-late `crates/sparq-zk/**` tail.
    TRUNCATING = (["Cargo.lock"]
                  + [f"crates/sparq-core/src/m{i:03d}.rs" for i in range(110)]
                  + [f"crates/sparq-zk/src/z{i}.rs" for i in range(4)])

    def setUp(self):
        self.policy = M.load_policy()
        self.live = [f"area:{a}" for a in
                     sorted(self.policy.members | self.policy.non_crate)]

    def fake(self, files, cap=None, labels=()):
        return FakeGH({7: {"labels": list(labels), "files": list(files), "cap": cap}},
                      self.live)

    def test_the_truncated_tail_really_would_have_changed_the_answer(self):
        """Fixture-validity check, so the guard test below cannot pass for the wrong
        reason. The first 100 entries and the full 115 must derive DIFFERENT area sets,
        and the truncated one must be a PROPER SUBSET — otherwise `incomplete-paths` in
        the next test would prove nothing."""
        known = self.policy.members | self.policy.non_crate
        seen, _ = M.derive_areas(self.TRUNCATING[:100], self.policy, known)
        truth, _ = M.derive_areas(self.TRUNCATING, self.policy, known)
        self.assertEqual(set(seen), {"deps", "sparq-core"})
        self.assertEqual(set(truth), {"deps", "sparq-core", "sparq-zk"})
        self.assertTrue(seen < truth, "fixture does not demonstrate under-reservation")

    def test_a_truncated_file_list_fails_closed_instead_of_deriving_a_subset(self):
        """THE guard. The API serves only 100 of 115 entries while `changedFiles` still
        reports 115; the cross-check must catch the disagreement and emit NOTHING.

        Delete the completeness check in `plan_for_pr`/`derive_areas` and this goes red
        with `area:deps` + `area:sparq-core` applied and `area:sparq-zk` missing — the PR
        would then be editing `crates/sparq-zk/**` while the scheduler dispatched a
        `sparq-zk` issue against it."""
        f = self.fake(self.TRUNCATING, cap=100)
        out = _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.writes, [], "a TRUNCATED file list must not narrow anything")
        self.assertEqual(f.prs[7]["labels"], [])
        self.assertIn("incomplete-paths", out)
        self.assertNotIn("area:sparq-core", out)

    def test_more_than_one_hundred_paths_are_all_enumerated(self):
        """The paging half. Same 115 entries, no cap: every page must be read, so the
        full `{deps, sparq-core, sparq-zk}` is derived and applied.

        Stop paginating (or go back to `gh pr view --json files`) and this goes red — the
        cross-check then reports `incomplete-paths` and nothing is applied at all."""
        f = self.fake(self.TRUNCATING)
        _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(sorted(f.prs[7]["labels"]),
                         ["area:deps", "area:sparq-core", "area:sparq-zk"])

    def test_the_file_list_is_never_read_from_the_capped_graphql_field(self):
        """Structural, over BOTH entry points — the `--pr` path and the backfill, which
        was the second consumer of `--json files`. No `--json` selection anywhere may
        request `files`; the list must come from a `--paginate`d REST `/pulls/N/files`
        call. Reintroducing `--json ...,files` on EITHER path reds here even if the
        cross-check were also removed (a merely-fetched `files` field is the trap: it is
        inert until someone reads it, and then it under-reserves silently)."""
        for argv, n in ((["--pr", "7", "--apply"], 7), (["--backfill", "--apply"], 7)):
            f = self.fake(self.TRUNCATING)
            _run(f, argv)
            for call in f.calls:
                if "--json" not in call:
                    continue
                fields = call[call.index("--json") + 1].split(",")
                self.assertNotIn("files", fields,
                                 f"`--json files` is GraphQL files(first: 100): {call}")
                self.assertIn("changedFiles", fields,
                              f"the authoritative count must be fetched too: {call}")
            rest = [c for c in f.calls
                    if c[0] == "api" and any(f"/pulls/{n}/files" in a for a in c)]
            self.assertEqual(len(rest), 1,
                             f"expected one REST file enumeration for {argv}: {f.calls}")
            self.assertIn("--paginate", rest[0], "the REST enumeration must page")

    def test_the_backfill_uses_the_same_enumeration_as_the_single_pr_path(self):
        """The backfill was the second consumer of `--json files`; a guard on only the
        `--pr` path would leave it under-reserving."""
        f = FakeGH({11: {"labels": [], "files": self.TRUNCATING, "cap": 100},
                    12: {"labels": [], "files": self.TRUNCATING}}, self.live)
        _run(f, ["--backfill", "--apply"])
        self.assertEqual(f.prs[11]["labels"], [], "truncated PR must stay __global__")
        self.assertEqual(sorted(f.prs[12]["labels"]),
                         ["area:deps", "area:sparq-core", "area:sparq-zk"])

    def test_a_cross_crate_rename_implicates_the_source_crate_too(self):
        """Renames report only the NEW path (`filename` in REST, `path` in GraphQL). A
        move out of `crates/sparq-core/` into `crates/sparq-zk/` really touches BOTH
        crates, so `previous_filename` is consumed. Stop consuming it and this goes red
        with only `area:sparq-zk` — under-reservation again, same quantifier slip."""
        f = self.fake([("crates/sparq-zk/src/moved.rs", "crates/sparq-core/src/moved.rs")])
        _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(sorted(f.prs[7]["labels"]),
                         ["area:sparq-core", "area:sparq-zk"])

    def test_a_rename_out_of_an_unmapped_path_fails_closed(self):
        """The consumed previous path is held to the SAME standard as a current one: an
        unattributable source poisons the PR rather than being quietly ignored."""
        f = self.fake([("crates/sparq-zk/src/moved.rs", "no/such/top/level/moved.rs")])
        out = _run(f, ["--pr", "7", "--apply"])
        self.assertEqual(f.writes, [])
        self.assertIn("unresolved", out)

    def test_a_rename_counts_as_one_entry_against_changedFiles(self):
        """Cross-check arithmetic: a rename contributes TWO paths but ONE changed file.
        Counting paths instead of entries would make every rename look truncated and
        send correct PRs back to `__global__` (safe, but wrong and silent)."""
        paths, entries, _ = M.fetch_changed_files(
            REPO, 7,
            runner=self.fake([("crates/sparq-zk/a.rs", "crates/sparq-core/a.rs"),
                              "crates/sparq-zk/b.rs"]))
        self.assertEqual(entries, 2)
        self.assertEqual(sorted(paths), ["crates/sparq-core/a.rs",
                                         "crates/sparq-zk/a.rs", "crates/sparq-zk/b.rs"])

    def test_paths_are_complete_only_on_an_exact_confident_agreement(self):
        self.assertTrue(M.paths_are_complete(646, 646))
        self.assertTrue(M.paths_are_complete(0, 0))
        self.assertFalse(M.paths_are_complete(100, 646))     # the measured #3581 case
        self.assertFalse(M.paths_are_complete(3000, 4000))   # the REST 3000-file ceiling
        self.assertFalse(M.paths_are_complete(5, None))      # count unavailable
        self.assertFalse(M.paths_are_complete(5, "5"))       # not an int
        self.assertFalse(M.paths_are_complete(None, 5))

    def test_a_pr_record_with_no_completeness_verdict_fails_closed(self):
        """A future caller that assembles a PR record without establishing completeness
        must get `__global__`, not a narrowing derived from an unchecked list. Flip the
        `is True` to a truthy/`get(..., True)` default and this goes red."""
        known = self.policy.members | self.policy.non_crate
        blind = M.plan_for_pr({"number": 9, "labels": [],
                               "files": ["crates/sparq-core/src/lib.rs"]},
                              self.policy, known)
        self.assertEqual(blind["add"], [])
        self.assertEqual(blind["reason"], "incomplete-paths")
        self.assertEqual(blind["partition"], [M.GLOBAL])


# ======================================================================== YAML seam


class TestWorkflowSeam(unittest.TestCase):
    """Structural assertions over .github/workflows/pr-area-label.yml.

    A substring or `count(...) == N` check over the workflow TEXT does not catch
    `if: false`, a deleted step, or a swapped call-site argument — so everything below
    walks the PARSED document."""

    @classmethod
    def setUpClass(cls):
        cls.wf = _yaml(WORKFLOW)
        cls.jobs = cls.wf["jobs"]
        cls.job = cls.jobs["label"]
        cls.steps = cls.job["steps"]

    # --- triggers ------------------------------------------------------------------
    def test_triggers_cover_every_event_that_changes_the_path_set(self):
        on = _on_block(self.wf)
        self.assertEqual(sorted(on["pull_request"]["types"]),
                         ["opened", "ready_for_review", "reopened", "synchronize"])
        self.assertIn("merge_group", on)

    def test_pull_request_trigger_has_no_paths_filter(self):
        """A `paths:` filter here re-opens the starvation hole for whatever it excludes."""
        on = _on_block(self.wf)
        pr = on["pull_request"]
        self.assertNotIn("paths", pr)
        self.assertNotIn("paths-ignore", pr)

    def test_uses_pull_request_not_pull_request_target(self):
        """`pull_request_target` would hand a FORK's PR a WRITE token — the escalation
        this design deliberately refuses."""
        self.assertNotIn("pull_request_target", _on_block(self.wf))

    # --- the `if:` seam ------------------------------------------------------------
    def test_no_job_level_if_anywhere(self):
        """Catches `if: false` and every other job-level guard mutation."""
        guarded = sorted(j for j, spec in self.jobs.items() if "if" in spec)
        self.assertEqual(guarded, [], f"job-level `if:` found on {guarded}; every skip "
                                      f"decision belongs in Python (see the workflow header)")

    def test_no_step_level_if_anywhere(self):
        """Catches `if: false` on the labelling step, the classic surviving mutant."""
        guarded = [s.get("name") or s.get("uses") for s in self.steps if "if" in s]
        self.assertEqual(guarded, [], f"step-level `if:` found on {guarded}")

    # --- the call site -------------------------------------------------------------
    def test_the_labelling_step_exists_and_passes_exactly_the_documented_arguments(self):
        """Catches a DELETED step and a WRONG INPUT. Each argument is asserted as a whole
        token pair, and dropping `--apply` (which would make the live workflow a silent
        no-op that still reports success) reds too."""
        runs = [s["run"] for s in self.steps if "run" in s]
        self.assertEqual(len(runs), 1, "expected exactly one run step")
        run = " ".join(runs[0].split())          # normalise line continuations
        self.assertIn("python3 scripts/pr-area-labels.py", run)
        for arg in ('--repo "$TARGET_REPO"', '--pr "$PR_NUMBER"',
                    '--head-repo "$PR_HEAD_REPO"', "--apply"):
            self.assertIn(arg, run, f"missing/altered call-site argument: {arg}")
        self.assertNotIn("--dry-run", run)
        self.assertNotIn("--backfill", run)

    def test_the_env_mapping_binds_exactly_the_right_github_fields(self):
        """The WRONG-INPUT seam, pinned by EQUALITY. Every `github.*` value reaches the
        shell through `env:` (no expression interpolation in the `run:` body), so swapping
        a field — `github.event.number` for the PR number, `head.sha` for the head repo —
        changes this mapping and reds. A substring check would not: the old spelling can
        remain in a comment."""
        step = next(s for s in self.steps if "run" in s)
        self.assertEqual(step["env"], {
            "GH_TOKEN": "${{ secrets.GITHUB_TOKEN }}",
            "TARGET_REPO": "${{ github.repository }}",
            "PR_NUMBER": "${{ github.event.pull_request.number || 0 }}",
            "PR_HEAD_REPO": "${{ github.event.pull_request.head.repo.full_name || '' }}",
        })
        # ... and no EXECUTABLE line of the run body interpolates an expression
        # (comment lines may still name one for documentation).
        code = "\n".join(ln for ln in step["run"].splitlines()
                         if not ln.lstrip().startswith("#"))
        self.assertNotIn("${{", code, "expression interpolated into an executable line")

    def test_the_run_block_guards_the_bootstrap_case(self):
        """The checkout is the DEFAULT BRANCH, so the deriver is absent on the PR that
        introduces it (measured: this job failed exactly that way on #4170) and on any ref
        after a revert. The guard must exist, must exit 0, must be a ::warning (a genuine
        disappearance has to be visible, not look like a healthy no-op), and — the
        discriminating part — must guard the SAME path the step then executes."""
        run = next(s["run"] for s in self.steps if "run" in s)
        flat = " ".join(run.split())
        guards = re.findall(r"if \[ ! -f (\S+) \]; then", flat)
        self.assertEqual(len(guards), 1, "expected exactly one existence guard")
        invoked = re.search(r"python3 (\S+\.py)", flat)
        self.assertIsNotNone(invoked)
        self.assertEqual(guards[0], invoked.group(1),
                         "the guarded path and the invoked path must be the same file")
        self.assertIn("exit 0", flat)
        self.assertIn("::warning title=pr-area-label inert::", flat)
        self.assertNotIn("::notice title=pr-area-label inert::", flat)

    def test_deriver_and_policy_paths_referenced_by_the_workflow_exist(self):
        run = " ".join(" ".join(s["run"] for s in self.steps if "run" in s).split())
        script = next(tok for tok in run.split() if tok.endswith(".py"))
        self.assertTrue((REPO_ROOT / script).is_file(), f"{script} does not exist")

    # --- the privileged-token posture ---------------------------------------------
    def test_checkout_uses_default_branch_not_pr_head(self):
        """The job holds `pull-requests: write`; running PR-authored code under it is the
        GITHUB_ENV-class escalation. Checkout MUST be pinned to the default branch."""
        checkouts = [s for s in self.steps if "actions/checkout" in str(s.get("uses", ""))]
        self.assertEqual(len(checkouts), 1)
        with_ = checkouts[0].get("with") or {}
        self.assertEqual(with_.get("ref"), "${{ github.event.repository.default_branch }}")
        self.assertIs(with_.get("persist-credentials"), False)

    def test_permissions_are_exactly_the_least_privilege_set(self):
        """`issues: write` is present ONLY because `POST /repos/{o}/{r}/labels` — the
        explicit new-crate label creation (#4582) — offers no narrower scope; GitHub does
        not accept `pull-requests: write` for the repository label collection. Anything
        else appearing here (notably `contents: write` on a job that holds a write token
        and reads a PR's paths) must red."""
        self.assertEqual(self.wf["permissions"],
                         {"contents": "read", "issues": "write",
                          "pull-requests": "write"})
        for job in self.jobs.values():
            self.assertNotIn("permissions", job, "a job-level permissions block would "
                                                 "override the least-privilege default")

    def test_every_action_is_sha_pinned(self):
        unpinned = []
        for step in self.steps:
            uses = step.get("uses")
            if not uses:
                continue
            ref = uses.rsplit("@", 1)[-1]
            if len(ref) != 40 or not all(c in "0123456789abcdef" for c in ref):
                unpinned.append(uses)
        self.assertEqual(unpinned, [], f"actions not SHA-pinned: {unpinned}")

    def test_no_secret_other_than_the_scoped_github_token(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        bad = [ln.strip() for ln in text.splitlines()
               if "secrets." in ln and "secrets.GITHUB_TOKEN" not in ln]
        self.assertEqual(bad, [], f"unexpected secret reference: {bad}")

    def test_job_name_is_not_declared_advisory(self):
        """The gate excludes a check IFF it is in .github/advisory-registry.json; the name
        token alone is diagnostic. Pin the intended GATING posture from both sides."""
        self.assertNotIn("advisory", self.job["name"].lower())
        registry = json.loads((REPO_ROOT / ".github" / "advisory-registry.json")
                              .read_text(encoding="utf-8"))
        self.assertNotIn(self.job["name"], registry["jobs"])


class TestSelfTestIsActuallyInvoked(unittest.TestCase):
    """The call-site mutant class: a suite nobody runs cannot go red. routing-self-tests
    must INVOKE this file and the deriver's own `--self-test`, and must WATCH the paths
    that can break either."""

    @classmethod
    def setUpClass(cls):
        cls.wf = _yaml(ROUTING_WF)

    def _run_block(self):
        steps = self.wf["jobs"]["validate"]["steps"]
        return " ".join(" ".join(s["run"] for s in steps if "run" in s).split())

    def test_routing_self_tests_invokes_this_suite_and_the_deriver_self_test(self):
        run = self._run_block()
        self.assertIn("python3 scripts/tests/test_pr_area_labels.py", run)
        self.assertIn("python3 scripts/pr-area-labels.py --self-test", run)

    def test_routing_self_tests_watches_every_file_this_suite_pins(self):
        on = _on_block(self.wf)
        watched = set(on["pull_request"].get("paths") or []) & set(
            on["push"].get("paths") or [])
        for path in ("scripts/pr-area-labels.py", "ci/area-labels.toml",
                     "scripts/tests/test_pr_area_labels.py",
                     ".github/workflows/pr-area-label.yml"):
            self.assertIn(path, watched, f"{path} is not in BOTH paths filters — a change "
                                         f"to it would not run this suite")

    def test_the_validate_job_has_no_if_guard(self):
        self.assertNotIn("if", self.wf["jobs"]["validate"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
