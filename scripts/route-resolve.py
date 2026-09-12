#!/usr/bin/env python3
# [OPUS-4.8] Routing resolver — the consumer contract for orchestration/routing.toml (review D2).
"""route-resolve.py — resolve an issue's labels to (model_chain, agent, escalate).

Implements the PRECEDENCE the review flagged as unspecified: **security-label override > explicit
role > [defaults]**, FIRST MATCH WINS. `match_labels` rules match if any listed keyword is a
SUBSTRING of any issue label (so `zk` matches `area:sparq-zk`). Because the table lists security
rules first, an `impl` issue that also touches `area:sparq-zk` routes to Opus (soundness), not Fable.
"""
import sys

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib


# [OPUS-5] DEPRECATION REGISTER (maintainer directive 2026-07-26: "deprecate the use of fable and
# opus entirely in favour of opus5"). These are the aliases and the concrete provider ids that must
# never again occupy a routing position. Keeping the ids here — not only the aliases — is what makes
# the deprecation stick: re-adding `[models.fable]` under a DIFFERENT alias name still trips the
# provider-id half of the guard.
DEPRECATED_ALIASES = {"fable", "opus"}
DEPRECATED_PROVIDER_MODELS = {"claude-fable-5", "claude-opus-4-8"}

# [OPUS-5] Cheap anthropic tiers, deprecated FOR DOCS WRITING on 2026-07-26 ("deprecate sonnet and
# haiku for docs writing in favor of gpt 5.6 sol"). role=docs was their ONLY routing consumer, so
# after that directive they hold no routing position at all and this set is the whole rule. They are
# deliberately still in the [models] CATALOG and still serve non-routing, non-docs-writing roles in
# the harness agent configs (sparq-pkg-nl NL retrieval, sparq-verify-mechanical, sparq-rust-impl,
# sparq-issue-sweeper, sparq-context-monitor) — this guard governs the routing table only.
NOT_A_ROUTING_TARGET = {"haiku", "sonnet"}

# `terra` (the codex CLI default model) stays docs-only: it is sol's same-provider fallback in the
# docs chain and must not appear anywhere else.
DOCS_ONLY = {"terra"}

# [SPARQ agent] LEGACY `area:gui` CARVE-OUT — THE COMPATIBILITY MECHANISM (review of PR #4211).
#
# Historical maintainer directive 2026-07-26: "Opus5 should be prioritised over sol on all tasks
# for which they are both possible implementors; except for GUI work where sol should remain
# prioritised", boundary settled the same day as "Let's just go with area:gui work".
# The 2026-09-02 protocol now makes every implementation route Sol-first. This mechanism is
# idempotent for the current routing table, but remains binding when resolving older target refs and
# therefore must stay aligned between PLAN and CLAIM during the rollout.
#
# The first attempt expressed this as a `role:gui` ROUTE and had scripts/triage.py derive that role
# from `area:gui`. Review proved it could never fire, for two independent reasons:
#   (1) `role:gui` did not exist as a repo label, and `gh issue edit --add-label` does NOT create a
#       missing label — it fails. Both writers discarded that failure while the sibling
#       `status:ready` write succeeded, so GUI issues promoted role-LESS and fell to [defaults].
#   (2) Even with the label, `_role()` returns an EXPLICIT `role:*` before it ever consults the
#       surface labels, and all 35 open `area:gui` issues already carried one (33 role:impl,
#       1 role:perf, 1 role:research). `bd-to-issues.role_for()` never emits `role:gui` either.
# Net effect measured on the live backlog: GUI work moved from sol-first to opus5-first — the
# directive's ONLY exception was INVERTED for 100% of it.
#
# So the carve-out is now applied HERE, at the layer that actually decides, keyed on the label the
# issues ALREADY carry. It needs no new label, no relabelling of the backlog, and no write that can
# fail: `resolve()` reads live labels at plan time.
#
# It is a CHAIN-ORDER rule, not a route: it re-orders the chain of whatever route the issue resolves
# to and leaves the ROLE and the AGENT untouched. That is exactly what the directive asks for (it
# speaks only about model priority), and it is why a `role:impl` GUI issue keeps `sparq-rust-impl`
# — the desktop GUI is Rust/Tauri, not the Next.js site — instead of being rerouted.
#
# EXACT LABEL, never a substring: `"gui" in lb` would false-match `area:guide`/`area:guidance`. And
# deliberately NOT a routing `match_labels` rule: the review lane's arm-side security classifier
# UNIONS every `match_labels` keyword, so "gui" there would human-arm every GUI PR.
#
# THE SELECTOR IS `area:gui` AND NOTHING ELSE. `area:site`, `area:site-specs`, `area:site-papers`,
# `surface:frontend`, `dashboard` are NOT GUI. They historically took the Opus-first site/default
# route and now take the common Sol-first implementation route. Widening this exact legacy selector
# over a site* label remains a compatibility mistake and is asserted against.
GUI_CARVE_OUT_LABELS = frozenset({"area:gui"})
GUI_CARVE_OUT_LEAD = "sol"
# [OPUS-5] THE ROLES WHERE THE CARVE-OUT MAY *ADD* sol BACK, not merely re-order it (registry #738).
#
# WHY THIS BECAME NECESSARY. The carve-out above was a pure re-ordering, and that was sufficient only
# while every affected route still HAD sol in its chain. On 2026-07-26 the maintainer, shown the
# measurement in registry #738 (`role:impl` first-attempt yield: sol 18% vs opus5 86%, n=74, same
# route and same brief; 4/4 same-issue crossovers), chose "remove sol from impl fallback" — so
# `role:impl` became `["opus5"]`. A `["opus5"]` chain does not contain sol, so the both-implementors
# condition DECLINES and the carve-out goes inert: all 33 open `area:gui` + `role:impl` issues would
# have resolved opus5-only. That is the exact inversion of the maintainer's one stated exception that
# PR #4211 was written to fix, re-created by an edit two directives later and with no symptom at all
# (both resolvers agree on the wrong answer, so the PLAN/CLAIM agreement harness cannot see it).
#
# So for the roles named here — and ONLY these — the carve-out ADDS its lead to a chain that lacks
# it. The narrowing is what preserves the original invariant's purpose:
#   * `role:research` / `role:review` / `role:soundness` are single-provider for AUTHORSHIP reasons
#     and escalate on exhaustion; a silent cross-provider rung would hide a stall that is meant to be
#     visible. The registry mechanism REFUSES to parse a declaration naming them
#     (chain_preference.INJECT_FORBIDDEN_ROLES), so this is structural, not merely undeclared.
#   * A ROLELESS issue (the `[defaults]` branch) can never be injected into: no role, nothing to
#     authorise it, fail-closed means decline. `[defaults]` still contains sol anyway.
#   * `role:perf` / `role:site` / `role:ci` / `role:docs` all still contain sol, so they take the
#     RE-ORDER path and are deliberately absent here — this set is the minimum that closes the gap.
GUI_CARVE_OUT_INJECT_ROLES = frozenset({"impl"})


def gui_carve_out(labels, chain, role=None):
    """Return `chain` with sol leading IFF this is GUI work and sol is a permitted implementor of it.

    Two modes, mirroring the registry's shared `chain_preference.apply_preferences` exactly (the
    registry reads the `[[chain_preference]]` declaration in orchestration/routing.toml; this
    function is the PLAN-side half, and `--self-test` asserts the two agree row by row):

    * sol ALREADY in the chain -> "for which they are BOTH possible implementors" is the directive's
      own qualifier, encoded literally: the chain must contain BOTH sol and opus5, and the result is
      a strict PERMUTATION (re-order only).
    * sol ABSENT -> the result is the chain with sol PREPENDED, but only when `role` is in
      GUI_CARVE_OUT_INJECT_ROLES. `role=None` never injects. This preserves compatibility with
      historical target refs where `role:impl` was a single-rung `["opus5"]` chain, without making
      `role:research` (Anthropic-side + escalating) quietly cross-provider.

    Idempotent in both modes: applying it to an already-sol-first chain is a no-op.
    """
    if not (set(labels) & GUI_CARVE_OUT_LABELS):
        return chain
    if GUI_CARVE_OUT_LEAD in chain:
        if not {GUI_CARVE_OUT_LEAD, "opus5"} <= set(chain):
            return chain
        return [GUI_CARVE_OUT_LEAD] + [m for m in chain if m != GUI_CARVE_OUT_LEAD]
    # HONESTY NOTE, found by mutating this very line: `role is None` is NOT the binding guard —
    # `None not in GUI_CARVE_OUT_INJECT_ROLES` is already true for every well-formed allow-list, so
    # deleting the `role is None` half leaves behaviour identical and no test can see it. It is kept
    # because it states the intent at the point of decision, but the MEMBERSHIP test is what carries
    # the property, and that is what the assertions pin. (Same shape as the note in the registry's
    # no_change_routing.excluded_tiers.)
    if role is None or role not in GUI_CARVE_OUT_INJECT_ROLES:
        return chain
    if "opus5" not in chain:
        # `requires` minus the lead: the route must still offer the other model the directive names,
        # so an unrelated single-rung chain never acquires sol.
        return chain
    return [GUI_CARVE_OUT_LEAD] + list(chain)


def _chains(doc):
    """Yield (where, chain) for every routing position in the table: defaults + every route."""
    yield "defaults", list(doc.get("defaults", {}).get("model_chain", []))
    for r in doc.get("route", []):
        where = ("role:" + r["role"]) if r.get("role") else \
            ("match_labels:" + ",".join(r.get("match_labels", []))) or "<unnamed>"
        yield where, list(r.get("model_chain", []))


def validate_routing(doc):
    """Structural invariants a routing table must satisfy before ANY resolution — enforced in
    resolve() so a violating table fails LOUDLY at PLAN time instead of silently routing.

    FAIL-CLOSED. Every rule here raises rather than dropping the offending alias, because the
    alternative — resolving anyway and letting the chain fall through — is exactly how a deprecated
    model keeps serving traffic after it is "removed".
    """
    models = set(doc.get("models", {}))
    errs = []

    # (1) DEPRECATION GUARD — fable / opus-4.8 may not occupy ANY routing position. This is the
    # regression guard for the 2026-07-26 directive: it is what turns the deprecation from a
    # one-time edit into an invariant.
    for name, spec in doc.get("models", {}).items():
        if name in DEPRECATED_ALIASES:
            errs.append(f"[models.{name}] is a DEPRECATED alias (maintainer 2026-07-26: fable and "
                        f"opus-4.8 are retired in favour of opus5)")
        pm = spec.get("provider_model")
        if pm in DEPRECATED_PROVIDER_MODELS:
            errs.append(f"[models.{name}] pins DEPRECATED provider_model '{pm}' (retired "
                        f"2026-07-26 in favour of claude-opus-5)")
    for where, chain in _chains(doc):
        for m in chain:
            if m in DEPRECATED_ALIASES:
                errs.append(f"{where}: model_chain names DEPRECATED alias '{m}' — use 'opus5'")

    # (2) FAIL CLOSED ON AN UNKNOWN MODEL. Previously only routing-validate.py (a separate CI step)
    # checked chain membership against the catalog, so a chain naming a model the catalog no longer
    # defines still RESOLVED — it just handed an unresolvable alias to the registry's account
    # selector, which found no account serving it and silently walked to the next rung. With
    # fable/opus now deleted from the catalog that failure mode is live, so refuse here instead.
    for where, chain in _chains(doc):
        for m in chain:
            if m not in models and m not in DEPRECATED_ALIASES:
                errs.append(f"{where}: model '{m}' is not in the [models] catalog — refusing to "
                            f"resolve (an unresolvable rung must fail, never fall through)")

    # (3) Cheap anthropic tiers hold no routing position (2026-07-26 docs-writing directive).
    for where, chain in _chains(doc):
        for m in chain:
            if m in NOT_A_ROUTING_TARGET:
                errs.append(f"{where}: model_chain names '{m}', which is not a routing target "
                            f"(docs writing moved to sol, maintainer 2026-07-26)")

    # (4) terra remains docs-only.
    if DOCS_ONLY & set(doc.get("defaults", {}).get("model_chain", [])):
        errs.append("defaults: names a docs-only model (" + ",".join(sorted(DOCS_ONLY)) + ")")
    for r in doc.get("route", []):
        if DOCS_ONLY & set(r.get("model_chain", [])) and r.get("role") != "docs":
            where = r.get("role") or ",".join(r.get("match_labels", [])) or "<unnamed>"
            errs.append(f"{where}: names a docs-only model outside role=docs")

    if errs:
        raise ValueError("routing table is invalid; refusing to resolve:\n  - " +
                         "\n  - ".join(errs))


def resolve(labels, doc):
    """Return (model_chain, agent, escalate). `labels`: iterable of the issue's labels."""
    validate_routing(doc)
    labels = set(labels)

    def role_of(lbs):
        for lb in lbs:
            if lb.startswith("role:"):
                return lb[5:]
        return None

    role = role_of(labels)
    for r in doc.get("route", []):
        kws = r.get("match_labels")
        if kws:  # security-label rule: any keyword is a substring of any label
            if any(k in lb for lb in labels for k in kws):
                # A SECURITY surface is returned UNMODIFIED — the GUI preference must never
                # re-order a soundness chain. area:gui + area:sparq-zk is a ZK issue first.
                return list(r["model_chain"]), r["agent"], bool(r.get("escalate"))
        elif "role" in r and role is not None and r["role"] == role:
            # `role` is passed so the injection allow-list can be evaluated: adding sol to a chain
            # that lacks it is legal only for GUI_CARVE_OUT_INJECT_ROLES. The registry's CLAIM-side
            # mechanism receives the same value at the same point.
            return (gui_carve_out(labels, list(r["model_chain"]), role=role), r["agent"],
                    bool(r.get("escalate")))
    d = doc.get("defaults", {})
    # role is None here BY CONSTRUCTION (a roleless issue), so the defaults branch can never be
    # injected into — passed explicitly rather than left to the parameter default.
    return (gui_carve_out(labels, list(d.get("model_chain", [])), role=None),
            d.get("agent"), bool(d.get("escalate")))


def _self_test():
    doc = tomllib.load(open("orchestration/routing.toml", "rb"))
    ok = True

    def chk(n, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {n}: {got} (want {want})")

    def raises(n, fn, needle):
        """Assert fn() raises ValueError whose message contains `needle`. A deprecation guard that
        cannot be shown to REJECT something is a comment, not a guard."""
        nonlocal ok
        try:
            fn()
        except ValueError as e:
            good = needle in str(e)
            ok = ok and good
            print(f"  {'ok  ' if good else 'FAIL'} {n}: raised {'with' if good else 'WITHOUT'} "
                  f"{needle!r}")
        else:
            ok = False
            print(f"  FAIL {n}: did NOT raise (expected {needle!r})")

    # impl + a security surface (area:sparq-zk) -> security rule wins over role -> Opus 5 alone
    # (the opus-4.8 tail fallback was deprecated 2026-07-26), escalate
    mc, ag, esc = resolve(["role:impl", "area:sparq-zk"], doc)
    chk("impl+zk -> opus5/escalate", (mc, ag, esc), (["opus5"], "sparq-reviewer", True))
    # [SPARQ agent] 2026-09-02 protocol: ordinary implementation is Sol-first, with Opus retained as
    # the continuity fallback for current Anthropic-authored repairs and total Sol unavailability.
    mc, ag, esc = resolve(["role:impl", "area:sparq-core"], doc)
    chk("impl -> SOL-FIRST with Opus continuity fallback", (mc, ag, esc),
        (["sol", "opus5"], "sparq-rust-impl", True))
    chk("role:impl preserves both providers for continuity",
        sorted({doc["models"][m]["provider"] for m in mc}), ["anthropic", "openai"])
    chk("role:impl has an explicit total-exhaustion exit", esc, True)
    # [OPUS-5] docs -> SOL-led (maintainer 2026-07-26: docs writing off haiku/sonnet onto gpt-5.6
    # sol). terra is sol's same-provider fallback; opus5 the cross-provider tail.
    chk("docs -> sol-led with bounded exhaustion",
        resolve(["role:docs", "area:x"], doc),
        (["sol", "terra", "opus5"], "sparq-docs", True))
    chk("docs chain has no cheap anthropic tier",
        sorted(set(resolve(["role:docs", "area:x"], doc)[0]) & NOT_A_ROUTING_TARGET), [])
    site_mc, _site_agent, site_esc = resolve(["role:site", "area:site"], doc)
    chk("site -> Sol-first implementation", (site_mc, site_esc), (["sol", "opus5"], True))
    # CI is implementation under the same Sol-first protocol.
    mc, ag, esc = resolve(["role:ci", "area:ci"], doc)
    chk("ci -> Sol-first implementation", (mc, ag, esc),
        (["sol", "opus5"], "sparq-ci-infra", True))
    chk("ci chain has no sub-frontier tier", sorted(set(mc) & {"sonnet", "haiku"}), [])
    default_mc, _default_agent, default_esc = resolve(["area:sparq-core"], doc)
    chk("no role -> Sol-first defaults",
        (default_mc, default_esc), (["sol", "opus5"], True))
    perf_mc, _perf_agent, perf_esc = resolve(["role:perf", "area:sparq-engine"], doc)
    chk("perf -> Sol-first implementation",
        (perf_mc, perf_esc), (["sol", "opus5"], True))

    # ---------------------------------------------------------------------------------------------
    # [SPARQ agent] SOL IMPLEMENTATION + OPUS REVIEW (maintainer protocol 2026-09-02).
    # ---------------------------------------------------------------------------------------------
    for _role in ("impl", "site", "gui", "ci", "perf"):
        mc, _agent, esc = resolve([f"role:{_role}"], doc)
        chk(f"role:{_role} is Sol-first with Opus continuity fallback",
            (mc, esc), (["sol", "opus5"], True))
    chk("role:gui routes to the site agent", resolve(["role:gui"], doc)[1], "sparq-site")
    chk("role:impl has an explicit total-exhaustion exit",
        (sorted({doc["models"][m]["provider"] for m in resolve(["role:impl"], doc)[0]}),
         resolve(["role:impl"], doc)[2]), (["anthropic", "openai"], True))
    chk("research -> opus5 only", resolve(["role:research"], doc)[0], ["opus5"])
    # review role -> opus5 + escalate
    chk("review -> opus5/escalate", resolve(["role:review"], doc)[1:], ("sparq-reviewer", True))

    # ---------------------------------------------------------------------------------------------
    # [OPUS-5] THE CARVE-OUT, END TO END: LABELS -> MODEL CHAIN (review of PR #4211).
    # Everything above this block asserts on the routing TABLE. The defect the review found lived
    # one layer BELOW that: the table said the right thing, and no real issue could reach it. These
    # assertions start from the labels a live issue actually carries and end at the chain the
    # dispatcher will walk, which is the only place the directive is either honoured or inverted.
    # ---------------------------------------------------------------------------------------------
    # THE decisive case: 33 of the 35 open area:gui issues carry an explicit role:impl. Before this
    # mechanism they resolved role:impl -> ["opus5", "sol"], i.e. opus5-FIRST — the exact inversion.
    chk("area:gui + an explicit role:impl -> SOL-first (the 33-issue case)",
        resolve(["area:gui", "role:impl", "priority:P2"], doc)[0], ["sol", "opus5"])
    chk("area:gui + role:impl keeps its implementor agent (carve-out re-orders, never re-routes)",
        resolve(["area:gui", "role:impl"], doc)[1], "sparq-rust-impl")
    chk("area:gui with NO role at all -> defaults, SOL-first",
        resolve(["area:gui", "priority:P2"], doc)[0], ["sol", "opus5"])
    # Compatibility behavior: the current route is already Sol-first, while the retained
    # declaration still handles an older Opus-only target revision identically on PLAN and CLAIM.
    chk("the live role:impl route is Sol-first with the continuity fallback",
        [r["model_chain"] for r in doc["route"] if r.get("role") == "impl"],
        [["sol", "opus5"]])
    chk("a re-order-only legacy rule declines an Opus-only chain",
        gui_carve_out({"area:gui", "role:impl"}, ["opus5"], role=None), ["opus5"])
    chk("...and its legacy injection allow-list restores Sol-first continuity",
        gui_carve_out({"area:gui", "role:impl"}, ["opus5"], role="impl"), ["sol", "opus5"])
    chk("area:gui + role:impl still ESCALATES (it inherits the impl route's exit; sol is a "
        "preference on top, not a replacement for the exit)",
        resolve(["area:gui", "role:impl"], doc)[2], True)
    for _role in ("impl", "site", "ci", "perf", "gui"):
        mc = resolve(["area:gui", f"role:{_role}"], doc)[0]
        chk(f"area:gui + role:{_role} -> Sol-first implementation", mc, ["sol", "opus5"])
    # Non-GUI implementation takes the same Sol-first default.
    for _area in ("area:site", "area:site-specs", "area:site-papers", "area:sitemap"):
        chk(f"{_area} + role:impl -> Sol-first implementation",
            resolve([_area, "role:impl"], doc)[0], ["sol", "opus5"])
    chk("role:site + area:site -> Sol-first implementation end to end",
        resolve(["role:site", "area:site"], doc)[0], ["sol", "opus5"])
    chk("surface:frontend is not in the carve-out",
        gui_carve_out({"surface:frontend"}, ["opus5"], role="impl"), ["opus5"])
    # EXACT label: a substring selector would sweep area:guide into the sol carve-out. Now that the
    # carve-out can ADD sol, a false match is no longer merely a re-order — it would hand a
    # deliberately excluded model back to a non-GUI issue, so these rows carry more weight.
    for _near in ("area:guide", "area:guidance", "area:gui-toolkit", "xarea:gui", "gui"):
        chk(f"{_near} does NOT false-match the carve-out",
            gui_carve_out({_near}, ["opus5"], role="impl"), ["opus5"])
    # Security still wins, and its chain is returned UNTOUCHED by the preference rule.
    chk("area:gui + a security surface -> soundness lane, unmodified",
        resolve(["area:gui", "area:sparq-zk", "role:impl"], doc),
        (["opus5"], "sparq-reviewer", True))
    # ...AND WITH A CROSS-PROVIDER SOUNDNESS CHAIN. The review of this PR observed that the
    # security exemption had no red test: against the shipped `["opus5"]` chain the both-
    # implementors condition declines for an unrelated reason, so applying the carve-out to the
    # security return was an UNDETECTABLE mutation. Asserted now, on a fixture whose soundness
    # chain is cross-provider, rather than after some future edit makes the real one so.
    import copy as _cp
    _xsec = _cp.deepcopy(doc)
    for _r in _xsec.get("route", []):
        if _r.get("match_labels"):
            _r["model_chain"] = ["opus5", "sol"]
            break
    chk("a CROSS-PROVIDER security route is STILL returned unmodified under area:gui (the "
        "exemption is the ROUTE CLASS, not an accident of that chain being single-model)",
        resolve(["area:gui", "area:sparq-zk", "role:impl"], _xsec)[0], ["opus5", "sol"])
    chk("...while the same table still applies the carve-out on the role branch (so the check "
        "above is an exemption, not a dead fixture)",
        gui_carve_out({"area:gui"}, ["opus5"], role="impl"), ["sol", "opus5"])
    # "for which they are BOTH possible implementors": a chain without sol is left alone, so the
    # carve-out cannot quietly turn research (anthropic-side + escalate) into a cross-provider route.
    chk("area:gui + role:research is NOT rewritten (sol is not an implementor there)",
        resolve(["area:gui", "role:research"], doc)[:1] + resolve(["area:gui", "role:research"], doc)[2:],
        (["opus5"], True))
    chk("area:gui + role:docs keeps the docs chain and the docs agent",
        resolve(["area:gui", "role:docs"], doc)[:2], (["sol", "terra", "opus5"], "sparq-docs"))
    # The selector itself, so a future widening has to edit an asserted constant.
    chk("the carve-out selector is EXACTLY {area:gui}", sorted(GUI_CARVE_OUT_LABELS), ["area:gui"])
    # Idempotence: applying the rule to an already-sol-first chain must not duplicate the lead.
    chk("carve-out is idempotent", gui_carve_out({"area:gui"}, ["sol", "opus5"]), ["sol", "opus5"])

    # ---------------------------------------------------------------------------------------------
    # [OPUS-5] THE CARVE-OUT MUST BE DECLARED IN THE TABLE, NOT ONLY IN THIS FILE.
    #
    # This resolver is only the PLAN half. CLAIM (registry `dispatch-claim._route_matches`)
    # re-derives the same route with REGISTRY-owned `policy-resolve.py` from THIS TABLE fetched at
    # the protected default tip, and demands EXACT equality — so a rule that lives only in this
    # module makes every issue it selects raise DispatchError, defer `route-policy-failed`, and,
    # because the comparison is a pure function of labels and the table, defer again on every
    # subsequent tick FOREVER. Measured on this branch before the declaration existed: 34 of the
    # 35 open `area:gui` issues diverged (33 role:impl + 1 role:perf).
    #
    # The registry's shared `chain_preference` mechanism reads the `[[chain_preference]]` block
    # from this file. These assertions pin the two representations to each other, in BOTH
    # directions, so neither can be edited alone:
    #   * deleting or narrowing the TOML block         -> red (CLAIM would stop applying the rule)
    #   * widening/renaming the Python constants       -> red (PLAN would apply a different rule)
    # ---------------------------------------------------------------------------------------------
    declared = doc.get("chain_preference", [])
    chk("the live table DECLARES exactly one chain preference (CLAIM reads this, not the Python)",
        len(declared), 1)
    decl = declared[0] if declared else {}
    chk("the declared selector equals GUI_CARVE_OUT_LABELS",
        sorted(decl.get("labels", [])), sorted(GUI_CARVE_OUT_LABELS))
    chk("the declared lead equals GUI_CARVE_OUT_LEAD", decl.get("lead"), GUI_CARVE_OUT_LEAD)
    chk("the declared `requires` is the both-implementors condition this module applies",
        sorted(decl.get("requires", [])), sorted({GUI_CARVE_OUT_LEAD, "opus5"}))
    chk("the declared lead is inside `requires` (so the rule can only RE-ORDER a chain, never "
        "INJECT a model into one that deliberately excludes it)",
        decl.get("lead") in decl.get("requires", []), True)
    chk("the declaration names no field the registry mechanism does not implement",
        sorted(decl.keys()), ["inject_roles", "labels", "lead", "requires"])
    chk("every model the declaration names is in the live [models] catalog",
        sorted({decl.get("lead"), *decl.get("requires", [])} - set(doc.get("models", {}))), [])
    # [OPUS-5] THE INJECTION ALLOW-LIST, pinned in BOTH directions (registry #738). Deleting
    # `inject_roles` from the TOML reds the first row (CLAIM would stop applying the rule and GUI
    # impl work would silently go opus5-only); widening the Python constant reds it too.
    chk("the declared `inject_roles` equals GUI_CARVE_OUT_INJECT_ROLES",
        sorted(decl.get("inject_roles", [])), sorted(GUI_CARVE_OUT_INJECT_ROLES))
    chk("the declaration DOES name an inject role (a re-order-only declaration would disarm the "
        "carve-out for every single-rung implementor chain)",
        bool(decl.get("inject_roles")), True)
    chk("every role in `inject_roles` is a role this table actually declares (a typo would make "
        "the carve-out silently never fire)",
        sorted(set(decl.get("inject_roles", []))
               - {r["role"] for r in doc.get("route", []) if r.get("role")}), [])
    chk("`inject_roles` names NO escalating single-provider authorship lane (the registry mechanism "
        "refuses these at parse time; asserted here so the table cannot even propose one)",
        sorted(set(decl.get("inject_roles", [])) & {"research", "review", "soundness"}), [])
    # BEHAVIOURAL EQUIVALENCE, not just field equality: run this module's rule and the declared
    # rule's semantics over the same rows and require identical chains. A declaration that agreed
    # field-by-field but was applied to a different set of routes would still split PLAN from CLAIM.
    # The `role` argument is threaded through BOTH sides — without it the comparison would only ever
    # exercise the re-order path and the injection half would be untested on either side.
    def _declared_rule(labels, chain, role):
        if not (set(labels) & set(decl.get("labels", []))):
            return list(chain)
        lead = decl.get("lead")
        requires = set(decl.get("requires", []))
        if lead in chain:
            if not requires <= set(chain):
                return list(chain)
            return [lead] + [m for m in chain if m != lead]
        if role is None or role not in set(decl.get("inject_roles", [])):
            return list(chain)
        if not (requires - {lead}) <= set(chain):
            return list(chain)
        return [lead] + list(chain)

    for _labels in (["area:gui", "role:impl"], ["area:gui", "role:perf"], ["area:gui"],
                    ["area:gui", "role:research"], ["area:gui", "role:review"],
                    ["area:gui", "role:soundness"], ["area:gui", "role:gui"],
                    ["area:gui", "role:site"], ["area:gui", "role:ci"],
                    ["area:gui", "role:docs"],
                    ["area:site", "role:impl"], ["area:guide", "role:impl"],
                    ["area:sparq-core", "role:impl"], ["surface:frontend", "role:site"]):
        _role = next((lb[5:] for lb in _labels if lb.startswith("role:")), None)
        for _chain in (["opus5", "sol"], ["opus5"], ["sol", "terra", "opus5"], ["sol", "opus5"],
                       ["luna"], []):
            chk(f"declared rule == this module's rule for {_labels} over {_chain}",
                _declared_rule(_labels, _chain, _role),
                gui_carve_out(set(_labels), list(_chain), role=_role))
    # ...and the equivalence loop is NOT vacuous: at least one row must actually exercise the
    # INJECTION branch, or the two implementations could agree by both never injecting.
    chk("the equivalence loop exercises the injection branch",
        (_declared_rule(["area:gui", "role:impl"], ["opus5"], "impl"),
         gui_carve_out({"area:gui", "role:impl"}, ["opus5"], role="impl")),
        (["sol", "opus5"], ["sol", "opus5"]))

    # ---------------------------------------------------------------------------------------------
    # [OPUS-5] THE DEPRECATION IS AN INVARIANT, NOT A ONE-TIME EDIT (maintainer 2026-07-26).
    # Every assertion below fails if the corresponding guard clause in validate_routing() is
    # deleted or weakened, and the LIVE-TABLE sweeps fail if a deprecated alias is reintroduced
    # into orchestration/routing.toml by any future edit.
    # ---------------------------------------------------------------------------------------------
    live_chains = {w: c for w, c in _chains(doc)}
    for where, chain in live_chains.items():
        chk(f"live table: {where} names no deprecated alias",
            sorted(set(chain) & DEPRECATED_ALIASES), [])
        chk(f"live table: {where} names no cheap anthropic tier",
            sorted(set(chain) & NOT_A_ROUTING_TARGET), [])
    chk("live catalog defines no deprecated alias",
        sorted(set(doc.get("models", {})) & DEPRECATED_ALIASES), [])
    chk("live catalog pins no deprecated provider id",
        sorted({s.get("provider_model") for s in doc.get("models", {}).values()}
               & DEPRECATED_PROVIDER_MODELS), [])
    chk("live catalog still defines opus5 -> claude-opus-5",
        doc.get("models", {}).get("opus5", {}).get("provider_model"), "claude-opus-5")
    chk("live catalog still defines sol -> gpt-5.6-sol",
        doc.get("models", {}).get("sol", {}).get("provider_model"), "gpt-5.6-sol")

    def _mutate(**kw):
        """A copy of the LIVE table with one field replaced — mutation-tests the guard against the
        real doc rather than a hand-built toy that could drift away from it."""
        import copy
        d = copy.deepcopy(doc)
        for k, v in kw.items():
            if k == "defaults_chain":
                d["defaults"]["model_chain"] = v
            elif k == "catalog":
                d["models"].update(v)
        return d

    raises("REJECTS a reintroduced `fable` rung in a chain",
           lambda: resolve(["role:impl"], _mutate(defaults_chain=["sol", "opus5", "fable"])),
           "DEPRECATED alias 'fable'")
    raises("REJECTS a reintroduced `opus` (4.8) rung in a chain",
           lambda: resolve(["role:impl"], _mutate(defaults_chain=["sol", "opus5", "opus"])),
           "DEPRECATED alias 'opus'")
    raises("REJECTS a re-added [models.fable] catalog entry",
           lambda: resolve(["role:impl"], _mutate(catalog={"fable": {
               "provider": "anthropic", "harness": "claude",
               "provider_model": "claude-fable-5", "credential_format": "claude-oauth-token"}})),
           "DEPRECATED alias")
    # the provider-id half: a retired model smuggled back under an INNOCENT alias name.
    raises("REJECTS claude-opus-4-8 smuggled back under a fresh alias",
           lambda: resolve(["role:impl"], _mutate(catalog={"legacy": {
               "provider": "anthropic", "harness": "claude",
               "provider_model": "claude-opus-4-8", "credential_format": "claude-oauth-token"}})),
           "DEPRECATED provider_model 'claude-opus-4-8'")
    raises("REJECTS claude-fable-5 smuggled back under a fresh alias",
           lambda: resolve(["role:impl"], _mutate(catalog={"legacy": {
               "provider": "anthropic", "harness": "claude",
               "provider_model": "claude-fable-5", "credential_format": "claude-oauth-token"}})),
           "DEPRECATED provider_model 'claude-fable-5'")
    raises("REJECTS haiku creeping back into a chain (docs writing moved to sol)",
           lambda: resolve(["role:impl"], _mutate(defaults_chain=["haiku", "sol"])),
           "not a routing target")
    raises("REJECTS sonnet creeping back into a chain",
           lambda: resolve(["role:impl"], _mutate(defaults_chain=["sonnet", "sol"])),
           "not a routing target")
    # FAIL CLOSED: an unresolvable rung must refuse, never fall through to the next one.
    raises("FAILS CLOSED on a model absent from the catalog",
           lambda: resolve(["role:impl"], _mutate(defaults_chain=["sol", "ghost"])),
           "not in the [models] catalog")
    raises("REJECTS terra outside role=docs",
           lambda: resolve(["role:impl"], _mutate(defaults_chain=["terra", "sol"])),
           "docs-only")

    print("route-resolve self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


def main():
    if "--self-test" in sys.argv:
        return _self_test()
    if len(sys.argv) > 1:
        doc = tomllib.load(open("orchestration/routing.toml", "rb"))
        mc, ag, esc = resolve(sys.argv[1].split(","), doc)
        print(f"model_chain={mc} agent={ag} escalate={esc}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
