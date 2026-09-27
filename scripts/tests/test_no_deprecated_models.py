#!/usr/bin/env python3
# [OPUS-5] Regression guard for the 2026-07-26 model deprecation.
"""test_no_deprecated_models.py — a deprecated model may not occupy ANY routing position.

MAINTAINER DIRECTIVE 2026-07-26: "deprecate the use of fable and opus entirely in favour of opus5"
and "deprecate sonnet and haiku for docs writing in favor of gpt 5.6 sol".

Deleting the aliases from orchestration/routing.toml once is an EDIT; it decays the moment someone
copies an old chain back in. This suite is what makes it an INVARIANT, and it is deliberately
CROSS-SURFACE, because the routing table is not the only place a model gets selected:

  1. orchestration/routing.toml      — the issue-native routing table (chains + catalog)
  2. .claude/settings.json           — PreToolUse `type: "agent"` hooks carry their own `model`
  3. .claude/workflows/*.js          — `agent({model: ...})` dispatch sites + the TIER table
  4. .claude/agents/*.md             — role-agent frontmatter `model:`

Surface 2 is why this file exists rather than a routing-table-only assertion: the sparq-perf-reviewer
arm gate sat at `"model": "opus"` — a BARE ALIAS — long after the routing table had moved to opus5.
A routing-table-scoped guard would have stayed green while the live PR-arming gate ran on the
deprecated model.

BARE ALIASES ARE THEMSELVES A FINDING. `opus` resolved to claude-opus-4-8 for days after Opus 5
shipped, so "it points at the right model today" is not a property you can assert about an alias.
Every routing position must name a FULL model id.
"""
import importlib.util
import json
import re
import sys
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "routing-self-tests.yml"
TRIAGE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "triage-issue.yml"

# Retired 2026-07-26. Both halves matter: the ALIAS (what a chain writes) and the concrete PROVIDER
# ID (what the alias resolved to) — banning only the alias lets the model return under a new name.
DEPRECATED_ALIASES = {"fable", "opus"}
DEPRECATED_PROVIDER_MODELS = {"claude-fable-5", "claude-opus-4-8"}

# Aliases whose target has moved before and can move again. A routing position must not use them.
BARE_ALIASES = {"opus", "fable", "sonnet", "haiku", "opus5", "sol", "luna", "terra"}

# The surviving top tier, probe-verified 2026-07-26 (`claude --model claude-opus-5 -p` -> OK,
# modelUsage canonicalModel "claude-opus-5").
OPUS5_ID = "claude-opus-5"


def _routing_doc():
    try:
        import tomllib
    except ModuleNotFoundError:  # pragma: no cover
        import tomli as tomllib
    with open(REPO_ROOT / "orchestration" / "routing.toml", "rb") as fh:
        return tomllib.load(fh)


def _load_script(mod_name, filename):
    """Import a hyphenated scripts/*.py module (same shape as dispatch-plan.py's loader)."""
    spec = importlib.util.spec_from_file_location(mod_name, REPO_ROOT / "scripts" / filename)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class TestRoutingTable(unittest.TestCase):
    """Surface 1 — orchestration/routing.toml."""

    @classmethod
    def setUpClass(cls):
        cls.doc = _routing_doc()

    def _chains(self):
        yield "defaults", list(self.doc.get("defaults", {}).get("model_chain", []))
        for r in self.doc.get("route", []):
            where = r.get("role") or ",".join(r.get("match_labels", [])) or "<unnamed>"
            yield where, list(r.get("model_chain", []))

    def test_no_chain_names_a_deprecated_alias(self):
        """MUTANT: put `fable` or `opus` back in any model_chain => RED."""
        for where, chain in self._chains():
            self.assertEqual(
                sorted(set(chain) & DEPRECATED_ALIASES), [],
                f"{where}: model_chain names a model retired on 2026-07-26; use 'opus5'")

    def test_catalog_defines_no_deprecated_alias_or_provider_id(self):
        """MUTANT: re-add [models.fable], or pin claude-opus-4-8 under any alias => RED."""
        for name, spec in self.doc.get("models", {}).items():
            self.assertNotIn(name, DEPRECATED_ALIASES,
                             f"[models.{name}] is a retired alias")
            self.assertNotIn(
                spec.get("provider_model"), DEPRECATED_PROVIDER_MODELS,
                f"[models.{name}] pins a retired provider id under a non-retired alias name")

    def test_opus5_is_the_sole_anthropic_routing_tier(self):
        """Every anthropic model REACHABLE from a chain must be opus5. This is the positive form
        of the rule: the negative tests above ban two known names, this one bans an unknown third."""
        reachable = {m for _, chain in self._chains() for m in chain}
        anthropic = {m for m in reachable
                     if self.doc["models"].get(m, {}).get("provider") == "anthropic"}
        self.assertEqual(anthropic, {"opus5"},
                         "opus5 must be the only anthropic model reachable from a routing chain")

    def test_opus5_pins_the_full_probe_verified_id(self):
        self.assertEqual(self.doc["models"]["opus5"]["provider_model"], OPUS5_ID)

    def test_docs_chain_leads_with_sol_and_names_no_cheap_anthropic_tier(self):
        """MUTANT: put haiku/sonnet back at the head of role=docs => RED.

        This is the SECOND half of the directive ("deprecate sonnet and haiku for docs writing in
        favor of gpt 5.6 sol"). Scoped to role=docs on purpose: haiku and sonnet are NOT banned
        from the repo — they still serve non-docs-writing roles in .claude/agents (mechanical
        retrieval, mechanical verification, bulk implementation, triage, context monitoring),
        which the maintainer did not deprecate.
        """
        docs = [r for r in self.doc["route"] if r.get("role") == "docs"]
        self.assertEqual(len(docs), 1, "exactly one role=docs route expected")
        chain = docs[0]["model_chain"]
        self.assertEqual(chain[0], "sol", "docs writing must lead with sol (gpt-5.6)")
        self.assertTrue(docs[0].get("escalate"),
                        "docs writing needs a bounded exit when every provider is unavailable")
        self.assertEqual(sorted(set(chain) & {"haiku", "sonnet"}), [],
                         "docs writing was moved off the cheap anthropic tiers on 2026-07-26")

    def test_no_chain_is_empty(self):
        """An escalation chain that cannot escalate is worse than a deprecated rung: it dead-ends
        silently. Every chain must retain at least one reachable model."""
        for where, chain in self._chains():
            self.assertTrue(chain, f"{where}: model_chain is empty — it can never dispatch")

    def test_single_model_chains_terminate_explicitly(self):
        """Collapsing ["opus5", "opus"] onto ["opus5"] removed a rung. A one-rung chain is only
        safe if exhaustion has a DEFINED exit — `escalate = true` (park to a human) — or if the
        chain is cross-provider (another provider's outage cannot starve it). Otherwise a single
        capacity outage stalls the role with no escape."""
        for r in self.doc.get("route", []):
            chain = r.get("model_chain", [])
            if len(chain) > 1:
                continue
            where = r.get("role") or ",".join(r.get("match_labels", []))
            providers = {self.doc["models"][m]["provider"] for m in chain}
            self.assertTrue(
                r.get("escalate") or len(providers) > 1,
                f"{where}: single-model chain {chain} without `escalate = true` — on chain "
                f"exhaustion it can only defer forever with no human exit")


class TestProviderPreference(unittest.TestCase):
    """[SPARQ agent] The 2026-09-02 protocol makes model responsibility explicit.

    Sol is the primary default implementation tier. Opus 5 is retained behind it for continuity
    with current Anthropic-authored fixes and total Sol unavailability, and is the sole
    review/soundness tier. These tests pin whole chains rather than only their first elements.
    """

    IMPLEMENTATION_ROLES = ("impl", "site", "gui", "ci", "perf")
    GUI_ROLE = "gui"

    @classmethod
    def setUpClass(cls):
        cls.doc = _routing_doc()
        cls.routes = {r["role"]: r for r in cls.doc["route"] if r.get("role")}

    def test_implementation_routes_are_sol_first(self):
        """MUTANT: add or substitute any implementation tier => RED."""
        for role in self.IMPLEMENTATION_ROLES:
            self.assertEqual(self.routes[role]["model_chain"], ["sol", "opus5"],
                             f"role:{role} must be Sol-first implementation")

    def test_defaults_chain_is_sol_first(self):
        self.assertEqual(self.doc["defaults"]["model_chain"], ["sol", "opus5"])
        self.assertTrue(self.doc["defaults"].get("escalate"))

    def test_review_and_soundness_routes_are_opus5_only(self):
        for role in ("review", "soundness"):
            self.assertEqual(self.routes[role]["model_chain"], ["opus5"])
            self.assertTrue(self.routes[role].get("escalate"))

    def test_implementation_routes_preserve_cross_provider_continuity(self):
        for role in self.IMPLEMENTATION_ROLES:
            providers = {self.doc["models"][m]["provider"]
                         for m in self.routes[role]["model_chain"]}
            self.assertEqual(providers, {"anthropic", "openai"})

    def test_implementation_routes_have_an_explicit_exhaustion_exit(self):
        """Total implementation-chain exhaustion must park visibly rather than defer forever."""
        for role in self.IMPLEMENTATION_ROLES:
            self.assertTrue(self.routes[role].get("escalate"),
                            f"role:{role} needs a bounded exit when both providers are unavailable")

    def test_gui_carve_out_keeps_sol_first(self):
        """MUTANT: delete the role:gui route, or flip it to opus5-first => RED."""
        self.assertIn(self.GUI_ROLE, self.routes,
                      "the area:gui carve-out route is missing entirely")
        chain = self.routes[self.GUI_ROLE]["model_chain"]
        self.assertEqual(chain[0], "sol",
                         "GUI work keeps sol first (original-builder steer, task #331)")

    def test_gui_route_keeps_the_continuity_fallback(self):
        self.assertEqual(self.routes[self.GUI_ROLE]["model_chain"], ["sol", "opus5"])

    def test_carve_out_selector_is_exactly_area_gui(self):
        """THE ONE THAT MATTERS MOST. "GUI" informally reads as covering the site surfaces, so the
        likely future mistake is widening the sol carve-out back over `site*`. The carve-out is
        `area:gui` and nothing else (maintainer: "Let's just go with area:gui work").

        MUTANT: add "area:site" (or any site* label) to triage's GUI_SURFACE_LABELS => RED.
        """
        triage_src = (REPO_ROOT / "scripts" / "triage.py").read_text(encoding="utf-8")
        m = re.search(r"(?m)^GUI_SURFACE_LABELS = \(([^)]*)\)", triage_src)
        self.assertIsNotNone(m, "GUI_SURFACE_LABELS not found or reshaped")
        labels = [x.strip().strip("\"'") for x in m.group(1).split(",") if x.strip()]
        self.assertEqual(labels, ["area:gui"],
                         "the sol carve-out selector must be EXACTLY area:gui — no site* label, "
                         "no surface:frontend, no dashboard")

    def test_site_surfaces_are_not_in_the_carve_out(self):
        """The same property from the other side: the generic UI label set (which routes to
        role:site) must not contain area:gui, and the gui set must not contain any
        site* label. MUTANT: move area:gui back into UI_SURFACE_LABELS => RED."""
        triage_src = (REPO_ROOT / "scripts" / "triage.py").read_text(encoding="utf-8")
        ui = re.search(r"(?m)^UI_SURFACE_LABELS = \(([^)]*)\)", triage_src)
        self.assertIsNotNone(ui, "UI_SURFACE_LABELS not found or reshaped")
        ui_labels = [x.strip().strip("\"'") for x in ui.group(1).split(",") if x.strip()]
        self.assertNotIn("area:gui", ui_labels,
                         "area:gui must derive role:gui, not role:site — otherwise the carve-out "
                         "is inexpressible on older target refs during the protocol rollout")
        self.assertIn("area:site", ui_labels, "role:site must still cover area:site")


class TestLabelsToModelChain(unittest.TestCase):
    """[OPUS-5] THE DECISIVE SUITE — LABELS IN, MODEL CHAIN OUT (review of PR #4211).

    Every assertion in `TestProviderPreference` above reads `orchestration/routing.toml` or a
    constant in `scripts/triage.py`. All of them were GREEN while the maintainer's only exception
    was INVERTED for 100% of the live GUI backlog, because the table said the right thing and no
    real issue could reach it:

      * `role:gui` was not a label in this repo, and `gh issue edit --add-label` does not create a
        missing one — it fails, and both writers discarded that failure while the sibling
        `status:ready` write succeeded, so GUI issues promoted role-LESS onto `[defaults]`;
      * even with the label, `triage._role()` returns an EXPLICIT `role:*` before it consults any
        surface label, and 35 of 35 open `area:gui` issues already carried one (33 `role:impl`,
        1 `role:perf`, 1 `role:research`).

    A guard that stops at the routing table cannot see either. These tests start from the labels a
    live issue actually carries and end at the chain the dispatcher will walk — the layer where the
    directive is either honoured or inverted.
    """

    @classmethod
    def setUpClass(cls):
        cls.doc = _routing_doc()
        cls.rr = _load_script("route_resolve_e2e", "route-resolve.py")

    def chain(self, labels):
        return self.rr.resolve(labels, self.doc)[0]

    # -- the exception binds -------------------------------------------------------------------
    def test_area_gui_with_a_preexisting_explicit_role_is_sol_first(self):
        """THE ONE THAT WOULD HAVE CAUGHT IT. 33 of 35 open `area:gui` issues carry `role:impl`
        (e.g. #3367). Before the fix they resolved `role:impl` -> ["opus5","sol"], i.e. the exact
        inversion of "except for GUI work where sol should remain prioritised".

        MUTANT: delete the carve-out from `route-resolve.resolve()` => RED.
        """
        for role in ("impl", "perf", "site", "ci", "gui"):
            chain = self.chain(["area:gui", f"role:{role}", "priority:P2"])
            self.assertEqual(chain[0], "sol",
                             f"area:gui + role:{role} must stay sol-first (maintainer 2026-07-26)")

    def test_area_gui_with_no_role_at_all_is_sol_first(self):
        """The fail-open case the write path produced: a GUI issue promoted with NO role label
        resolves through `[defaults]`, which is now Sol-first."""
        self.assertEqual(self.chain(["area:gui", "priority:P2", "area:sparq-gui"])[0], "sol")

    def test_gui_implementation_is_sol_first_end_to_end(self):
        for role in ("impl", "perf", "site", "ci", "gui"):
            self.assertEqual(self.chain(["area:gui", f"role:{role}"]), ["sol", "opus5"])

    def test_gui_carve_out_re_orders_but_never_re_routes(self):
        """The directive speaks only about MODEL priority. The carve-out must not change which
        agent implements the work — the desktop GUI is Rust/Tauri, so a `role:impl` GUI issue
        keeps `sparq-rust-impl`. MUTANT: turn the carve-out back into a route => RED."""
        self.assertEqual(self.rr.resolve(["area:gui", "role:impl"], self.doc)[1],
                         self.rr.resolve(["role:impl"], self.doc)[1])
        self.assertEqual(self.rr.resolve(["area:gui", "role:perf"], self.doc)[1],
                         self.rr.resolve(["role:perf"], self.doc)[1])

    # -- the exclusion that already worked, now asserted at the deciding layer ------------------
    def test_site_surfaces_are_sol_first_end_to_end(self):
        for area in ("area:site", "area:site-specs", "area:site-papers", "area:sitemap"):
            self.assertEqual(self.chain([area, "role:impl", "priority:P2"]), ["sol", "opus5"])
            self.assertEqual(self.chain([area, "role:site", "priority:P2"]), ["sol", "opus5"])
        self.assertEqual(self.chain(["surface:frontend", "role:site"]), ["sol", "opus5"])
        self.assertEqual(self.chain(["dashboard", "role:site"]), ["sol", "opus5"])

    def test_the_selector_is_an_exact_label_not_a_substring(self):
        """`"gui" in label` would sweep `area:guide` into the sol carve-out."""
        self.assertEqual(self.rr.gui_carve_out({"area:guide"}, ["opus5"], role="impl"),
                         ["opus5"])
        self.assertEqual(self.rr.gui_carve_out({"kind:guidance"}, ["opus5"], role="impl"),
                         ["opus5"])

    def test_carve_out_label_set_is_exactly_area_gui(self):
        """MUTANT: widen `GUI_CARVE_OUT_LABELS` => RED. Pinned as a set as well as behaviourally,
        so widening it over a label with no open issues still fails."""
        self.assertEqual(sorted(self.rr.GUI_CARVE_OUT_LABELS), ["area:gui"])

    def test_both_gui_selectors_agree(self):
        """Two places name the GUI surface — the resolver's carve-out (which decides the chain) and
        triage's `GUI_SURFACE_LABELS` (which derives the role). They must not drift apart: a widening
        of either one is the likely future mistake."""
        triage_src = (REPO_ROOT / "scripts" / "triage.py").read_text(encoding="utf-8")
        m = re.search(r"(?m)^GUI_SURFACE_LABELS = \(([^)]*)\)", triage_src)
        self.assertIsNotNone(m, "GUI_SURFACE_LABELS not found or reshaped")
        triage_labels = {x.strip().strip("\"'") for x in m.group(1).split(",") if x.strip()}
        self.assertEqual(triage_labels, set(self.rr.GUI_CARVE_OUT_LABELS))

    # -- precedence ----------------------------------------------------------------------------
    def test_a_security_surface_still_wins_over_the_gui_carve_out(self):
        """`area:gui` + `area:sparq-zk` is a ZK issue first. The soundness chain must be returned
        UNMODIFIED — a preference rule must never re-order a soundness lane."""
        self.assertEqual(self.rr.resolve(["area:gui", "area:sparq-zk", "role:impl"], self.doc),
                         (["opus5"], "sparq-reviewer", True))

    def test_the_carve_out_never_injects_sol_into_an_authorship_pinned_chain(self):
        """The directive's own qualifier is "tasks for which they are BOTH possible implementors".
        `role:research` / `role:review` / `role:soundness` are deliberately anthropic-side +
        escalating; the carve-out must leave them alone rather than quietly making one
        cross-provider, which would hide a stall that is meant to be visible.

        Since 2026-07-26 the carve-out CAN add sol back (see the tests below), so this is no longer
        implied by a blanket both-in-chain condition — it is enforced by the `inject_roles`
        allow-list, and the registry mechanism refuses to even parse these roles in that field.
        MUTANT: add "research" to GUI_CARVE_OUT_INJECT_ROLES => RED."""
        for role in ("research", "review", "soundness"):
            chain, _agent, escalate = self.rr.resolve(["area:gui", f"role:{role}"], self.doc)
            self.assertEqual(chain, ["opus5"],
                             f"role:{role} must stay single-provider under area:gui")
            self.assertTrue(escalate, f"role:{role} must still escalate")
        self.assertEqual(
            sorted(set(self.rr.GUI_CARVE_OUT_INJECT_ROLES)
                   & {"research", "review", "soundness"}), [],
            "no authorship-pinned escalating lane may be in the injection allow-list")

    def test_gui_impl_work_uses_the_sol_implementation_lane(self):
        impl_route = next(r for r in self.doc["route"] if r.get("role") == "impl")
        self.assertEqual(impl_route["model_chain"], ["sol", "opus5"])
        for labels in (["area:gui", "role:impl"],
                       ["area:gui", "role:impl", "priority:P2"],
                       ["area:gui", "role:impl", "area:sparq-gui"]):
            self.assertEqual(self.chain(labels), ["sol", "opus5"])
        self.assertEqual(self.rr.resolve(["area:gui", "role:impl"], self.doc)[1],
                         "sparq-rust-impl",
                         "the carve-out re-orders/leads the chain and never re-routes the agent")

    def test_the_injection_allow_list_is_declared_in_the_table_not_only_in_python(self):
        """CLAIM re-derives the route from the routing TOML with registry-owned code and
        `_route_matches` demands EXACT chain equality. An allow-list that existed only in this
        repo's Python would make every `area:gui` + `role:impl` issue defer `route-policy-failed`
        on every tick, forever — the #4211 failure mode.

        MUTANT: delete `inject_roles` from routing.toml, or widen the Python constant => RED."""
        declared = self.doc.get("chain_preference", [])
        self.assertEqual(len(declared), 1, "exactly one chain preference is declared")
        self.assertEqual(sorted(declared[0].get("inject_roles", [])),
                         sorted(self.rr.GUI_CARVE_OUT_INJECT_ROLES),
                         "the TOML declaration and the resolver constant must not drift")
        self.assertTrue(declared[0].get("inject_roles"),
                        "a re-order-only declaration disarms the carve-out on a single-rung chain")
        declared_roles = {r["role"] for r in self.doc.get("route", []) if r.get("role")}
        self.assertLessEqual(set(declared[0]["inject_roles"]), declared_roles,
                             "every inject_roles entry must name a role this table declares — a "
                             "typo would make the carve-out silently never fire")

    def test_injection_is_scoped_to_the_exact_gui_label(self):
        """On an older Opus-only target ref, this compatibility rule may add Sol to impl work.
        A false-matching selector would therefore alter non-GUI routing during the rollout.

        MUTANT: make the selector a substring test => RED (area:guide would gain a sol rung)."""
        for near in ("area:guide", "area:guidance", "area:gui-toolkit", "kind:guidance",
                     "area:site", "surface:frontend", "dashboard"):
            self.assertEqual(self.rr.gui_carve_out({near}, ["opus5"], role="impl"), ["opus5"],
                             f"{near} is not area:gui and must not gain a sol rung")

    def test_a_roleless_gui_issue_is_never_injected_into(self):
        """`role=None` (the `[defaults]` branch) has no role to check against the allow-list, so it
        must decline. The live defaults chain still contains sol, so it is sol-first by re-order —
        the assertion that carries the property is the direct one on `gui_carve_out`.

        MUTANT: drop the `role is None` clause => RED."""
        self.assertEqual(self.chain(["area:gui", "priority:P2"]), ["sol", "opus5"])
        self.assertEqual(self.rr.gui_carve_out({"area:gui"}, ["opus5"], role=None), ["opus5"])

    def test_gui_docs_keep_the_docs_route(self):
        """Docs writing already leads with sol by the separate directive; the carve-out must not
        change the docs agent or chain."""
        self.assertEqual(self.rr.resolve(["area:gui", "role:docs"], self.doc)[:2],
                         (["sol", "terra", "opus5"], "sparq-docs"))

    # -- non-GUI work is unaffected ------------------------------------------------------------
    def test_non_gui_implementation_work_is_sol_first_end_to_end(self):
        for role in ("impl", "site", "ci", "perf"):
            self.assertEqual(self.chain([f"role:{role}", "area:sparq-core", "priority:P1"]),
                             ["sol", "opus5"])
        self.assertEqual(self.chain(["area:sparq-core", "priority:P1"]), ["sol", "opus5"])

    def test_non_gui_impl_work_keeps_the_opus_continuity_fallback(self):
        for area in ("area:sparq-core", "area:sparq-engine", "area:orchestration"):
            self.assertEqual(self.chain(["role:impl", area, "priority:P1"]), ["sol", "opus5"])
        self.assertTrue(self.rr.resolve(["role:impl", "area:sparq-core"], self.doc)[2],
                        "and it must escalate, so a capacity outage is not a silent stall")


class TestLabelWritesAreFailClosed(unittest.TestCase):
    """[OPUS-5] The write path that turned "carve-out missing" into "carve-out INVERTED".

    `gh issue edit --add-label` does not create a missing label; it fails and applies nothing from
    that call. Both writers used ONE edit PER label and discarded the result, so a failed `role:*`
    write left the SUCCESSFUL `status:ready` write standing — the issue promoted with no role and
    `route-resolve` fell through to `[defaults]`. Fail-open on the dispatch path.

    Measured against a stubbed `gh` (role:gui absent from the repo, label-create denied):
    the pre-fix step exits 0 having applied `status:ready` alone; the post-fix step exits 1 having
    applied nothing.
    """

    @classmethod
    def setUpClass(cls):
        cls.raw = TRIAGE_WORKFLOW.read_text(encoding="utf-8")
        cls.doc = yaml.safe_load(cls.raw)
        cls.steps = cls.doc["jobs"]["triage"]["steps"]
        cls.step = next(s for s in cls.steps
                        if "Static triage" in (s.get("name") or ""))
        cls.retriage_src = (REPO_ROOT / "scripts" / "retriage.py").read_text(encoding="utf-8")

    def test_the_triage_step_exists(self):
        """Anti-vacuity: if the step is renamed away, every assertion below would scan nothing."""
        self.assertIn("run", self.step)

    def test_the_label_write_is_not_neutralised_by_a_trailing_true(self):
        """MUTANT: restore `|| true` on the `gh issue edit` line => RED. This is the exact edit
        that made the promotion fail-open."""
        for line in self.step["run"].splitlines():
            if "gh issue edit" in line:
                self.assertNotRegex(
                    line.strip(), r"(\|\|\s*true|;\s*true|\|\|\s*:)\s*$",
                    "a discarded label-write status is fail-open on the dispatch path")

    def test_the_step_uses_pipefail(self):
        """Without `set -euo pipefail` the write failure does not fail the step."""
        self.assertIn("set -euo pipefail", self.step["run"])

    def test_the_delta_is_written_by_a_single_all_or_nothing_edit(self):
        """MUTANT: go back to `for l in $add; do gh issue edit … --add-label "$l"; done` => RED.
        Per-label edits are the defect even WITHOUT `|| true`: with `set -e` the loop aborts on the
        first failure, but any label already applied in an earlier iteration stays — and the
        sort order puts `role:*` and `status:ready` in separate calls."""
        run = self.step["run"]
        self.assertNotRegex(
            run, r"for\s+l\s+in\s+\$add;\s*do\s+gh\s+issue\s+edit",
            "adds must not be applied one gh call per label")
        self.assertEqual(run.count("gh issue edit"), 1,
                         "the whole label delta must go in ONE gh issue edit invocation")

    def test_missing_labels_are_ensured_before_the_write(self):
        """MUTANT: delete the `gh label create` ensure => a brand-new routing label reds every
        triage run instead of being created. The ensure is best-effort; the ADD is the hard gate."""
        run = self.step["run"]
        self.assertIn("gh label create", run)
        self.assertLess(run.index("gh label create"), run.index("gh issue edit"),
                        "labels must be ensured BEFORE the edit that needs them")

    def test_auto_creation_is_restricted_to_the_labels_triage_emits(self):
        """The ensure must not be able to mint an arbitrary repo label."""
        self.assertRegex(self.step["run"], r"role:\*\|status:\*\|needs:\*")

    def test_retriage_reads_the_return_code_of_its_label_write(self):
        """MUTANT: drop the rc check in `retriage.apply_labels` => RED. `_gh` is `check=False`, so
        an unread return code is a silently discarded write inside a cron."""
        src = self.retriage_src
        self.assertIn("def apply_labels(", src)
        body = src[src.index("def apply_labels("):]
        body = body[:body.index("\n\ndef ") if "\n\ndef " in body else len(body)]
        self.assertIn("r.returncode != 0", body,
                      "apply_labels must read the return code of the edit")
        self.assertIn("return False", body,
                      "a failed write must be reported to the caller, not discarded")

    def test_retriage_main_exits_non_zero_when_a_write_failed(self):
        """MUTANT: `return 0` unconditionally in main() => RED. A cron that exits 0 having failed
        to promote is invisible."""
        src = self.retriage_src
        main = src[src.index("def main():"):]
        self.assertRegex(main, r"if failed:[\s\S]*?return 1",
                         "retriage must exit non-zero when any label write failed")

    def test_retriage_writes_the_delta_in_one_call(self):
        """The all-or-nothing property, on the cron side. MUTANT: restore the per-label loop
        (`for lb in add: _gh([... "--add-label", lb])`) => RED."""
        main = self.retriage_src[self.retriage_src.index("def main():"):]
        self.assertNotRegex(main, r"for lb in add:",
                            "retriage must not apply adds one gh call per label")


class TestSettingsHooks(unittest.TestCase):
    """Surface 2 — .claude/settings.json agent hooks. The bare-alias regression lived HERE."""

    @classmethod
    def setUpClass(cls):
        cls.settings = json.loads(
            (REPO_ROOT / ".claude" / "settings.json").read_text(encoding="utf-8"))

    def _hook_models(self):
        found = []

        def walk(node, path):
            if isinstance(node, dict):
                if node.get("type") == "agent" and "model" in node:
                    found.append((path, node["model"]))
                for k, v in node.items():
                    walk(v, f"{path}.{k}")
            elif isinstance(node, list):
                for i, v in enumerate(node):
                    walk(v, f"{path}[{i}]")

        walk(self.settings, "settings")
        return found

    def test_at_least_one_agent_hook_is_scanned(self):
        """Anti-vacuity: if the settings shape changes so no hook is found, every assertion below
        passes over an EMPTY list. Fail instead."""
        self.assertTrue(self._hook_models(),
                        "no `type: agent` hook with a `model` found — the scan below is vacuous")

    def test_no_hook_uses_a_deprecated_model(self):
        """MUTANT: set the perf-reviewer arm gate back to `"model": "opus"` => RED."""
        for path, model in self._hook_models():
            self.assertNotIn(model, DEPRECATED_ALIASES, f"{path}: retired alias {model!r}")
            self.assertNotIn(model, DEPRECATED_PROVIDER_MODELS,
                             f"{path}: retired provider id {model!r}")

    def test_no_hook_uses_a_bare_alias(self):
        """MUTANT: swap any full id for its bare alias => RED. `opus` silently meant claude-opus-4-8
        for days after Opus 5 shipped; an alias is not a pin."""
        for path, model in self._hook_models():
            self.assertNotIn(
                model, BARE_ALIASES,
                f"{path}: {model!r} is a bare alias whose target can move — pin the full model id")


class TestWorkflowJsDispatch(unittest.TestCase):
    """Surface 3 — .claude/workflows/*.js `model:` dispatch values."""

    @classmethod
    def setUpClass(cls):
        cls.files = sorted((REPO_ROOT / ".claude" / "workflows").glob("*.js"))

    def _model_literals(self):
        """Every `model: '<literal>'` string in the workflow JS. `model: null` (the
        attribution-only TIER rows) is intentionally not a string and so not collected."""
        pat = re.compile(r"""\bmodel\s*:\s*(['"])([^'"]+)\1""")
        for f in self.files:
            for m in pat.finditer(f.read_text(encoding="utf-8")):
                yield f.name, m.group(2)

    def test_workflow_files_are_present(self):
        self.assertTrue(self.files, "no .claude/workflows/*.js found — scan would be vacuous")

    def test_at_least_one_model_literal_is_scanned(self):
        self.assertTrue(list(self._model_literals()),
                        "no `model:` literal found — the scan below is vacuous")

    def test_no_dispatchable_deprecated_model(self):
        """MUTANT: restore `model: 'claude-fable-5'` on the fable-5 TIER row => RED."""
        for fname, model in self._model_literals():
            self.assertNotIn(model, DEPRECATED_PROVIDER_MODELS,
                             f"{fname}: dispatches retired model {model!r}")
            self.assertNotIn(model, DEPRECATED_ALIASES,
                             f"{fname}: dispatches retired alias {model!r}")

    def test_top_tier_dispatch_uses_the_full_id(self):
        """The `fable`/`opus` TIER keys are retained as stable dispatch TOKENS, but both must
        resolve to the full claude-opus-5 id — never to the bare alias."""
        drain = (REPO_ROOT / ".claude" / "workflows" / "fable-architect-drain.js").read_text(
            encoding="utf-8")
        for key in ("fable", "opus"):
            m = re.search(rf"^\s*{key}:\s*\{{\s*model:\s*'([^']+)'", drain, re.M)
            self.assertIsNotNone(m, f"TIER row {key!r} not found or reshaped")
            self.assertEqual(m.group(1), OPUS5_ID,
                             f"TIER row {key!r} must dispatch the full {OPUS5_ID} id")

    def test_attribution_only_rows_are_not_dispatchable(self):
        """The downgrade rows keep their marker/trailer (attribution is not routing) but must
        carry no dispatchable model. MUTANT: give them a model id back => RED."""
        drain = (REPO_ROOT / ".claude" / "workflows" / "fable-architect-drain.js").read_text(
            encoding="utf-8")
        for key in ("fable-5", "opus-4-8"):
            m = re.search(rf"^\s*'{re.escape(key)}':\s*\{{\s*model:\s*([^,]+),", drain, re.M)
            self.assertIsNotNone(m, f"TIER row {key!r} not found or reshaped")
            self.assertEqual(m.group(1).strip(), "null",
                             f"TIER row {key!r} must not be dispatchable")

    def test_dispatch_model_fails_closed_on_a_non_dispatchable_tier(self):
        """Returning `undefined` for a null-model row would pass NO --model flag, i.e. silently
        serve the session default. The helper must THROW. MUTANT: delete the throw => RED."""
        drain = (REPO_ROOT / ".claude" / "workflows" / "fable-architect-drain.js").read_text(
            encoding="utf-8")
        body = drain[drain.index("const dispatchModel"):]
        body = body[:body.index("\n}\n") + 3]
        self.assertIn("throw new Error", body,
                      "dispatchModel must refuse a non-dispatchable tier, not return undefined")


class TestAgentFrontmatter(unittest.TestCase):
    """Surface 4 — .claude/agents/*.md `model:` frontmatter."""

    @classmethod
    def setUpClass(cls):
        cls.agents = sorted((REPO_ROOT / ".claude" / "agents").glob("*.md"))

    def _models(self):
        for f in self.agents:
            text = f.read_text(encoding="utf-8")
            if not text.startswith("---"):
                continue
            fm = text.split("---", 2)[1]
            m = re.search(r"(?m)^model:\s*(\S+)\s*$", fm)
            if m:
                yield f.name, m.group(1)

    def test_agents_are_present(self):
        self.assertTrue(list(self._models()), "no agent frontmatter models found — vacuous scan")

    def test_no_agent_targets_a_deprecated_model(self):
        """MUTANT: set any agent to `model: opus` or `model: claude-fable-5` => RED."""
        for name, model in self._models():
            self.assertNotIn(model, DEPRECATED_ALIASES, f"{name}: retired alias {model!r}")
            self.assertNotIn(model, DEPRECATED_PROVIDER_MODELS,
                             f"{name}: retired provider id {model!r}")


class TestWorkflowWiring(unittest.TestCase):
    """THE YAML SEAM. A guard no workflow RUNS is not a guard.

    Substring assertions over the workflow text do NOT catch `if: false` — the text still contains
    the call site while the step never executes. So this parses the YAML STRUCTURALLY and asserts
    on the resolved job/step objects.
    """

    @classmethod
    def setUpClass(cls):
        cls.raw = WORKFLOW.read_text(encoding="utf-8")
        cls.doc = yaml.safe_load(cls.raw)
        cls.job = cls.doc["jobs"]["validate"]
        cls.steps = cls.job["steps"]

    def _run_steps(self):
        return [s for s in self.steps if "run" in s]

    def test_job_exists_and_has_no_skip_condition(self):
        """MUTANT: add `if: false` (or any `if:`) to the validate job => RED."""
        self.assertNotIn("if", self.job,
                         "a job-level `if:` can skip the whole routing gate")

    def test_no_gating_step_has_a_skip_condition(self):
        """MUTANT: add `if: false` to the self-test step => RED. This is the assertion a substring
        or count(...) check over the workflow text cannot make."""
        for step in self._run_steps():
            self.assertNotIn(
                "if", step,
                f"step {step.get('name')!r} carries an `if:` — it can silently stop running")

    def test_this_suite_is_actually_invoked(self):
        """MUTANT: delete the invocation line, or the whole step => RED."""
        runs = "\n".join(s["run"] for s in self._run_steps())
        self.assertIn("python3 scripts/tests/test_no_deprecated_models.py", runs,
                      "this suite is never RUN — its assertions are dead in CI")

    def test_invocation_is_not_neutralised_by_a_trailing_true(self):
        """MUTANT: append `|| true` to ANY invocation in this workflow => RED.

        [OPUS-5] WIDENED, found by mutation. This test previously matched only the line invoking
        THIS suite, so `python3 scripts/route-resolve.py --self-test || true` — which silently
        discards the entire routing-resolver contract, including the `area:gui` carve-out and the
        role:impl chain — SURVIVED. Exit-zero swallowing has bitten this repo repeatedly; the
        assertion has to cover every gating invocation in the job, not just this file's own."""
        checked = 0
        for step in self._run_steps():
            for line in step["run"].splitlines():
                stripped = line.strip()
                if not stripped.startswith("python3 "):
                    continue
                checked += 1
                self.assertNotRegex(
                    stripped, r"(\|\|\s*true|;\s*true|\|\|\s*:)\s*$",
                    f"the exit status of `{stripped}` is discarded")
        self.assertGreater(checked, 5,
                           "no invocations were inspected — the run blocks were not parsed")

    def test_the_routing_contract_scripts_are_actually_invoked(self):
        """[OPUS-5] A guard nobody runs cannot go red. The routing change of registry #738 lives in
        `orchestration/routing.toml` + `scripts/route-resolve.py`, and its assertions live in those
        scripts' own `--self-test`s — so this gate must invoke each of them.

        MUTANT: delete any of these invocation lines (or the whole step) => RED."""
        runs = "\n".join(s["run"] for s in self._run_steps())
        for invocation in ("python3 scripts/routing-validate.py --self-test",
                           "python3 scripts/route-resolve.py --self-test",
                           "python3 scripts/dispatch-plan.py --self-test",
                           "python3 scripts/tests/test_no_deprecated_models.py"):
            self.assertIn(invocation, runs, f"`{invocation}` is never RUN in this gate")
        self.assertRegex(runs, r"(?m)^\s*python3 scripts/routing-validate\.py\s*$",
                         "the validator must also run against the LIVE routing.toml, not only "
                         "against its own fixtures")

    def test_run_block_uses_pipefail(self):
        """Without `set -euo pipefail` a failing early command does not fail the step. Applied to
        EVERY run step in the gating job, not only the one invoking this suite."""
        for step in self._run_steps():
            self.assertIn("set -euo pipefail", step["run"],
                          f"run step {step.get('name', '<unnamed>')!r} lacks pipefail")

    def test_this_file_is_a_path_trigger_on_both_triggers(self):
        """MUTANT: drop the paths entry => this suite stops running on its own PRs => RED.
        Scoped to the trigger section: the filename also appears in the run: block, so a
        whole-file search would pass for the wrong reason."""
        trigger_section = self.raw[:self.raw.index("permissions:")]
        self.assertEqual(
            trigger_section.count('"scripts/tests/test_no_deprecated_models.py"'), 2,
            "must be a path trigger on BOTH pull_request and push")

    def test_routing_surfaces_are_path_triggers(self):
        """The surfaces this suite guards must re-run it when THEY change — otherwise a
        deprecated model can be reintroduced by a PR that never runs this gate."""
        trigger_section = self.raw[:self.raw.index("permissions:")]
        for surface in ("orchestration/routing.toml", ".claude/settings.json",
                        ".claude/workflows/fable-architect-drain.js",
                        # [OPUS-5] the labels -> chain surfaces. Without these a PR could re-break
                        # the carve-out (or re-open the label write path) and never run this gate.
                        "scripts/route-resolve.py", "scripts/triage.py", "scripts/retriage.py",
                        ".github/workflows/triage-issue.yml"):
            self.assertEqual(
                trigger_section.count(f'"{surface}"'), 2,
                f"{surface} must re-run this gate on BOTH pull_request and push")

    def test_merge_group_trigger_present(self):
        self.assertIn("merge_group", self.doc.get(True, self.doc.get("on", {})),
                      "the merge queue must evaluate this gate")

    def test_gate_is_not_declared_advisory(self):
        """MUTANT: declare routing-self-tests/validate advisory => ci-summary stops gating => RED."""
        registry = json.loads(
            (REPO_ROOT / ".github" / "advisory-registry.json").read_text(encoding="utf-8"))
        declared = {(e.get("workflow"), e.get("job_id"))
                    for e in registry.get("jobs", {}).values()}
        self.assertNotIn(("routing-self-tests.yml", "validate"), declared,
                         "declaring this job advisory stops ci-summary gating on it")


if __name__ == "__main__":
    sys.exit(not unittest.main(verbosity=2, exit=False).result.wasSuccessful())
