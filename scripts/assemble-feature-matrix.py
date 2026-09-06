#!/usr/bin/env python3
# Assemble the feature-matrix `opt-in-features` strategy matrix from the per-crate
# fragment files in .github/feature-matrix.d/*.yml and emit it as ONE JSON object,
# `{"include": [ {name, crate, features, test}, ... ]}`, ready for `fromJSON` in the
# workflow. [OPUS-4.8] bead sq-ibrze.
#
# WHY: the opt-in feature matrix used to be a single static `include:` list inside
# feature-matrix.yml. Every opt-in-feature PR appended a leg to that ONE shared list,
# so concurrent PRs collided textually on adjacent additions (this cost ~8 merge-fixer
# passes in one session). Splitting the list into per-crate fragment files means adding
# a crate's leg edits only that crate's fragment — concurrent PRs for DIFFERENT crates
# never touch the same file, so they auto-merge.
#
# GATE-CRITICAL CONTRACT (do NOT relax): the assembled leg's check-run NAME in CI is
# `opt-in <name>` (the job is `name: opt-in ${{ matrix.name }}`). The single required
# `ci-summary / gate` aggregator discovers every leg as a REQUIRED check BY NAME, so the
# set of emitted `name` values MUST stay byte-identical to the pre-split static list, or
# branch protection stops recognising required checks. This script therefore:
#   - reads fragments in DETERMINISTIC (sorted-filename) order so the emitted order is
#     stable run-to-run (order does not change check-run NAMES, but determinism keeps
#     logs/diffs clean);
#   - validates every leg has exactly {name, crate, features, test} with the right types
#     and a NON-EMPTY `features` (a matrix leg cannot pass an empty `--features`); and
#   - FAILS LOUD on a duplicate `name` across fragments (two legs with the same check-run
#     name would collapse into one gating check — a silent coverage hole).
#
# Usage:
#   python3 scripts/assemble-feature-matrix.py            # prints {"include": [...]} JSON
#   python3 scripts/assemble-feature-matrix.py --names    # prints the sorted check-run
#                                                         #   names ("opt-in <name>"), one
#                                                         #   per line (for the gate-name
#                                                         #   preservation proof / tests)
#   ... --select-mode <mode> --affected '<json array>'    # [FABLE-5] sq-fmx4u.3: when
#                                                         #   mode == "selected", keep only
#                                                         #   legs whose crate is affected;
#                                                         #   ANY other/malformed input
#                                                         #   fail-closes to the FULL set
#   ... --event <name> --tier <test|check> [--shard e|r]  # [SONNET-4.6] sq-ldg8c: tier +
#                                                         #   event awareness (see below)
# Exit non-zero (with a diagnostic on stderr) on any malformed fragment.
#
# THE TWO-FILE SCOPE RULE ([SONNET-4.6] issue #2384). The rule keys off the emitted
# leg-NAME set, nothing else. Adding, renaming or removing a leg is a TWO-FILE change:
# the crate fragment AND the gate-name golden
# `scripts/tests/feature-matrix-legnames.golden.txt`, which test_feature_matrix_assemble.py
# compares byte-for-byte. Any bead / issue / PR that changes the emitted leg-name set MUST
# also scope the golden, or its own spec forbids the update it requires. A fragment edit
# that CANNOT change a `name:` — `features`, `test`, `tier`, `tier-reason` or comments —
# leaves the name set untouched and stays a SINGLE-file change; do not scope the golden
# into it. Regenerate the golden (never hand-edit):
#   python3 scripts/assemble-feature-matrix.py --names \
#     > scripts/tests/feature-matrix-legnames.golden.txt
# See `.github/feature-matrix.d/README.md` for the full contract.
#
# TIER + EVENT AWARENESS (bead sq-ldg8c; design research/feature-matrix-pyramid.md §3/§5).
# A fragment leg MAY carry an optional `tier:` field (plus an optional reviewed
# `tier-reason:` override the ENFORCER reads — the assembler only allows the key):
#   - MISSING tier            => `test`  (a full build+test+clippy leg; the default).
#   - `tier: test`            => full leg.
#   - `tier: check`           => a DEMOTED leg (the T1 check tier — compile+clippy only).
#   - anything else / null    => HARD ERROR, exit non-zero. A demotion is NEVER inferred
#                                from an unrecognised value ("never silently demotes").
# The `--event <name>` + `--tier <test|check>` flags then partition the legs:
#   - TIERED events (pull_request, merge_group): `--tier test` (default) emits the legs
#     whose effective tier is `test`; `--tier check` emits the `check`-tier legs (the
#     separate T1 output). A demoted leg thus LEAVES the per-PR `opt-in <name>` matrix and
#     is covered by the check-tier job instead.
#   - BACKSTOP events (push, schedule, workflow_dispatch, unknown/absent): the FULL
#     per-merge backstop — ALL legs run as FULL legs, so `--tier test` emits EVERY leg
#     (byte-identical to today) and `--tier check` emits NONE (the check tier is empty on
#     a full run).
# `--shard engine|rest` further splits the (tier/selection-filtered) legs for the T1 job's
# two build shards: `engine` = the sparq-engine legs (each recompiles the engine frontend
# per feature state — the dominant cost), `rest` = every other crate.
# The tier/shard filters AND with the existing --select-mode selection filter. `--names`
# ALWAYS dumps the FULL set (the gate-name proof must stay tier/event-independent).
#
# BEHAVIOUR-PRESERVATION INVARIANT (sq-ldg8c): with ZERO fragments annotated (no `tier:`
# field anywhere — the state at merge), every event emits exactly today's leg set and the
# check tier is empty. The demotion flip is a separate fragments-only bead (sq-s5dvo).

import glob
import json
import os
import sys

try:
    import yaml
except ImportError:  # pragma: no cover - CI always has pyyaml (actions/setup or apt)
    sys.stderr.write("error: PyYAML is required (pip install pyyaml)\n")
    sys.exit(2)

FRAGMENT_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    ".github",
    "feature-matrix.d",
)

REQUIRED_KEYS = {"name", "crate", "features", "test"}
# [SONNET-4.6] sq-ldg8c: keys a fragment leg MAY carry in addition to REQUIRED_KEYS.
# `tier` demotes a leg to the check tier (see the header); `tier-reason` is an override
# the ENFORCER (feature-matrix-tiers.py) reads — the assembler only allows the key.
# [FABLE-5] CI-economy grouping: `weight` (positive number) is an OPTIONAL explicit
# cost estimate for the leg (unit ≈ minutes of marginal work on a warm dependency
# cache). When absent, a crate-source-size heuristic supplies the default — see
# leg_weight(). The weight only steers bin-packing (--grouped); it never changes
# WHICH legs run or their gate-critical `opt-in <name>` check-run names.
# [OPUS-5] `test-reason` is a second enforcer-only key: a written justification on a
# `test: false` leg for why that (crate, feature)'s coverage lives outside `cargo test`.
# Like `tier-reason` the assembler only ALLOWS it; feature-matrix-tiers.py reads it.
OPTIONAL_KEYS = {"tier", "tier-reason", "test-reason", "weight"}
VALID_TIERS = ("test", "check")

# [FABLE-5] CI-economy grouping (maintainer directive 2026-07-18): bin-pack the
# per-leg matrix into a SMALL number of grouped runner jobs, each targeting <5 min
# wall time. GROUP_CAPACITY is the per-group weight budget in the same unit as
# `weight` (≈ minutes of marginal work on a warm dependency cache — the group job
# pays checkout/toolchain/cache-restore ONCE, so per-leg setup cost is excluded).
GROUP_CAPACITY = 5.0
CRATES_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "crates"
)
_CRATE_KB_CACHE = {}


def _crate_rs_kb(crate):
    """Total .rs source KiB under crates/<crate>/ (0 if absent) — the size signal
    the default leg-weight heuristic uses. Memoised; deterministic on a checkout."""
    if crate not in _CRATE_KB_CACHE:
        total = 0
        for root, _dirs, files in os.walk(os.path.join(CRATES_DIR, crate)):
            for fn in files:
                if fn.endswith(".rs"):
                    try:
                        total += os.path.getsize(os.path.join(root, fn))
                    except OSError:
                        pass
        _CRATE_KB_CACHE[crate] = total // 1024
    return _CRATE_KB_CACHE[crate]


def leg_weight(leg):
    """Effective weight of a leg: the fragment's explicit `weight` if given, else
    a crate-size heuristic (bigger crate => longer compile per feature state; a
    tested leg pays its test run on top). Calibrated so 1.0 ≈ one warm-cache
    minute: base 0.3 (clippy + orchestration) + KiB/2000 (compile) + 0.2 if the
    leg runs tests. sparq-engine (~2.6 MiB of .rs) lands ≈ 1.8; a small crate
    ≈ 0.6. Tune per-leg via the fragment `weight:` field when reality disagrees."""
    explicit = leg.get("weight")
    if explicit is not None:
        return float(explicit)
    kb = _crate_rs_kb(leg["crate"])
    return round(0.3 + kb / 2000.0 + (0.2 if leg["test"] else 0.0), 3)

# [SONNET-4.6] sq-ldg8c: events on which the leg set is PARTITIONED by tier. Every other
# event (push/schedule/workflow_dispatch/unknown/absent) is a FULL per-merge backstop —
# ALL legs run as full legs and the check tier is empty (byte-identical to today).
TIERED_EVENTS = frozenset({"pull_request", "merge_group"})

# The T1 check-tier build shards (design §3): the sparq-engine legs recompile the engine
# frontend per feature state (the dominant cost) and are isolated from the `rest`.
ENGINE_CRATE = "sparq-engine"
VALID_SHARDS = ("engine", "rest")


def load_legs():
    fragments = sorted(glob.glob(os.path.join(FRAGMENT_DIR, "*.yml")))
    if not fragments:
        sys.stderr.write(f"error: no fragment files found under {FRAGMENT_DIR}\n")
        sys.exit(1)
    legs = []
    seen_names = {}
    for path in fragments:
        rel = os.path.relpath(path)
        with open(path, "r", encoding="utf-8") as fh:
            data = yaml.safe_load(fh)
        if data is None:
            # An empty (comment-only) fragment is allowed — a crate placeholder.
            continue
        if not isinstance(data, list):
            sys.stderr.write(
                f"error: {rel}: top level must be a YAML list of legs, got "
                f"{type(data).__name__}\n"
            )
            sys.exit(1)
        for idx, leg in enumerate(data):
            where = f"{rel}[{idx}]"
            if not isinstance(leg, dict):
                sys.stderr.write(f"error: {where}: leg must be a mapping\n")
                sys.exit(1)
            keys = set(leg.keys())
            # [SONNET-4.6] sq-ldg8c: REQUIRED_KEYS must all be present; extras are allowed
            # ONLY from OPTIONAL_KEYS (tier / tier-reason / test-reason / weight). Any other
            # key is still a HARD error (the pre-tier behaviour, minus the optional keys).
            missing = REQUIRED_KEYS - keys
            extra = keys - REQUIRED_KEYS - OPTIONAL_KEYS
            if missing or extra:
                msg = []
                if missing:
                    msg.append(f"missing {sorted(missing)}")
                if extra:
                    msg.append(f"unexpected {sorted(extra)}")
                sys.stderr.write(f"error: {where}: bad keys ({'; '.join(msg)})\n")
                sys.exit(1)
            if not isinstance(leg["name"], str) or not leg["name"].strip():
                sys.stderr.write(f"error: {where}: `name` must be a non-empty string\n")
                sys.exit(1)
            if not isinstance(leg["crate"], str) or not leg["crate"].strip():
                sys.stderr.write(f"error: {where}: `crate` must be a non-empty string\n")
                sys.exit(1)
            if not isinstance(leg["features"], str) or not leg["features"].strip():
                # cargo rejects a bare `--features` with no value, so an empty feature
                # set is never valid for a leg (the default build belongs in ci.yml's
                # workspace lane, not here).
                sys.stderr.write(
                    f"error: {where}: `features` must be a non-empty comma list\n"
                )
                sys.exit(1)
            if not isinstance(leg["test"], bool):
                sys.stderr.write(f"error: {where}: `test` must be a boolean\n")
                sys.exit(1)
            # [SONNET-4.6] sq-ldg8c: normalise the optional tier. A MISSING tier defaults
            # to `test` (a full leg — behaviour-preserving). A present-but-unrecognised
            # value (typo, null, non-string) is a HARD ERROR: a demotion is only ever a
            # reviewed `tier: check` edit, never inferred from a malformed value.
            tier = leg.get("tier", "test")
            if not isinstance(tier, str) or tier not in VALID_TIERS:
                sys.stderr.write(
                    f"error: {where}: `tier` must be one of {list(VALID_TIERS)} "
                    f"(got {tier!r}); a missing `tier` defaults to 'test'. An "
                    f"unrecognised value is never silently demoted to the check tier.\n"
                )
                sys.exit(1)
            # [FABLE-5] CI-economy grouping: optional explicit weight — a positive,
            # finite number. Anything else is a HARD error (a malformed weight must
            # never silently skew the bin-packing).
            weight = leg.get("weight")
            if weight is not None and (
                isinstance(weight, bool)
                or not isinstance(weight, (int, float))
                or not weight > 0
                or weight != weight  # NaN
                or weight == float("inf")
            ):
                sys.stderr.write(
                    f"error: {where}: `weight` must be a positive finite number "
                    f"(got {weight!r}); omit it to use the crate-size heuristic\n"
                )
                sys.exit(1)
            name = leg["name"]
            if name in seen_names:
                sys.stderr.write(
                    f"error: duplicate leg name {name!r} in {where} "
                    f"(first seen in {seen_names[name]}); two legs with the same "
                    f"check-run name would collapse into one gating check\n"
                )
                sys.exit(1)
            seen_names[name] = where
            legs.append(
                {
                    "name": leg["name"],
                    "crate": leg["crate"],
                    "features": leg["features"],
                    "test": leg["test"],
                    # Internal only: the normalised tier drives filter_legs_by_tier and is
                    # STRIPPED before the matrix JSON is emitted (the workflow leg shape
                    # stays exactly {name, crate, features, test}).
                    "tier": tier,
                    # Internal only: explicit weight (None => leg_weight() heuristic).
                    "weight": weight,
                }
            )
    return legs


def filter_legs_by_selection(legs, select_mode, affected_json):
    """[FABLE-5] sq-fmx4u.3 (design §5.2): change-based selection over the leg list.

    Keep only the legs whose `crate` is in the affected closure — but ONLY when
    the selection pre-job says `--select-mode selected`. Every other input is
    FAIL-CLOSED to the FULL leg set (running more is always sound, design §2/§4.3):
      * select_mode empty / "shadow" / "full" / anything else  => full set
      * affected missing, unparsable, or not a list of strings => full set
        (with a loud stderr warning — that combination means a wiring bug, and
        the sound degradation is the status quo, never a skip).
    An affected closure of [] legitimately yields ZERO legs; the workflow's
    `setup` job emits a `legs` count so the matrix job skips instead of
    exploding on an empty `include`. Note: matrix-key selection cannot be a
    job-level `if:` on GitHub Actions — the `matrix` context is not available
    there (docs: contexts availability), which is why this filtering happens at
    assembly time. The gate aggregator discovers checks by polling, so an
    unassembled leg is simply absent (never an "expected but missing" hang);
    requiredness continues to flow through `ci-summary / gate`.
    """
    if select_mode != "selected":
        return legs
    affected = None
    if affected_json:
        try:
            affected = json.loads(affected_json)
        except json.JSONDecodeError:
            affected = None
    if not isinstance(affected, list) or not all(isinstance(a, str) for a in affected):
        sys.stderr.write(
            "warning: --select-mode selected but --affected is missing/malformed; "
            "FAILING CLOSED to the full leg set (sq-fmx4u.3, design §4.3)\n"
        )
        return legs
    keep = set(affected)
    return [leg for leg in legs if leg["crate"] in keep]


def filter_legs_by_tier(legs, event, tier):
    """[SONNET-4.6] sq-ldg8c (design §3/§5): partition the leg list by tier for `event`.

    - TIERED event (pull_request / merge_group): return the legs whose effective
      tier equals the requested tier. `tier: check` legs are thus EXCLUDED from the
      default test-tier matrix and surface only under `--tier check` (the T1 output).
    - Any OTHER event (push / schedule / workflow_dispatch / unknown / absent): the
      FULL per-merge backstop — ALL legs run as full legs, so `--tier test` (the
      default) returns EVERY leg (byte-identical to today) and `--tier check` returns
      NONE (the check tier is empty on a full run).

    `tier` defaults to 'test' (the matrix output) when None. An unrecognised `--tier`
    value is a HARD ERROR (exit non-zero) — never a silent demotion. This filter ANDs
    with filter_legs_by_selection (order-independent; both narrow the set).
    """
    requested = "test" if tier is None else tier
    if requested not in VALID_TIERS:
        sys.stderr.write(
            f"error: --tier must be one of {list(VALID_TIERS)} (got {tier!r})\n"
        )
        sys.exit(2)
    if event not in TIERED_EVENTS:
        # FULL backstop: everything is a full leg; the check tier is empty.
        return list(legs) if requested == "test" else []
    return [leg for leg in legs if leg.get("tier", "test") == requested]


def filter_legs_by_shard(legs, shard):
    """[SONNET-4.6] sq-ldg8c (design §3): split the (already tier/selection-filtered)
    legs into the T1 check-tier's two build shards.

      * shard == "engine" => the sparq-engine legs (each recompiles the engine
        frontend per feature state — the dominant cost, isolated so the rest share a
        warm target dir);
      * shard == "rest"   => every other crate's legs;
      * shard is None     => no split (return the legs unchanged).

    An unrecognised shard value is a HARD ERROR (exit non-zero).
    """
    if shard is None:
        return legs
    if shard not in VALID_SHARDS:
        sys.stderr.write(
            f"error: --shard must be one of {list(VALID_SHARDS)} (got {shard!r})\n"
        )
        sys.exit(2)
    if shard == "engine":
        return [leg for leg in legs if leg["crate"] == ENGINE_CRATE]
    return [leg for leg in legs if leg["crate"] != ENGINE_CRATE]


def group_legs(legs, capacity=GROUP_CAPACITY):
    """[FABLE-5] CI-economy grouping: deterministically bin-pack legs into groups.

    Same-crate legs are clustered FIRST (they share the group's warm target dir —
    consecutive feature-states of one crate recompile only the crate itself, never
    the dependency stack), then each crate's legs are chunked to the capacity and
    the chunks are packed first-fit-decreasing into bins. A single leg heavier
    than the capacity gets its own chunk (never dropped, never split).

    Returns a list of groups, each {"group", "cache_crate", "cache_save", "count",
    "legs"} where `legs` is the ORDERED list of leg dicts (workflow shape:
    name/crate/features/test). Group names/ids are NOT gate-critical — the
    gate-critical `opt-in <name>` per-leg check-runs are emitted by
    scripts/run-feature-matrix-group.py from inside the group job, name-preserved
    byte-for-byte.

    `cache_save` marks the ONE deterministic cache-seed group per `cache_crate` —
    see pick_cache_seeds()."""
    by_crate = {}
    for leg in legs:
        by_crate.setdefault(leg["crate"], []).append(leg)
    # Crates ordered by total weight desc (then name for determinism).
    crate_order = sorted(
        by_crate,
        key=lambda c: (-sum(leg_weight(leg) for leg in by_crate[c]), c),
    )
    chunks = []  # (weight, crate, [legs]) — same-crate, each <= capacity where possible
    for crate in crate_order:
        cur, cur_w = [], 0.0
        for leg in by_crate[crate]:
            w = leg_weight(leg)
            if cur and cur_w + w > capacity:
                chunks.append((cur_w, crate, cur))
                cur, cur_w = [], 0.0
            cur.append(leg)
            cur_w += w
        if cur:
            chunks.append((cur_w, crate, cur))
    # First-fit-decreasing over the chunks (stable: weight desc, then crate name,
    # then original chunk position).
    bins = []  # each: {"weight": float, "chunks": [(weight, crate, legs)]}
    ordered_chunks = sorted(
        ((w, crate, i, chunk) for i, (w, crate, chunk) in enumerate(chunks)),
        key=lambda t: (-t[0], t[1], t[2]),
    )
    for w, crate, _i, chunk in ordered_chunks:
        placed = False
        for b in bins:
            if b["weight"] + w <= capacity:
                b["chunks"].append((w, crate, chunk))
                b["weight"] += w
                placed = True
                break
        if not placed:
            bins.append({"weight": w, "chunks": [(w, crate, chunk)]})
    groups = []
    for idx, b in enumerate(bins, start=1):
        # Dominant crate = the heaviest chunk's crate (chunks were appended in
        # weight-desc order, so the first chunk is the heaviest) — it keys the
        # group's rust-cache shared-key, reusing the existing per-crate cache
        # entries (sq-3sbrr strategy unchanged).
        dominant = b["chunks"][0][1]
        group_legs_flat = [leg for _w, _c, chunk in b["chunks"] for leg in chunk]
        groups.append(
            {
                "group": f"g{idx:02d} {dominant}",
                "cache_crate": dominant,
                "count": len(group_legs_flat),
                "legs": group_legs_flat,
                "weight": round(b["weight"], 3),
            }
        )
    pick_cache_seeds(groups)
    return groups


def _feature_breadth(group):
    """Number of DISTINCT cargo features the group compiles for its `cache_crate`.

    The union over the group's dominant-crate legs — the proxy for "how much of
    that crate's optional dependency graph does building this group populate in
    `target/`". A `service` leg pulls ureq/serde_json; a bare `time-travel` leg
    pulls nothing extra, so the wider group is the more valuable cache seed."""
    feats = set()
    for leg in group["legs"]:
        if leg["crate"] != group["cache_crate"]:
            continue
        feats.update(f.strip() for f in leg["features"].split(",") if f.strip())
    return len(feats)


def _group_identity(group):
    """Stable identity of a group: the sorted tuple of the leg names it carries.

    Used as pick_cache_seeds' FINAL tie-breaker. It must be a property of the
    group ITSELF, never its position in the list handed to that function —
    a positional index makes the seed depend on the order groups happen to be
    supplied in, which is the very scheduling-noise dependence sq-an4by removes.
    Legs are PARTITIONED across groups, so this identity is unique."""
    return tuple(sorted(leg["name"] for leg in group["legs"]))


def pick_cache_seeds(groups):
    """[OPUS-5] sq-an4by: mark exactly ONE cache-seed group per `cache_crate`
    (`cache_save: True`); every other group with that key is RESTORE-ONLY.

    WHY: `cache_crate` keys the rust-cache `shared-key`, and a crate's legs
    bin-pack into SEVERAL groups (sparq-engine: 12). The Actions cache is
    IMMUTABLE — the first writer of a key wins and every later save of the same
    key is rejected — so on a push-to-main run all N same-crate groups raced to
    save one key: the winner (hence WHICH feature states' dependencies got
    cached) was scheduling noise, and the 11 losers each still packed and
    uploaded a whole `target/` before the API 409'd it. Feature-specific
    dependencies could therefore go permanently uncached: once a key is written
    for a given lockfile+rustc hash, later runs restore it EXACTLY, and
    rust-cache skips saving on an exact hit, so the losing groups' deps never got
    a second chance.

    The seed is chosen deterministically — widest feature breadth first (see
    _feature_breadth: the group whose build populates the most of that crate's
    optional dep graph), then heaviest, then the lexicographically-smallest
    _group_identity (the group's own leg names, NOT its position in the list
    passed here: a positional index would let a reordered list win a different
    group, reintroducing the order dependence) — so the cached
    content is a stable, deliberately-chosen superset rather than a race result.
    Saving only ever happens on push-to-main, where ci_select.py returns
    mode=full and the event is a BACKSTOP one, so the seed is computed over the
    same complete leg set every time — PR-run selection filtering can change
    which group is flagged, but those runs are restore-only anyway.

    Mutates `groups` in place; every group gets the key.

    RESIDUAL (deliberate): the seed caches only ITS OWN legs' dependencies. A dep
    reachable only from a feature exclusive to a NON-seed group is still built
    cold. Fixing that needs either a synthetic superset-feature build (features
    across legs are not always co-enableable — several legs are
    `--no-default-features`) or one cache bucket per group, which is what
    sq-3sbrr's per-crate keying deliberately abandoned: 46 buckets would blow the
    repo's 10 GB Actions-cache budget and LRU-evict everything."""
    seen = {}
    for g in groups:
        g["cache_save"] = False
        rank = (-_feature_breadth(g), -g["weight"], _group_identity(g))
        best = seen.get(g["cache_crate"])
        if best is None or rank < best[0]:
            seen[g["cache_crate"]] = (rank, g)
    for _rank, g in seen.values():
        g["cache_save"] = True
    return groups


def _flag_value(argv, flag):
    """Value of `--flag value` in argv, or None."""
    for i, a in enumerate(argv):
        if a == flag and i + 1 < len(argv):
            return argv[i + 1]
    return None


def main():
    legs = load_legs()
    if "--names" in sys.argv[1:]:
        # The golden gate-name proof ALWAYS dumps the full set — selection must
        # never make the byte-identical name contract unverifiable.
        for name in sorted(f"opt-in {leg['name']}" for leg in legs):
            print(name)
        return
    argv = sys.argv[1:]
    # AND-composed filters (order-independent): selection narrows by affected crate,
    # tier partitions by event+tier, shard splits the check tier's build shards.
    legs = filter_legs_by_selection(
        legs,
        _flag_value(argv, "--select-mode"),
        _flag_value(argv, "--affected"),
    )
    legs = filter_legs_by_tier(
        legs,
        _flag_value(argv, "--event"),
        _flag_value(argv, "--tier"),
    )
    legs = filter_legs_by_shard(legs, _flag_value(argv, "--shard"))
    # Strip the internal `tier`/`weight` keys so the emitted leg shape stays exactly
    # {name, crate, features, test} (the workflow matrix contract). Rebuilding in
    # this fixed key order keeps the default output BYTE-IDENTICAL to the pre-tier
    # assembler (sq-ldg8c behaviour-preservation invariant).
    include = [
        {
            "name": leg["name"],
            "crate": leg["crate"],
            "features": leg["features"],
            "test": leg["test"],
        }
        for leg in legs
    ]
    if "--grouped" in argv:
        # [FABLE-5] CI-economy grouping: emit ONE matrix entry per bin-packed GROUP
        # of legs. `legs` is a JSON-encoded STRING (GitHub matrix values must be
        # scalars) — the group job passes it to scripts/run-feature-matrix-group.py,
        # which runs each leg and emits its gate-critical `opt-in <name>` check-run
        # (name byte-identical to the per-leg matrix this replaces). `count` totals
        # feed the workflow's `legs` output (skip-on-zero unchanged).
        groups = group_legs(legs)
        ginclude = []
        for g in groups:
            stripped = [
                {
                    "name": leg["name"],
                    "crate": leg["crate"],
                    "features": leg["features"],
                    "test": leg["test"],
                }
                for leg in g["legs"]
            ]
            ginclude.append(
                {
                    "group": g["group"],
                    "cache_crate": g["cache_crate"],
                    # sq-an4by: the rust-cache `save-if` gate — True for exactly one
                    # group per cache_crate (pick_cache_seeds), so same-key groups no
                    # longer race to write the immutable Actions cache.
                    "cache_save": g["cache_save"],
                    "count": g["count"],
                    "legs": json.dumps(
                        stripped, ensure_ascii=False, separators=(",", ":")
                    ),
                }
            )
        print(
            json.dumps({"include": ginclude}, ensure_ascii=False, separators=(",", ":"))
        )
        return
    # Emit a single-line JSON object so the workflow can capture it with
    # `echo "matrix=$(...)" >> "$GITHUB_OUTPUT"` and feed `fromJSON(... .matrix)`.
    print(json.dumps({"include": include}, ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    main()
