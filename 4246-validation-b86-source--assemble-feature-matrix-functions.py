# scripts/assemble-feature-matrix.py:166-273
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

# scripts/assemble-feature-matrix.py:313-337
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

# scripts/assemble-feature-matrix.py:364-432
def group_legs(legs, capacity=GROUP_CAPACITY):
    """[FABLE-5] CI-economy grouping: deterministically bin-pack legs into groups.

    Same-crate legs are clustered FIRST (they share the group's warm target dir —
    consecutive feature-states of one crate recompile only the crate itself, never
    the dependency stack), then each crate's legs are chunked to the capacity and
    the chunks are packed first-fit-decreasing into bins. A single leg heavier
    than the capacity gets its own chunk (never dropped, never split).

    Returns a list of groups, each {"group", "cache_crate", "count", "legs"} where
    `legs` is the ORDERED list of leg dicts (workflow shape: name/crate/features/
    test). Group names/ids are NOT gate-critical — the gate-critical `opt-in
    <name>` per-leg check-runs are emitted by scripts/run-feature-matrix-group.py
    from inside the group job, name-preserved byte-for-byte."""
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
    return groups

# scripts/assemble-feature-matrix.py:443-513
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
