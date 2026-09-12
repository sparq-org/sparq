#!/usr/bin/env python3
# [OPUS-4.8] Issue-native orchestration: the readiness engine (replaces `bd ready` + push-frontier).
"""ready-issues.py — compute the dispatchable frontier from GitHub issues, FAIL-CLOSED.

Per the GPT-5.6 review (C1/S2), readiness requires POSITIVE, bot-attested state — never mere absence
of a quarantine label. An issue is READY iff, in priority order, ALL hold:
  * OPEN, and
  * carries `status:ready` (positive attestation the triage/trust pipeline set), and
  * carries exactly ONE valid `priority:P0..P4` (ambiguous/invalid priority → excluded), and
  * carries a `role:*` label, and
  * carries NO gate label (`needs:*`, `trust:untrusted`) and is NOT busy
    (`status:in-progress|blocked|deferred|untriaged`), and
  * has zero open blockers, and
  * none of its PACKAGES (`area:<crate>`) is already taken by an active open PR, an in-progress
    issue, or an earlier-selected ready issue. Human-parked artifacts (`needs:user`,
    `review:needs-user`, `status:blocked`) reserve nothing. A no-package / cross-cutting issue
    reserves a **global partition** that serializes it against ALL other work
    (shared lockfiles/CI/workspace configs).

The snapshot uses real cursor pagination (`gh api --paginate`) with an explicit fail-closed
ceiling. Open blockers are the UNION of GitHub's NATIVE issue dependencies and the legacy
validated `Blocked-by: #NN` body markers — see `open_blocker_count`, which also WARNS per issue
when the two channels disagree, so the union never absorbs their drift silently. Pure
`compute_ready()` is unit-tested; the CLI wraps it over the paginated fetch.
"""
import argparse
import contextlib
import io
import json
import os
import re
import subprocess
import sys
import tomllib

GATE_LABELS = ("needs:", "trust:untrusted")
# [OPUS-5] `status:in-progress-review` was MISSING here while the registry's dispatch.yml keeps
# such rows in `ready_input` precisely because it believes compute_ready() will (a) never select
# them and (b) still let them RESERVE their package. Neither held: the label is not a gate label,
# so an in-progress-review issue that also carried `status:ready` was SELECTABLE, and the reserve
# branch below only fired on `status:in-progress`, so its crate was left free for a second
# dispatch. Both halves fail OPEN into a double-dispatch; see IN_FLIGHT_STATUS below.
BUSY_STATUS = {"status:in-progress", "status:in-progress-review", "status:blocked",
               "status:deferred", "status:untriaged"}
# The statuses that make an OPEN ISSUE in-flight work occupying its area (not merely excluded).
IN_FLIGHT_STATUS = {"status:in-progress", "status:in-progress-review"}
# [GPT-5.6] Parked is not in-flight: these terminal, human-owned snapshot artifacts cannot
# advance autonomously, so they must not reserve an area indefinitely. Removing the label in a
# later snapshot restores occupancy immediately; there is no remembered park state.
PARKED_AREA_LABELS = {"needs:user", "review:needs-user", "status:blocked"}
# [OPUS-5] sparq#4819. The MACHINE park — set by the registry's park policy when the review loop
# runs out of rounds — is DELIBERATELY not in PARKED_AREA_LABELS. Unlike the three human-owned
# labels above, a machine park can be lifted by the machine on any tick, so "cannot advance
# autonomously" is not true of it by definition and its area must NOT be released on the label
# alone. It is released only against a POSITIVE, per-row proof of inertness (`INERT_FIELD`), which
# the two partition legs must agree on; see `is_provably_inert`.
MACHINE_PARK_PR_LABEL = "review:parked"
# The field on an occupancy row that carries that proof. It is NOT derived here: the registry's
# `_pull_inactivity_decision` (scripts/dispatch-claim.py) is the ONE implementation of "is this PR
# provably inert", and this engine CONSUMES its answer rather than minting a second one — two
# partition legs disagreeing about the same PR is the whole defect sparq#4819 describes. Absent,
# non-boolean, or False ⇒ the row keeps holding its areas (fail closed).
INERT_FIELD = "inert"
# [OPUS-4.8] an epic is a tracking umbrella (its children are the work) — never dispatchable, even
# with a full ready label-set + zero blockers. Excluded here so a worker never "implements" an epic.
NON_DISPATCHABLE = "kind:epic"
GLOBAL = "__global__"  # the cross-cutting partition (serializes against everything)
_PRIO = re.compile(r"^priority:P([0-4])$")   # only P0..P4 are valid
_PKG = re.compile(r"^area:(.+)$")
_ROLE = re.compile(r"^role:.+$")

# ---------------------------------------------------------------------------
# Partition-key algebra (sparq#4336) — CONTAINMENT-aware, TOTAL.
#
# [OPUS-5] The under-serialisation defect this replaces: `conflict()` compared partition keys by
# EXACT-STRING set overlap, so a key naming a region INSIDE a crate never overlapped its parent
# crate. Five such labels already exist in production — `sparq-server-http`, `sparq-core-nt-dict`,
# `sparq-core-store`, `sparq-engine-exec`, `sparq-conformance-floors` — so an `area:sparq-server`
# issue and an `area:sparq-server-http` issue entered the frontier in the SAME tick, and an
# `area:sparq-server-http` issue entered despite an open PR holding `area:sparq-server`. Two
# workers in one crate with no lock between them. Measured base rate for that pair sharing a file:
# 57.1% of same-crate 24h PR pairs (research/crate-region-parallelism.md §4). Textual collisions
# surface as merge conflicts; SEMANTIC ones (both compile, both pass, together broken) are
# invisible to git and reach the merge-group gate at the cost of a dequeue plus a batch bisect.
#
# Under-serialisation is the CORRUPTING direction. Over-serialisation only costs delay. So the
# mapping below is TOTAL and biased to over-reserve: every key resolves to the coarsest partition
# that could contain it, and a key we cannot place at all resolves to GLOBAL.
_SEP = "-"                       # the hierarchy separator inside an `area:` key
_PARTITION_MEMO = {}             # key -> path, for the default (workspace-derived) root set
_WORKSPACE_ROOTS = None          # lazily scanned; None = not yet read
# The manifest that makes the scan CHECKABLE. `crates/<name>` members are an explicit list here
# (no globs), so the tree can state its own floor and a crate added next month raises it with no
# code change — see `assert_workspace_tree`.
WORKSPACE_MANIFEST = "Cargo.toml"
# The one directory the workspace keeps its crates in. Mirrors `pr-area-labels.py`'s CRATES_DIR,
# whose `workspace_members()` parses the same manifest field for the PR-side deriver.
CRATES_DIR = "crates"


class DegeneratePartitionRoots(RuntimeError):
    """The tree handed to the partition algebra is not this workspace, so it cannot be partitioned.

    [OPUS-5] `workspace_roots()` derives partition semantics from a DIRECTORY LISTING, so the
    algebra silently changes meaning when the listing is wrong. If `crates/` is absent, no
    `sparq-*` key resolves through rule 2 and EVERY one of them falls to rule 3's head segment
    `sparq` — one mega-partition that every sparq crate conflicts on. The frontier then collapses
    to ~1 and the census line looks completely normal, because `candidates` and `top-contended`
    are computed from label sets and never mention the collapse.

    MEASURED (2026-07-28, live sparq snapshot, sparq's own engine): with the real tree the ready
    lane selected 4 rows; with a scripts-only tree — same snapshot, same code, same labels — it
    selected 2, and 185 of 377 refusals were attributed to a single phantom `sparq-algos` key held
    by one PR. That was an ACCIDENT in a repro harness, not a hypothetical: nothing in the engine,
    the workflow, or the census could tell the two runs apart.

    So the scan is asserted instead of trusted, and a violation REFUSES TO PLAN. Returning an
    empty frontier would have been worse than useless — it prints `frontier=0` and reads as an
    ordinary fully-contended tick. A raise cannot be mistaken for a plan.
    """


def _repo_root():
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


_GLOB_CHARS = "*?["             # cargo expands `members` with the `glob` crate


def _crate_member_root(member):
    """(partition root, is_glob) for one `[workspace] members` entry, or (None, False).

    Only `crates/...` entries matter — a member elsewhere in the tree (sparq declares none, but
    `exclude` names `gui/src-tauri` and `vendor/spargebra`) names no `crates/` partition root.

    THE FORMS CARGO ACCEPTS, all of which this must survive (PR #4925 review — every one of them
    refused a complete, valid tree):
      * `crates/sparq-core`        -> ("sparq-core", False)   the plain form sparq uses today
      * `crates/sparq-core/`       -> ("sparq-core", False)   a trailing slash is legal
      * `crates/sparq-core/derive` -> ("sparq-core", False)   a NESTED member (the proc-macro
        pattern) is a member, but it is NOT a partition root: it lives INSIDE `sparq-core` and
        `workspace_roots` never scans that deep, so the root it must be checked against is its
        first segment under `crates/`.
      * `crates/*`                 -> (None, True)            a glob delegates the member list to
        the tree itself, so it names no specific root and cannot be checked against one.
    """
    if not isinstance(member, str):
        return None, False
    # `.strip()` handles whitespace padding; the empty-segment filter below already absorbs a
    # trailing slash and a doubled separator, so there is deliberately no `.strip("/")` here — it
    # was measured to change NO input (a mutation probe could not kill it), and an unkillable
    # guard reads as protection while providing none.
    parts = [p for p in member.strip().split("/") if p]
    if len(parts) < 2 or parts[0] != CRATES_DIR:
        return None, False
    if any(c in parts[1] for c in _GLOB_CHARS):
        return None, True
    return parts[1], False


def declared_crate_roots(repo_root):
    """`(explicit roots, globbed)` — the `crates/` partition roots `Cargo.toml` names.

    The floor for `assert_workspace_tree` is read from the manifest rather than written down, so
    it tracks the workspace automatically. `globbed` is True when any member delegates to a glob;
    the caller must then fall back to a structural floor, because a glob states no root to check.
    Returns `(set(), False)` when the manifest is missing or unreadable — the caller treats that
    as its own failure, not as a floor of zero.
    """
    path = os.path.join(repo_root, WORKSPACE_MANIFEST)
    try:
        with open(path, "rb") as handle:
            manifest = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError):
        return set(), False
    members = (manifest.get("workspace") or {}).get("members")
    if not isinstance(members, list):
        return set(), False
    roots, globbed = set(), False
    for member in members:
        root, is_glob = _crate_member_root(member)
        if root is not None:
            roots.add(root)
        globbed = globbed or is_glob
    return roots, globbed


def _crates_dir_is_populated(base):
    """Does `base/crates` exist and hold at least one directory? The floor a GLOB delegates to."""
    try:
        entries = os.listdir(os.path.join(base, CRATES_DIR))
    except OSError:
        return False
    return any(not e.startswith(".") and os.path.isdir(os.path.join(base, CRATES_DIR, e))
               for e in entries)


def assert_workspace_tree(base, names):
    """Refuse a scanned root set that cannot be this workspace. Raises DegeneratePartitionRoots.

    THREE conditions, all derived FROM THE TREE — there is no magic number here, and adding a
    crate raises the floor by itself:

      1. `Cargo.toml` must be present and name `crates/` workspace members, explicitly or by
         glob. Its absence means we are not looking at the repository root at all (the exact
         shape of the accident that motivated this: a target tree containing only `scripts/`).
      2. EVERY explicitly declared member must have been scanned as a root. This catches what
         condition 1 cannot — a sparse or partial checkout that has the manifest and only some of
         `crates/`, where the missing crates' keys each collapse into `sparq` while the present
         ones resolve correctly. A partly-wrong partition is not safer than a wholly-wrong one;
         it is harder to notice.
      3. If any member is a GLOB, `crates/` must exist and hold at least one crate directory.

    WHY CONDITION 3 EXISTS, and why the obvious fix without it is worse than no fix (PR #4925
    review round 2). Normalising globs so `members = ["crates/*"]` stops being a false positive
    leaves `declared` EMPTY, which makes condition 2 vacuous and condition 1 satisfied — the
    guard becomes a COMPLETE NO-OP for exactly that manifest. MEASURED against a model of the
    normalise-only fix: a glob manifest with `crates/` MISSING, and one with `crates/` EMPTY,
    were both ACCEPTED. Those are the original defect, restored, under the one manifest edit
    (`members = ["crates/*"]`) most likely to be made routinely. A glob states no root to check,
    so it delegates the floor to the tree, and this is that floor.

    ONE LIMITATION, STATED RATHER THAN PAPERED OVER: under a glob a PARTIAL checkout (some crates
    present, others not) is undetectable here, because the manifest has delegated the member list
    to the very tree we are trying to check and there is no independent count to compare against.
    Conditions 1 and 3 still hold; condition 2 has nothing to work with. sparq declares its 67
    members explicitly today, so all three conditions are live on the real tree.
    """
    declared, globbed = declared_crate_roots(base)
    if not declared and not globbed:
        raise DegeneratePartitionRoots(
            f"refusing to partition: no readable `[workspace] members` naming `{CRATES_DIR}/` in "
            f"{base}/{WORKSPACE_MANIFEST}. The partition algebra resolves `area:` keys against "
            f"the directory listing of {base}, and a tree that is not this workspace collapses "
            "every unrecognised `sparq-*` key onto the single head-segment partition `sparq` — a "
            "frontier computed from that is wrong in a way no census line reveals.")
    if globbed and not _crates_dir_is_populated(base):
        raise DegeneratePartitionRoots(
            f"refusing to partition: {base}/{WORKSPACE_MANIFEST} declares its workspace members "
            f"by glob, which delegates the member list to the tree, but {base}/{CRATES_DIR} is "
            "missing or holds no crate directory. There is nothing to partition, and every "
            "`sparq-*` key would collapse onto the head-segment partition `sparq`.")
    missing = sorted(declared - set(names))
    if missing:
        shown = ", ".join(missing[:8]) + (f" (+{len(missing) - 8} more)" if len(missing) > 8 else "")
        raise DegeneratePartitionRoots(
            f"refusing to partition: {len(missing)} of {len(declared)} declared workspace "
            f"member(s) are absent from the scanned tree at {base}: {shown}. Each missing crate's "
            "`area:` key would resolve to the head segment `sparq` instead of to itself, silently "
            "merging unrelated crates into one partition and collapsing the frontier.")


def workspace_roots(repo_root=None):
    """The RECOGNISED partition roots, READ FROM THE REPOSITORY TREE — deliberately not a table.

    A name is a recognised root iff the workspace really contains it as a partition: a crate
    directory under `crates/`, or a top-level repository directory. The tree is the only authority
    that can tell `sparq-engine-serialize` (a REAL sibling crate, its own partition) apart from
    `sparq-engine-exec` (a REGION inside `sparq-engine`) — as strings the two are identical in
    shape, and a hand-written table of which is which is exactly what goes stale. Reading the tree
    means a crate added next month registers itself with no code change, and a region label
    invented next month resolves to its crate with no code change.

    The registry's `dispatch.yml` CLONES this repo and runs this script, so the same tree is
    present there; `--dump-partitions` exports the resolved mapping for a parity fixture.

    [OPUS-5] The scan is now ASSERTED against the workspace manifest before it is returned or
    memoized (`assert_workspace_tree`) — reading semantics off a directory listing means a wrong
    listing silently changes them, and the resulting frontier is indistinguishable from a healthy
    one. A caller that passes an explicit `roots` SET to `partition_path`/`keys_conflict` is
    unaffected: it supplied the roots and owns them (that is how the hermetic fixtures work).
    """
    global _WORKSPACE_ROOTS
    if repo_root is None and _WORKSPACE_ROOTS is not None:
        return _WORKSPACE_ROOTS
    base = repo_root if repo_root is not None else _repo_root()
    names = set()
    for parent in (base, os.path.join(base, CRATES_DIR)):
        try:
            entries = os.listdir(parent)
        except OSError:
            continue
        names.update(e for e in entries
                     if not e.startswith(".") and os.path.isdir(os.path.join(parent, e)))
    assert_workspace_tree(base, names)     # BEFORE the memo: never cache a degenerate scan
    if repo_root is None:
        _WORKSPACE_ROOTS = names
    return names


def _ancestors(key):
    """Every `-`-delimited ancestor prefix of `key`, LONGEST first (`key` itself included)."""
    segs = key.split(_SEP)
    return [_SEP.join(segs[:i]) for i in range(len(segs), 0, -1)]


def partition_path(key, roots=None):
    """TOTAL map from an `area:` key to the hierarchical PARTITION PATH it reserves.

    Resolution, longest-recognised-ancestor first, so the failure direction is STRUCTURAL rather
    than dependent on anyone remembering to register a label:

      1. `GLOBAL` (and any empty/degenerate key) -> `()`, the root of the hierarchy. `()` is a
         prefix of every path, so GLOBAL conflicts with everything — the existing fail-closed
         backstop, now expressed as containment instead of two special cases in `conflict()`.
      2. The longest `-`-ancestor of `key` that the WORKSPACE recognises -> `(that ancestor,)`.
         `sparq-core-store` and `sparq-core-nt-dict` both resolve to `sparq-core`, so they conflict
         with their parent crate AND with each other (same crate, same files — a sibling hole would
         be the identical defect one level down). `sparq-engine-serialize` IS a crate directory, so
         it resolves to ITSELF and does NOT collapse into `sparq-engine`: genuinely unrelated
         crates stay parallel and the frontier survives.
      3. Otherwise the key's HEAD segment -> `(head,)`. A key whose parent the workspace does not
         know still declares a parent in its own structure, and honouring it over-reserves. This
         is what makes an invented `area:upstream-noir` conflict with `area:upstream` with no code
         change, and it leaves every single-segment key (`upstream`, `cli`, `docs`, ...) exactly
         where it is today — those name nothing narrower, so they cannot be under-serialising.

    [OPUS-5] sparq#5128 — WHAT RULE 3'S BUCKET ACTUALLY HOLDS, and why a not-yet-landed crate is
    deliberately left in it. `keys_conflict` compares paths SEGMENT-WISE, never as strings, so
    `("sparq",)` and `("sparq-core",)` are DISJOINT tuples: a key naming a crate that exists only
    on a PR branch resolves to `("sparq",)` and collides with NO crate the tree already knows. The
    bucket holds exactly the keys the tree cannot place — other not-yet-landed crates, and typos.
    The reading that it "conflicts with every `sparq-*` key" is true only of the DEGENERATE tree
    (`DegeneratePartitionRoots`), where rule 2 fires for nothing and everything falls to rule 3;
    that case is refused before it can be planned, not resolved here.

    The residual — two not-yet-landed crates sharing the bucket — is KEPT, and the proposal to
    promote such a key to its own root when its PR adds `crates/<name>/Cargo.toml` is DECLINED,
    for two independent reasons:
      * The bucket is the ONLY thing serialising such a pair, so freeing the key is the
        corrupting direction. NOT the reason first recorded here, which was wrong and is left
        stated so it is not re-derived: nothing ENFORCES that a PR adding
        `crates/<x>/Cargo.toml` also registers `<x>` in the root `[workspace] members`.
        `gate-new-crate.py` (G1) requires a README, plus a benchmark and a `SKILL.md` where
        applicable, and never membership; `assert_workspace_tree` checks only the DECLARED ->
        disk direction (a declared member with no directory, plus an empty tree under a glob),
        so an unregistered `crates/<x>/` directory passes both. Registering it is convention.
        What IS enforced runs AGAINST the promotion. `pr-area-labels.py::derive_areas` is
        all-or-nothing: `crates/<x>/...` attributes to an `area:<x>` label that does not exist
        while the crate is unlanded unless one was hand-created, a single unattributable path
        makes the whole PR `unresolved`, and an unresolved PR derives NO labels — not even the
        `area:workspace` its root-manifest edit would otherwise map to. Nor does failing closed
        widen it instead: `_reserving_packages` gives an unattributable OCCUPANT nothing to
        reserve (only a no-area CANDIDATE goes to `__global__`). Promoting the keys would
        therefore dispatch two workers onto a root manifest
        they both intend to edit, with nothing left to hold them apart; the shared bucket
        over-reserves instead, which is the safe direction.
      * It is not expressible in the published contract. `--dump-partitions` exports a pure
        key -> path mapping for the registry's second occupancy leg; a rule whose answer depends
        on WHICH PR carries the key would resolve one way for an issue and another for a PR, and
        the two legs would disagree — the drift sparq#4929 reports rather than a fix for it.

    The path is currently never deeper than one element ON PURPOSE: a sub-region collapses INTO its
    container rather than becoming a child of it. research/crate-region-parallelism.md §8 rejects
    intra-crate region partitioning as a parallelism lever (14.5% ceiling), and a two-level path
    would reopen the sibling hole. The prefix-based predicate below is depth-agnostic, so a future
    measured, gated subpartition (that record's Phase 1) can add depth without touching it.
    """
    if roots is None:
        memo = _PARTITION_MEMO
        if key in memo:
            return memo[key]
        path = _resolve(key, workspace_roots())
        memo[key] = path
        return path
    return _resolve(key, roots)


def _resolve(key, roots):
    if not key or key == GLOBAL:
        return ()
    for ancestor in _ancestors(key):
        if ancestor in roots:
            return (ancestor,)
    head = key.split(_SEP)[0]
    return (head,) if head else ()


def keys_conflict(a, b, roots=None):
    """Whether two `area:` keys reserve overlapping work — CONTAINMENT, not string equality.

    True iff one partition path is a prefix of the other, i.e. one region contains the other.
    Reflexive, symmetric, and NOT transitive-closed beyond containment: `sparq-core` and
    `sparq-engine` remain independent.
    """
    pa, pb = partition_path(a, roots), partition_path(b, roots)
    return pa[:len(pb)] == pb or pb[:len(pa)] == pa


# ---------------------------------------------------------------------------
# NON-RESERVING (cross-cutting) PARTITIONS — THE ONE PLACE THIS IS DECLARED.
#
# [OPUS-5 2026-07-28] These partitions still ROUTE work and are still valid candidate keys — a
# candidate declaring `area:ci` is derived, counted and dispatchable exactly as before. They
# simply do not OCCUPY: an in-flight PR or in-progress issue holding one of them no longer blocks
# a new worker from being dispatched onto an issue that declares it. See `_reserving_packages`.
#
# THE MEASURED BASIS (live sparq snapshot, open non-parked PRs holding each area, counting
# holder PAIRS that share at least one changed file):
#
#     area          holder pairs   sharing >=1 file
#     area:ci            120          6   ( 5%)   -> exempt
#     area:docs           66          2   ( 3%)   -> exempt
#     area:deps            3          3   (100%)  -> NOT exempt (every pair collides on
#                                                    Cargo.lock; serialising it is correct)
#     crate areas          -          -   (57.1%) -> NOT exempt
#                                                    (research/crate-region-parallelism.md §4)
#
# So the reservation on `ci`/`docs` was refusing ~99% of a partition-starved frontier to prevent a
# 3-5% file collision, while `deps` and the crate areas are serialising real overlap and stay.
# Measured counterfactual on that same snapshot, through this engine: baseline frontier 1;
# `ci` non-reserving 2; `ci`+`docs` non-reserving 3; adding `deps` would give 4 (not taken).
#
# It is also aimed at the right axis only in part, and the comment says so rather than
# overselling it: `ci`/`docs` reservation never serialised PR-vs-PR contention at all (14 open PRs
# co-hold `ci` and 10 co-hold `docs` right now, concurrently — a PR enters those partitions by
# TOUCHING A PATH, with zero admission control). All it ever did was refuse dispatch.
#
# FAIL-SAFE DIRECTION: `non_reserving_partitions()` validates this declaration and returns the
# EMPTY set — i.e. today's fully-reserving behaviour — for anything malformed. Never the reverse.
NON_RESERVING_PARTITIONS = frozenset({"ci", "docs"})


def non_reserving_partitions(declared=None):
    """`NON_RESERVING_PARTITIONS`, VALIDATED. Anything malformed degrades to RESERVING.

    A wrong answer here is asymmetric: too SMALL a set costs dispatch width (today's behaviour,
    which the fleet has been running), while too LARGE a set silently un-serialises real work.
    So the whole declaration is voided by a single bad entry rather than partially honoured, and
    `GLOBAL` (and any degenerate key, which `partition_path` maps to the `()` root that contains
    every partition) can NEVER appear in it — exempting the root would exempt everything, which
    is exactly the "fail toward exempt everything" outcome this must not have.
    """
    raw = NON_RESERVING_PARTITIONS if declared is None else declared
    if isinstance(raw, str) or not isinstance(raw, (set, frozenset, list, tuple)):
        return frozenset()
    names = set()
    for name in raw:
        if not isinstance(name, str) or not name.strip() or name.strip() == GLOBAL:
            return frozenset()
        if partition_path(name.strip()) == ():        # degenerate key -> the containing root
            return frozenset()
        names.add(name.strip())
    return frozenset(names)


def reserves_partition(key, exempt=None):
    """Whether an OCCUPANT declaring `key` reserves it, or merely routes on it.

    Expressed on the PARTITION PATH, not the raw label, so it agrees with `keys_conflict`: every
    key that resolves INTO the `ci` partition (`ci`, and e.g. the live `ci-fragments`) is exempt
    together with it. A per-string exemption would leave `ci-fragments` reserving a partition that
    `ci` itself does not, which reads as a bug to the next person and behaves like one.
    """
    path = partition_path(key)
    if not path:                       # GLOBAL / degenerate: contains everything, never exempt
        return True
    exempt = non_reserving_partitions() if exempt is None else exempt
    return path[0] not in exempt


def partition_dump(keys):
    """`--dump-partitions`' payload: the partition contract, as JSON, offline and API-free.

    [OPUS-5] sparq#4929 adds `non_reserving` + `reserves` alongside the roots/resolution #4365
    exported. The registry has TWO occupancy legs and they do not read this repository the same
    way: `dispatch.yml`'s readiness step `load_dispatch`es `scripts/dispatch-plan.py` and can call
    `reserves_partition` directly, but the CLAIM/assemble leg (`dispatch-claim.py::
    busy_packages_of_pulls`) is a separate script, and if it has no loaded planner then the ONLY
    thing left for it to do is re-type `{"ci", "docs"}` into its own source — a second copy of the
    declaration, which is the drift sparq#4929 reports rather than a fix for it. So the verdict is
    published on the offline channel too, computed by the same validated predicate.

    `reserves` is the per-key ANSWER (`reserves_partition`), not the set: a key that resolves INTO
    an exempt partition (the live `ci-fragments`) is exempt with it, and exact-string membership
    gets that wrong. `non_reserving` is the VALIDATED declaration, so a malformed one publishes the
    empty set — fully reserving, today's behaviour — exactly as the in-process path does.
    """
    return {"roots": sorted(workspace_roots()),
            "non_reserving": sorted(non_reserving_partitions()),
            "resolved": {k: list(partition_path(k)) for k in keys},
            "reserves": {k: reserves_partition(k) for k in keys}}


# --- open blockers: NATIVE GitHub dependencies UNIONED with the legacy body markers -------------
# [OPUS-5] Until this landed, BOTH readers of "is this issue blocked" (this file and the registry's
# dispatch.yml planner step) derived `open_blockers` ONLY by regexing `Blocked-by: #NN` out of the
# issue BODY. GitHub issue dependencies are generally available and the maintainer uses the native
# UI, so a dependency added that way had ZERO effect on dispatch. MEASURED 2026-07-26 over all 1368
# open sparq issues (cursor-paginated, len == GraphQL totalCount): 112 issues carry a native edge
# (170 edges) and 0 of the 427 issues created since 2026-07-20 carry either channel — the marker
# channel is effectively dead for new work while the native channel is the one being written.
#
# UNION, never replace. 3 open issues (#2833, #2975, #3817) carry a marker with NO native edge, and
# the fail-safe direction is one-way: `exclusion_reason` keys on `open_blockers > 0`, so MISSING an
# edge dispatches an issue that is genuinely blocked, while OVER-counting one only delays it.
_MARKER_BLOCKED_BY = re.compile(r"[Bb]locked-by:\s*#(\d+)")
# GitHub's REST list payload carries this per non-PR issue at no extra request. `blocked_by` counts
# only OPEN blockers (`total_blocked_by` counts closed ones too) — MEASURED against GraphQL
# `blockedBy` filtered to state=OPEN over all 1368 open issues: identical sets AND identical
# per-issue counts, and 16 issues have total_blocked_by > blocked_by (a closed blocker), confirming
# a satisfied dependency is already excluded. So a CLOSED blocker never holds an issue here.
NATIVE_SUMMARY = "issue_dependencies_summary"
# A summary that is PRESENT but malformed is a schema change we cannot interpret. Admitting it as
# "0 blockers" is the fail-OPEN direction (it dispatches work that may be blocked), so it counts as
# one unknown blocker instead: the issue is held, loudly, until a human looks.
MALFORMED_SUMMARY_BLOCKERS = 1


def native_summary_spoke(issue):
    """Did the NATIVE channel actually report a count for this issue?

    True iff `issue_dependencies_summary` is PRESENT and well-formed — i.e. exactly the case where
    `native_open_blockers` returns a real count rather than 0-for-absent or the fail-closed
    `MALFORMED_SUMMARY_BLOCKERS` placeholder. The single source of that validity rule; both
    `native_open_blockers` and the drift check in `open_blocker_count` branch on it, so the two
    cannot drift apart.
    """
    summary = issue.get(NATIVE_SUMMARY)
    value = summary.get("blocked_by") if isinstance(summary, dict) else None
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def native_open_blockers(issue, warn=None):
    """OPEN blockers from GitHub's NATIVE dependency edges, per `issue_dependencies_summary`.

    ABSENT summary -> 0 (the honest reading: this snapshot carries no native-dependency data, so
    only the marker channel can speak). That absence is NOT silent — `_fetch` raises a corpus-level
    alarm if NO issue in a whole snapshot carries the field, which is what a GitHub field rename
    would look like and is otherwise indistinguishable from "nothing is blocked".
    PRESENT-but-malformed -> MALFORMED_SUMMARY_BLOCKERS (fail closed, see above).
    """
    summary = issue.get(NATIVE_SUMMARY)
    if summary is None:
        return 0
    if not native_summary_spoke(issue):
        if warn is not None:
            detail = (f"{NATIVE_SUMMARY} is not an object" if not isinstance(summary, dict)
                      else f"{NATIVE_SUMMARY}.blocked_by is {summary.get('blocked_by')!r}, not a "
                           "non-negative int")
            warn(f"#{issue.get('number', '?')}: {detail} — holding the issue (fail-closed)")
        return MALFORMED_SUMMARY_BLOCKERS
    return summary["blocked_by"]


def marker_open_blockers(body, open_numbers):
    """OPEN blockers from validated `Blocked-by: #NN` BODY markers (the legacy channel)."""
    return sum(1 for n in _MARKER_BLOCKED_BY.findall(body or "") if int(n) in set(open_numbers))


def open_blocker_count(issue, open_numbers, warn=None):
    """The UNION of both blocker channels, as the count `exclusion_reason` consumes.

    `max` is the exact union for the only decision that is made from it — an issue is held iff
    `native > 0 or marker_open > 0`, which is precisely `max(...) > 0`. It is a LOWER BOUND on the
    cardinality of the union of the two blocker SETS (the native channel reports a count, not
    numbers, so a native blocker distinct from every marker blocker cannot be added to the marker
    count without double-counting the overlap). Under-reporting the count can only understate the
    delay in `--diagnose`; it can never flip a held issue to ready. MEASURED over all 1368 open
    issues: 0 issues where the two channels disagree on the count, so today max == the true union.

    [OPUS-5] That MEASURED agreement is now ENFORCED rather than merely recorded. The union keeps
    dispatch correct under drift, but it also ABSORBS drift silently — which is the whole
    complaint in #3817: two sources of truth that agree today will diverge, and only the native
    one is what a maintainer sees or edits in the GitHub UI. So a per-issue disagreement is
    reported through `warn`. It is a WARNING, not a hold: the union already decided the dispatch
    question fail-closed, and hard-failing would take the frontier down over a data-entry slip.

    Detection is COUNT-level only, and that is a real limit: the native channel reports a count,
    not blocker numbers, so `native == marker` does not prove the two channels name the SAME
    blockers — one native edge plus one unrelated marker reads as agreement here. This catches
    drift in cardinality (an edge added in the UI, a marker line dropped from a body), not
    substitution. Only rows where the native channel actually SPOKE are compared: an absent
    summary means the snapshot carries no native data (`native_channel_alarm` owns that case) and
    a malformed one already warned in `native_open_blockers`, so neither is re-reported here.
    """
    native = native_open_blockers(issue, warn)
    marker = marker_open_blockers(issue.get("body"), open_numbers)
    if warn is not None and native != marker and native_summary_spoke(issue):
        warn(f"#{issue.get('number', '?')}: BLOCKER-CHANNEL DRIFT — native GitHub dependencies "
             f"report {native} open blocker(s), `Blocked-by:` body markers report {marker}. "
             f"Dispatch is unaffected (the union, {max(native, marker)}, still holds the issue), "
             "but the two sources of truth have diverged and one of them needs fixing.")
    return max(native, marker)


def labels_of(issue):
    return {lb["name"] if isinstance(lb, dict) else lb for lb in issue.get("labels", [])}


def valid_priority(labels):
    """Exactly one valid priority:P0..P4 → its int; zero or multiple or out-of-range → None."""
    ps = {int(m.group(1)) for lb in labels for m in [_PRIO.match(lb)] if m}
    return next(iter(ps)) if len(ps) == 1 else None


def declared_packages(labels):
    """The SET of area:<crate> packages ACTUALLY declared — empty when none are, no global fallback.

    [OPUS-5] Separated from `packages_of` because the empty→GLOBAL fallback is only sound for a
    CANDIDATE (a no-area issue is cross-cutting work that must serialize against everything).
    Applying it to an in-flight artifact inverts the meaning: it turns "we cannot attribute this
    to a crate" into "this seizes every crate". Nothing labels PRs with `area:` — all 60 open
    sparq PRs carried none — so the old rule let a single unlabelled PR hold __global__ and
    reduce the whole frontier to zero. See `_reserving_packages`.
    """
    return {m.group(1) for lb in labels for m in [_PKG.match(lb)] if m}


def packages_of(labels):
    """The SET of all area:<crate> packages; empty → the serializing global partition.

    CANDIDATE-side rule (fail-closed): an unlabelled issue is treated as cross-cutting.
    """
    return declared_packages(labels) or {GLOBAL}


def _reserving_packages(labels):
    """OCCUPANCY-side rule: an in-flight artifact reserves ONLY the areas it declares.

    [OPUS-5] The deliberate asymmetry with `packages_of`. Fail-closed on a candidate costs one
    dispatch; fail-closed on an occupant costs the ENTIRE fleet, because an unattributable
    occupant would serialize every package at once. An occupant we cannot attribute therefore
    reserves nothing and is instead handled by the linked-issue suppression the registry planner
    applies (`Closes #N` / `sparq-agent/issue-N-*` heads) and, since reg#677, by folding it into
    its source issue's unit — see `unit_reservations`.

    [OPUS-5 2026-07-27] The original justification said "nothing in the pipeline applies `area:`
    labels to PRs". That is NO LONGER TRUE — `scripts/pr-area-labels.py` (pr-area-label.yml) now
    derives them from changed paths, and 109 of 123 open PRs carry one. The RULE is unchanged and
    still right: the deriver is itself fail-closed (it emits no label on unresolved or
    cross-cutting paths), so 9 open PRs still declare nothing, and making those 9 seize
    `__global__` would reproduce the measured whole-fleet stall. Only the stated reason needed
    correcting; leaving a false premise in place is how a correct rule gets "fixed" back.

    [OPUS-5 2026-07-28] ...and of the areas it declares, only the RESERVING ones — the
    cross-cutting `NON_RESERVING_PARTITIONS` (`ci`, `docs`) route work without occupying it. That
    declaration, its measured basis and its fail-safe live in ONE place above; nothing else in
    this file knows the names. This is the OCCUPANCY half ONLY: `packages_of` (the candidate side)
    is untouched, so candidacy is unchanged, and a SELECTED candidate still reserves its own areas
    through `compute_ready`'s `reserve(pkgs, it)` — per-tick width stays one worker per partition,
    which is why the measured effect is +1 frontier row per exempted partition and not +49.
    """
    return {key for key in declared_packages(labels) if reserves_partition(key)}


def has_role(labels):
    return any(_ROLE.match(lb) for lb in labels)


def is_gated(labels):
    return any(lb == g or lb.startswith(g) for lb in labels for g in GATE_LABELS)


def is_busy(labels):
    return bool(labels & BUSY_STATUS)


def is_parked(labels):
    return bool(labels & PARKED_AREA_LABELS)


def is_provably_inert(artifact):
    """Whether the SNAPSHOT PRODUCER attested that this PR row cannot advance or land.

    [OPUS-5] sparq#4819. STRICTLY a consumer of the registry's `_pull_inactivity_decision`, whose
    answer arrives on the row as `INERT_FIELD`. That predicate proves one thing and only one
    thing — a DRAFT with no latched auto-merge, coherent across the listing/detail split-snapshot
    read — and it fails closed on every malformed, latched, non-draft, or head-mismatched shape.
    Nothing about that proof is re-derived here, because a second implementation of it is exactly
    the drift that let sparq's PLAN leg reserve crates the registry's CLAIM leg was freeing in the
    same tick.

    `is True` and not truthiness, deliberately: a producer that stamps a string, a dict, or 1 has
    not proved anything, and a truthiness test would read all three as proof. Absent ⇒ False, so an
    engine invoked WITHOUT the widened occupancy input (sparq's own `--self-test`, any standalone
    run, a registry that has not shipped the producer yet) behaves EXACTLY as before this change.

    WHAT THE PROOF IS WORTH, MEASURED — do not read "provably inert" as "will never land".
    Over the 120 sparq PRs that ever carried `review:parked` (census, 2026-07-14..28; the label
    itself is only ~5 days old, so long parks are structurally unobservable and every rate below
    is a FLOOR):
      * 120/120 were DRAFT at their first park, so the draft conjunct discriminates NOTHING on
        this cohort — 33 of 33 currently-parked open PRs satisfy the whole predicate;
      * 75% were un-parked again (80–88% among parks old enough to have resumed), median 6.2 h
        (N=130 park→unpark pairs, p90 17.3 h);
      * 34% (41/120) went on to MERGE, median 12.4 h after the park, 13 of them past 24 h;
      * 30/120 had auto-merge enabled STRICTLY AFTER the park and 24 of those merged — the latch
        bit is a snapshot, re-read each tick, never a durable property of the PR.
    So this releases a partition a re-admitted PR can return to. That trade is accepted
    deliberately (sparq#4819): the release is confined to crates where the parked PR is the ONLY
    open holder — a union over holders means one un-parked PR keeps the key — so the exposure is
    1 open PR becoming 2 on ~14 low-traffic crates, never a deepening of the 25-to-30-deep
    `docs`/`ci` buckets, which stay held. A dwell-threshold narrowing was measured and REJECTED,
    not skipped: gating on ≥12 h idle leaves the live frontier at 1 (i.e. buys nothing), because
    the crates with a waiting candidate are held by RECENT parks.
    """
    return artifact.get(INERT_FIELD) is True


def occupies_area(artifact):
    """Whether an otherwise in-flight PR/issue occupies its areas in this snapshot.

    [OPUS-5] sparq#4819 adds the SECOND, conditional release. The asymmetry between the two is the
    point, not an inconsistency:

    * `PARKED_AREA_LABELS` (human park) releases on the LABEL ALONE. Nobody can say when a human
      will act, so the option the hold buys has unbounded price.
    * `MACHINE_PARK_PR_LABEL` releases only against a per-row PROOF that the PR is a defused draft
      with no latch. The machine can un-park on any tick, so on the label alone this would free a
      crate a re-admitted PR resumes into.

    PR ROWS ONLY. The proof is about `draft` + `auto_merge`, which no issue has; an issue carrying
    the label keeps its areas. That is the conservative direction and it keeps this carve-out
    exactly co-extensive with the registry leg's own (`busy_packages_of_pulls`, PR-scoped).
    """
    labels = labels_of(artifact)
    if is_parked(labels):
        return False
    if (MACHINE_PARK_PR_LABEL in labels and "pull_request" in artifact
            and is_provably_inert(artifact)):
        return False
    return True


def _artifact_name(artifact):
    if artifact is None:
        return "preseeded occupancy"
    kind = "pr" if "pull_request" in artifact else "issue"
    return f"{kind}#{artifact.get('number', '?')}"


def exclusion_reason(labels, open_blockers=0):
    """Why this label-set is NOT an enumerable ready candidate, or None when it is.

    Label/state gates ONLY — package serialization is a separate, capacity-shaped question
    answered by compute_ready(). Shared by ready_candidates() and the --diagnose taxonomy so the
    two can never drift apart.
    """
    if "status:ready" not in labels:
        return "no status:ready attestation"
    if NON_DISPATCHABLE in labels:
        return f"{NON_DISPATCHABLE} tracking umbrella"
    gates = sorted(lb for lb in labels for g in GATE_LABELS if lb == g or lb.startswith(g))
    if gates:
        return f"gated by {gates[0]}"
    busy = sorted(labels & BUSY_STATUS)
    if busy:
        return f"busy: {busy[0]}"
    parked = sorted(labels & PARKED_AREA_LABELS)
    if parked:
        return f"parked: {parked[0]}"
    if valid_priority(labels) is None:
        return "no single valid priority:P0..P4"
    if not has_role(labels):
        return "no role:* label"
    if int(open_blockers) > 0:
        return f"{int(open_blockers)} open blocker(s)"
    return None


def ready_candidates(issues, log=None):
    """The DRAINABLE backlog: every open issue whose LABELS make it dispatchable.

    [OPUS-5] Distinct from compute_ready(), which additionally serializes to one issue per
    package and therefore answers a CONCURRENCY question, not a "how much work is available"
    question. Conflating the two makes a healthy 200-item backlog behind a 4-wide frontier
    indistinguishable from an empty one. Returns [(priority, number, issue, packages)].

    `log`, when supplied, receives one attributable line per issue that carries the
    `status:ready` attestation and was nonetheless dropped — a silent `continue` here is what
    lets a label-regressed issue leave the frontier forever with zero signal. Issues without
    the attestation are NOT logged (they are not candidates and would flood the log).
    """
    out = []
    for it in issues:
        if str(it.get("state", "OPEN")).upper() != "OPEN":
            continue
        if "pull_request" in it:             # PRs reserve work; they are never issue candidates
            continue
        L = labels_of(it)
        reason = exclusion_reason(L, it.get("open_blockers", 0))
        if reason is not None:
            if log is not None and "status:ready" in L:
                log(f"defer #{it.get('number', '?')}: {reason}")
            continue
        out.append((valid_priority(L), it.get("number", 0), it, packages_of(L)))
    return out


def roleless_ready(issues):
    """The SILENT-INVISIBILITY class: open, attested, un-gated, un-blocked — and yet NO `role:*`.

    [OPUS-5] Ported from the registry planner (its issue #225, where 117 accumulated unnoticed).
    `ready_candidates` drops these BEFORE any plan row exists, so they appear in no plan and in
    no diagnostic and drain never. The fail-closed drop is CORRECT — a role is never guessed —
    but its silence was not, so callers report this count loudly. Pure; returns sorted numbers.
    """
    numbers = []
    for it in issues:
        if str(it.get("state", "OPEN")).upper() != "OPEN" or "pull_request" in it:
            continue
        L = labels_of(it)
        if "status:ready" not in L or NON_DISPATCHABLE in L:
            continue
        if is_gated(L) or is_busy(L) or is_parked(L):
            continue
        if int(it.get("open_blockers", 0)) > 0:
            continue
        if not has_role(L):
            numbers.append(it.get("number", 0))
    return sorted(numbers)


def _own_reservation(row):
    """What ONE snapshot row reserves BY ITSELF, under the existing per-half occupancy rules.

    Extracted verbatim from `compute_ready`'s occupancy loop so `unit_reservations` can union
    member reservations WITHOUT restating (and so drifting from) the rule. Every active open PR is
    in flight, drafts included; an open issue occupies only while `status:in-progress*`; the shared
    parked predicate vetoes both. A row that reserves nothing returns the empty set.
    """
    if str(row.get("state", "OPEN")).upper() != "OPEN" or not occupies_area(row):
        return set()
    labels = labels_of(row)
    if "pull_request" in row or labels & IN_FLIGHT_STATUS:
        return _reserving_packages(labels)
    return set()


def unit_reservations(issues, source_links=None):
    """Occupancy as ONE reservation per UNIT OF WORK — a PR together with the issues it closes.

    [OPUS-5] A worker PR and its source issue are the SAME unit of work, and each was reserving
    independently: MEASURED on the live sparq snapshot (2026-07-27, 1473 open issues / 123 open
    PRs) 65 occupying PRs plus 46 in-flight issues produced 158 reservations over 49 distinct
    partition keys — 20 of those reservations duplicates of a key the unit's other half already
    held. `source_links` (a PR-number -> source-issue-number map from `source_issue_links`) folds
    each pair into one reservation of the UNION, attributed to the PR.

    Two things this deliberately is NOT, both refuted by measurement rather than by argument:

    * It is NOT a frontier lever. `conflict()` tests membership in the SET of held keys, so a
      second occupant on an already-held key is a no-op. Deduplicating 158 -> 138 reservations
      leaves the held set at 49 keys and the live frontier unmoved (3 -> 3). Anything that DOES
      widen the frontier here widens it by RELEASING a key, and releasing is the corrupting
      direction — see the next point.
    * It is NOT permission to drop the issue half. MEASURED over the 94 open PRs with at least one
      open linked source issue: 31 pairs have PR ⊋ issue, 18 identical, 6 have a source issue with
      NO `area:` at all — but 13 have PR ⊊ issue and 26 are INCOMPARABLE (each side declares a key
      the other lacks). So in 39/94 = 41% of pairs the PR's file-derived key set is NOT a superset
      and dropping the issue's reservation would free a key the unit really occupies. The union is
      the only rule that is safe in both directions.

    MONOTONE BY CONSTRUCTION: a unit reserves `⋃ _own_reservation(member)`, so its reservation is a
    superset of every member's own — the dedup can never under-serialise relative to today, whoever
    the members are. Registry CLAIM's extra `areas |= issue_areas or {GLOBAL_PACKAGE}` fail-closed
    step is NOT adopted here: applied to this population it drives the live frontier to 0
    (measured) — the same whole-fleet seizure `_reserving_packages` documents and exists to
    prevent.

    `source_links=None` (the default, and what the registry's `dispatch.yml` passes today) yields
    exactly the legacy per-row reservations, in the legacy order — see
    `test_unit_reservations_without_links_is_identical_to_the_legacy_loop`.
    """
    links = source_links or {}
    by_number = {}
    for row in issues:
        by_number.setdefault(row.get("number"), row)

    def sources_of(pr_number):
        for number in sorted(links.get(pr_number) or ()):
            row = by_number.get(number)
            if row is not None and "pull_request" not in row:
                yield number, row

    consumed = {number for row in issues if "pull_request" in row
                for number, _ in sources_of(row.get("number"))}
    out = []
    for row in issues:                      # input order preserved: attribution is order-sensitive
        number = row.get("number")
        if "pull_request" not in row:
            if number in consumed:          # already reserved as part of its PR's unit
                continue
            areas = _own_reservation(row)
        else:
            areas = set(_own_reservation(row))
            for _number, source in sources_of(number):
                areas |= _own_reservation(source)
        if areas:
            out.append((areas, row))
    return out


def compute_ready(issues, in_progress_packages=None, conflict_log=None, source_links=None):
    """Conflict-free, priority-ordered, FAIL-CLOSED CONCURRENCY FRONTIER.

    This is the one-per-package concurrency WIDTH, not the size of the drainable backlog — use
    `ready_candidates()` for the latter. compute_ready() ⊆ ready_candidates() always.

    `conflict_log`, when supplied, receives one attribution line per conflict-excluded candidate;
    the live default writes those diagnostics to stderr without polluting the frontier rows.

    `source_links`, when supplied, makes a PR and the issues it closes reserve ONCE, as the union
    of both halves — see `unit_reservations`. Omitted, occupancy is byte-identical to the legacy
    per-row loop, so the registry's existing `compute_ready(ready_input)` call is unaffected and
    the two repositories may merge in either order.
    """
    # [OPUS-5] EAGER, so the refusal is a property of the TREE and not of the board. `conflict()`
    # below is the only consumer of the workspace-derived roots, and it is not reached at all when
    # nothing is held or nothing is a candidate — on those ticks a degenerate tree would plan
    # silently and the guard would fire only later, on a busier board. Asserting here makes every
    # call refuse identically. Costs one memoized directory listing per process.
    workspace_roots()
    blockers = {}

    def reserve(pkgs, artifact):
        for pkg in sorted(pkgs):
            blockers.setdefault(pkg, []).append(artifact)

    def conflict(pkgs):
        """The held key that CONTAINS-or-is-contained-by one of `pkgs`, or None.

        [OPUS-5] sparq#4336: was exact-string set overlap (`pkgs & blockers.keys()`) plus two
        hand-written GLOBAL special cases. Exact-string overlap under-serialised every sub-crate
        key against its parent crate; the GLOBAL cases are now just the `()` path being a prefix
        of everything, so there is ONE rule instead of three. Attribution reports the RAW held
        label (not its resolved partition) so a conflict line still names the artifact's own key;
        the coarsest holder wins, then alphabetical, so the message stays deterministic.
        """
        held = [key for key in blockers if any(keys_conflict(key, p) for p in pkgs)]
        if not held:
            return None
        area = min(held, key=lambda key: (len(partition_path(key)), key))
        return area, blockers[area][0]

    for pkg in sorted(set(in_progress_packages or ())):
        reserve({pkg}, None)
    # [GPT-5.6] Every active open PR is in flight (drafts included); open issues occupy only while
    # status:in-progress. The shared parked predicate is applied before either can reserve areas.
    # [OPUS-5] ...and a PR plus the issues it closes are ONE unit reserving the union ONCE.
    for areas, artifact in unit_reservations(issues, source_links):
        reserve(areas, artifact)
    cands = ready_candidates(issues)
    cands.sort(key=lambda c: (c[0], c[1]))   # priority then number (deterministic)
    ready = []
    for _p, _n, it, pkgs in cands:
        held = conflict(pkgs)
        if held is not None:
            area, blocker = held
            message = (f"conflict #{it.get('number', '?')}: area {area} held by "
                       f"{_artifact_name(blocker)}")
            if conflict_log is None:
                print(message, file=sys.stderr)
            else:
                conflict_log(message)
            continue
        reserve(pkgs, it)
        ready.append(it)
    return ready


def _self_test():
    def iss(n, labels, blk=0, state="OPEN"):
        return {"number": n, "state": state, "labels": labels, "open_blockers": blk}

    R = ["status:ready", "role:impl"]

    def quiet(_message):
        pass

    F = [
        iss(1, R + ["priority:P2", "area:sparq-core"]),
        iss(2, R + ["priority:P0", "area:sparq-core"]),
        iss(3, R + ["priority:P1", "area:sparq-engine"]),
        iss(4, R + ["priority:P1", "area:sparq-engine", "needs:user"]),         # gated
        iss(5, R + ["priority:P1", "area:sparq-zk"], blk=2),                     # blocked
        iss(6, R + ["priority:P0", "area:sparq-hdt"], state="CLOSED"),           # closed
        iss(7, R + ["priority:P1", "trust:untrusted", "area:sparq-geo"]),        # untrusted
        iss(8, ["priority:P3", "role:impl", "area:sparq-text"]),                 # not status:ready
        iss(9, R + ["priority:P1", "priority:P2", "area:sparq-sim"]),            # ambiguous priority
        iss(10, R + ["priority:P1", "area:sparq-fedplan", "status:in-progress"]),# in-progress fedplan
        iss(11, R + ["priority:P4"]),                                            # no package -> global
        iss(12, R + ["priority:P1", "area:sparq-hdt"]),                          # hdt (free)
        iss(13, R + ["priority:P0", "area:sparq-text", "kind:epic"]),            # epic -> excluded
    ]
    ok = True

    def check(name, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {name}: {got} (want {want})")

    ready = compute_ready(F, conflict_log=quiet)
    # eligible: 1,2,3,12 (+11 global). 4 gated, 5 blocked, 6 closed, 7 untrusted, 8 no-ready,
    # 9 ambiguous-prio, 10 in-progress, 13 epic (kind:epic → excluded despite a P0 ready label-set).
    # Order by prio: #2(P0 core) -> #3(P1 engine) -> #12(P1 hdt) -> #11(P4 global). core taken by #2
    # so #1(P2 core) excluded. #11 global: only selectable if nothing taken -> excluded.
    check("ready order", [i["number"] for i in ready], [2, 3, 12])
    check("existing readiness fixtures unchanged-green", [i["number"] for i in ready], [2, 3, 12])
    # a P0 epic with an otherwise-perfect ready label-set must NOT dispatch (tracking umbrella):
    check("epic excluded", 13 in [i["number"] for i in ready], False)
    # a lone global issue with an empty board is selectable:
    check("lone global", [i["number"] for i in compute_ready(
        [iss(11, R + ["priority:P4"])], conflict_log=quiet)], [11])
    # global blocks everything else:
    g = compute_ready(
        [iss(11, R + ["priority:P0"]), iss(12, R + ["priority:P1", "area:sparq-hdt"])],
        conflict_log=quiet)
    check("global serializes", [i["number"] for i in g], [11])

    # [GPT-5.6] Parked-occupancy tripwires. These are end-to-end through compute_ready so deleting
    # the predicate, broadening it to every draft, or removing attribution makes --self-test red.
    def pr(n, labels, draft=True):
        return {"number": n, "state": "OPEN", "labels": labels,
                "pull_request": {}, "draft": draft}

    waiting = iss(20, R + ["priority:P1", "area:sparq-store"])
    parked = pr(70, ["area:sparq-store", "needs:user"])
    check("needs:user-parked draft PR does not block ready issue",
          [i["number"] for i in compute_ready([parked, waiting], conflict_log=quiet)], [20])
    unparked = {**parked, "labels": ["area:sparq-store"]}
    check("park-label removal restores snapshot occupancy",
          compute_ready([unparked, waiting], conflict_log=quiet), [])

    # ---------------------------------------------------------------------------------------
    # [OPUS-5] sparq#4819 MACHINE-PARK tripwires. EVERY row runs END-TO-END through
    # compute_ready: an assertion that only inspects `is_provably_inert`'s shape would stay green
    # while the call site in `occupies_area` was deleted, which is the surviving-mutant class this
    # estate keeps measuring. The four mutants each row is written to kill are named inline, and
    # three of them PRESERVE the structure (`is_provably_inert` still exists, still takes a row,
    # is still called from `occupies_area`) while breaking the behaviour claimed for it.
    # ---------------------------------------------------------------------------------------
    def park_pr(n, extra=(), **fields):
        row = pr(n, ["area:sparq-store", MACHINE_PARK_PR_LABEL] + list(extra))
        row.update(fields)
        return row

    def frontier(*rows):
        return [i["number"] for i in compute_ready(list(rows) + [waiting], conflict_log=quiet)]

    # THE HEADLINE CLAIM: an ATTESTED-inert machine-parked PR releases its crate.
    check("attested-inert review:parked PR frees its area",
          frontier(park_pr(74, **{INERT_FIELD: True})), [20])
    # MUTANT 1 — "just add review:parked to PARKED_AREA_LABELS" (the fix the issue forbids) and
    # MUTANT 2 — free on the `draft` bit instead of the attestation. Both make these red: an
    # UNATTESTED parked draft is exactly the shape both mutants would free.
    check("review:parked with NO inertness attestation keeps holding",
          frontier(park_pr(75)), [])
    check("review:parked attested NOT inert keeps holding",
          frontier(park_pr(76, **{INERT_FIELD: False})), [])
    # MUTANT 3 — `is True` weakened to truthiness (`bool(...)`, `if artifact.get(INERT_FIELD):`).
    # Structure-preserving: the helper, its name, and its call site all survive. A producer that
    # stamps a string or an int has proved NOTHING and must not free a crate.
    check("truthy-but-not-True attestations prove nothing",
          [frontier(park_pr(77, **{INERT_FIELD: value}))
           for value in ("yes", 1, ["proof"], {"inert": True})], [[], [], [], []])
    # MUTANT 4 — the `and` linking label to proof turned into `or`, or the label test dropped.
    # Structure-preserving. An inert-attested PR that is NOT machine-parked is an ordinary
    # in-flight draft and must keep its crate.
    check("inertness alone (no review:parked) frees nothing",
          frontier(pr(78, ["area:sparq-store"]) | {INERT_FIELD: True}), [])
    # MUTANT 5 — the `"pull_request" in artifact` clause dropped. An ISSUE has no draft/latch
    # surface for the registry predicate to have proved anything about, so a stamped issue row
    # must be ignored. In-progress so it is an occupant at all.
    check("a machine-parked ISSUE row is never freed by an attestation",
          frontier({**iss(79, ["status:in-progress", "area:sparq-store",
                               MACHINE_PARK_PR_LABEL]), INERT_FIELD: True}), [])
    # The carve-out must not have widened CANDIDATE enumeration (sparq#4819 constraint: 386
    # candidates is correct). `review:parked` is deliberately absent from PARKED_AREA_LABELS, so
    # it is not an exclusion reason — adding it there to "simplify" reds this AND the four rows
    # above that depend on the unconditional-free behaviour it would introduce.
    check("review:parked is not a candidate-exclusion reason",
          exclusion_reason(set(R + ["priority:P1", MACHINE_PARK_PR_LABEL])), None)
    parked_logs = []
    compute_ready([park_pr(80), waiting], conflict_log=parked_logs.append)
    check("an unattested park still names itself in the conflict log",
          parked_logs, ["conflict #20: area sparq-store held by pr#80"])

    active = pr(71, ["area:sparq-store", "review:changes"])
    active_logs = []
    check("non-parked draft PR still blocks",
          compute_ready([active, waiting], conflict_log=active_logs.append), [])
    check("conflict log names the blocking artifact",
          active_logs, ["conflict #20: area sparq-store held by pr#71"])
    per_exclusion_logs = []
    global_then_two = compute_ready([
        iss(30, R + ["priority:P0"]),
        iss(31, R + ["priority:P1", "area:sparq-core"]),
        iss(32, R + ["priority:P2", "area:sparq-engine"]),
    ], conflict_log=per_exclusion_logs.append)
    check("one conflict log per excluded candidate",
          ([it["number"] for it in global_then_two], per_exclusion_logs),
          ([30], ["conflict #31: area __global__ held by issue#30",
                  "conflict #32: area __global__ held by issue#30"]))

    in_progress = iss(72, ["status:in-progress", "area:sparq-store"])
    check("status:in-progress issue still blocks",
          compute_ready([in_progress, waiting], conflict_log=quiet), [])
    check("all terminal park labels remove occupancy",
          [[it["number"] for it in compute_ready(
              [pr(73 + i, ["area:sparq-store", label]), waiting], conflict_log=quiet)]
           for i, label in enumerate(sorted(PARKED_AREA_LABELS))], [[20], [20], [20]])
    # [OPUS-5] sparq#4336 CONTAINMENT fixtures. The registry's dispatch.yml runs THIS --self-test,
    # so these are the assertions that gate the fleet's own copy of the key algebra.
    check("sub-crate key resolves to its crate", partition_path("sparq-server-http"),
          ("sparq-server",))
    check("real sibling crate resolves to itself", partition_path("sparq-engine-serialize"),
          ("sparq-engine-serialize",))
    check("invented sub-crate key resolves to its crate", partition_path("sparq-server-zzz"),
          ("sparq-server",))
    check("degenerate key falls all the way to global", partition_path(""), ())
    check("unrelated crates do not conflict",
          keys_conflict("sparq-core", "sparq-engine"), False)

    # ---------------------------------------------------------------------------------------
    # [OPUS-5] THE DEGENERATE-TREE GUARD. These build real directories rather than stubbing the
    # scan, because the defect IS the scan: `workspace_roots()` reads a directory listing and the
    # algebra silently changes meaning when the listing is wrong. Every row below goes red if
    # `assert_workspace_tree` is deleted or if either of its two conditions is dropped.
    # ---------------------------------------------------------------------------------------
    import shutil
    import tempfile

    _made = []

    def _tree(members, present=None, manifest=True, raw=None, body=None):
        """A throwaway repo root. `members` are declared in Cargo.toml as `crates/<name>`;
        `present` (default: all of them) are the crate directories actually created. `raw`
        replaces the member list with VERBATIM strings (globs, nested members, trailing
        slashes); `body` replaces the whole manifest text (malformed-TOML cases)."""
        base = tempfile.mkdtemp(prefix="ready-roots-")
        _made.append(base)
        os.makedirs(os.path.join(base, "scripts"))
        for name in (members if present is None else present):
            os.makedirs(os.path.join(base, CRATES_DIR, name), exist_ok=True)
        if manifest:
            listed = ", ".join(f'"{m}"' for m in
                               (raw if raw is not None else [f"crates/{n}" for n in members]))
            with open(os.path.join(base, WORKSPACE_MANIFEST), "w", encoding="utf-8") as handle:
                handle.write(body if body is not None
                             else f"[workspace]\nresolver = \"2\"\nmembers = [{listed}]\n")
        return base

    def _refusal(base):
        try:
            workspace_roots(base)
        except DegeneratePartitionRoots as exc:
            return str(exc)
        return ""

    _crates = ["sparq-core", "sparq-engine", "sparq-algos", "sparq-kb"]
    # (1) THE ACCIDENT, REPRODUCED: a scripts-only tree — no Cargo.toml, no crates/ — is exactly
    # what the repro harness handed the engine, and it planned a frontier from a phantom
    # partition with no diagnostic anywhere.
    _scripts_only = _tree([], manifest=False)
    _msg = _refusal(_scripts_only)
    check("[degenerate] a scripts-only tree REFUSES to partition",
          ("refusing to partition" in _msg, "Cargo.toml" in _msg), (True, True))
    # ...and the refusal is NON-VACUOUS: on that same tree the algebra really does merge two
    # unrelated crates into one `sparq` partition. This is the harm the guard exists to stop, so
    # it is asserted directly — deleting the guard makes the row above green and leaves THIS
    # collapse in place, which is precisely how the accident went unnoticed.
    check("[degenerate] ...and that tree really would merge unrelated crates",
          (partition_path("sparq-core", roots={"scripts"}),
           partition_path("sparq-algos", roots={"scripts"}),
           keys_conflict("sparq-core", "sparq-algos", roots={"scripts"})),
          (("sparq",), ("sparq",), True))
    # (2) THE HEALTHY PATH IS UNCHANGED: a complete tree returns its roots and does not raise.
    _full = _tree(_crates)
    check("[degenerate] a complete workspace tree is accepted",
          (_refusal(_full),
           sorted(n for n in workspace_roots(_full) if n not in {"scripts", "crates"})),
          ("", sorted(_crates)))
    # ...and on it the same two keys are INDEPENDENT — the guard changes no semantics, it only
    # refuses trees where the semantics would be wrong.
    _roots = workspace_roots(_full)
    check("[degenerate] ...and unrelated crates stay independent there",
          keys_conflict("sparq-core", "sparq-algos", roots=_roots), False)
    # (3) THE PARTIAL CHECKOUT — manifest present, only some crates on disk. Condition 1 alone
    # cannot see this: the missing crates' keys collapse to `sparq` while the present ones
    # resolve correctly, so the partition is partly right, which is harder to notice than
    # wholly wrong. Dropping the member-existence half of the guard makes this row red.
    _partial = _tree(_crates, present=_crates[:2])
    _msg = _refusal(_partial)
    check("[degenerate] a PARTIAL checkout (manifest + some crates) also refuses",
          ("2 of 4 declared workspace member(s) are absent" in _msg,
           "sparq-algos" in _msg, "sparq-kb" in _msg), (True, True, True))
    # (4) A manifest with no `crates/*` members is not a floor of zero — it is an unusable
    # manifest, and admitting it would let an empty `members = []` disable the guard entirely.
    _empty_members = _tree([], present=["sparq-core"])
    check("[degenerate] an empty member list is a refusal, never a floor of zero",
          "refusing to partition" in _refusal(_empty_members), True)
    # (5) compute_ready REFUSES rather than returning a plausible-looking empty frontier: an
    # empty result prints `frontier=0` and reads as an ordinary fully-contended tick.
    _saved_roots, _saved_memo = _WORKSPACE_ROOTS, dict(_PARTITION_MEMO)
    globals()["_WORKSPACE_ROOTS"] = None
    globals()["_PARTITION_MEMO"] = {}
    _saved_repo_root = globals()["_repo_root"]
    globals()["_repo_root"] = lambda: _scripts_only
    try:
        _planned = compute_ready([iss(90, R + ["priority:P1", "area:sparq-core"])],
                                 conflict_log=quiet)
        _outcome = f"PLANNED {[i['number'] for i in _planned]}"
    except DegeneratePartitionRoots:
        _outcome = "REFUSED"
    finally:
        globals()["_repo_root"] = _saved_repo_root
        globals()["_WORKSPACE_ROOTS"] = _saved_roots
        globals()["_PARTITION_MEMO"] = _saved_memo
    check("[degenerate] compute_ready REFUSES on a degenerate tree (never a quiet empty plan)",
          _outcome, "REFUSED")
    # -------------------------------------------------------------------------------------
    # [OPUS-5] PR #4925 REVIEW, VERDICT: fail — THE FALSE-POSITIVE SURFACE.
    # The first cut of this guard parsed `members` with `m.split("/", 1)[1]`, which is correct
    # for exactly ONE of the forms cargo accepts. Every other legal form refused a COMPLETE,
    # VALID tree, and `members = ["crates/*"]` — a routine edit — would have hard-stopped PLAN
    # for BOTH target repositories on the next tick, having merged green because
    # routing-self-tests.yml's `paths:` filter covered neither `Cargo.toml` nor `crates/**`.
    #
    # `declared_crate_roots` had NO direct coverage, which is why a normalisation fix could pass
    # with no test edits at all. It has some now: each shape below is a NAMED row that goes red
    # if the normalisation is removed.
    # -------------------------------------------------------------------------------------
    check("[members] the plain form sparq uses today",
          declared_crate_roots(_tree(["a", "b"])), ({"a", "b"}, False))
    check("[members] a TRAILING SLASH is legal cargo and names the same root",
          declared_crate_roots(_tree(["a", "b"], raw=["crates/a/", "crates/b"])),
          ({"a", "b"}, False))
    check("[members] a NESTED sub-crate member resolves to its CONTAINING crate root",
          declared_crate_roots(_tree(["a"], raw=["crates/a", "crates/a/derive"])),
          ({"a"}, False))
    check("[members] a GLOB names no root and reports itself as a glob",
          declared_crate_roots(_tree(["a"], raw=["crates/*"])), (set(), True))
    check("[members] a glob+explicit MIX keeps the explicit root and the glob flag",
          declared_crate_roots(_tree(["a"], raw=["crates/*", "crates/a"])), ({"a"}, True))
    check("[members] `?` and `[` are glob metacharacters too",
          (declared_crate_roots(_tree(["a"], raw=["crates/sparq-?"]))[1],
           declared_crate_roots(_tree(["a"], raw=["crates/[ab]"]))[1]), (True, True))
    # Whitespace padding and a doubled separator. This row exists because a mutation probe found
    # the original `.strip("/")` UNKILLABLE (the empty-segment filter already absorbed every
    # trailing slash), while `.strip()` — the part that is load-bearing — had no coverage at all.
    # The dead call is gone; this pins the live one.
    check("[members] whitespace padding and a doubled separator normalise to the same root",
          (declared_crate_roots(_tree(["a"], raw=[" crates/a "])),
           declared_crate_roots(_tree(["a"], raw=["crates//a"]))),
          (({"a"}, False), ({"a"}, False)))
    check("[members] members OUTSIDE crates/ name no crate root",
          declared_crate_roots(_tree(["a"], raw=["gui/src-tauri", "vendor/spargebra"])),
          (set(), False))
    check("[members] a missing or malformed manifest yields no roots and no glob",
          (declared_crate_roots(_tree([], manifest=False)),
           declared_crate_roots(_tree([], body="[workspace\nmembers = ")),
           declared_crate_roots(_tree([], body="[workspace]\nmembers = 3\n"))),
          ((set(), False), (set(), False), (set(), False)))

    # THE FOUR FALSE POSITIVES, END-TO-END: each is a COMPLETE tree and must be ACCEPTED.
    # Each row goes red if `_crate_member_root`'s normalisation is reverted to `split("/", 1)`.
    for _label, _raw in (("glob only `crates/*`", ["crates/*"]),
                         ("glob + explicit mix", ["crates/*", "crates/a"]),
                         ("nested sub-crate member", ["crates/a", "crates/a/derive"]),
                         ("trailing slash", ["crates/a/", "crates/b"])):
        check(f"[members] a COMPLETE tree declaring {_label} is ACCEPTED",
              _refusal(_tree(["a", "b"], raw=_raw)), "")

    # ...AND THE HOLE THAT NORMALISATION ALONE OPENS. With globs normalised away, `declared` is
    # empty for a glob-only manifest, so condition 2 is vacuous and condition 1 is satisfied —
    # the guard silently becomes a NO-OP under precisely the edit most likely to be made. A glob
    # delegates the member list to the tree, so the tree must carry the floor. Both rows below
    # are ACCEPTED by a normalise-only fix (measured) and are the reason condition 3 exists.
    check("[members] a glob manifest with NO crates/ dir still REFUSES",
          ("declares its workspace members by glob" in _refusal(_tree([], raw=["crates/*"])),
           "missing or holds no crate directory" in _refusal(_tree([], raw=["crates/*"]))),
          (True, True))
    _empty_crates = _tree([], raw=["crates/*"])
    os.makedirs(os.path.join(_empty_crates, CRATES_DIR), exist_ok=True)
    check("[members] ...and a glob manifest whose crates/ is EMPTY refuses too",
          "refusing to partition" in _refusal(_empty_crates), True)
    # the guard is still LIVE under a glob: a populated crates/ passes, so condition 3 is a
    # floor and not a blanket refusal of globs.
    check("[members] a glob manifest with a populated crates/ is accepted",
          _refusal(_tree(["a"], raw=["crates/*"])), "")

    for _path in (_scripts_only, _full, _partial, _empty_members, *_made):
        shutil.rmtree(_path, ignore_errors=True)
    check("sibling regions of one crate conflict",
          keys_conflict("sparq-core-store", "sparq-core-nt-dict"), True)
    check("parent+child enter the frontier together? (must not)",
          [i["number"] for i in compute_ready(
              [iss(80, R + ["priority:P1", "area:sparq-server"]),
               iss(81, R + ["priority:P1", "area:sparq-server-http"])], conflict_log=quiet)], [80])
    check("child+parent, reversed order (must not)",
          [i["number"] for i in compute_ready(
              [iss(80, R + ["priority:P1", "area:sparq-server-http"]),
               iss(81, R + ["priority:P1", "area:sparq-server"])], conflict_log=quiet)], [80])
    check("open PR on the parent crate blocks a sub-crate issue",
          compute_ready([pr(82, ["area:sparq-server"]),
                         iss(83, R + ["priority:P1", "area:sparq-server-http"])],
                        conflict_log=quiet), [])
    check("unknown key under an unknown parent still over-reserves",
          keys_conflict("upstream", "upstream-noir"), True)
    check("single-segment unknown key keeps its own partition",
          partition_path("deps"), ("deps",))
    # [OPUS-5] sparq#5128 — RULE 3'S BUCKET, PINNED. The report that a not-yet-landed crate's key
    # "conflicts with EVERY `sparq-*` key" reads the path as a STRING; `keys_conflict` compares it
    # SEGMENT-WISE, so it collides with nothing the tree already recognises. Revert the comparison
    # to a string prefix and the first row goes red.
    check("a not-yet-landed crate key does NOT collide with a landed crate",
          (partition_path("sparq-foo"),
           keys_conflict("sparq-foo", "sparq-core"),
           keys_conflict("sparq-foo", "sparq-server-http")),
          (("sparq",), False, False))
    check("...but two not-yet-landed crate keys DO share rule 3's bucket",
          keys_conflict("sparq-foo", "sparq-bar"), True)
    # ...and that residual is KEPT because the bucket is the only thing serialising such a pair.
    # Their PRs cannot be relied on to co-hold a narrower reserving partition instead: a
    # `crates/<x>/...` path attributes to an `area:<x>` label that does not exist while the crate
    # is unlanded, `derive_areas` is all-or-nothing, so such a PR derives NOTHING — not even the
    # `area:workspace` its root-manifest edit would map to — and an unattributable occupant
    # reserves nothing. Promote the keys and the pair below dispatches together onto a manifest
    # both would edit; leave them in the bucket and only one goes.
    check("two not-yet-landed crate keys serialise on rule 3's bucket",
          [i["number"] for i in compute_ready(
              [iss(84, R + ["priority:P0", "area:sparq-foo"]),
               iss(85, R + ["priority:P1", "area:sparq-bar"])], conflict_log=quiet)],
          [84])
    # ...and it is the SHARED bucket doing that, not the priority ordering: give the same pair
    # keys the tree DOES recognise and both dispatch, so the row above is not vacuous.
    check("...disjoint landed-crate keys in the same pair dispatch together",
          [i["number"] for i in compute_ready(
              [iss(84, R + ["priority:P0", "area:sparq-core"]),
               iss(85, R + ["priority:P1", "area:sparq-engine"])], conflict_log=quiet)],
          [84, 85])
    # ---------------------------------------------------------------------------------------
    # NATIVE dependency edges (the maintainer's own triage action). Every row below is written to
    # go RED if the native read is deleted, if the union is turned into a replacement, or if the
    # closed-blocker exemption is broken. They run END-TO-END through compute_ready wherever the
    # decision (dispatch / hold) is what matters — a pure-count assertion alone would survive
    # `_fetch` never calling open_blocker_count at all, so the `_fetch`-shaped rows are included.
    # ---------------------------------------------------------------------------------------
    def raw_issue(n, labels, body="", summary=None, pr=False):
        """A row in the SHAPE `_fetch` receives from `gh api repos/../issues`."""
        row = {"number": n, "state": "open", "labels": [{"name": lb} for lb in labels],
               "body": body}
        if summary is not None:
            row[NATIVE_SUMMARY] = summary
        if pr:
            row["pull_request"] = {}
        return row

    def summary(open_blockers, total=None):
        return {"blocked_by": open_blockers, "blocking": 0,
                "total_blocked_by": open_blockers if total is None else total, "total_blocking": 0}

    ready_labels = R + ["priority:P1", "area:sparq-core"]
    # (1) THE REGRESSION THIS EXISTS FOR: a native edge and NO body marker must hold the issue.
    native_only = raw_issue(40, ready_labels, body="no marker here", summary=summary(1))
    check("native blocked_by with NO body marker excludes from ready",
          [it["number"] for it in compute_ready(_fetch_rows([native_only]), conflict_log=quiet)],
          [])
    check("...and the same issue with the native edge cleared IS ready",
          [it["number"] for it in compute_ready(
              _fetch_rows([raw_issue(40, ready_labels, body="no marker here",
                                     summary=summary(0))]), conflict_log=quiet)],
          [40])
    # (2) the LEGACY channel must keep working — union, never replace. #2833/#2975/#3817 are live
    # marker-only rows, so a replacement would silently drop them.
    blocker = raw_issue(41, ["role:impl"])
    marker_only = raw_issue(42, ready_labels, body="Blocked-by: #41", summary=summary(0))
    check("marker-only edge (native says zero) still excludes from ready",
          [it["number"] for it in compute_ready(_fetch_rows([blocker, marker_only]),
                                                conflict_log=quiet)], [])
    # (3) a CLOSED blocker must NOT hold the child on either channel. Native: `blocked_by` counts
    # only open blockers while `total_blocked_by` counts the closed one (MEASURED on 16 live
    # issues, e.g. #3264 blocked_by=0 total_blocked_by=1). Marker: #43 is absent from the open set.
    closed_blocked = raw_issue(44, ready_labels, body="Blocked-by: #43", summary=summary(0, total=2))
    check("issue whose ONLY blocker is CLOSED is NOT excluded",
          [it["number"] for it in compute_ready(_fetch_rows([closed_blocked]), conflict_log=quiet)],
          [44])
    # (4) union arithmetic, at the count level, on every channel combination.
    check("open_blocker_count unions both channels (never replaces either)",
          [open_blocker_count(raw_issue(1, [], body=b, summary=s), {41})
           for b, s in (("", None), ("", summary(0)), ("", summary(3)),
                        ("Blocked-by: #41", summary(0)), ("Blocked-by: #41", summary(3)),
                        ("Blocked-by: #99", summary(0)))],
          [0, 0, 3, 1, 3, 0])
    # (5) a PRESENT-but-malformed summary must FAIL CLOSED (hold), never admit.
    warnings = []
    check("malformed native summary holds the issue and says so",
          ([native_open_blockers(raw_issue(45, [], summary=s), warnings.append)
            for s in ({"blocked_by": -1}, {"blocked_by": "1"}, {"blocked_by": True},
                      {"blocked_by": None}, ["not", "a", "dict"])],
           len(warnings)),
          ([MALFORMED_SUMMARY_BLOCKERS] * 5, 5))
    check("...and it is the FRONTIER that holds, not just the count",
          [it["number"] for it in compute_ready(
              _fetch_rows([raw_issue(45, ready_labels, summary={"blocked_by": "1"})]),
              conflict_log=quiet)], [])
    # (6) the DARK-CHANNEL alarm: absent-on-every-issue is a schema regression, not a quiet repo.
    check("native-channel-dark snapshot raises the alarm",
          [("DARK" in line, NATIVE_SUMMARY in line) for line in native_channel_alarm(
              [raw_issue(50, []), raw_issue(51, []), raw_issue(52, [], pr=True)])],
          [(True, True)])
    check("one issue carrying the summary is enough to keep the channel LIT",
          native_channel_alarm([raw_issue(50, []), raw_issue(51, [], summary=summary(0))]), [])
    check("a PR-only snapshot never raises the dark alarm",
          native_channel_alarm([raw_issue(52, [], pr=True)]), [])

    # (7) CHANNEL DRIFT (#3817): the union keeps dispatch correct, but it must not swallow a
    # disagreement between the two sources of truth. Only rows where the native channel actually
    # spoke are compared, and a malformed summary must warn ONCE (its own message), not twice.
    def drift_warnings(issue):
        seen = []
        open_blocker_count(issue, {41}, seen.append)
        return [w for w in seen if "DRIFT" in w]

    check("native-only edge (no marker) is reported as channel drift",
          len(drift_warnings(raw_issue(60, [], body="", summary=summary(1)))), 1)
    check("marker-only edge (native says zero) is reported as channel drift",
          len(drift_warnings(raw_issue(61, [], body="Blocked-by: #41", summary=summary(0)))), 1)
    check("channels that AGREE are silent (both zero, and both one)",
          [len(drift_warnings(raw_issue(62, [], body="", summary=summary(0)))),
           len(drift_warnings(raw_issue(63, [], body="Blocked-by: #41", summary=summary(1))))],
          [0, 0])
    check("a marker whose blocker is CLOSED does not read as drift",
          len(drift_warnings(raw_issue(64, [], body="Blocked-by: #99", summary=summary(0, 1)))), 0)
    check("an ABSENT summary is never drift — that channel never spoke",
          len(drift_warnings(raw_issue(65, [], body="Blocked-by: #41"))), 0)
    # body deliberately carries NO marker, so the fail-closed placeholder (1) differs from the
    # marker count (0) — without the `native_summary_spoke` suppression this row WOULD drift-warn.
    malformed = raw_issue(66, [], body="", summary={"blocked_by": "1"})
    malformed_seen = []
    open_blocker_count(malformed, {41}, malformed_seen.append)
    check("a MALFORMED summary warns once about itself, not again as drift",
          [len(malformed_seen), "DRIFT" in "".join(malformed_seen)], [1, False])
    check("...and drift detection never changes the count the frontier consumes",
          [open_blocker_count(raw_issue(67, [], body=b, summary=s), {41})
           for b, s in (("", summary(1)), ("Blocked-by: #41", summary(0)),
                        ("Blocked-by: #41", summary(3)))], [1, 1, 3])
    check("valid_priority single", valid_priority({"priority:P0"}), 0)
    check("valid_priority ambiguous", valid_priority({"priority:P1", "priority:P2"}), None)
    check("valid_priority out-of-range", valid_priority({"priority:P7"}), None)
    check("packages multi", packages_of({"area:a", "area:b"}), {"a", "b"})
    check("packages none->global", packages_of({"role:impl"}), {GLOBAL})
    check("untriaged is busy", is_busy({"status:untriaged"}), True)
    # paginated-snapshot flattening: multi-page merge, PR rows retained for occupancy, junk tolerated
    check("flatten pages retains PRs", _flatten_pages(
        [[{"number": 1}, {"number": 2, "pull_request": {}}], [{"number": 3}], "junk", [None]]),
        [{"number": 1}, {"number": 2, "pull_request": {}}, {"number": 3}])
    # ---------------------------------------------------------------------------------------
    # [OPUS-5 2026-07-28] NON-RESERVING cross-cutting partitions. END-TO-END through
    # compute_ready, and run HERE and not only in scripts/tests/ because the registry's
    # dispatch.yml executes THIS --self-test against every target before it plans anything — so
    # a tree whose exemption set has been widened to a colliding partition fails at the gate the
    # fleet actually passes through. `scripts/tests/test_readiness_visibility.py::
    # TestNonReservingCrossCuttingPartitions` carries the full contract.
    # ---------------------------------------------------------------------------------------
    def held_board(area, held_by=None):
        return [pr(70, [f"area:{held_by or area}"]),
                iss(20, R + ["priority:P1", f"area:{area}"])]

    def offered(area, held_by=None):
        return [i["number"] for i in compute_ready(held_board(area, held_by),
                                                   conflict_log=quiet)]

    check("a PR holding area:ci no longer refuses a ci-only candidate", offered("ci"), [20])
    check("...same for area:docs", offered("docs"), [20])
    check("...and for a key that resolves INTO the ci partition", offered("ci", "ci-fragments"),
          [20])
    # The SAFETY half. deps: 3 of 3 live holder pairs collide, all on Cargo.lock. Crate areas:
    # 57.1% (research/crate-region-parallelism.md §4). Widening the set to either is the mutation
    # this line exists to kill.
    check("area:deps STILL reserves (every live deps pair collides on Cargo.lock)",
          offered("deps"), [])
    check("crate areas STILL reserve", offered("sparq-core"), [])
    check("sub-crate containment still reserves through the exemption",
          offered("sparq-core-store", "sparq-core"), [])
    check("the global partition can never be exempted", offered("sparq-core", GLOBAL), [])
    # The FAIL-SAFE, both directions: a malformed declaration degrades to TODAY's behaviour, and
    # a well-formed one is honoured — without the second line a fail-safe that voided everything
    # would look perfect.
    _declared = NON_RESERVING_PARTITIONS
    try:
        for _broken in (None, "ci", 7, {"ci": True}, {"ci", 7}, ["ci", ""], {GLOBAL}):
            globals()["NON_RESERVING_PARTITIONS"] = _broken
            check(f"malformed exemption {_broken!r} falls back to RESERVING", offered("ci"), [])
        globals()["NON_RESERVING_PARTITIONS"] = frozenset({"ci"})
        check("...and a well-formed exemption is honoured (fail-safe is not vacuous)",
              offered("ci"), [20])
    finally:
        globals()["NON_RESERVING_PARTITIONS"] = _declared
    # [OPUS-5 2026-07-31] sparq#4929: the OFFLINE channel publishes the same verdict. The registry's
    # CLAIM/assemble leg is a different script from its readiness step and may hold no loaded
    # planner; if `--dump-partitions` does not carry the exemption, its only remaining option is to
    # re-type the set, which is the drift #4929 reports. Asserted on the payload builder rather
    # than on the CLI so the shape is pinned, not just the plumbing.
    _dump = partition_dump(["ci", "ci-fragments", "deps"])
    check("--dump-partitions publishes the validated exemption", _dump["non_reserving"],
          ["ci", "docs"])
    check("...and the per-key verdict, containment-aware (a set alone loses `ci-fragments`)",
          _dump["reserves"], {"ci": False, "ci-fragments": False, "deps": True})
    check("...without dropping the #4365 keys the registry already reads",
          (sorted(_dump), _dump["resolved"]["ci-fragments"]),
          (["non_reserving", "reserves", "resolved", "roots"], ["ci"]))
    # SCOPE: candidacy untouched, and a SELECTED candidate still reserves — per-tick width stays
    # one worker per partition, which is why the live frontier moved 1 -> 3 and not 1 -> ~50.
    check("candidate keying for ci/docs is unchanged", (packages_of({"area:ci"}),
                                                        packages_of({"area:docs"})),
          ({"ci"}, {"docs"}))
    check("a selected ci candidate still reserves ci for the tick",
          [i["number"] for i in compute_ready(
              [iss(20, R + ["priority:P0", "area:ci"]), iss(21, R + ["priority:P1", "area:ci"])],
              conflict_log=quiet)], [20])
    print("ready-issues self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


def _flatten_pages(pages):
    """Flatten `gh api --paginate --slurp` output, retaining PRs as occupancy artifacts."""
    return [i for page in pages for i in (page if isinstance(page, list) else [])
            if isinstance(i, dict)]


_LINK_HEAD = re.compile(r"^sparq-agent/issue-([1-9][0-9]*)-")
_CLOSES = re.compile(r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#([1-9][0-9]*)\b")
TRUSTED_ASSOCIATIONS = {"OWNER", "MEMBER", "COLLABORATOR"}


def source_issue_links(pulls, repo):
    """PR number -> the set of issue numbers that PR is the WORKING ARTIFACT for.

    [OPUS-5] The pairing behind `unit_reservations`, and the single definition of PR->issue
    linkage in this file: `linked_issue_numbers` is now its union, so the suppression set and the
    occupancy pairing can never disagree about what "covered by an open PR" means. Two rules that
    each re-derive linkage is exactly the mint-vs-adopt drift that lets one layer free a key the
    other still holds.

    A fork PR must never link an issue: only a same-repo `sparq-agent/issue-N-*` head is
    pipeline-owned provenance (a fork's head branch text is attacker-controlled), and a closing
    keyword in a body counts only from a trusted author association. PRs with no linkage are
    absent from the map rather than present-and-empty, so `unit_reservations` treats them as
    single-member units.
    """
    links = {}
    for pull in pulls:
        head = pull.get("head") or {}
        ref = head.get("ref") or ""
        body = pull.get("body") or ""
        same_repo = ((head.get("repo") or {}).get("full_name") == repo)
        app_pr = same_repo and _LINK_HEAD.match(ref) is not None
        found = set()
        if app_pr:
            found.update(int(n) for n in _LINK_HEAD.findall(ref))
        if app_pr or str(pull.get("author_association", "")).upper() in TRUSTED_ASSOCIATIONS:
            found.update(int(n) for n in _CLOSES.findall(body))
        if found:
            links.setdefault(pull.get("number"), set()).update(found)
    return links


def linked_issue_numbers(pulls, repo):
    """Issues already covered by an open PR — the suppression the REGISTRY planner applies.

    [OPUS-5] Mirrors dispatch.yml's `linked_issue_numbers` so the local CLI previews the same
    frontier the orchestrator dispatches. Derived from `source_issue_links` so the suppression
    set is, by construction, exactly the set of issues that are somebody's unit-of-work member.
    """
    return set().union(set(), *source_issue_links(pulls, repo).values())


def _fetch_pulls(repo):
    out = subprocess.run(
        ["gh", "api", "--paginate", "--slurp", f"repos/{repo}/pulls?state=open&per_page=100"],
        capture_output=True, text=True, check=True).stdout
    return [p for page in json.loads(out or "[]") if isinstance(page, list)
            for p in page if isinstance(p, dict)]


def _fetch(repo, ceiling=10000):
    """Open-issue snapshot via REAL cursor pagination (`gh api --paginate` follows Link headers),
    replacing the old single-page `--limit 1000` fetch that FAILED CLOSED at exactly 1000 open
    issues — the full bd migration (~900 beads on top of organic issues) crosses that. The explicit
    ceiling still fails closed on a runaway snapshot."""
    out = subprocess.run(
        ["gh", "api", "--paginate", "--slurp",
         f"repos/{repo}/issues?state=open&per_page=100"],
        capture_output=True, text=True, check=True).stdout
    pages = json.loads(out or "[]")
    raw = _flatten_pages(pages)
    if len(raw) >= ceiling:
        raise SystemExit(f"refusing: fetched {len(raw)} >= ceiling {ceiling} — snapshot looks "
                         "runaway (fail-closed). Raise the ceiling deliberately if the backlog "
                         "is really that large.")
    issues = _fetch_rows(raw, warn=lambda m: print(f"::warning::{m}", file=sys.stderr))
    for line in native_channel_alarm(raw):
        print(line, file=sys.stderr)
    return issues


def _fetch_rows(raw, warn=None):
    """The PURE half of `_fetch`: GitHub issue payloads -> readiness-engine rows.

    Split out so `--self-test` exercises the REAL row builder. Asserting on `open_blocker_count`
    alone would stay green with `_fetch` never calling it — which is exactly the shape of the bug
    being fixed (a correct blocker rule that no dispatcher consulted).
    """
    open_numbers = {i["number"] for i in raw if "pull_request" not in i}
    issues = []
    for i in raw:
        row = {"number": i["number"], "state": i["state"], "labels": i["labels"],
               "open_blockers": 0}
        if "pull_request" in i:
            row["pull_request"] = i["pull_request"]
            row["draft"] = i.get("draft")
        else:
            row["open_blockers"] = open_blocker_count(i, open_numbers, warn)
        issues.append(row)
    return issues


def native_channel_alarm(raw):
    """The GUARD against the native blocker channel going DARK without anyone noticing.

    `native_open_blockers` reads an ABSENT `issue_dependencies_summary` as 0 — correct for an old
    snapshot, and indistinguishable from "GitHub renamed the field" if nobody checks. MEASURED
    2026-07-26: the field is present on 1368/1368 open sparq ISSUES and 0/104 PR rows, so "no
    non-PR row carries it" is a schema regression, not a quiet repo. Returns the lines to print
    (pure, so the check itself is testable rather than a side effect nobody exercises).
    """
    non_pr = [i for i in raw if isinstance(i, dict) and "pull_request" not in i]
    if not non_pr:
        return []
    if any(isinstance(i.get(NATIVE_SUMMARY), dict) for i in non_pr):
        return []
    return [f"::warning::NATIVE BLOCKER CHANNEL IS DARK: none of {len(non_pr)} open issues carries "
            f"`{NATIVE_SUMMARY}`. Native GitHub dependencies are being IGNORED and only "
            "`Blocked-by: #NN` body markers can hold an issue — a maintainer's native dependency "
            "edits have no effect on dispatch until this is fixed."]


def dispatchable_view(issues, linked=()):
    """The rows the ORCHESTRATOR feeds to compute_ready — the single source of the local preview.

    [OPUS-5] Mirrors dispatch.yml's `ready_input` comprehension exactly:

        ready_input = [row for row in readiness_input
                       if "status:in-progress" in row["labels"]
                       or "status:in-progress-review" in row["labels"]
                       or (row["number"] not in linked and trusted(...))]

    The `or` matters and a plain `number not in linked` is WRONG for the dominant live shape.
    A `status:in-progress-review` issue is normally covered by its OWN worker PR, so it is
    always in `linked`; dropping it frees the crate it is actively occupying and the next tick
    dispatches a SECOND worker onto it. dispatch.yml keeps those rows deliberately — "In-progress
    rows are KEPT as inputs: compute_ready never selects them (they are busy), but they must
    still RESERVE their package in its taken-seeding". Executed counterexample, in-review #100 on
    sparq-core covered by its own PR plus attested #200 on sparq-core: the `not in linked` rule
    yields [200], the orchestrator yields [].

    Residual, deliberate divergence: dispatch.yml ALSO requires `trusted(issue, bots)`, which
    needs the issue AUTHOR and the registry's per-repo trusted-bot list. The local snapshot
    carries neither, so the local preview is an UPPER BOUND on the orchestrator frontier — it
    may show a row the registry will drop as untrusted. `trust:untrusted` (the label the triage
    pipeline applies) is still excluded here via GATE_LABELS. Parity is claimed on the
    linked/in-flight axis only.
    """
    linked = set(linked)
    return [it for it in issues
            if it.get("number") not in linked or labels_of(it) & IN_FLIGHT_STATUS]


def occupancy_parity(issues, source_links=None):
    """The PR-HALF-STRIPPED occupancy divergence, as a re-runnable measurement.

    [OPUS-5] `dispatchable_view` claims local/orchestrator parity on the linked/in-flight axis, and
    on the CANDIDATE axis it holds. This measures the OCCUPANCY axis: how much of the held key set
    is contributed by the PR half alone, i.e. exactly what PLAN loses if it is handed the ISSUE
    half only.

    This is a SIZE, not a verdict. Whether the registry actually strips is decided by
    `pr_row_occupancy_probe()` — see there, and do NOT reintroduce an unconditional warning off
    this number: as of registry #773 the orchestrator does NOT strip, so `unheld` is non-empty on
    every healthy board and a warning keyed on it can never clear.

    Returns (pr_aware_keys, issue_only_keys, unheld) where `unheld` is the set of keys the
    issue-only view fails to hold. Pure, so `--diagnose` and the test suite read the same number.
    """
    def keys(rows, links):
        held = set()
        for areas, _artifact in unit_reservations(rows, links):
            held |= areas
        return held
    issue_only = [row for row in issues if "pull_request" not in row]
    pr_aware = keys(issues, source_links)
    stripped = keys(issue_only, source_links)
    return pr_aware, stripped, pr_aware - stripped


# The probe id-space the registry uses. Kept as constants so the mirror below and the tests that
# characterise it cannot drift apart by a typo'd literal.
_PROBE_LABELS = ("status:ready", "priority:P0", "role:impl", "area:__pr_probe__")
_PROBE_PR, _PROBE_RIVAL = 999000001, 999000002


def pr_row_occupancy_probe(engine=None):
    """Does THIS repo's readiness engine still earn PR rows in the orchestrator's occupancy input?

    [OPUS-5 2026-07-27] A VERBATIM mirror of `pr_row_aware()` in the registry's
    `.github/workflows/dispatch.yml` (jeswr/agent-account-registry, PR #773, merged 15:54Z). That
    workflow does not strip PR rows any more — it folds every open PR into PLAN's occupancy input
    — but ONLY IF the target repo's planner passes this probe, and a planner that fails it keeps
    exactly the old issue-only behaviour. So "does the orchestrator see our PR rows?" is not a
    property of dispatch.yml at all: it is a property of the function right here in this file, and
    it is the ONLY thing that can silently turn PR-half occupancy back off.

    Why the mirror lives on THIS side of the seam: when the probe fails, the registry says so in
    the REGISTRY's own log, on a repo nobody working in sparq reads. The failure would be caused
    by a sparq commit, land in sparq's CI green, and surface as an unexplained double-dispatch.
    Running the same probe locally puts the alarm where the cause is.

    Called EXACTLY as the registry calls it — two positional-only invocations of `compute_ready`
    with no keyword arguments — so a signature change that breaks the orchestrator's call breaks
    this one too. Two obligations, both executed:

      1. SAFETY — a PR row is never returned as a dispatch candidate (else PLAN emits a plan row
         whose `number` is a pull request and launches an impl worker against it).
      2. EFFECT — a PR row really RESERVES the `area:` keys it declares, so an otherwise-ready
         issue on that key is held. An engine that merely ignores PR rows passes obligation 1
         while making the whole fold an expensive no-op.

    Fail-closed: an engine that raises is one we cannot characterise, so it does not pass.
    Returns `(ok, why)`; `why` is the registry's own wording, so the two logs read alike.
    """
    compute = compute_ready if engine is None else engine
    pull = {"number": _PROBE_PR, "state": "OPEN", "labels": list(_PROBE_LABELS),
            "open_blockers": 0, "pull_request": {}}
    rival = {"number": _PROBE_RIVAL, "state": "OPEN", "labels": list(_PROBE_LABELS),
             "open_blockers": 0}
    try:
        # The engine's default `conflict_log` writes attribution to stderr; the registry tolerates
        # that noise, but on `--diagnose` stderr is otherwise clean, so swallow only the probe's
        # own two calls. The CALL ITSELF stays argument-identical to the orchestrator's.
        with contextlib.redirect_stderr(io.StringIO()):
            alone = [row.get("number") for row in compute([pull])]
            paired = [row.get("number") for row in compute([pull, rival])]
    except Exception as exc:                       # noqa: BLE001 — characterising a hostile target
        return False, f"probe raised {type(exc).__name__}"
    if alone or _PROBE_PR in paired:
        return False, "planner DISPATCHES a pull-request row as if it were an issue"
    if paired:
        return False, "planner does not RESERVE a pull request's declared area"
    return True, "reserves PR areas and never dispatches a PR row"


def diagnose(issues, linked=(), source_links=None):
    """Re-runnable VISIBILITY taxonomy: why the open backlog is not on the frontier.

    Returns (counts, roleless, candidates, frontier, units). Every open issue lands in exactly one
    bucket, so the buckets sum to the open-issue count and no class can hide.

    [OPUS-5] `units` is `unit_reservations(visible, source_links)` — the OCCUPANCY accounting, which
    is the only thing `source_links` can change here. The held KEY SET is provably invariant under
    folding (a unit reserves the union of exactly the members' own reservations, so the union over
    all units equals the union over all members), hence `frontier` is identical with and without
    `source_links` and cannot witness the fold. Returning `units` is what makes the parameter
    observable at this layer — without it the argument would be an equivalent mutant, green under
    deletion, which is precisely how a call site rots.
    """
    linked = set(linked)
    counts, open_issues = {}, []
    for it in issues:
        if str(it.get("state", "OPEN")).upper() != "OPEN" or "pull_request" in it:
            continue
        open_issues.append(it)
        if it.get("number") in linked and not (labels_of(it) & IN_FLIGHT_STATUS):
            reason = "covered by an open linked PR"
        else:
            reason = exclusion_reason(labels_of(it), it.get("open_blockers", 0)) or "ENUMERABLE"
        counts[reason] = counts.get(reason, 0) + 1
    visible = dispatchable_view(issues, linked)
    # `source_links` is deliberately NOT passed to compute_ready here: this call discards the
    # conflict log, and the frontier is PROVABLY invariant under folding (see the docstring and
    # `test_the_held_key_set_is_invariant_under_folding`). Passing it would be an argument no test
    # could ever kill — an equivalent mutant, green under deletion, i.e. a call site that rots.
    return (counts, roleless_ready(open_issues), ready_candidates(visible),
            compute_ready(visible, conflict_log=lambda _m: None),
            unit_reservations(visible, source_links))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default="sparq-org/sparq")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--diagnose", action="store_true",
                    help="print the full visibility taxonomy instead of just the frontier")
    ap.add_argument("--dump-partitions", action="store_true",
                    help="print the recognised partition roots (and the resolution of any KEYs) "
                         "as JSON — the machine-readable contract the registry's dispatch.yml "
                         "mirror must agree with; offline, no API calls")
    ap.add_argument("keys", nargs="*", metavar="KEY",
                    help="area: keys to resolve with --dump-partitions")
    args = ap.parse_args()
    if args.self_test:
        return _self_test()
    if args.dump_partitions:
        json.dump(partition_dump(args.keys), sys.stdout, indent=2, sort_keys=True)
        print()
        return 0
    issues = _fetch(args.repo)
    source_links = source_issue_links(_fetch_pulls(args.repo), args.repo)
    linked = set().union(set(), *source_links.values())
    visible = dispatchable_view(issues, linked)
    if args.diagnose:
        counts, roleless, cands, frontier, units = diagnose(issues, linked, source_links)
        total = sum(counts.values())
        print(f"open issues: {total}")
        for reason, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {n:5d}  {100 * n / total:5.1f}%  {reason}")
        print(f"\ndrainable backlog (ready_candidates): {len(cands)}")
        print(f"concurrency frontier (compute_ready): {len(frontier)}")
        print(f"unit occupancy: {len(units)} unit(s), "
              f"{sum(len(a) for a, _ in units)} reservation(s) over "
              f"{len(set().union(set(), *[a for a, _ in units]))} partition key(s)")
        # [OPUS-5 2026-07-27] Gated on the PROBE, not on `unheld`. `unheld` is non-empty whenever
        # any open PR holds a key its source issue does not — i.e. on every healthy board — so the
        # old unconditional warning could never clear once registry #773 landed, and an alarm that
        # cannot clear trains its readers to skip it. The probe is the real precondition: it is
        # what dispatch.yml itself branches on, so this fires when, and only when, the gap is back.
        # Reported on EVERY diagnose including the healthy one — a check that goes silent when it
        # passes is a check nobody notices has stopped running.
        pr_aware_ok, probe_why = pr_row_occupancy_probe()
        pr_aware_keys, issue_only, unheld = occupancy_parity(visible, source_links)
        if not pr_aware_ok:
            print(f"\n::warning:: ORCHESTRATOR OCCUPANCY GAP: this repo's readiness engine FAILS "
                  f"the registry's pull-request-awareness probe ({probe_why}), so dispatch.yml "
                  f"falls back to stripping PR rows from its readiness input and PLAN holds "
                  f"{len(issue_only)} of {len(pr_aware_keys)} partition key(s). {len(unheld)} "
                  f"key(s) held here by an open PR are FREE there: "
                  f"{', '.join(sorted(unheld)[:20])}")
        else:
            print(f"\norchestrator occupancy: dispatch.yml RESERVES this repo's open PR rows "
                  f"({probe_why}); {len(unheld)} of {len(pr_aware_keys)} held key(s) come from a "
                  f"PR half and would be released if that regressed")
        if roleless:
            print(f"\n::warning:: {len(roleless)} attested issue(s) carry NO role:* label and are "
                  f"INVISIBLE to dispatch: {', '.join('#%d' % n for n in roleless[:20])}")
        return 0
    for it in compute_ready(visible, source_links=source_links):
        L = labels_of(it)
        print(f"P{valid_priority(L)}  #{it['number']:5}  {sorted(packages_of(L))}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
