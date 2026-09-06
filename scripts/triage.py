#!/usr/bin/env python3
# [OPUS-4.8] Issue-native orchestration: static triage (assign role/priority/package + ready-state).
"""triage.py — the deterministic, no-LLM part of issue triage.

Given an issue's labels + type, decide the labels to ADD/REMOVE and whether it is triage-complete:
  * role     — from a `kind:*` label or the bead/issue type (feature/bug->impl, docs->docs, ...).
  * priority — kept if a valid single `priority:P0..P4` is present; otherwise triage is incomplete.
  * package  — the existing `area:<crate>` labels are the package (kept as-is). A NO-package issue is
               NOT promotable: in the issue-native readiness engine a no-area issue reserves the
               serializing `__global__` partition, so at backlog scale each no-area `status:ready`
               issue would collapse the whole frontier behind it (audit-2026-07-17). Such an issue is
               parked `needs:area` (a gate the readiness engine already respects + maintainer-visible)
               until an area is assigned, then it re-triages to ready via the retriage cron.
  * ready    — an issue is `status:ready` iff it has a valid single priority AND a role AND an
               `area:<crate>` AND is not gated/untrusted/epic. Otherwise it becomes `status:untriaged`
               (missing role/priority) or is `needs:area`-parked (missing area) for the retriage cron
               / an LLM pass (a model call is only needed when role/priority cannot be derived).

Fail-closed: ambiguity or missing role/priority yields `status:untriaged`, never `status:ready`.
Pure `triage()` is unit-tested; the workflow applies the returned label delta.
"""
import re
import sys

ROLE_BY_KIND = {"docs": "docs", "design": "research", "research": "research", "perf": "perf",
                "test": "impl", "ci": "ci", "site": "site", "security": "soundness"}
ROLE_BY_TYPE = {"feature": "impl", "bug": "impl", "task": "impl", "chore": "ci",
                "spike": "research", "epic": "impl"}
SEC_KEYWORDS = ("zk", "mpc", "reasoner", "crypto", "auth", "e2ee")
# [FABLE-5] UI/front-end surfaces route role:site (maintainer decision 2026-07-17). EXACT labels,
# not substrings: a substring set would false-match (e.g. "gui" in "guide") and UI keywords must
# NOT enter routing match_labels, which the arm-side security classifier unions into its keyword
# set. Model preference belongs to orchestration/routing.toml, not this label-to-role classifier.
UI_SURFACE_LABELS = ("area:site", "surface:frontend", "dashboard")
# [OPUS-5] `area:gui` gets its OWN role (maintainer directive 2026-07-26). area:gui used to sit in
# UI_SURFACE_LABELS above and therefore derived role:site, conflating the explicit GUI surface with
# area:site / surface:frontend / dashboard. Splitting it out makes role:gui independently routable
# while leaving model selection to orchestration/routing.toml.
# EXACT label, never a substring: a substring test would match "guide"/"guidance".
#
# THIS IS NOT WHERE THE CARVE-OUT BINDS. This rule fires only for an issue with NO explicit role —
# the explicit-role branch in _role() returns first — and 35 of 35 open area:gui issues already
# carry one (33 role:impl, 1 role:perf, 1 role:research), so on its own it reached NONE of the live
# backlog and GUI work silently took the then-opus5-first default. The binding rule is
# scripts/route-resolve.py `gui_carve_out()`, which reads `area:gui` directly at plan time
# regardless of role. Keep the two label sets in sync — `test_both_gui_selectors_agree`.
GUI_SURFACE_LABELS = ("area:gui",)
# [FABLE-5] STANDING RULE — frontier-tier CI/infrastructure authorship (maintainer decision
# 2026-07-17, same pattern as the UI→codex rule above / PR #3416): infra-surface labels derive
# role:ci so infra work (.github/workflows, gate aggregators, orchestration/ + dispatch scripts)
# reaches the FRONTIER-ONLY ci chain in orchestration/routing.toml (fable/terra — sonnet/haiku no
# longer author infra). EXACT labels, not substrings ("ci" as a substring would match nearly every
# label), and deliberately NOT routing match_labels (the arm-side security classifier unions those
# keywords — infra keywords there would human-arm every infra PR). Security keywords still win.
INFRA_SURFACE_LABELS = ("area:ci", "area:ci-fragments", "area:orchestration", "area:workflows",
                        "area:release")
_PRIO = re.compile(r"^priority:P([0-4])$")


def _valid_priority(labels):
    ps = {m.group(1) for lb in labels for m in [_PRIO.match(lb)] if m}
    return len(ps) == 1


# [OPUS-5] `needs:area` is the ONE `needs:*` label this function owns: it is applied below when an
# otherwise-complete issue has no crate, and cleared in the ready branch once one lands. Everything
# else in the namespace is an external gate.
SELF_CLEARING_GATE = "needs:area"


def _blocking_gates(labels):
    """The `needs:*` gates that must block `status:ready`, matching ready-issues.py `is_gated`.

    [OPUS-5] This used to be the single literal `"needs:user" not in labels`, so triage attested
    `status:ready` on `needs:ec2` / `needs:docker` / `needs:zk` / `needs:upstream` /
    `needs:maintainer` issues. ready-issues.py treats EVERY `needs:*` as a hard dispatch gate, so
    the two components disagreed about what "gated" means and the readiness engine was the only
    thing keeping those issues off the frontier — a single-point defence for a class that is
    gated for real external reasons (an EC2 spend authorisation, a missing upstream release).

    Measured live on sparq-org/sparq 2026-07-26: 21 open issues carried BOTH `status:ready` and a
    real `needs:*` gate (needs:ec2 12, needs:docker 4, needs:zk 3, needs:external-subject 2,
    needs:upstream 1, needs:maintainer 1). It also bounded the retriage reach fix in this PR:
    5 of the 79 newly-promotable issues carry `needs:ec2` and must NOT be attested ready.

    Narrowing only — an issue that stops being `ready` here is fail-closed and cannot reach the
    frontier, so this can never open a dispatch path. `SELF_CLEARING_GATE` is excluded because
    treating it as blocking would deadlock it: `ready` would be False forever, and the `remove`
    that lifts the park only runs in the ready branch.
    """
    return {lb for lb in labels if lb.startswith("needs:")} - {SELF_CLEARING_GATE}


def _role(labels, issue_type):
    # a security-surface keyword forces the soundness lane regardless of kind/type/explicit role
    if any(k in lb for lb in labels for k in SEC_KEYWORDS):
        return "soundness"
    # [OPUS-4.8] respect an EXPLICIT single role:* label (e.g. a seeded/migrated issue that already
    # carries its role) — deriving a second role would make resolve() reject an ambiguous role set.
    explicit = sorted(lb[5:] for lb in labels if lb.startswith("role:"))
    if len(explicit) == 1:
        return explicit[0]
    for lb in labels:
        if lb.startswith("kind:") and lb[5:] in ROLE_BY_KIND:
            return ROLE_BY_KIND[lb[5:]]
    # [OPUS-5] area:gui derives role:gui — the sol-first carve-out — and is tested BEFORE the
    # generic UI rule below, which would otherwise swallow it into role:site again. Same
    # precedence slot otherwise (after security, an explicit role:*, and kind).
    if any(lb in GUI_SURFACE_LABELS for lb in labels):
        return "gui"
    # [SPARQ agent] UI-surface labels derive role:site (now the common Sol-first implementation
    # chain) before the
    # generic type map, after kind (so kind:docs about the site stays docs) and after an explicit
    # role:* label. NOTE: area:gui is deliberately NOT here — see GUI_SURFACE_LABELS.
    if any(lb in UI_SURFACE_LABELS for lb in labels):
        return "site"
    # [FABLE-5] infra-surface labels derive role:ci (the frontier-only fable/terra chain) before
    # the generic type map — same precedence slot as the UI rule: after security (soundness wins),
    # after an explicit role:*, after kind (so kind:docs about the CI stays docs).
    if any(lb in INFRA_SURFACE_LABELS for lb in labels):
        return "ci"
    return ROLE_BY_TYPE.get(issue_type)


def triage(labels, issue_type="task", trusted=True):
    """Return {add:set, remove:set, ready:bool, role:str|None}. If not trusted, triage is a no-op
    (the trust layer quarantines/notifies; content is never inspected here)."""
    labels = set(labels)
    if not trusted or "trust:untrusted" in labels:
        return {"add": set(), "remove": set(), "ready": False, "role": None}
    role = _role(labels, issue_type)
    add, remove = set(), set()
    if role:
        add.add(f"role:{role}")
        # [OPUS-4.8] single-role invariant: strip any OTHER role:* labels so the dispatcher's
        # resolve() never sees an ambiguous role set (a security keyword may override an explicit role).
        remove |= {lb for lb in labels if lb.startswith("role:") and lb != f"role:{role}"}
    # [FABLE-5] a no-area issue reserves the readiness engine's serializing __global__ partition, so
    # it must NEVER auto-promote to status:ready (each one collapses the whole dispatch frontier at
    # backlog scale — audit-2026-07-17). Park it needs:area (a gate ready-issues.py already respects,
    # and maintainer-visible) so a human/LLM assigns a crate; then the retriage cron re-promotes it
    # — but ONLY if the sweep can see the park and trusts its author. retriage's fetch is paginated
    # (retriage.py `_fetch_label`) precisely so "the next tick picks it up" is not conditional on
    # the issue being one of the 500 newest; that was false for 219 of 719 parks (issue #3831).
    has_area = any(lb.startswith("area:") for lb in labels)
    # [OPUS-4.8] an epic is a tracking umbrella, never dispatchable — it must not gain status:ready
    # (the readiness engine also excludes kind:epic as the hard dispatch gate; this keeps the tracker
    # honest so an epic never *shows* as ready).
    ready = (bool(role) and _valid_priority(labels) and has_area and not _blocking_gates(labels)
             and "kind:epic" not in labels)
    if ready:
        add.add("status:ready")
        remove.add("status:untriaged")
        remove.add("needs:area")      # an area landed since a prior no-area park — clear the gate
    else:
        add.add("status:untriaged")   # fail-closed: not dispatchable until complete
        remove.add("status:ready")
        # a triage-complete-but-no-area issue: mark WHY it is parked so it is maintainer-actionable
        # and stays out of the frontier (needs:* is a hard gate) instead of silently reserving global.
        if (bool(role) and _valid_priority(labels) and not has_area
                and "kind:epic" not in labels and not _blocking_gates(labels)):
            add.add("needs:area")
    return {"add": add - labels, "remove": remove & labels, "ready": ready, "role": role}


def _self_test():
    ok = True

    def chk(n, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {n}: {got} (want {want})")

    # complete: priority + derivable role -> ready
    r = triage(["priority:P1", "area:sparq-core"], "feature")
    chk("feature ready", (r["ready"], "role:impl" in r["add"], "status:ready" in r["add"]), (True, True, True))
    # missing priority -> untriaged
    r = triage(["area:sparq-core"], "feature")
    chk("no priority -> untriaged", (r["ready"], "status:untriaged" in r["add"]), (False, True))
    # ambiguous priority -> untriaged
    r = triage(["priority:P1", "priority:P2"], "feature")
    chk("ambiguous priority", r["ready"], False)
    # docs kind -> role:docs
    chk("docs role", triage(["priority:P3", "kind:docs"], "task")["role"], "docs")
    # security keyword forces soundness
    chk("sec soundness", triage(["priority:P1", "area:sparq-zk"], "feature")["role"], "soundness")
    # needs:user -> not ready
    chk("needs:user gated", triage(["priority:P1", "kind:docs", "needs:user"], "task")["ready"], False)
    # untrusted -> no-op
    chk("untrusted no-op", triage(["priority:P1", "trust:untrusted"], "feature"), {"add": set(), "remove": set(), "ready": False, "role": None})
    # [OPUS-4.8] respect an explicit role:* — do NOT derive a second (ambiguity broke autonomous dispatch)
    r = triage(["priority:P2", "role:site", "area:site"], "feature")
    chk("explicit role respected", (r["role"], "role:impl" in r["add"], any(x.startswith("role:") for x in r["remove"])),
        ("site", False, False))
    # [OPUS-4.8] an epic is never dispatchable, even with a full priority+role label-set
    chk("epic not ready", triage(["priority:P1", "role:impl", "kind:epic"], "epic")["ready"], False)
    # single-role invariant: a double-labelled issue is stripped to one role
    r = triage(["priority:P2", "role:impl", "role:site", "area:site"], "feature")
    chk("single-role invariant", (len([x for x in (({"role:impl", "role:site"} | r["add"]) - r["remove"]) if x.startswith("role:")]) == 1), True)
    # [FABLE-5] UI-surface ownership: a UI-surface label derives role:site (the codex/GPT-5.6-led
    # chain) even when the issue type would derive impl; kind (docs) and security still win.
    chk("ui surface -> site", triage(["priority:P2", "area:site"], "feature")["role"], "site")
    # [OPUS-5] the area:gui carve-out (maintainer 2026-07-26): area:gui gets its OWN role so the
    # routing table can keep it sol-first while everything else prefers opus5.
    chk("gui surface -> gui (NOT site)", triage(["priority:P2", "area:gui"], "task")["role"], "gui")
    # THE DISCRIMINATION THAT MATTERS: "GUI" informally reads as covering the site surfaces, so a
    # future editor could easily widen the carve-out back over them. NO site* area may derive
    # role:gui — that is the exact property, asserted independently of which non-gui role each
    # one happens to take (area:site -> site; the narrower area:site-specs / area:site-papers are
    # not exact UI_SURFACE_LABELS and fall through to the type map, which is fine: every one of
    # those roles historically used the opus5-first default).
    for _area in ("area:site", "area:site-specs", "area:site-papers", "area:sitemap"):
        chk(f"{_area} does NOT derive role:gui (not swept into the sol carve-out)",
            triage(["priority:P2", _area], "task")["role"] == "gui", False)
    chk("surface:frontend stays role:site", triage(["priority:P2", "surface:frontend"], "task")["role"], "site")
    chk("a 'guide' label does NOT false-match the gui carve-out",
        triage(["priority:P2", "area:guide"], "task")["role"], "impl")
    chk("explicit impl on ui surface stays impl",
        triage(["priority:P2", "role:impl", "area:site"], "feature")["role"], "impl")
    chk("site docs stay docs", triage(["priority:P3", "kind:docs", "area:site"], "task")["role"], "docs")
    chk("ui+sec -> soundness", triage(["priority:P1", "area:site", "area:sparq-zk"], "feature")["role"], "soundness")
    # [FABLE-5] frontier-tier infra authorship: an infra-surface label derives role:ci (the
    # frontier-only fable/terra chain) even when the issue type would derive impl; kind (docs)
    # and security keywords still win (soundness), matching the UI-rule precedence.
    chk("infra surface -> ci", triage(["priority:P1", "area:ci"], "feature")["role"], "ci")
    chk("orchestration surface -> ci",
        triage(["priority:P2", "area:orchestration"], "task")["role"], "ci")
    chk("infra docs stay docs", triage(["priority:P3", "kind:docs", "area:ci"], "task")["role"], "docs")
    chk("infra+sec -> soundness",
        triage(["priority:P1", "area:ci", "area:sparq-zk"], "feature")["role"], "soundness")
    # [FABLE-5] no-area guard: a complete priority+role issue with NO area:* is NOT promoted to ready
    # (it would reserve the serializing __global__ partition); it parks needs:area instead.
    r = triage(["priority:P1", "role:impl"], "feature")
    chk("no-area not ready", (r["ready"], "status:ready" in r["add"]), (False, False))
    chk("no-area parks needs:area", ("needs:area" in r["add"], "status:untriaged" in r["add"]), (True, True))
    # an epic with no area does NOT get needs:area (it is untriaged-by-design, not area-actionable)
    chk("epic no-area no needs:area", "needs:area" in triage(["priority:P1", "kind:epic"], "epic")["add"], False)
    # a needs:user-gated no-area issue is not double-parked with needs:area
    chk("gated no-area no needs:area", "needs:area" in triage(["priority:P1", "needs:user"], "task")["add"], False)
    # once an area lands, retriage clears the needs:area gate and promotes
    r = triage(["priority:P1", "role:impl", "area:sparq-core", "needs:area", "status:untriaged"], "feature")
    chk("area landed -> ready + clears needs:area", (r["ready"], "needs:area" in r["remove"]), (True, True))
    # [OPUS-5] EVERY needs:* gate blocks readiness, not just needs:user — ready-issues.py has
    # always treated the whole namespace as a hard dispatch gate and this function did not, so an
    # externally-gated issue was attested `status:ready` with the readiness engine as the only
    # thing keeping it off the frontier. Deleting a gate from `_blocking_gates` reds these.
    for gate in ("needs:ec2", "needs:docker", "needs:zk", "needs:upstream", "needs:maintainer",
                 "needs:external-subject", "needs:user"):
        r = triage(["priority:P1", "role:impl", "area:sparq-core", gate], "feature")
        chk(f"{gate} blocks status:ready",
            (r["ready"], "status:ready" in r["add"], "status:untriaged" in r["add"]),
            (False, False, True))
        # ...and an externally-gated issue is not ALSO double-parked with needs:area
        chk(f"{gate} no-area is not double-parked",
            "needs:area" in triage(["priority:P1", "role:impl", gate], "task")["add"], False)
    # the self-clearing gate must NOT be treated as blocking, or it can never be lifted: `ready`
    # would be False forever and the `remove` that clears it only runs in the ready branch.
    chk("needs:area is self-clearing, not a permanent block",
        _blocking_gates({"needs:area"}), set())
    # an unknown future needs:* label is gated by DEFAULT (namespace rule, not an allow-list)
    chk("an unknown needs:* gate blocks by default",
        triage(["priority:P1", "role:impl", "area:sparq-core", "needs:whatever-lands-next"],
               "feature")["ready"], False)
    # the gate set agrees with the readiness engine's, which is the whole point of the change
    chk("gate namespace matches ready-issues.py is_gated",
        _blocking_gates({"needs:ec2", "needs:area", "role:impl", "area:x", "trust:untrusted"}),
        {"needs:ec2"})
    print("triage self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--labels", default="", help="comma-separated current labels")
    ap.add_argument("--type", default="task")
    ap.add_argument("--untrusted", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return _self_test()
    labels = [x for x in a.labels.split(",") if x.strip()]
    r = triage(labels, a.type, trusted=not a.untrusted)
    print("ADD: " + " ".join(sorted(r["add"])))
    print("REMOVE: " + " ".join(sorted(r["remove"])))
    return 0


if __name__ == "__main__":
    sys.exit(main())
