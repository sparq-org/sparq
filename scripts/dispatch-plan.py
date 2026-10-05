#!/usr/bin/env python3
# [OPUS-4.8] Issue-native orchestration: the pure dispatch PLANNER (Phase 3-A) — dry-run / read-only.
"""dispatch-plan.py — compose the readiness engine + route resolver into a dispatch plan.

This is the PURE, read-only planner. It walks the conflict-free, priority-ordered ready frontier
produced by `ready-issues.compute_ready`, resolves each issue's route via `route-resolve.resolve`
against `orchestration/routing.toml`, reserves the issue's first package, and emits a plan row per
issue: {number, priority, package, role, model_chain, agent, escalate}.

It is DELIBERATELY dry-run: it does NOT claim an account and does NOT trigger a worker. Those are the
two credential-gated seams (see `claim_and_dispatch`) that require the private account registry's
claim step and the automation identity — both pending the maintainer's broker/token-placement
sign-off. Live mode (`--dry-run`) stops at "would dispatch"; it never crosses that boundary.

Fail-closed and honest (per the Phase-1/2 review posture): if an issue's route cannot be resolved to
a concrete role, the plan row is flagged with role=None/agent=None rather than guessed. `plan_dispatch`
is pure (no network, no side effects) and unit-tested by `--self-test`.
"""
import argparse
import os
import sys

# The foundation scripts live alongside this one; import their pure functions (they hyphenate their
# filenames, so load by path rather than `import ready-issues`).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import importlib.util


def _load(modname, filename):
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), filename)
    spec = importlib.util.spec_from_file_location(modname, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_ready = _load("ready_issues", "ready-issues.py")
_route = _load("route_resolve", "route-resolve.py")

compute_ready = _ready.compute_ready
packages_of = _ready.packages_of
labels_of = _ready.labels_of
valid_priority = _ready.valid_priority
resolve = _route.resolve
# [OPUS-5] The registry's dispatch.yml does `getattr(dispatch, "roleless_ready", None)` and, when
# a target planner lacks it, prints "target planner has no roleless_ready() — ready-but-roleless
# issues are NOT counted for this target" and skips the enumeration entirely. sparq WAS that
# target: the silent-invisibility class went unreported here on every tick. Re-exported from the
# readiness engine so the orchestrator reports a real number for sparq instead of degrading.
roleless_ready = _ready.roleless_ready
ready_candidates = _ready.ready_candidates
GLOBAL = _ready.GLOBAL
# [OPUS-5] sparq#4365 — THE PARTITION RESOLVER, ON THE MODULE THE REGISTRY ALREADY LOADS.
#
# #4360 made this repo's `conflict()` containment-aware: an `area:` key resolves through a TOTAL,
# workspace-tree-derived map and two keys conflict iff one partition path is a prefix of the other,
# so `area:sparq-server-http` now conflicts with `area:sparq-server`. PER sparq#4365 — which is a
# statement about the PRIVATE `jeswr/agent-account-registry`, not something this repository can
# check — its `dispatch.yml` mirrors the same key space in `busy_packages_of_pulls` and still
# compares by EXACT STRING, so the two legs disagree on every sub-crate key. That disagreement is
# the double-dispatch class: sparq defers the child, the registry does not, and two workers land in
# one crate with no lock.
#
# #4365 proposed porting the six-line resolver into `dispatch.yml`. These four exports are offered
# INSTEAD, because a port is a second implementation of a rule whose whole point is that it is
# derived rather than written down. `dispatch.yml` already clones this checkout and already does
# `load_dispatch(<target>/scripts/dispatch-plan.py)`, so calling `keys_conflict` off the loaded
# module gets the resolver AND the tree-derived root set from the same checkout, for free and
# without a table to go stale — which is exactly the argument #4360 made when it found #4336's
# hand-written table already wrong (`sparq-conformance-floors` is a real crate, not a region).
#
# HONESTLY SCOPED: this is the sparq HALF, and it is not the fix. Exporting a name the registry
# does not yet read changes nothing on its own — the registry degrades SILENTLY on a missing
# attribute (see `_registry_contract`, and the two prior occurrences it records), which is why
# `orchestration/registry-contract.toml` pins these names and a parity fixture as DATA and
# `--self-test` asserts both. Until a registry PR calls them, `busy_packages_of_pulls` compares
# however it compares today and #4365 stays open; nothing below can observe that.
#
# `DegeneratePartitionRoots` rides along because `workspace_roots` REFUSES an unverifiable tree
# rather than collapsing to a phantom partition; a caller that cannot name that exception can only
# catch it as a bare `Exception`, which is how the refusal becomes indistinguishable from a bug.
partition_path = _ready.partition_path
keys_conflict = _ready.keys_conflict
workspace_roots = _ready.workspace_roots
DegeneratePartitionRoots = _ready.DegeneratePartitionRoots
# [OPUS-5] sparq#4819 — THE SAME SILENT-INVISIBILITY CLASS AS `roleless_ready`, THREE LINES UP.
# The registry's `inert_aware` probe reads `getattr(planner, "INERT_FIELD")` and
# `getattr(planner, "MACHINE_PARK_PR_LABEL")` off THIS module — it loads
# `<target>/scripts/dispatch-plan.py`, never `ready-issues.py`. Defining them only in the readiness
# engine left both attributes `None` here, so the probe returned "planner declares no inertness
# contract", the field was never stamped onto any occupancy row, and the whole carve-out was a
# no-op that printed `NOT STAMPED` in a plain (unannotated) line.
#
# WHAT THAT IS WORTH — stated as the two things that are actually DURABLE, because an earlier
# revision of this comment hard-coded a frontier delta and the number did not survive the week.
#   1. WITHOUT these names the effect is CATEGORICAL, not statistical: `inert_aware` refuses,
#      nothing is ever stamped onto an occupancy row, and the carve-out cannot fire on any board.
#   2. WITH them, what is guaranteed is the RELEASE, not a frontier gain: an `area:` key held only
#      by attested machine parks is released, and a key retaining any holder that is not itself an
#      attested machine park is NOT.
# The frontier delta is neither of those. It is a PER-TICK YIELD — a released key moves the
# frontier only if some candidate happened to be waiting on exactly that key — so it is a property
# of the board on the day, not of this code. Three verbatim replays of the registry's readiness
# step over the same week disagreed on it (+7, +2, +3) while agreeing on the release semantics
# every time; that spread IS the finding, and it is why no number is pinned here.
# For a CURRENT figure, read the registry's own per-tick census rather than trusting this comment:
#   `parked-release census <repo>: stamped|NOT STAMPED (...); N of M open PR(s) attested inert;
#    K machine-parked row(s) release their areas: ...`
#   `partition census <repo>: candidates= frontier= partition-deferred= ...`
#
# Bound to the engine's own objects, never re-declared: a second literal here could drift from the
# value `occupies_area` actually consults, which is the two-legs-disagree defect in miniature.
INERT_FIELD = _ready.INERT_FIELD
MACHINE_PARK_PR_LABEL = _ready.MACHINE_PARK_PR_LABEL
is_provably_inert = _ready.is_provably_inert
# [OPUS-5] sparq#4929 — THE NON-RESERVING CROSS-CUTTING PARTITIONS, ON THE SAME MODULE.
#
# sparq#4928 made `ci`/`docs` non-reserving for OCCUPANCY in this repo's readiness engine
# (`ready-issues.py::NON_RESERVING_PARTITIONS`, with its measured basis and its fail-safe stated
# there once). Per sparq#4929 — again a statement about the PRIVATE registry, which nothing here
# can check — that is the PLAN leg only. The registry has a second, independent occupancy leg
# (`dispatch-claim.py::busy_packages_of_pulls`, applied at `filter_busy_area_items` on assemble
# and again at `revalidate_items_against_live_pulls` at CLAIM time) that derives a busy-area union
# from open worker PRs and has no cross-cutting exemption. Unless it mirrors the declaration, the
# rows PLAN now offers are re-deferred one layer down and the widening is nominal.
#
# THE SAME ARGUMENT AS #4365 ONE BLOCK UP, so the same shape of answer: the exemption is offered
# as a CALLABLE off the module `load_dispatch` already loads, not as a rule to re-type into
# `dispatch.yml`. A second, hand-written copy of `{"ci", "docs"}` over there is free to drift from
# this one, and two legs disagreeing about one partition is precisely what #4929 reports — the
# drift class sparq#4819 documents.
#
# WHAT IS DELIBERATELY *NOT* EXPORTED: the raw `NON_RESERVING_PARTITIONS` frozenset. A caller that
# reads the declaration directly gets the UNVALIDATED value and silently loses the fail-safe —
# `non_reserving_partitions()` is what voids the whole declaration on a single bad entry (and can
# never return the global/degenerate root, which would exempt everything). `reserves_partition`
# is the predicate to prefer: it answers on the PARTITION PATH, so a key that resolves INTO `ci`
# (the live `ci-fragments`) is exempt together with it, which an exact-string `key in {...}` mirror
# gets wrong — see the `[sparq#4929]` self-test rows.
#
# HONESTLY SCOPED, as with #4365: this is the sparq half and it is NOT the fix. Exporting a name
# the registry does not yet call changes nothing, and nothing in this repository can observe
# whether `busy_packages_of_pulls` has started calling it. The before/after the issue asks for is
# measurable only in the registry, where the `ledger` provenance half is readable.
non_reserving_partitions = _ready.non_reserving_partitions
reserves_partition = _ready.reserves_partition

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib


def _role_of(labels):
    """The issue's declared role from its `role:<r>` label, or None. Deterministic (sorted)."""
    for lb in sorted(labels):
        if lb.startswith("role:"):
            return lb[5:]
    return None


def plan_dispatch(ready_issues, routing_doc):
    """Compose the ready frontier + routing into a dispatch plan. PURE — no I/O, no side effects.

    `ready_issues` is the already priority-ordered, conflict-free output of `compute_ready` (each a
    dict with at least `number` + `labels`). Returns a list of plan rows, one per ready issue:
      {number, priority, package, role, model_chain, agent, escalate}
    `package` is the FIRST (sorted, deterministic) of the issue's packages — the partition the ready
    engine already reserved for it. If routing falls through to [defaults] because the issue carries
    no `role:` label, the row is flagged unresolved: role=None, agent=None (fail-closed, never guessed).
    """
    plan = []
    for it in ready_issues:
        labels = labels_of(it)
        role = _role_of(labels)
        # Package semantics mirror the registry's plan_package byte-for-byte (issue #3691):
        # exactly one unique area -> that area; zero or multiple -> the serializing GLOBAL
        # partition. The old first-of-sorted pick disagreed with the registry cross-check on
        # multi-area issues, perma-deferring them at dispatch.
        pkgs = packages_of(labels)
        package = next(iter(pkgs)) if len(pkgs) == 1 else _ready.GLOBAL
        model_chain, agent, escalate = resolve(labels, routing_doc)
        if role is None:
            # No declared role → the resolver returned [defaults]; do NOT guess an agent/role.
            row = {
                "number": it.get("number", 0),
                "priority": valid_priority(labels),
                "package": package,
                "role": None,
                "model_chain": [],
                "agent": None,
                "escalate": False,
            }
        else:
            row = {
                "number": it.get("number", 0),
                "priority": valid_priority(labels),
                "package": package,
                "role": role,
                "model_chain": list(model_chain),
                "agent": agent,
                "escalate": bool(escalate),
            }
        plan.append(row)
    return plan


def claim_and_dispatch(entry):
    """CREDENTIAL-GATED SEAM — the broker/worker boundary. The planner NEVER calls this.

    Turning a plan row into a live worker requires TWO capabilities this public planner does not (and
    must not) hold:

      1. The account claim. `select-and-claim.py` lives in the PRIVATE `agent-account-registry` repo
         (`jeswr/agent-account-registry`). It consumes the row's `model_chain` to lease the first
         AVAILABLE account (respecting per-account limits, the lease, and cache affinity), returning a
         restored credential. That claim is gated on the maintainer's token-placement / broker
         sign-off.
      2. The worker trigger. Dispatching the claimed run is a GitHub Actions worker job that runs under
         the automation identity `REGISTRY_ADMIN_TOKEN` (the registry-admin secret), NOT any human PAT.

    Both are deliberately absent here so the planner stays public, pure, and safe to run anywhere.
    When the broker sign-off + automation identity land, THIS function is the single place to wire
    them; every caller upstream stops at "would dispatch".
    """
    raise NotImplementedError(
        "claim_and_dispatch is the credential-gated broker/worker seam: it needs the private "
        "agent-account-registry claim (select-and-claim.py, gated on the maintainer's "
        "token-placement/broker sign-off) and the worker Actions job under REGISTRY_ADMIN_TOKEN "
        "(the automation identity). The dry-run planner never calls it."
    )


# ---------------------------------------------------------------------------------------------------
# Self-test: fixtures exercising the composition end-to-end against the REAL routing.toml.
def _routing_doc():
    here = os.path.dirname(os.path.abspath(__file__))
    toml = os.path.join(os.path.dirname(here), "orchestration", "routing.toml")
    with open(toml, "rb") as fh:
        return tomllib.load(fh)


def _registry_contract():
    """[OPUS-5] sparq#4819 — the names jeswr/agent-account-registry reads off THIS module.

    Deliberately DATA (`orchestration/registry-contract.toml`) rather than a constant in this
    file: the registry DEGRADES silently on a missing attribute, so the only thing that can catch
    a rename is an expectation a rename sweep over `scripts/` does not reach. Opened unguarded —
    an unreadable or malformed contract file must abort `--self-test`, never be skipped, because
    "the contract could not be read" and "the contract is satisfied" are the two states this whole
    mechanism exists to keep apart.
    """
    here = os.path.dirname(os.path.abspath(__file__))
    toml = os.path.join(os.path.dirname(here), "orchestration", "registry-contract.toml")
    with open(toml, "rb") as fh:
        return tomllib.load(fh)


def _self_test():
    doc = _routing_doc()
    ok = True

    def chk(name, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {name}: {got} (want {want})")

    def iss(n, labels, blk=0, state="OPEN"):
        return {"number": n, "state": state, "labels": labels, "open_blockers": blk}

    R = ["status:ready"]

    # --- Fixture: an impl issue → OPUS5-ONLY chain + escalate -------------------------------------
    # [OPUS-5] maintainer decision 2026-07-26 on the registry #738 measurement ("Remove sol from
    # impl fallback"): sol 18% vs opus5 86% in-cell first-attempt yield, n=74. The WHOLE chain is
    # asserted, not its head — the planner row is what the registry's CLAIM step compares for EXACT
    # equality, so a demoted-not-removed sol must red here too.
    impl = compute_ready([iss(1, R + ["priority:P1", "role:impl", "area:sparq-core"])])
    p_impl = plan_dispatch(impl, doc)
    chk("impl -> single row", len(p_impl), 1)
    row = p_impl[0]
    chk("impl row", (row["role"], row["model_chain"], row["agent"], row["escalate"]),
        ("impl", ["opus5"], "sparq-rust-impl", True))
    chk("impl package", row["package"], "sparq-core")
    chk("impl priority", row["priority"], 1)

    # --- Fixture: a security/zk issue → opus + escalate (security override beats role) -----------
    sec = compute_ready([iss(2, R + ["priority:P0", "role:impl", "area:sparq-zk"])])
    p_sec = plan_dispatch(sec, doc)
    row = p_sec[0]
    chk("zk -> opus5 only", row["model_chain"], ["opus5"])
    chk("zk -> reviewer/escalate", (row["agent"], row["escalate"]), ("sparq-reviewer", True))
    chk("zk role stays declared", row["role"], "impl")

    # --- Fixture: a docs issue → its route (SOL-led since 2026-07-26) ----------------------------
    # [OPUS-5] maintainer directive: docs WRITING moved off the cheap anthropic tiers (haiku/
    # sonnet) onto gpt-5.6 sol. The second assertion is the one that goes red if either tier is
    # put back into the docs chain.
    docs = compute_ready([iss(3, R + ["priority:P2", "role:docs", "area:sparq-docs"])])
    p_docs = plan_dispatch(docs, doc)
    row = p_docs[0]
    chk("docs -> sol", row["model_chain"][0], "sol")
    chk("docs row has no cheap anthropic tier",
        sorted(set(row["model_chain"]) & {"sonnet", "haiku"}), [])
    chk("docs -> sparq-docs", row["agent"], "sparq-docs")

    # --- Fixture: a ci/infra issue → frontier-only chain (standing rule 2026-07-17) --------------
    # No sub-frontier model (sonnet/haiku) in the plan row's chain: the registry claim step can
    # only serve a frontier account or DEFER the item to the next tick — degradation to a cheaper
    # authoring tier is impossible by construction (see routing-validate's frontier-floor check).
    ci = compute_ready([iss(8, R + ["priority:P1", "role:ci", "area:ci"])])
    row = plan_dispatch(ci, doc)[0]
    chk("ci -> frontier-only row", (row["role"], row["model_chain"], row["agent"], row["escalate"]),
        ("ci", ["opus5", "sol"], "sparq-ci-infra", False))
    chk("ci row has no sub-frontier tier", sorted(set(row["model_chain"]) & {"sonnet", "haiku"}), [])

    # --- Fixture: package-conflict pair → only the higher-priority one is planned ----------------
    pair = compute_ready([
        iss(4, R + ["priority:P2", "role:impl", "area:sparq-engine"]),   # lower priority
        iss(5, R + ["priority:P0", "role:impl", "area:sparq-engine"]),   # higher priority (wins)
    ])
    p_pair = plan_dispatch(pair, doc)
    chk("conflict -> one row", len(p_pair), 1)
    chk("conflict -> higher prio kept", p_pair[0]["number"], 5)

    # --- Fixture: an empty frontier → empty plan -------------------------------------------------
    chk("empty frontier -> empty plan", plan_dispatch([], doc), [])
    chk("no ready -> empty",
        plan_dispatch(compute_ready([iss(6, ["priority:P1", "role:impl", "area:x"])]), doc), [])

    # --- Fixture: an issue with NO declared role → fail-closed (role/agent None) -----------------
    # (compute_ready requires a role, so bypass it and feed plan_dispatch a roleless issue directly
    #  to prove the resolver-fallthrough is flagged, never guessed.)
    p_norole = plan_dispatch([iss(7, ["priority:P1", "area:sparq-core"])], doc)
    row = p_norole[0]
    chk("no-role -> flagged", (row["role"], row["agent"], row["model_chain"]), (None, None, []))

        # --- #3691 package-semantics fixtures (mirror registry plan_package) --------------------------
    one = plan_dispatch([iss(90, R + ["priority:P2", "role:impl", "area:sparq-core"])], doc)
    chk("#3691 exactly-one area maps to that area", one[0]["package"], "sparq-core")
    multi = plan_dispatch([iss(91, R + ["priority:P2", "role:perf", "area:bench", "area:sparq-serve"])], doc)
    chk("#3691 multi-area maps to the GLOBAL serializing partition (first-of-sorted pick => red)",
        multi[0]["package"], _ready.GLOBAL)
    zero = plan_dispatch([iss(92, R + ["priority:P2", "role:docs"])], doc)
    chk("#3691 zero-area maps to GLOBAL", zero[0]["package"], _ready.GLOBAL)

    # --- [OPUS-5] DEPRECATION SWEEP over every row this self-test planned ------------------------
    # The per-fixture assertions above pin the chains we happened to sample. This sweeps ALL of
    # them, so a deprecated alias reintroduced into a route the fixtures do not cover still fails.
    _dep = {"fable", "opus"}
    _all_rows = [r for plan in (p_impl, p_sec, p_docs, plan_dispatch(ci, doc), p_pair)
                 for r in plan]
    chk("no planned row names a deprecated alias",
        sorted({m for r in _all_rows for m in r["model_chain"]} & _dep), [])

    # --- [OPUS-5] sparq#4819 THE CROSS-REPO INTERLOCK, REPLAYED AGAINST THIS FILE ----------------
    # WHY THIS EXISTS, AND WHY IT IS HERE AND NOT IN ready-issues.py. The registry's dispatch.yml
    # `readiness` step does `load_dispatch(<target>/scripts/dispatch-plan.py)` and then probes THE
    # LOADED MODULE. ready-issues.py's own suite proved the ENGINE honours the carve-out and was
    # 9/9 green while this module exported neither name, so the registry read
    # `getattr(planner, "INERT_FIELD", None) is None`, printed "planner declares no inertness
    # contract", and stamped nothing — the whole two-PR change was a measured no-op. A guard on
    # the wrong module is not a guard.
    #
    # `_probe` below is the registry's `inert_aware` VERBATIM (jeswr/agent-account-registry
    # .github/workflows/dispatch.yml, step `id: readiness`), reproduced rather than approximated:
    # what must hold is the behaviour of the other repo's code against this file, and an
    # approximation of it can agree with us while the real one refuses.
    #
    # THE NAMES COME FROM DATA, NOT FROM THIS FILE. They are the REGISTRY's wire names, not ours
    # to rename: dispatch.yml hard-codes "INERT_FIELD"/"MACHINE_PARK_PR_LABEL", so renaming the
    # Python constant — however consistently across sparq's own prod and tests — silently returns
    # the pipeline to NOT STAMPED. Holding the expectation in `orchestration/registry-contract.toml`
    # is what makes such a rename RED here instead of green-and-dead in production: a rename sweep
    # over `scripts/` cannot carry the expectation along with the thing it is checking. MEASURED:
    # renaming the identifier in prod AND every sparq test leaves ready-issues.py 9/9 green and
    # reds three rows here; a blind textual rename that ALSO rewrote the literals in this file was
    # green until the expectation moved out of it.
    _contract = _registry_contract()["planner_exports"]
    _WIRE_NAMES = tuple(_contract["required_attributes"])

    def _as_registry_loads_it():
        """This file, loaded byte-for-byte the way `load_dispatch` loads it — not `globals()`.

        A `globals()` read would pass on a module that only defines the name at self-test time;
        the registry gets whatever `exec_module` leaves on the module object, so that is what is
        probed."""
        here = os.path.dirname(os.path.abspath(__file__))
        spec = importlib.util.spec_from_file_location(
            "target_dispatch_plan", os.path.join(here, "dispatch-plan.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def _probe(planner):
        """VERBATIM `inert_aware` from the registry's readiness step. Returns (field, why)."""
        field = getattr(planner, "INERT_FIELD", None)
        label = getattr(planner, "MACHINE_PARK_PR_LABEL", None)
        if not isinstance(field, str) or not isinstance(label, str):
            return None, "planner declares no inertness contract"
        area = "area:__inert_probe__"
        rival = {"number": 999000012, "state": "OPEN", "open_blockers": 0,
                 "labels": ["status:ready", "priority:P0", "role:impl", area]}
        holder = {"number": 999000011, "state": "OPEN", "open_blockers": 0,
                  "pull_request": {}, "labels": [area, label]}
        try:
            held = [row.get("number") for row in planner.compute_ready(
                [dict(holder), dict(rival)])]
            freed = [row.get("number") for row in planner.compute_ready(
                [dict(holder, **{field: True}), dict(rival)])]
        except Exception as exc:                              # noqa: BLE001 — mirrors the registry
            return None, f"probe raised {type(exc).__name__}"
        if held:
            return None, ("planner RELEASES a machine-parked area with no attestation "
                          "— refusing to feed it attestations")
        if freed != [999000012]:
            return None, "planner ignores the inertness attestation"
        return field, (f"releases an attested machine park via `{field}` and holds an "
                       "unattested one")

    _mod = _as_registry_loads_it()
    # (a) THE NAMES. Every attribute the registry reads off this module must be PRESENT on it, as
    # loaded. `roleless_ready`/`ready_candidates` ride along because they degrade the same silent
    # way and had no module-level assertion either.
    chk("[sparq#4819] dispatch-plan exports every name registry-contract.toml declares",
        sorted(name for name in _WIRE_NAMES if not hasattr(_mod, name)), [])
    # (b) THE VALUES. `MACHINE_PARK_PR_LABEL` is a real GitHub label and `INERT_FIELD` is the key
    # the registry WRITES onto the occupancy row (`row[field] = ...`); both are wire values, so a
    # "tidy-up" of either detaches this engine from the attestations it is sent. Compared as a
    # whole mapping so a name dropped from the contract file is as red as a value changed.
    chk("[sparq#4819] ...bound to the wire VALUES the registry writes",
        {name: getattr(_mod, name, None) for name in _contract["values"]},
        dict(_contract["values"]))
    # (c) THE BEHAVIOUR, through THIS module's compute_ready. Asserted on the full (field, why)
    # pair, not on truthiness: every refusal path returns (None, <reason>), and the reason names
    # which half broke — so this row reports the registry's own diagnosis rather than "not ok".
    chk("[sparq#4819] the registry's inert_aware probe STAMPS against this module",
        _probe(_mod),
        ("inert", "releases an attested machine park via `inert` and holds an unattested one"))
    # (d) ...and the refusal direction is reachable, so (c) cannot be passing because `_probe`
    # returns its happy value unconditionally. A planner missing the names must be REFUSED with
    # the registry's exact wording — this is the state sparq was in when the pair was measured a
    # no-op, reproduced here as a fixture so it can never be the silent state again.
    class _NoContract:
        compute_ready = staticmethod(_mod.compute_ready)
    chk("[sparq#4819] ...and refuses a planner without the names, by the registry's own reason",
        _probe(_NoContract), (None, "planner declares no inertness contract"))

    # --- #4365: the PARTITION RESOLVER offered to `busy_packages_of_pulls` -----------------------
    # Same failure shape as #4819 one section up, on a different name set. #4360 made THIS repo's
    # `conflict()` containment-aware; the registry's `dispatch.yml` mirrors the key space in
    # `busy_packages_of_pulls` and still compares by exact string, so the two legs disagree on
    # every sub-crate key. Rather than port the rule into `dispatch.yml` — a second implementation
    # of a rule whose whole value is being DERIVED from the tree — the resolver is exported off
    # the module `load_dispatch` already loads from the checkout `dispatch.yml` already clones.
    #
    # These rows assert the sparq half only. They cannot observe the registry, and passing them
    # does NOT mean `busy_packages_of_pulls` has been changed.
    _partition = _registry_contract()["partition_resolver"]
    _FIXTURE = _partition["parity_fixture"]

    # (e) THE NAMES, as loaded — an absent export is the silent-degrade state, not an error.
    chk("[sparq#4365] dispatch-plan exports the partition resolver the registry must call",
        sorted(n for n in _partition["required_attributes"] if not hasattr(_mod, n)), [])

    # (f) THE MAPPING, through THIS module. Every rule the fixture pins, in one comparison, so a
    # dropped fixture row is as red as a changed resolution.
    chk("[sparq#4365] ...and reproduces registry-contract.toml's parity fixture",
        {k: list(_mod.partition_path(k)) for k in _FIXTURE}, dict(_FIXTURE))

    # (g) ...and it is NOT exact-string equality — the whole point. Scanned over the fixture keys
    # UNIONED WITH THE PARENTS THEY RESOLVE TO, because the headline disagreement of #4365 is
    # `sparq-server-http` vs `sparq-server` and the parent is not itself a fixture key: a scan over
    # the fixture alone would miss every region/parent pair and report only sibling collisions.
    # The list is both the bug report — each pair is a key the registry's `busy_packages_of_pulls`
    # currently treats as free — and the mutation guard: flatten `partition_path` to the identity,
    # or revert `keys_conflict` to `a == b`, and it goes EMPTY rather than merely wrong.
    _universe = sorted(set(_FIXTURE) | {p for path in _FIXTURE.values() for p in path})
    _disagree = sorted((a, b) for i, a in enumerate(_universe) for b in _universe[i + 1:]
                       if _mod.keys_conflict(a, b) != (a == b))
    chk("[sparq#4365] ...and disagrees with exact-string comparison on the containment pairs",
        _disagree,
        # The degenerate key contains EVERYTHING (it is the `()` root), so it pairs with all 12
        # named keys; the rest are the region/parent and region/sibling pairs #4360 introduced.
        [("", k) for k in _universe if k != ""] +
        [("sparq-core", "sparq-core-nt-dict"), ("sparq-core", "sparq-core-store"),
         ("sparq-core-nt-dict", "sparq-core-store"),
         ("sparq-engine", "sparq-engine-exec"),
         ("sparq-server", "sparq-server-http"),
         ("upstream", "upstream-noir")])

    # --- #4929: the CROSS-CUTTING EXEMPTION offered to `busy_packages_of_pulls` ------------------
    # THE THIRD OCCURRENCE OF THE SAME SHAPE. #4928 made `ci`/`docs` non-reserving on the PLAN leg
    # (`ready-issues.py`, whose own suite pins it end-to-end through `compute_ready`). #4929 reports
    # that the registry's CLAIM/assemble leg (`dispatch-claim.py::busy_packages_of_pulls`) keeps its
    # own busy-area union with no exemption, so the two extra rows PLAN offers are re-deferred one
    # layer down and the widening is nominal. As with #4819 and #4365 the answer is a CALLABLE off
    # the module `load_dispatch` already loads, not a set to re-type into the registry's source.
    #
    # Same honest scope as #4365: these rows assert the sparq half. They cannot observe the
    # registry, and passing them does NOT mean `busy_packages_of_pulls` has been changed.
    _exempt = _registry_contract()["non_reserving_partitions"]

    # (h) THE NAMES, as loaded — an absent export is the silent-degrade state, not an error.
    chk("[sparq#4929] dispatch-plan exports the exemption the registry's CLAIM leg must call",
        sorted(n for n in _exempt["required_attributes"] if not hasattr(_mod, n)), [])

    # (i) THE DECLARED SET, through the VALIDATED accessor — the raw constant is deliberately not
    # exported, so this is also the assertion that the validated path is the only reachable one.
    chk("[sparq#4929] ...and the validated declaration matches the pinned set",
        sorted(_mod.non_reserving_partitions()), sorted(_exempt["declared"]["partitions"]))

    # (j) THE PREDICATE, through THIS module. One comparison over the whole fixture, so a dropped
    # row is as red as a changed verdict. `deps` and the crate areas are in here as the SAFETY
    # half: #4929 asks explicitly that they stay reserving on both legs.
    chk("[sparq#4929] ...and reproduces registry-contract.toml's reserving fixture",
        {k: _mod.reserves_partition(k) for k in _exempt["parity_fixture"]},
        dict(_exempt["parity_fixture"]))

    # (k) ...and it is NOT `key in {"ci", "docs"}` — the reason to call the predicate rather than
    # copy the set. `ci-fragments` is a live key that resolves INTO the `ci` partition, so a
    # per-string mirror would keep reserving a partition `ci` itself does not. This row goes EMPTY
    # if `reserves_partition` is ever flattened to exact-string membership.
    _string_mirror = {k for k in _exempt["parity_fixture"]
                      if k in set(_exempt["declared"]["partitions"])}
    chk("[sparq#4929] ...and disagrees with exact-string membership on the containment keys",
        sorted(k for k in _exempt["parity_fixture"]
               if _mod.reserves_partition(k) != (k not in _string_mirror)),
        ["ci-fragments"])

    # (l) THE FAIL-SAFE, both directions, INJECTED rather than monkey-patched: malformed degrades
    # to fully RESERVING (today's behaviour), a well-formed declaration is honoured. Without the
    # second half a validator that voided everything would pass the first half perfectly.
    #
    # `None` is NOT in this list and must not be: through the parameter it is the SENTINEL meaning
    # "use the repository's own declaration", so it returns `{ci, docs}` — asserted as such by row
    # (i). A declaration that is literally `None` is the monkey-patched case and is covered by
    # `ready-issues.py --self-test`, which owns the constant. Writing it here instead reds this row
    # for the wrong reason, which is how it was first written.
    chk("[sparq#4929] ...and a malformed declaration degrades to RESERVING, never the reverse",
        sorted({bool(_mod.non_reserving_partitions(bad)) for bad in (
            "ci", 7, {"ci": True}, {"ci", 7}, ["ci", ""], ("ci", None), {_ready.GLOBAL})}),
        [False])
    chk("[sparq#4929] ...and a well-formed one is honoured (the fail-safe is not vacuous)",
        sorted(_mod.non_reserving_partitions({"ci"})), ["ci"])

    print("dispatch-plan self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


# ---------------------------------------------------------------------------------------------------
# Dry-run CLI: fetch live ready issues (reusing the readiness engine's live path), plan, print.
def _print_table(plan):
    if not plan:
        print("(no ready issues — the dispatch plan is empty; nothing to dispatch)")
        return
    cols = ["number", "priority", "package", "role", "model_chain", "agent", "escalate"]

    def cell(row, c):
        v = row[c]
        if c == "number":
            return f"#{v}"
        if c == "priority":
            return f"P{v}" if v is not None else "P?"
        if c == "model_chain":
            return ">".join(v) if v else "-"
        return str(v) if v is not None else "-"

    widths = {c: max(len(c), *(len(cell(r, c)) for r in plan)) for c in cols}
    header = "  ".join(c.ljust(widths[c]) for c in cols)
    print(header)
    print("  ".join("-" * widths[c] for c in cols))
    for r in plan:
        print("  ".join(cell(r, c).ljust(widths[c]) for c in cols))
    print(f"\n{len(plan)} issue(s) would be dispatched (dry-run — no account claimed, no worker "
          "triggered).")


def main():
    ap = argparse.ArgumentParser(description="Pure dispatch planner (dry-run) for issue-native "
                                             "orchestration.")
    ap.add_argument("--repo", default="sparq-org/sparq")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--dry-run", action="store_true",
                    help="fetch live ready issues, build + print the plan (read-only; never "
                         "dispatches)")
    args = ap.parse_args()
    if args.self_test:
        return _self_test()
    if args.dry_run:
        issues = _ready._fetch(args.repo)          # reuse the readiness engine's live `gh` fetch
        ready = compute_ready(issues)
        plan = plan_dispatch(ready, _routing_doc())
        _print_table(plan)
        return 0
    ap.print_help()
    return 0


if __name__ == "__main__":
    sys.exit(main())
