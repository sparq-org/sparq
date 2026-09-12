#!/usr/bin/env python3
# [OPUS-4.8] Hermetic unit/dry-run tests for the flow-on engine (bead sq-ncvq.2,
# epic sq-ncvq). Authored by Opus 4.8 (Fable unavailable; flag for re-review when
# Fable returns).
#
# Fully hermetic: imports scripts/flow-on.py and exercises rule evaluation +
# idempotency-key generation against fixture diffs. NO live `gh` calls — we only
# drive `evaluate()` and `main(--dry-run)`, neither of which touches the network.
#
# Run:  python3 scripts/tests/test_flow_on.py
# (stdlib only; no pytest required.)

from __future__ import annotations

import importlib.util
import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
FLOW_ON = REPO_ROOT / "scripts" / "flow-on.py"
RULES = REPO_ROOT / "scripts" / "flow-on-rules.toml"


def _load_module():
    spec = importlib.util.spec_from_file_location("flow_on", FLOW_ON)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    # Register before exec so dataclasses can resolve forward annotations
    # (cls.__module__ lookup in _is_type) for the module-level @dataclass types.
    sys.modules["flow_on"] = mod
    spec.loader.exec_module(mod)
    return mod


flow_on = _load_module()


class RuleLoadingTest(unittest.TestCase):
    def test_rules_load_and_are_well_formed(self):
        rules = flow_on.load_rules(RULES)
        self.assertTrue(rules, "expected at least one rule")
        ids = {r.id for r in rules}
        # The taxonomy rules the design requires must all be present.
        for required in (
            "changed-public-feature-docs",
            "new-bench-dashboard-row",
            "competitor-feature-gather",
            # [OPUS-4.8] ZK circuit gate-count follow-on (a PR-description
            # deliverable, present in the rules file) — enforce the contract.
            "new-zk-circuit-gatecount",
        ):
            self.assertIn(required, ids)
        # [OPUS-4.8] (bead sq-l0a0) The new-crate bench/SKILL follow-on rule was
        # REMOVED — merge gate G1 (scripts/gate-new-crate.py) already enforces a
        # registered bench + README + SKILL in the SAME PR, so a post-merge
        # follow-on for them was pure redundant noise (minted #245/#246/#264/#265).
        self.assertNotIn("new-crate-bench-and-skill", ids)
        # Every rule has a trigger + at least one create template.
        for r in rules:
            self.assertTrue(
                r.when_paths or r.when_new_paths or r.when_label or r.when_title,
                f"{r.id} has no trigger",
            )
            self.assertTrue(r.creates, f"{r.id} has no create templates")


class NewCrateTest(unittest.TestCase):
    """[OPUS-4.8] (bead sq-l0a0) A new-crate PR must yield NO bench/SKILL
    follow-on: merge gate G1 already guarantees a registered bench + README +
    SKILL in the SAME PR, so the old reactive follow-on was pure redundant noise
    (minted #245/#246/#264/#265). The rule was removed."""

    def setUp(self):
        self.rules = flow_on.load_rules(RULES)

    def _eval(self, changed, added, labels=None, title="add sparq-foo crate"):
        return flow_on.evaluate(
            self.rules, 42, title, changed, added, labels or []
        )

    def test_new_crate_with_bench_and_skill_yields_no_redundant_follow_on(self):
        # A new crate that ships (as G1 requires) its bench + README + SKILL must
        # NOT mint a bench/SKILL follow-on — G1 owns that, in-PR.
        added = [
            "crates/sparq-foo/Cargo.toml",
            "crates/sparq-foo/src/lib.rs",
            "crates/sparq-foo/README.md",
        ]
        changed = added + ["bench/benchmarks.toml", "skills/sparq-foo/SKILL.md"]
        fos = self._eval(changed=changed, added=added)
        keys = {fo.dedup_key for fo in fos}
        self.assertNotIn("new-crate-bench-sparq-foo", keys)
        self.assertNotIn("new-crate-skill-sparq-foo", keys)
        self.assertNotIn("new-crate-bench-and-skill", {fo.rule_id for fo in fos})

    def test_new_crate_alone_yields_no_redundant_follow_on(self):
        # Even a bare new crate (which G1 would BLOCK at merge) mints no flow-on
        # follow-on — the redundant rule is gone.
        added = ["crates/sparq-foo/Cargo.toml", "crates/sparq-foo/src/lib.rs"]
        fos = self._eval(changed=added, added=added)
        self.assertNotIn("new-crate-bench-and-skill", {fo.rule_id for fo in fos})

    def test_modifying_existing_crate_file_does_not_fire_new_crate_rule(self):
        # Cargo.toml is CHANGED but not ADDED -> when_new_paths must not match.
        changed = ["crates/sparq-core/Cargo.toml", "crates/sparq-core/src/store/mod.rs"]
        added: list[str] = []
        fos = self._eval(changed=changed, added=added)
        self.assertNotIn("new-crate-bench-and-skill", {fo.rule_id for fo in fos})


class PerCrateFanOutTest(unittest.TestCase):
    """[OPUS-5] (#5242) `{crate}` must range over EVERY crate the PR added.

    build_context() resolves `{crate}` to a single crate, so a PR landing two new
    crates minted a `{crate}`-keyed follow-on for only the first — while merge gate
    G1 (`gate-new-crate.py::added_crates`) already iterates over all of them. These
    pin the fan-out and the two properties it must not break: a crate-agnostic
    template is still minted exactly once, and a single-crate PR is unchanged."""

    TWO_CRATES = [
        "crates/sparq-alpha/Cargo.toml",
        "crates/sparq-alpha/src/lib.rs",
        "crates/sparq-beta/Cargo.toml",
        "crates/sparq-beta/src/lib.rs",
    ]

    def _crate_rule(self):
        """A synthetic new-crate rule whose templates name `{crate}`."""
        return flow_on.Rule(
            id="synthetic-new-crate",
            when_new_paths=["crates/*/Cargo.toml"],
            creates=[
                flow_on.CreateTemplate(
                    dedup_key="synthetic-{crate}",
                    title="chore: follow up on new crate {crate}",
                    body="PR #{pr} added `crates/{crate}`.",
                )
            ],
        )

    def _crate_labelled_rule(self):
        """Same trigger, but `{crate}` also appears in a LABEL (which
        validate_crate_identity permits once dedup_key names it)."""
        return flow_on.Rule(
            id="synthetic-labelled",
            when_new_paths=["crates/*/Cargo.toml"],
            creates=[
                flow_on.CreateTemplate(
                    dedup_key="follow-up-{crate}",
                    title="chore: follow up on {crate}",
                    body="PR #{pr}.",
                    labels=["docs", "area:{crate}"],
                )
            ],
        )

    def _crate_agnostic_rule(self):
        """Same trigger, but no `{crate}` anywhere — must stay single-minted."""
        return flow_on.Rule(
            id="synthetic-pr-scoped",
            when_new_paths=["crates/*/Cargo.toml"],
            creates=[
                flow_on.CreateTemplate(
                    dedup_key="synthetic-pr-{pr}",
                    title="chore: one per PR",
                    body="PR #{pr}.",
                )
            ],
        )

    def test_two_new_crates_mint_a_follow_on_for_each(self):
        fos = flow_on.evaluate(
            [self._crate_rule()], 42, "add two crates",
            self.TWO_CRATES, self.TWO_CRATES, [],
        )
        self.assertEqual(
            sorted(fo.dedup_key for fo in fos),
            ["synthetic-sparq-alpha", "synthetic-sparq-beta"],
        )
        # The title and body are expanded per crate too, not just the key.
        self.assertEqual(
            sorted(fo.title for fo in fos),
            [
                "chore: follow up on new crate sparq-alpha",
                "chore: follow up on new crate sparq-beta",
            ],
        )
        for fo in fos:
            crate = fo.dedup_key.removeprefix("synthetic-")
            self.assertIn(f"`crates/{crate}`", fo.body)

    def test_per_crate_labels_are_expanded_not_left_literal(self):
        # validate_crate_identity() ACCEPTS `{crate}` in a label, so evaluate()
        # must expand it: an unexpanded label would put the literal (and invalid)
        # `area:{crate}` on both issues instead of the per-crate label.
        fos = flow_on.evaluate(
            [self._crate_labelled_rule()], 42, "add two crates",
            self.TWO_CRATES, self.TWO_CRATES, [],
        )
        by_key = {fo.dedup_key: fo for fo in fos}
        self.assertEqual(
            sorted(by_key), ["follow-up-sparq-alpha", "follow-up-sparq-beta"]
        )
        for key, fo in by_key.items():
            crate = key.removeprefix("follow-up-")
            self.assertIn(f"area:{crate}", fo.labels)
            self.assertIn("docs", fo.labels)
            self.assertEqual(
                [lb for lb in fo.labels if "{" in lb],
                [],
                f"unexpanded placeholder label on {key}: {fo.labels}",
            )

    def test_fan_out_covers_exactly_gate_g1s_added_crate_set(self):
        # The proactive (G1) and reactive (flow-on) halves must agree on WHICH
        # crates a diff adds — the disagreement #5242 reported.
        gnc = flow_on._gnc
        contexts = flow_on.build_contexts(42, "t", self.TWO_CRATES, self.TWO_CRATES)
        self.assertEqual(
            sorted({c["crate"] for c in contexts}),
            sorted(gnc.added_crates(self.TWO_CRATES)),
        )

    def test_crate_agnostic_template_is_still_minted_once(self):
        # Fanning out must not multiply a rule that never names {crate}: its
        # dedup_key is identical in every context, so it collapses.
        fos = flow_on.evaluate(
            [self._crate_agnostic_rule()], 42, "add two crates",
            self.TWO_CRATES, self.TWO_CRATES, [],
        )
        self.assertEqual([fo.dedup_key for fo in fos], ["synthetic-pr-42"])

    def test_single_new_crate_is_unchanged(self):
        added = ["crates/sparq-alpha/Cargo.toml", "crates/sparq-alpha/src/lib.rs"]
        fos = flow_on.evaluate(
            [self._crate_rule()], 42, "add one crate", added, added, [],
        )
        self.assertEqual([fo.dedup_key for fo in fos], ["synthetic-sparq-alpha"])

    def test_a_new_file_in_an_existing_crate_is_not_treated_as_a_new_crate(self):
        # Adding a module to sparq-core alongside a genuinely new crate: the base
        # {crate} resolution picks the EXISTING crate (it scans directories, not
        # Cargo.toml additions), so it must be replaced by the added-crate set —
        # otherwise the fan-out would mint a false "new crate sparq-core" follow-on.
        added = ["crates/sparq-core/src/new_mod.rs", "crates/sparq-new/Cargo.toml"]
        self.assertEqual(
            flow_on.build_context(42, "t", added, added)["crate"], "sparq-core"
        )
        contexts = flow_on.build_contexts(42, "t", added, added)
        self.assertEqual([c["crate"] for c in contexts], ["sparq-new"])
        fos = flow_on.evaluate([self._crate_rule()], 42, "t", added, added, [])
        self.assertEqual([fo.dedup_key for fo in fos], ["synthetic-sparq-new"])

    def test_no_added_crate_keeps_the_single_base_context(self):
        # A pure modification adds no crate, so there is nothing to fan out over
        # and {crate} still resolves from the changed pool exactly as before.
        changed = ["crates/sparq-core/src/store/mod.rs"]
        contexts = flow_on.build_contexts(42, "t", changed, [])
        self.assertEqual([c["crate"] for c in contexts], ["sparq-core"])

    def test_shipped_rules_are_unaffected_by_a_two_crate_diff(self):
        # Regression guard on the live rule table: none of its templates name
        # {crate}, so the fan-out must not change what a two-crate PR mints.
        rules = flow_on.load_rules(RULES)
        fos = flow_on.evaluate(rules, 42, "add two crates", self.TWO_CRATES, self.TWO_CRATES, [])
        keys = [fo.dedup_key for fo in fos]
        self.assertEqual(len(keys), len(set(keys)), f"duplicate follow-ons minted: {keys}")


class CrateIdentityRuleTest(unittest.TestCase):
    """[OPUS-5] (#5242) `{crate}` outside a crate-agnostic `dedup_key` is rejected.

    The fan-out expands per crate but evaluate() de-duplicates on the expanded
    dedup_key, so a template with `{crate}` in title/body and a crate-agnostic key
    collapses to the FIRST crate — silently dropping the rest. Minting them anyway
    is not an option: the key is the `flow-on-key` marker open_issue_exists()
    searches, so two issues sharing one key are indistinguishable across runs.
    Hence the identity rule, enforced at load time."""

    def _tmpl(self, **kw):
        base = dict(dedup_key="k-{pr}", title="t", body="b", labels=[])
        base.update(kw)
        return flow_on.CreateTemplate(**base)

    def test_crate_in_title_without_crate_in_key_is_rejected(self):
        with self.assertRaises(ValueError) as cm:
            flow_on.validate_crate_identity("r", self._tmpl(title="Follow up {crate}"))
        self.assertIn("title", str(cm.exception))
        self.assertIn("dedup_key", str(cm.exception))

    def test_crate_in_body_without_crate_in_key_is_rejected(self):
        with self.assertRaises(ValueError) as cm:
            flow_on.validate_crate_identity("r", self._tmpl(body="see crates/{crate}"))
        self.assertIn("body", str(cm.exception))

    def test_crate_in_a_label_without_crate_in_key_is_rejected(self):
        with self.assertRaises(ValueError) as cm:
            flow_on.validate_crate_identity("r", self._tmpl(labels=["docs", "area:{crate}"]))
        self.assertIn("labels[1]", str(cm.exception))

    def test_crate_in_key_permits_it_everywhere(self):
        flow_on.validate_crate_identity(
            "r",
            self._tmpl(
                dedup_key="k-{crate}", title="{crate}", body="{crate}", labels=["area:{crate}"]
            ),
        )

    def test_crate_agnostic_template_is_permitted(self):
        flow_on.validate_crate_identity("r", self._tmpl())

    def test_load_rules_rejects_an_incoherent_template(self):
        # End-to-end through the only production entry point for rules: a rule file
        # carrying the gap must fail to load rather than mint a partial fan-out.
        toml = """
[[rule]]
id = "bad-per-crate"
when_new_paths = ["crates/*/Cargo.toml"]
[[rule.create]]
dedup_key = "follow-up-{pr}"
title = "Follow up {crate}"
body = "PR #{pr}."
"""
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "rules.toml"
            path.write_text(toml)
            with self.assertRaises(ValueError) as cm:
                flow_on.load_rules(path)
        self.assertIn("bad-per-crate", str(cm.exception))

    def test_shipped_rules_satisfy_the_identity_rule(self):
        # load_rules() validates, so this passing IS the assertion for the live table.
        self.assertTrue(flow_on.load_rules(RULES))


class ChangedSurfaceTest(unittest.TestCase):
    def setUp(self):
        self.rules = flow_on.load_rules(RULES)

    def test_cli_pub_change_without_skill_fires_docs_rule(self):
        # [OPUS-4.8] (bead sq-l0a0) A NET public-API change (the path is in
        # pub_changed) without a SKILL.md DOES fire — the genuinely-useful case.
        changed = ["crates/sparq-cli/src/main.rs"]
        fos = flow_on.evaluate(
            self.rules, 7, "add --explain flag", changed, [], [],
            pub_changed={"crates/sparq-cli/src/main.rs"},
        )
        self.assertIn("changed-public-feature-docs", {fo.rule_id for fo in fos})

    def test_cli_comment_only_change_does_not_fire_docs_rule(self):
        # [OPUS-4.8] (bead sq-l0a0) REGRESSION (mis-fired #250): a comment-only /
        # lint-attribute-only edit to a public surface is NOT a net public-API
        # change (the path is absent from pub_changed), so it mints NO skill-sync
        # follow-on — even without a SKILL.md in the diff.
        changed = ["crates/sparq-cli/src/main.rs"]
        fos = flow_on.evaluate(
            self.rules, 250, "add SAFETY comments + #![warn] attrs", changed, [], [],
            pub_changed=set(),
        )
        self.assertNotIn("changed-public-feature-docs", {fo.rule_id for fo in fos})

    def test_cli_change_with_skill_update_suppresses_docs_rule(self):
        # A SKILL.md in the same diff means the docs were synced → suppress, even
        # on a real pub change.
        changed = ["crates/sparq-cli/src/main.rs", "skills/cli/SKILL.md"]
        fos = flow_on.evaluate(
            self.rules, 8, "add --explain flag", changed, [], [],
            pub_changed={"crates/sparq-cli/src/main.rs"},
        )
        self.assertNotIn("changed-public-feature-docs", {fo.rule_id for fo in fos})


class PubApiDiffPlumbingTest(unittest.TestCase):
    """[OPUS-4.8] (bead sq-l0a0) The docs rule reuses gate G2's shared pub-item
    multiset diff (scripts/pub_api_diff.py). Verify the patch→pub_changed plumbing
    distinguishes a genuine new `pub fn` from a comment/attr/relocation diff."""

    def test_pub_changed_from_patch_detects_new_pub_fn(self):
        patch = (
            "diff --git a/crates/sparq-cli/src/main.rs b/crates/sparq-cli/src/main.rs\n"
            "--- a/crates/sparq-cli/src/main.rs\n"
            "+++ b/crates/sparq-cli/src/main.rs\n"
            "@@ -1,0 +1,1 @@\n"
            "+pub fn brand_new_flag() {}\n"
        )
        self.assertEqual(
            flow_on.pub_changed_from_patch(patch),
            {"crates/sparq-cli/src/main.rs"},
        )

    def test_pub_changed_from_patch_ignores_comment_and_attr_only(self):
        patch = (
            "diff --git a/crates/sparq-cli/src/main.rs b/crates/sparq-cli/src/main.rs\n"
            "--- a/crates/sparq-cli/src/main.rs\n"
            "+++ b/crates/sparq-cli/src/main.rs\n"
            "@@ -1,0 +1,2 @@\n"
            "+// SAFETY: this is sound because ...\n"
            "+#![warn(clippy::all)]\n"
        )
        self.assertEqual(flow_on.pub_changed_from_patch(patch), set())

    def test_pub_changed_from_patch_ignores_pure_relocation(self):
        patch = (
            "diff --git a/crates/sparq-cli/src/main.rs b/crates/sparq-cli/src/main.rs\n"
            "--- a/crates/sparq-cli/src/main.rs\n"
            "+++ b/crates/sparq-cli/src/main.rs\n"
            "@@ -10,1 +10,0 @@\n"
            "-pub fn moved() {}\n"
            "@@ -40,0 +50,1 @@\n"
            "+pub fn moved() {}\n"
        )
        self.assertEqual(flow_on.pub_changed_from_patch(patch), set())


class CompetitorLabelTest(unittest.TestCase):
    def setUp(self):
        self.rules = flow_on.load_rules(RULES)

    def test_engine_change_requires_label(self):
        changed = ["crates/sparq-engine/src/exec/join.rs"]
        # Without the label, the gather rule must NOT fire.
        fos = flow_on.evaluate(self.rules, 9, "faster joins", changed, [], [])
        self.assertNotIn("competitor-feature-gather", {fo.rule_id for fo in fos})
        # With the label, it fires.
        fos = flow_on.evaluate(
            self.rules, 9, "faster joins", changed, [], ["competitor-relevant"]
        )
        self.assertIn("competitor-feature-gather", {fo.rule_id for fo in fos})


class NewBenchTest(unittest.TestCase):
    def setUp(self):
        self.rules = flow_on.load_rules(RULES)

    def test_new_bench_suite_triggers_dashboard_row(self):
        added = ["bench/watdiv/benchmarks.toml", "bench/watdiv/run.sh"]
        fos = flow_on.evaluate(self.rules, 11, "add watdiv bench", added, added, [])
        dash = [fo for fo in fos if fo.rule_id == "new-bench-dashboard-row"]
        self.assertTrue(dash)
        self.assertEqual(dash[0].dedup_key, "dashboard-row-watdiv")
        self.assertIn("watdiv", dash[0].title)

    def test_unfeatured_agent_token_harnesses_do_not_trigger_dashboard_row(self):
        # [GPT-5.6] These registry entries explicitly set featured=false: their
        # NON-CANONICAL token A/B verdicts never feed the performance dashboard.
        for suite in ("pkg-dogfood", "terse"):
            with self.subTest(suite=suite):
                added = [f"bench/{suite}/run.sh"]
                fos = flow_on.evaluate(
                    self.rules, 11, f"add {suite} research harness", added, added, []
                )
                self.assertNotIn(
                    "new-bench-dashboard-row", {fo.rule_id for fo in fos}
                )

    def test_unregistered_harness_does_not_trigger_dashboard_row(self):
        added = ["bench/token-research-spike/run.sh"]
        fos = flow_on.evaluate(self.rules, 11, "add token spike", added, added, [])
        self.assertNotIn("new-bench-dashboard-row", {fo.rule_id for fo in fos})


class ZkCircuitGatecountTest(unittest.TestCase):
    """[OPUS-4.8] sq-fyzq7 (#1655): the `new-zk-circuit-gatecount` rule must fire
    ONLY for a genuinely-new top-level `zk/<family>/` circuit — never for a new
    member inside the existing `zk/compose/` family (those are already covered by
    the hard `snapshot_covers_every_member` gate) — and must name the circuit from
    `zk/`, never from a `bench/` output dir."""

    def setUp(self):
        self.rules = flow_on.load_rules(RULES)

    def _zk_follow_ons(self, fos):
        return [fo for fo in fos if fo.rule_id == "new-zk-circuit-gatecount"]

    def test_new_compose_member_does_not_fire(self):
        # REGRESSION (mis-fired #1655): merged #1608 added `zk/compose/<member>/`
        # path_reach members (already baselined + covered by
        # snapshot_covers_every_member) and modified the bench gate-count JSON.
        # `fnmatch`'s `*` crosses `/`, so `zk/*/Nargo.toml` matched the two-level
        # member manifest; `exclude_new_paths = ["zk/compose/**"]` now drops it.
        added = [
            "zk/compose/path_reach_d2_k1_n16/Nargo.toml",
            "zk/compose/path_reach_d2_k1_n16/src/main.nr",
        ]
        changed = added + [
            "bench/zk-compose/gate_counts_latest.json",  # MODIFIED, not added
            "crates/sparq-zk-compose/tests/gate_count_snapshot.json",
        ]
        fos = flow_on.evaluate(
            self.rules, 1608, "feat(zk/compose): path_reach family", changed, added, []
        )
        self.assertEqual(self._zk_follow_ons(fos), [])
        # And no phantom `zk-compose` key leaks from any rule.
        self.assertNotIn("zk-gatecount-zk-compose", {fo.dedup_key for fo in fos})

    def test_new_top_level_family_fires_with_zk_derived_name(self):
        # A genuinely-new top-level family under zk/ DOES fire, and the circuit is
        # named from `zk/` (here `arith`), NOT from a co-changed `bench/` dir.
        added = ["zk/arith/Nargo.toml", "zk/arith/src/main.nr"]
        changed = added + ["bench/zk-compose/gate_counts_latest.json"]
        fos = flow_on.evaluate(
            self.rules, 2000, "feat(zk): arith circuit", changed, added, []
        )
        zk = self._zk_follow_ons(fos)
        self.assertEqual(len(zk), 1)
        self.assertEqual(zk[0].dedup_key, "zk-gatecount-arith")
        self.assertIn("zk/arith", zk[0].body)
        # The bench dir name must NOT contaminate the zk circuit name.
        self.assertNotIn("zk-compose", zk[0].dedup_key)
        self.assertNotIn("zk/zk-compose", zk[0].body)

    def test_new_family_with_member_subdir_fires_once(self):
        # A new WORKSPACE family (root + a member circuit) still yields exactly one
        # follow-on, named for the family root.
        added = [
            "zk/lattice/Nargo.toml",
            "zk/lattice/scan_a/Nargo.toml",
            "zk/lattice/scan_a/src/main.nr",
        ]
        fos = flow_on.evaluate(
            self.rules, 2100, "feat(zk): lattice family", added, added, []
        )
        zk = self._zk_follow_ons(fos)
        self.assertEqual(len(zk), 1)
        self.assertEqual(zk[0].dedup_key, "zk-gatecount-lattice")

    def test_modified_only_zk_family_does_not_fire(self):
        # Editing an EXISTING top-level circuit (changed, not added) must not fire
        # the new-family rule.
        changed = ["zk/arith/src/main.nr"]
        fos = flow_on.evaluate(
            self.rules, 2200, "tweak arith circuit", changed, [], []
        )
        self.assertEqual(self._zk_follow_ons(fos), [])


class RoutingLabelsTest(unittest.TestCase):
    """[FABLE-5] (#2474) GITHUB_TOKEN-created issues fire no `issues: opened`
    event, so triage-issue.yml can never label a flow-on follow-up post-hoc.
    Every minted follow-on must therefore be dispatch-ready AT CREATION: exactly
    one `role:*`, exactly one `priority:*`, and `status:ready` — computed by the
    shared static triage pass (scripts/triage.py) with a default priority:P3."""

    def setUp(self):
        self.rules = flow_on.load_rules(RULES)

    def _all_follow_ons(self):
        # One evaluate() per rule family so every template in the table is minted.
        fixtures = [
            # changed-public-feature-docs
            dict(changed=["crates/sparq-cli/src/main.rs"], added=[], labels=[],
                 title="add --explain flag",
                 pub_changed={"crates/sparq-cli/src/main.rs"}),
            # new-bench-dashboard-row
            dict(changed=["bench/watdiv/benchmarks.toml"],
                 added=["bench/watdiv/benchmarks.toml"], labels=[], title="bench",
                 pub_changed=None),
            # competitor-feature-gather
            dict(changed=["crates/sparq-engine/src/exec/join.rs"], added=[],
                 labels=["competitor-relevant"], title="joins", pub_changed=None),
            # new-zk-circuit-gatecount
            dict(changed=["zk/arith/Nargo.toml"], added=["zk/arith/Nargo.toml"],
                 labels=[], title="zk arith", pub_changed=None),
        ]
        out = []
        for fx in fixtures:
            out += flow_on.evaluate(
                self.rules, 99, fx["title"], fx["changed"], fx["added"],
                fx["labels"], fx["pub_changed"],
            )
        return out

    def test_every_follow_on_is_dispatch_ready_at_creation(self):
        fos = self._all_follow_ons()
        self.assertTrue(fos, "fixture should mint at least one follow-on per rule")
        for fo in fos:
            roles = [lb for lb in fo.labels if lb.startswith("role:")]
            prios = [lb for lb in fo.labels if lb.startswith("priority:")]
            self.assertEqual(len(roles), 1, f"{fo.rule_id}: {fo.labels}")
            self.assertEqual(len(prios), 1, f"{fo.rule_id}: {fo.labels}")
            self.assertIn("status:ready", fo.labels, fo.rule_id)
            self.assertNotIn("status:untriaged", fo.labels, fo.rule_id)

    def test_roles_derive_from_rule_kind_labels(self):
        by_rule = {fo.rule_id: fo for fo in self._all_follow_ons()}
        # kind:docs → role:docs; kind:perf → role:perf; the zk security keyword
        # forces role:soundness; default priority:P3 everywhere (no rule sets one).
        self.assertIn("role:docs", by_rule["changed-public-feature-docs"].labels)
        self.assertIn("role:docs", by_rule["new-bench-dashboard-row"].labels)
        self.assertIn("role:perf", by_rule["competitor-feature-gather"].labels)
        self.assertIn("role:soundness", by_rule["new-zk-circuit-gatecount"].labels)
        for fo in by_rule.values():
            self.assertIn("priority:P3", fo.labels, fo.rule_id)

    def test_dry_run_prints_routing_labels(self):
        # The dry-run plan is how a human previews a mint — it must show that the
        # issue will be born routed (labels line includes status:ready).
        with tempfile.TemporaryDirectory() as td:
            changed = Path(td) / "changed.txt"
            changed.write_text("bench/watdiv/benchmarks.toml\nbench/watdiv/run.sh\n")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(
                    ["--pr", "42", "--dry-run", "--changed-files", str(changed),
                     "--added-files", str(changed), "--title", "add widgets bench"]
                )
            out = buf.getvalue()
        self.assertEqual(rc, 0)
        self.assertIn("status:ready", out)
        self.assertIn("priority:P3", out)


class IdempotencyTest(unittest.TestCase):
    """The dedup-key marker is what open_issue_exists() searches for; verify the
    key is stable + embedded so a re-run is a no-op once the issue is open."""

    def setUp(self):
        self.rules = flow_on.load_rules(RULES)

    def test_dedup_key_stable_across_runs(self):
        # [OPUS-4.8] (bead sq-l0a0) Repointed off the removed new-crate rule onto
        # the new-bench-dashboard-row follow-on, which still fires and produces a
        # stable, non-empty dedup key.
        added = ["bench/watdiv/benchmarks.toml"]
        a = flow_on.evaluate(self.rules, 42, "t", added, added, [])
        b = flow_on.evaluate(self.rules, 42, "t", added, added, [])
        keys_a = sorted(fo.dedup_key for fo in a)
        self.assertIn("dashboard-row-watdiv", keys_a)
        self.assertEqual(keys_a, sorted(fo.dedup_key for fo in b))

    def test_marker_round_trips(self):
        key = "dashboard-row-widgets"
        body = "some body\n\n" + flow_on.key_marker(key)
        # open_issue_exists greps for exactly this marker substring.
        self.assertIn(flow_on.key_marker(key), body)


class DryRunTest(unittest.TestCase):
    """main(--dry-run) must print the would-create plan and make NO gh calls.

    We provide all inputs hermetically (changed-files/title) so fetch_pr_inputs
    is never reached, and --dry-run skips open_issue_exists/create_issue."""

    def test_dry_run_on_new_bench_suite_fixture(self):
        # [OPUS-4.8] (bead sq-l0a0) Repointed off the removed new-crate rule onto
        # the surviving new-bench-suite → dashboard-row follow-on.
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            changed = Path(td) / "changed.txt"
            changed.write_text("bench/watdiv/benchmarks.toml\nbench/watdiv/run.sh\n")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(
                    [
                        "--pr",
                        "42",
                        "--dry-run",
                        "--changed-files",
                        str(changed),
                        "--added-files",
                        str(changed),
                        "--title",
                        "add watdiv bench suite",
                    ]
                )
            out = buf.getvalue()
        self.assertEqual(rc, 0)
        self.assertIn("[dry-run] WOULD create", out)
        self.assertIn("dashboard-row-watdiv", out)

    def test_dry_run_on_new_crate_with_g1_artifacts_is_clean(self):
        # [OPUS-4.8] (bead sq-l0a0) A new-crate PR that ships its G1 artifacts
        # (bench + README + SKILL) triggers NO redundant flow-on follow-on.
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            changed = Path(td) / "changed.txt"
            changed.write_text(
                "crates/sparq-foo/Cargo.toml\ncrates/sparq-foo/src/lib.rs\n"
                "crates/sparq-foo/README.md\nbench/benchmarks.toml\n"
                "skills/sparq-foo/SKILL.md\n"
            )
            added = Path(td) / "added.txt"
            added.write_text("crates/sparq-foo/Cargo.toml\ncrates/sparq-foo/src/lib.rs\n")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(
                    [
                        "--pr",
                        "245",
                        "--dry-run",
                        "--changed-files",
                        str(changed),
                        "--added-files",
                        str(added),
                        "--title",
                        "add sparq-foo crate (with bench+SKILL, as G1 requires)",
                    ]
                )
            out = buf.getvalue()
        self.assertEqual(rc, 0)
        self.assertNotIn("new-crate-bench", out)
        self.assertNotIn("new-crate-skill", out)

    def test_dry_run_no_match_is_clean(self):
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            changed = Path(td) / "changed.txt"
            changed.write_text("README.md\n")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(
                    ["--pr", "1", "--dry-run", "--changed-files", str(changed), "--title", "docs"]
                )
            out = buf.getvalue()
        self.assertEqual(rc, 0)
        self.assertIn("no rules triggered", out)

    def test_hermetic_without_added_files_does_not_fire_new_path_rules(self):
        # [OPUS-4.8] Regression: when --added-files is omitted, `added` must
        # default to EMPTY, not to all changed files — otherwise a MODIFIED
        # benchmarks.toml would falsely fire the new-bench (when_new_paths)
        # rule. Here it is changed-but-not-added.
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            changed = Path(td) / "changed.txt"
            changed.write_text("bench/widgets/benchmarks.toml\n")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(
                    [
                        "--pr",
                        "7",
                        "--dry-run",
                        "--changed-files",
                        str(changed),
                        "--title",
                        "tweak widgets bench",
                    ]
                )
            out = buf.getvalue()
        self.assertEqual(rc, 0)
        self.assertNotIn("dashboard-row-widgets", out)
        self.assertIn("no rules triggered", out)

    def test_hermetic_comment_only_public_surface_is_clean(self):
        # [OPUS-4.8] (bead sq-l0a0) End-to-end via main(): a public-surface src
        # change WITHOUT --pub-diff-file (so pub_changed defaults to empty) mints
        # NO skill-sync follow-on — the #250 mis-fire is fixed at the CLI layer.
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            changed = Path(td) / "changed.txt"
            changed.write_text("crates/sparq-cli/src/main.rs\n")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(
                    ["--pr", "250", "--dry-run", "--changed-files", str(changed),
                     "--title", "add SAFETY comments"]
                )
            out = buf.getvalue()
        self.assertEqual(rc, 0)
        self.assertIn("no rules triggered", out)

    def test_hermetic_pub_change_fires_skill_sync(self):
        # [OPUS-4.8] (bead sq-l0a0) With --pub-diff-file marking a net pub change,
        # the skill-sync follow-on DOES fire (the useful case is preserved).
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            changed = Path(td) / "changed.txt"
            changed.write_text("crates/sparq-cli/src/main.rs\n")
            pub = Path(td) / "pub.txt"
            pub.write_text("crates/sparq-cli/src/main.rs\n")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(
                    ["--pr", "251", "--dry-run", "--changed-files", str(changed),
                     "--pub-diff-file", str(pub), "--title", "add --explain flag"]
                )
            out = buf.getvalue()
        self.assertEqual(rc, 0)
        self.assertIn("skill-sync-pr-251", out)


# --------------------------------------------------------------------------- #
# CI guard (bead sq-z0se) — refuse to MINT issues outside CI
# --------------------------------------------------------------------------- #
class CiGuardTest(unittest.TestCase):
    """The issue-MINTING path must refuse to run outside CI; an accidental
    dev-box run of the sibling drift scanner minted 20 spurious issues
    (#397-416). --dry-run is NEVER guarded (it makes zero gh calls)."""

    def _clear_ci_env(self) -> None:
        backup = {k: os.environ.get(k) for k in ("GITHUB_ACTIONS", "CI",
                                                 "FLOW_ON_ALLOW_LOCAL")}

        def restore() -> None:
            for k, v in backup.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

        self.addCleanup(restore)
        for k in backup:
            os.environ.pop(k, None)

    def _firing_inputs(self, td: str) -> list[str]:
        # A net pub-API change to a watched surface fires the skill-sync rule, so
        # `follow_ons` is non-empty and the guard is reached on the write path.
        changed = Path(td) / "changed.txt"
        changed.write_text("crates/sparq-cli/src/main.rs\n")
        pub = Path(td) / "pub.txt"
        pub.write_text("crates/sparq-cli/src/main.rs\n")
        return ["--pr", "999", "--changed-files", str(changed),
                "--pub-diff-file", str(pub), "--title", "add --explain flag"]

    def test_running_in_ci_detects_markers(self):
        self.assertTrue(flow_on.running_in_ci({"GITHUB_ACTIONS": "true"}))
        self.assertTrue(flow_on.running_in_ci({"CI": "1"}))

    def test_running_in_ci_false_when_unset_or_falsey(self):
        self.assertFalse(flow_on.running_in_ci({}))
        self.assertFalse(flow_on.running_in_ci({"CI": ""}))

    def test_require_ci_passes_in_ci(self):
        flow_on.require_ci({"GITHUB_ACTIONS": "true"})  # no raise

    def test_require_ci_passes_with_escape_hatch(self):
        flow_on.require_ci({"FLOW_ON_ALLOW_LOCAL": "1"})  # no raise

    def test_require_ci_refuses_outside_ci(self):
        err = io.StringIO()
        with redirect_stderr(err), self.assertRaises(SystemExit) as cm:
            flow_on.require_ci({})
        self.assertEqual(cm.exception.code, 2)
        self.assertIn("refusing to file GitHub issues outside CI", err.getvalue())

    def test_main_mint_path_refuses_outside_ci_before_any_gh_call(self):
        # A real (non-dry-run) run that triggers a rule must SystemExit on the
        # guard rather than shelling out to `gh` to mint issues.
        self._clear_ci_env()
        with tempfile.TemporaryDirectory() as td:
            buf, err = io.StringIO(), io.StringIO()
            with redirect_stdout(buf), redirect_stderr(err), self.assertRaises(
                SystemExit
            ) as cm:
                flow_on.main(self._firing_inputs(td))  # NO --dry-run
        self.assertEqual(cm.exception.code, 2)
        self.assertIn("refusing to file GitHub issues outside CI", err.getvalue())

    def test_main_dry_run_never_guarded_outside_ci(self):
        # --dry-run must succeed with no CI markers (local previews / tests).
        self._clear_ci_env()
        with tempfile.TemporaryDirectory() as td:
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = flow_on.main(self._firing_inputs(td) + ["--dry-run"])
        self.assertEqual(rc, 0)
        self.assertIn("skill-sync-pr-999", buf.getvalue())


if __name__ == "__main__":
    unittest.main(verbosity=2)
