#!/usr/bin/env python3
# Per-crate MUTATION-TESTING RATCHET gate (sq-8kt3, epic sq-qcnn).
#
# The test-QUALITY companion to the line-coverage ratchet (scripts/coverage-gate.py +
# bench/coverage-floor.json). Line coverage proves a line RAN; mutation testing proves a
# test would actually NOTICE if that line's behaviour changed. cargo-mutants applies small
# semantic mutations (`<` -> `<=`, `a && b` -> `a || b`, replace a body with its default,
# …) and re-runs the crate's tests per mutant. A mutant the tests still PASS on ("missed"
# / surviving) is a test GAP — code that 100% line coverage happily hides because some test
# executes the line without asserting on its result.
#
# Mirrors the coverage ratchet idiom EXACTLY (a committed per-crate FLOOR reviewed in
# diffs). Two modes:
#
#   --seed   <outcomes...>   regenerate bench/mutants-baseline.json from one or more
#                            cargo-mutants `mutants.out/outcomes.json` files (one per
#                            crate, or a workspace run). Each crate's ratchet is the
#                            SURVIVING-mutant CEILING set to measured_surviving + MARGIN
#                            (a little slack so a mutation-set reshuffle on a toolchain bump
#                            never trips the gate). The ceiling only ever FALLS (--seed will
#                            not RAISE it without --allow-higher) — fewer survivors == better
#                            tests, the ratchet of record.
#   --check  <outcomes...>   FAIL (exit 1) if any crate in the baseline has MORE surviving
#                            mutants than its committed ceiling. A crate present in the
#                            baseline but MISSING from the measured set is reported (only
#                            fatal with --require-all — the nightly tier need not measure
#                            every crate every run; whichever crates ran are checked).
#
# WHY SURVIVING-COUNT (not caught%) IS THE STABLE METRIC
# ------------------------------------------------------
# caught% = caught / (caught + missed) has a moving DENOMINATOR: cargo-mutants regenerates
# its mutant set from the current source, so a refactor that adds/removes mutable sites
# shifts the % even when test quality is unchanged — a noisy gate. The SURVIVING (missed)
# COUNT is the thing we actually want to drive to zero: it is an absolute count of
# detected test gaps, monotone in test quality, and unaffected by caught mutants being
# added elsewhere. So the ceiling ratchets the surviving COUNT down. caught% is recorded
# alongside for humans but is NOT the gate. `unviable` mutants (didn't compile) and
# `timeout` mutants are NOT counted as survivors (a timeout means the mutant changed
# behaviour enough to hang — effectively caught; an unviable mutant tests nothing).
#
# WHY a margin: cargo-mutants' generated set is deterministic for a fixed source+toolchain,
# but a compiler/std bump can shift which mutants are viable. MARGIN=2 (mutants) of slack
# keeps the gate meaningful without flaking on that. The ceiling is the ratchet of record —
# LOWER it deliberately (re-seed) as tests improve.
#
# Regenerate after tests improve (fewer survivors):
#   cargo mutants -p <crate> -o target/mutants/<crate>
#   scripts/mutants-gate.py --seed target/mutants/<crate>/outcomes.json
import argparse, json, os, re, sys

MARGIN = 2  # surviving-mutant slack above the measured value

# sq-has2g: the `-shard-<k>-<n>` suffix the nightly lane appends to a sub-sharded
# crate's output dir AND to its artifact name (ci.yml maps `--shard k/n` -> "-shard-k-n"), i.e.
# target/mutants/<crate>-shard-2-24/mutants.out/outcomes.json, and after `gh run download`
# mutants-outcomes-nightly-<crate>-shard-2-24/outcomes.json. cargo-mutants does NOT record the
# shard inside outcomes.json, so this path suffix is the ONLY available shard identity.
SHARD_SUFFIX = re.compile(r"-shard-(\d+)-(\d+)$")

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DEFAULT_BASELINE = os.path.join(ROOT, "bench", "mutants-baseline.json")

# Crates EXCLUDED from mutation testing — same rationale as the coverage ratchet's
# floor:0 crates (bench/coverage-floor.json). These are presence-gated there and would
# produce misleading surviving-mutant noise here (out-of-process / device / non-host
# target), so they are never seeded and are skipped if they somehow appear in a summary.
EXCLUDED = {
    "sparq-cli": "subprocess artifact: tests spawn the COMPILED binary (assert_cmd / "
                 "CARGO_BIN_EXE), so a library mutant is not exercised in-process and "
                 "would survive spuriously. Presence-gated in the coverage ratchet.",
    "sparq-conformance": "test-driver crate: the W3C suites run as the BINARIES "
                 "(`cargo run`), not `cargo test`, so mutants in the driver survive "
                 "spuriously. Floor 0 in the coverage ratchet.",
    "sparq-gpu": "GPU kernels need a real device; host-only mutation testing surfaces "
                 "spurious survivors.",
    "sparq-bench": "perf-harness crate (no #[test]s to catch mutants).",
    "sparq-py": "pyo3 bindings tested via pytest, not the host Rust test profile.",
}


def load(p):
    with open(p) as f:
        return json.load(f)


def crate_of(outcomes_path, doc):
    """Crate (package) name for an outcomes.json. cargo-mutants stamps the package into
    every mutant scenario (`scenario.Mutant.package`), so read it from the first mutant
    entry — robust to where the file lives on disk. Falls back to the mutants.out parent
    dir name only if no Mutant scenario is present (e.g. an all-unviable edge case)."""
    for o in doc.get("outcomes", []):
        sc = o.get("scenario")
        if isinstance(sc, dict) and "Mutant" in sc:
            pkg = sc["Mutant"].get("package")
            if pkg:
                return pkg
    # Fallback: the CI lane writes target/mutants/<crate>/mutants.out/outcomes.json, so
    # the crate is the dir ABOVE mutants.out.
    parent = os.path.dirname(os.path.abspath(outcomes_path))          # .../mutants.out
    if os.path.basename(parent) == "mutants.out":
        parent = os.path.dirname(parent)
    name = os.path.basename(parent)
    # sq-has2g: a SUB-SHARDED crate's dir carries the shard suffix the CI lane
    # appends (target/mutants/sparq-engine-shard-2-6/...). Strip it so all of a crate's
    # shards fall under one key and measured_rows() can aggregate them. Only reached when
    # the outcomes doc has no Mutant scenario to read the package from.
    return SHARD_SUFFIX.sub("", name)


def shard_of(outcomes_path):
    """sq-has2g: (k, n) SHARD IDENTITY for an outcomes.json, or None when the
    path carries none (an unsharded whole-crate run).

    Counting input FILES is not a shard set: n files tell you how many artifacts were passed,
    not WHICH `--shard k/n` slices they are. Without the identity, `--seed` cannot tell one
    complete shard from a 23-of-24 subset or from the same shard copied twice, and each of
    those sums to a ceiling below the whole-crate truth. The identity lives in the artifact /
    output DIRECTORY name (SHARD_SUFFIX above) — cargo-mutants does not put it in the JSON —
    so scan the path components leaf-to-root and take the first that carries it. Nothing
    downstream may INFER a shard set from the file count alone.
    """
    for part in reversed(os.path.abspath(outcomes_path).split(os.sep)):
        m = SHARD_SUFFIX.search(part)
        if m:
            return int(m.group(1)), int(m.group(2))
    return None


def run_label(outcomes_path, sid):
    """sq-has2g: short label naming ONE contributing outcomes.json in a
    diagnostic — "shard k/n" when the path carries an identity, else the run's output dir. The
    IMMEDIATE parent is always "mutants.out", so an unsharded run is named by the dir above
    it; with 24 engine shards, "mutants.out: only 4/400 completed" would not say which shard
    to re-run."""
    if sid is not None:
        return f"shard {sid[0]}/{sid[1]}"
    d = os.path.dirname(os.path.abspath(outcomes_path))
    if os.path.basename(d) == "mutants.out":
        d = os.path.dirname(d)
    return os.path.basename(d)


def summarise(doc):
    """Reduce a cargo-mutants outcomes.json to (surviving, caught, unviable, timeout,
    total_viable). Prefer the top-level rollup counts cargo-mutants writes
    (`missed`/`caught`/`timeout`/`unviable`); fall back to tallying per-mutant `summary`
    strings if an older format omits them. MissedMutant == surviving (a test GAP);
    CaughtMutant + Timeout == killed; Unviable is excluded from the denominator (it never
    compiled, so it tests nothing)."""
    if all(k in doc for k in ("missed", "caught", "timeout", "unviable")):
        surviving = doc["missed"]; caught = doc["caught"]
        timeout = doc["timeout"]; unviable = doc["unviable"]
    else:
        surviving = caught = unviable = timeout = 0
        for o in doc.get("outcomes", []):
            s = o.get("summary", "")
            if s == "MissedMutant":   surviving += 1
            elif s == "CaughtMutant": caught += 1
            elif s == "Timeout":      timeout += 1
            elif s == "Unviable":     unviable += 1
            # "Success" (the unmutated baseline) and anything else are ignored.
    total_viable = surviving + caught + timeout
    return surviving, caught, unviable, timeout, total_viable


def completeness(outcomes_path, completed):
    """sq-has2g: TRUNCATION DETECTION -> (planned, completed, unverifiable_reason).
    planned is None when completeness CANNOT be established, and the third element then says
    WHY (it is None whenever planned was determined).

    cargo-mutants writes mutants.out/outcomes.json INCREMENTALLY, and its rollup counts
    (`total_mutants`/`missed`/`caught`/…) describe only what has finished so far. So a run
    killed by the job timeout or a runner shutdown — the normal outcome for the heaviest
    crates, which is exactly why sparq-engine is still unseeded — leaves behind a perfectly
    well-formed artifact that is INDISTINGUISHABLE from a complete run of a smaller crate.
    Seeding from one would commit a ceiling derived from the handful of mutants that
    happened to finish: far too tight, and it would read as an excellent result.

    mutants.out/mutants.json is the FULL planned list for the run (already written up-front,
    and shard-aware — it holds only this `--shard k/n` slice), so comparing its length to
    the number of finished outcomes detects the truncation.

    sq-has2g review-2: when that list is missing, unreadable or the wrong shape
    the answer is NOT "assume complete". A truncated run whose sidecar simply never reached the
    artifact is indistinguishable from a whole one, so planned=None means NO EVIDENCE EITHER
    WAY, and the reason is returned so seed() can fail CLOSED on it (see seed()); --check, which
    can only ever under-report, keeps treating it as informational.
    """
    planned_path = os.path.join(os.path.dirname(os.path.abspath(outcomes_path)),
                                "mutants.json")
    if not os.path.exists(planned_path):
        return None, completed, "no mutants.json beside outcomes.json"
    try:
        planned = load(planned_path)
    except ValueError:
        return None, completed, "mutants.json is not valid JSON"
    except OSError as exc:
        return None, completed, f"mutants.json could not be read ({exc})"
    if not isinstance(planned, list):
        return None, completed, (f"mutants.json is a {type(planned).__name__}, not the expected "
                                 f"list of planned mutants")
    return len(planned), completed, None


def measured_rows(outcome_paths):
    """Build {crate: {surviving, caught, unviable, timeout, caught_pct, shards}} from
    outcomes files, skipping EXCLUDED crates.

    sq-has2g: SHARD AGGREGATION. `cargo mutants --shard k/n` slices a crate's
    mutant list into n disjoint parts, and the nightly runs each slice as its OWN matrix
    job writing its OWN outcomes.json (sparq-reason is already 3-sharded; sparq-engine is
    sharded here because it is the largest crate). Those per-shard files all report the
    SAME package, so the previous `rows[crate] = {...}` was LAST-WRITE-WINS: seeding a
    sharded crate from its n artifacts recorded only the final shard's counts and would
    have committed a ceiling roughly 1/n of the true surviving count — a bogus, far too
    tight floor that reads as a strong result. Since the shards partition the mutant set,
    the honest whole-crate measurement is the per-category SUM, so accumulate. `shards`
    records how many outcomes files contributed, which lets --check flag a PARTIAL
    measurement instead of silently under-reporting.

    `truncated` / `unverified` record, per contributing file, that it finished FEWER mutants
    than it planned / that its planned-mutant list could not be read at all — the two ways a
    run's outcomes may not be the whole story. seed() refuses on either (see seed()).

    `shard_ids` records each contributing file's (k, n) identity (None when unsharded) so
    seed() can REQUIRE a complete, disjoint set rather than trusting the file count; a file
    repeating an identity already counted for this crate is skipped here, because summing it
    would inflate the surviving count and could spuriously FAIL --check.
    """
    rows = {}
    seen_paths = set()
    for p in outcome_paths:
        # Guard against the same artifact being passed twice (e.g. a glob that expands
        # over both a copy and the original): summing would double-count it.
        real = os.path.realpath(p)
        if real in seen_paths:
            print(f"  skip {p} (duplicate path already counted)")
            continue
        seen_paths.add(real)
        doc = load(p)
        crate = crate_of(p, doc)
        if crate in EXCLUDED:
            print(f"  skip {crate} (excluded: see baseline _comment)")
            continue
        surviving, caught, unviable, timeout, total = summarise(doc)
        acc = rows.setdefault(crate, {
            "surviving": 0, "caught": 0, "timeout": 0, "unviable": 0, "shards": 0,
            "truncated": [], "unverified": [], "shard_ids": [], "repeated_shards": [],
        })
        sid = shard_of(p)
        if sid is not None and sid in acc["shard_ids"]:
            # Same `--shard k/n` slice supplied twice under DIFFERENT paths (two downloads of
            # one artifact, a copy alongside the original): the realpath guard above cannot see
            # it, and summing it would double-count that slice's mutants.
            print(f"  skip {p} (shard {sid[0]}/{sid[1]} of {crate} already counted)")
            acc["repeated_shards"].append(sid)
            continue
        acc["shard_ids"].append(sid)
        planned, done, unverifiable = completeness(p, surviving + caught + timeout + unviable)
        if unverifiable is not None:
            acc["unverified"].append((run_label(real, sid), unverifiable))
        elif done < planned:
            acc["truncated"].append((run_label(real, sid), done, planned))
        acc["surviving"] += surviving
        acc["caught"] += caught
        acc["timeout"] += timeout
        acc["unviable"] += unviable
        acc["shards"] += 1
    for crate, acc in rows.items():
        total = acc["surviving"] + acc["caught"] + acc["timeout"]
        killed = acc["caught"] + acc["timeout"]
        acc["caught_pct"] = round(100.0 * killed / total, 2) if total else 0.0
        if acc["shards"] > 1:
            print(f"  agg  {crate:<20} summed {acc['shards']} shard outcome file(s)")
        for name, done, planned in acc["truncated"]:
            print(f"  TRUNC {crate:<19} {name}: only {done}/{planned} mutants completed")
        for name, why in acc["unverified"]:
            print(f"  UNVER {crate:<19} {name}: completeness NOT established — {why}")
    return rows


def _comment_block():
    return [
        "MUTATION-TESTING RATCHET (sq-8kt3, epic sq-qcnn) — committed per-crate "
        "SURVIVING-mutant CEILING, reviewed in diffs exactly like the line-coverage ratchet "
        "(bench/coverage-floor.json). scripts/mutants-gate.py --check FAILS CI when a crate "
        "has MORE surviving (test-undetected) mutants than its ceiling. This measures test "
        "QUALITY: a mutant that survives is code a test ran over but never asserted on — a "
        "vacuous-test gap that line coverage cannot see.",
        f"ceiling = measured_surviving + {MARGIN} (a small margin so a toolchain bump that "
        "reshuffles the viable mutant set never trips the gate). The ceiling only ever FALLS "
        "(--seed will not raise it without --allow-higher). caught_pct is recorded for humans "
        "but the GATE is the surviving COUNT (stable: caught% has a moving denominator).",
        "Tiering: cargo-mutants rebuilds + reruns the suite ONCE PER MUTANT, so it is far too "
        "slow for per-commit. It runs in the NIGHTLY tier only — .github/workflows/ci.yml "
        "`mutants-nightly` (schedule + workflow_dispatch), and is ADVISORY (non-gating: the "
        "lane name contains \"advisory\" so the ci-summary aggregator excludes it) until this "
        "baseline is fully seeded across the workspace, then it ratchets to gating. It is NOT "
        "on the per-PR required ci-summary path — adding it there would blow the PR budget.",
        "Excluded crates (sparq-cli/-conformance/-gpu/-bench/-py and *-wasm): same rationale "
        "as the coverage ratchet's floor:0 crates — see scripts/mutants-gate.py EXCLUDED and "
        ".cargo/mutants.toml exclude_globs.",
        "HONESTY: a crate appears here ONLY once it has actually been measured by a real "
        "cargo-mutants run. Crates not yet listed are seeded on their first nightly run — the "
        "absence of a crate is NOT a claim that it has zero surviving mutants.",
        "Regenerate after tests improve: cargo mutants -p <crate> -o target/mutants/<crate> "
        "&& scripts/mutants-gate.py --seed target/mutants/<crate>/outcomes.json",
        "sq-has2g SHARDED CRATES (`shards: n` on the entry — sparq-reason, "
        "sparq-engine): the nightly splits these across n `cargo mutants --shard k/n` matrix "
        "jobs, one outcomes.json artifact each. Pass ALL n files to a SINGLE --seed "
        "invocation (scripts/mutants-gate.py --seed target/mutants/<crate>-shard-*/…/"
        "outcomes.json) — the shards partition the mutant set, so the whole-crate figure is "
        "their SUM and --seed aggregates them. Seeding from ONE shard would commit a ceiling "
        "~1/n of the truth, so --seed ENFORCES a complete set: it reads each artifact's "
        "`-shard-k-n` identity from its path and REFUSES a missing, repeated, or "
        "mixed-denominator shard (as it refuses a truncated run) unless "
        "--allow-partial-shards is passed. --check on a partial shard set is tolerated (it "
        "can only under-report, never spuriously fail) and prints a PARTIAL marker.",
        "sq-has2g COMPLETENESS FAILS CLOSED: --seed refuses any run that finished "
        "fewer mutants than its mutants.out/mutants.json planned (a job killed at its timeout "
        "leaves a well-formed but PARTIAL outcomes.json), AND refuses any run where that "
        "planned list is missing/unreadable/not a list — absent evidence is not evidence of a "
        "whole run. The escape hatches (--allow-truncated, --allow-unverified-completeness) "
        "are deliberate and are never appropriate for a ceiling that will later gate. --check "
        "is unaffected: an under-measured run can only under-report, never spuriously fail.",
    ]


def shard_denominator(row):
    """sq-has2g: the `--shard k/n` denominator n for a sub-sharded crate (which a
    complete set also has one artifact each of), else the number of contributing files. Read
    from the shard IDENTITY rather than the file count, so a deliberately partial
    (--allow-partial-shards) seed still records `shards: n` — recording `1` there would make
    the entry indistinguishable from a whole-crate run and would silence --check's PARTIAL
    marker on every later run of that crate."""
    ns = {n for _, n in (i for i in row["shard_ids"] if i is not None)}
    return max(ns) if ns else row["shards"]


def shard_set_errors(row):
    """sq-has2g: reasons this crate's outcomes files are NOT one whole-crate
    measurement -> a list of human-readable problems (empty == safe to seed from).

    A sharded crate's whole-crate surviving count is the SUM over shards, so the sum is only
    the truth when the inputs are EXACTLY the n disjoint slices of one `--shard k/n` split.
    Any of: a single slice, a 23-of-24 subset, the same slice twice under different paths, or
    slices from two different splits (k/24 mixed with k/8, which OVERLAP), yields a number
    below the whole-crate figure — and since a LOWER surviving count reads as BETTER test
    quality, it would commit a too-tight ceiling that looks like an excellent result. This is
    the same failure mode as a truncated run, so it fails CLOSED the same way.

    Both `--shard` index conventions are accepted (a complete 1..n or a complete 0..n-1 set),
    consistently within one crate: this gate must not silently depend on which one
    cargo-mutants uses, only on the set being COMPLETE.

    UNSHARDED mode is kept clearly distinct: no path carries an identity, and then exactly ONE
    file is expected (two whole-crate runs of the same crate must not be summed either).
    """
    ids = row["shard_ids"]
    problems = []
    if row["repeated_shards"]:
        dupes = ", ".join(f"{k}/{n}" for k, n in sorted(set(row["repeated_shards"])))
        problems.append(f"shard(s) {dupes} supplied more than once (a repeated slice cannot "
                        f"stand in for a missing one)")
    known = [i for i in ids if i is not None]
    if not known:
        if len(ids) > 1:
            problems.append(f"{len(ids)} outcomes files carry NO `-shard-k-n` identity — they "
                            f"look like {len(ids)} separate whole-crate runs, and summing "
                            f"those double-counts the crate")
        return problems
    if len(known) != len(ids):
        problems.append(f"{len(ids) - len(known)} of {len(ids)} outcomes files carry no "
                        f"`-shard-k-n` identity — a sharded and an unsharded run of the same "
                        f"crate overlap, so their sum is not a whole-crate figure")
        return problems
    denominators = sorted({n for _, n in known})
    if len(denominators) > 1:
        problems.append(f"conflicting shard denominators {denominators} — slices of different "
                        f"`--shard k/n` splits OVERLAP, so they cannot be summed")
        return problems
    n = denominators[0]
    ks = sorted({k for k, _ in known})
    if set(ks) not in ({i + 1 for i in range(n)}, set(range(n))):
        have = ", ".join(f"{k}/{n}" for k in ks)
        problems.append(f"{len(ks)} of {n} shards supplied ({have}) — an incomplete shard set "
                        f"sums to LESS than the whole-crate surviving count")
    return problems


def seed(outcome_paths, baseline_path, allow_higher, allow_truncated=False,
         allow_partial_shards=False, allow_unverified=False):
    measured = measured_rows(outcome_paths)
    # sq-has2g: REFUSE to seed from a truncated run. cargo-mutants writes
    # outcomes.json incrementally, so a job killed at its timeout yields a well-formed but
    # PARTIAL artifact; seeding from it commits a ceiling based only on the mutants that
    # happened to finish. That is the single most likely way this baseline acquires a
    # dishonest number, and it fails CLOSED here rather than being reviewable-in-diff (the
    # committed entry looks entirely plausible). --allow-truncated is the deliberate escape
    # hatch, and it is never appropriate for seeding a ceiling that will later gate.
    refused = False
    truncated = {c: r["truncated"] for c, r in measured.items() if r["truncated"]}
    if truncated and not allow_truncated:
        for crate, items in sorted(truncated.items()):
            for name, done, planned in items:
                print(f"::error::{crate}: {name} completed only {done}/{planned} mutants — "
                      f"the run was cut short (job timeout / runner shutdown), so its "
                      f"surviving count is NOT the whole-crate figure")
        print(f"\nrefusing to seed from {sum(len(v) for v in truncated.values())} truncated "
              f"run(s): re-run to completion, or pass --allow-truncated if you have "
              f"independently established the numbers are whole")
        refused = True
    # sq-has2g review-2: REFUSE a run whose completeness cannot be ESTABLISHED at
    # all. `planned=None` (no mutants.json, unreadable, or not the planned LIST) is the ABSENCE
    # of evidence, not evidence of a whole run — a job killed at its timeout whose sidecar never
    # made it into the artifact produces exactly this, so accepting it would leave the
    # dishonest-ceiling hole the truncation check above exists to close. Fail CLOSED, the same
    # way. --allow-unverified-completeness is the deliberate, loudly-named escape hatch for
    # legacy artifacts that predate the sidecar upload, and it is never appropriate for a
    # ceiling that will later gate. --check is deliberately NOT affected: an under-measured
    # --check can only under-report, never spuriously fail, so it stays backward-compatible and
    # merely prints the UNVER line from measured_rows().
    unverified = {c: r["unverified"] for c, r in measured.items() if r["unverified"]}
    if unverified and not allow_unverified:
        for crate, items in sorted(unverified.items()):
            for name, why in items:
                print(f"::error::{crate}: {name} has no usable planned-mutant list ({why}), so "
                      f"it cannot be distinguished from a run cut short by the job timeout")
        print(f"\nrefusing to seed from {sum(len(v) for v in unverified.values())} run(s) whose "
              f"completeness cannot be established: re-run so mutants.json is uploaded beside "
              f"outcomes.json, or pass --allow-unverified-completeness if you have "
              f"independently established the numbers are whole")
        refused = True
    # sq-has2g: REFUSE an INCOMPLETE / OVERLAPPING shard set, for the same reason
    # a truncated run is refused — both commit a ceiling derived from a fraction of the crate's
    # mutants, and both look entirely plausible in the reviewed diff. Truncation detection
    # cannot catch this: each shard of a 23-of-24 subset is individually COMPLETE against its
    # own mutants.json. --allow-partial-shards is the deliberate escape hatch and is never
    # appropriate for a ceiling that will later gate.
    incomplete = {c: p for c, p in ((c, shard_set_errors(r)) for c, r in measured.items()) if p}
    if incomplete and not allow_partial_shards:
        for crate, problems in sorted(incomplete.items()):
            for problem in problems:
                print(f"::error::{crate}: {problem}")
        print(f"\nrefusing to seed {len(incomplete)} crate(s) from an incomplete shard set: "
              f"pass ALL n `--shard k/n` artifacts to one --seed, or --allow-partial-shards "
              f"if you have independently established the numbers are whole")
        refused = True
    if refused:
        return 1
    existing = load(baseline_path)["crates"] if os.path.exists(baseline_path) else {}
    out = dict(existing)  # carry forward crates not measured this run
    lowered, raised, kept, new = [], [], [], []
    for crate, row in sorted(measured.items()):
        proposed = row["surviving"] + MARGIN
        prev = existing.get(crate, {}).get("ceiling")
        entry = {
            "ceiling": None,
            "measured_surviving": row["surviving"],
            "caught": row["caught"],
            "timeout": row["timeout"],
            "unviable": row["unviable"],
            "caught_pct": row["caught_pct"],
        }
        # sq-has2g: record the shard count for a SUB-SHARDED crate so the
        # committed entry states, in the reviewed diff, that it is the SUM of n disjoint
        # `--shard k/n` runs rather than one whole-crate run — and so --check can warn when
        # a later run supplies fewer shards than the entry was seeded from. Omitted for the
        # single-run crates so their existing entries stay byte-identical.
        shards = shard_denominator(row)
        if shards > 1:
            entry["shards"] = shards
        if prev is None:
            entry["ceiling"] = proposed
            new.append(f"{crate}={proposed} ({row['surviving']} survived)")
        elif proposed < prev:
            entry["ceiling"] = proposed
            lowered.append(f"{crate} {prev}->{proposed}")
        elif proposed > prev and allow_higher:
            entry["ceiling"] = proposed
            raised.append(f"{crate} {prev}->{proposed} (--allow-higher)")
        else:
            entry["ceiling"] = prev  # ratchet: never auto-raise
            kept.append(crate)
        out[crate] = entry
    doc = {"_comment": _comment_block(), "margin": MARGIN, "crates": out}
    with open(baseline_path, "w") as f:
        json.dump(doc, f, indent=2, sort_keys=True); f.write("\n")
    print(f"seeded {baseline_path}: {len(out)} crate(s) total")
    if new:     print("  NEW:     " + ", ".join(new))
    if lowered: print("  LOWERED: " + ", ".join(lowered))
    if raised:  print("  RAISED (--allow-higher): " + ", ".join(raised))
    if kept:    print(f"  kept:    {len(kept)} unchanged")
    return 0


def check(outcome_paths, baseline_path, require_all):
    ceilings = load(baseline_path)["crates"]
    measured = measured_rows(outcome_paths)
    fails, missing, oks, partial = [], [], [], []
    for crate, centry in sorted(ceilings.items()):
        ceiling = centry["ceiling"] if isinstance(centry, dict) else centry
        row = measured.get(crate)
        if row is None:
            missing.append(crate); continue
        # sq-has2g: a SUB-SHARDED crate is checked per shard job, so this run
        # may hold only a SLICE of the crate's mutants. Its surviving count is then a lower
        # bound on the whole-crate figure and comparing it to the whole-crate ceiling is
        # unsound in the SAFE direction — it can say "ok" but never spuriously FAIL. Report
        # it as PARTIAL so a green line is not read as whole-crate evidence.
        seeded_shards = centry.get("shards", 1) if isinstance(centry, dict) else 1
        if row["shards"] < seeded_shards:
            partial.append((crate, row["shards"], seeded_shards))
        surviving = row["surviving"]
        if surviving > ceiling:
            fails.append((crate, surviving, ceiling, row["caught_pct"]))
        else:
            oks.append((crate, surviving, ceiling, row["caught_pct"]))
    # Crates measured but not yet in the baseline: report (informational — they get
    # seeded next --seed; we never fail on an unknown crate).
    unseeded = sorted(set(measured) - set(ceilings))
    for crate, surv, ceil, pct in oks:
        print(f"  ok   {crate:<20} surviving {surv:>3} <= ceiling {ceil:<3} (caught {pct}%)")
    for crate in missing:
        print(f"  --   {crate:<20} MISSING (not measured in this run)")
    for crate in unseeded:
        r = measured[crate]
        of_n = f" [{r['shards']} shard(s) only]" if r["shards"] > 1 else ""
        print(f"  new  {crate:<20} surviving {r['surviving']:>3} (UNSEEDED — run --seed){of_n}")
    for crate, got, want in partial:
        print(f"  ??   {crate:<20} PARTIAL: {got}/{want} shard(s) measured — the surviving "
              f"count above is a LOWER BOUND, not whole-crate evidence")
    for crate, surv, ceil, pct in fails:
        print(f"  FAIL {crate:<20} surviving {surv:>3} >  ceiling {ceil:<3} (caught {pct}%)")
    bad = bool(fails) or (require_all and bool(missing))
    if fails:
        print(f"::error::mutation ratchet regressed for {len(fails)} crate(s): more "
              f"surviving mutants than the committed ceiling — add/strengthen tests")
    if require_all and missing:
        print(f"::error::{len(missing)} baseline crate(s) absent from this run "
              f"(--require-all): {', '.join(missing)}")
    print(f"\nmutation gate: {len(oks)} ok / {len(fails)} fail / {len(missing)} missing "
          f"/ {len(unseeded)} unseeded")
    return 1 if bad else 0


def main():
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    ap = argparse.ArgumentParser(description="per-crate mutation-testing ratchet gate")
    ap.add_argument("outcomes", nargs="+",
                    help="one or more cargo-mutants mutants.out/outcomes.json files "
                         "(target/mutants/<crate>/outcomes.json)")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--seed", action="store_true", help="(re)generate the baseline file")
    g.add_argument("--check", action="store_true", help="enforce the baseline file")
    ap.add_argument("--baseline", default=DEFAULT_BASELINE)
    ap.add_argument("--allow-higher", action="store_true",
                    help="permit --seed to RAISE a ceiling (deliberate regression)")
    ap.add_argument("--require-all", action="store_true",
                    help="--check fails if a baseline crate is absent from this run")
    ap.add_argument("--allow-truncated", action="store_true",
                    help="permit --seed from a run that did not finish all its planned "
                         "mutants (detected via mutants.json); normally refused")
    ap.add_argument("--allow-partial-shards", action="store_true",
                    help="permit --seed from an INCOMPLETE/overlapping `--shard k/n` set "
                         "(missing, repeated or mixed-denominator shards); normally refused "
                         "because the sum is then below the whole-crate surviving count")
    ap.add_argument("--allow-unverified-completeness", action="store_true",
                    help="permit --seed from a run whose completeness CANNOT be established "
                         "— no mutants.json beside outcomes.json, or it is unreadable/not the "
                         "planned list (e.g. a legacy artifact predating the sidecar upload); "
                         "normally refused, because such a run is indistinguishable from one "
                         "cut short by the job timeout")
    a = ap.parse_args()
    baseline = os.path.abspath(a.baseline)
    if a.seed:
        sys.exit(seed(a.outcomes, baseline, a.allow_higher, a.allow_truncated,
                      a.allow_partial_shards, a.allow_unverified_completeness))
    sys.exit(check(a.outcomes, baseline, a.require_all))


def self_test():
    """sq-8kt3: unit-test the PURE summary + ratchet logic on synthetic
    outcomes docs — no files, no cargo. Mirrors coverage-gate.py --self-test."""
    def doc(missed=0, caught=0, timeout=0, unviable=0):
        out = []
        out += [{"summary": "MissedMutant"}] * missed
        out += [{"summary": "CaughtMutant"}] * caught
        out += [{"summary": "Timeout"}] * timeout
        out += [{"summary": "Unviable"}] * unviable
        out += [{"summary": "Success"}]  # the unmutated baseline build — must be ignored
        return {"outcomes": out}

    # --- summarise: counts + caught% (timeout counts as caught; unviable excluded) ----
    s, c, u, t, tot = summarise(doc(missed=3, caught=7, timeout=1, unviable=4))
    assert (s, c, t, u, tot) == (3, 7, 1, 4, 11), (s, c, t, u, tot)
    # caught% = (caught+timeout)/total_viable = 8/11
    killed = c + t
    assert round(100.0 * killed / tot, 2) == round(800.0 / 11, 2)
    # empty viable set -> 0% not a div0
    s0, c0, u0, t0, tot0 = summarise(doc(unviable=5))
    assert tot0 == 0 and (s0, c0) == (0, 0)

    # --- the ratchet: ceiling = surviving + MARGIN, only ever FALLS -------------------
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        # write two crate outcomes under target/mutants/<crate>/outcomes.json shape
        # Each variant gets its OWN parent dir so distinct measurements of the same crate
        # do not overwrite one shared outcomes.json (crate_of reads the leaf dir name, so
        # the crate identity is preserved regardless of the unique parent).
        _n = [0]
        def write(crate, **kw):
            _n[0] += 1
            d = os.path.join(td, f"run{_n[0]}", crate); os.makedirs(d, exist_ok=True)
            p = os.path.join(d, "outcomes.json")
            with open(p, "w") as f: json.dump(doc(**kw), f)
            # sq-has2g review-2: --seed FAILS CLOSED without a readable
            # planned-mutant list, so every fixture expected to SEED ships one matching its
            # outcomes exactly (planned == completed => a COMPLETE run). The truncated and
            # unverifiable fixtures below write/omit their own deliberately.
            with open(os.path.join(d, "mutants.json"), "w") as f:
                json.dump([{"package": crate}] * sum(kw.values()), f)
            return p
        base = os.path.join(td, "baseline.json")
        pa = write("sparq-canon", missed=5, caught=30)
        # seed: new crate -> ceiling = 5 + MARGIN
        assert seed([pa], base, allow_higher=False) == 0
        b = load(base)["crates"]["sparq-canon"]
        assert b["ceiling"] == 5 + MARGIN, b
        assert b["measured_surviving"] == 5
        # check at the same level passes
        assert check([pa], base, require_all=False) == 0
        # MORE survivors than ceiling -> FAIL
        pa_worse = write("sparq-canon", missed=5 + MARGIN + 1, caught=30)
        assert check([pa_worse], base, require_all=False) == 1
        # FEWER survivors -> re-seed LOWERS the ceiling
        pa_better = write("sparq-canon", missed=1, caught=34)
        assert seed([pa_better], base, allow_higher=False) == 0
        assert load(base)["crates"]["sparq-canon"]["ceiling"] == 1 + MARGIN
        # a worse measurement does NOT auto-raise the (now lower) ceiling
        assert seed([pa_worse], base, allow_higher=False) == 0
        assert load(base)["crates"]["sparq-canon"]["ceiling"] == 1 + MARGIN
        # ...unless --allow-higher. pa_worse has (5 + MARGIN + 1) survivors, so the
        # raised ceiling is that + MARGIN.
        worse_surv = 5 + MARGIN + 1
        assert seed([pa_worse], base, allow_higher=True) == 0
        assert load(base)["crates"]["sparq-canon"]["ceiling"] == worse_surv + MARGIN
        # excluded crate is never seeded
        pcli = write("sparq-cli", missed=99, caught=0)
        n_before = len(load(base)["crates"])
        assert seed([pcli], base, allow_higher=False) == 0
        assert len(load(base)["crates"]) == n_before, "excluded crate must not seed"
        # --require-all: a baseline crate absent from the run fails
        pother = write("sparq-parse", missed=0, caught=10)
        assert seed([pother], base, allow_higher=False) == 0  # adds sparq-parse
        assert check([pa_better], base, require_all=True) == 1  # sparq-parse missing
        assert check([pa_better], base, require_all=False) == 0  # tolerated without flag

    # --- sq-has2g: SHARD AGGREGATION -------------------------------------
    # `cargo mutants --shard k/n` partitions a crate's mutant list across n jobs, each
    # writing its own outcomes.json for the SAME package. Those must SUM, not overwrite:
    # the old last-write-wins behaviour seeded a ceiling ~1/n of the true surviving count.
    with tempfile.TemporaryDirectory() as td:
        _m = [0]
        def shard(crate, k=None, n=None, missed=0, caught=0, timeout=0, unviable=0,
                  sidecar=True):
            """Write one run's outcomes.json in cargo-mutants' real shape: a Baseline entry
            plus one Mutant-scenario entry per mutant, each stamping the package so crate_of()
            resolves every shard of a crate to the same key.

            sidecar=True also writes the mutants.json planned list matching those outcomes
            exactly (a COMPLETE run) — --seed refuses a run without one. sidecar=False models
            an artifact that shipped outcomes.json alone.

            The directory reproduces the nightly lane's REAL naming — `<crate>-shard-<k>-<n>`
            for a `--shard k/n` slice (that suffix is the identity shard_of() reads), or an
            unsuffixed dir for a whole-crate run. Every call also gets a unique parent, so two
            calls with the SAME k/n are distinct paths (the realpath guard cannot dedupe them)
            — which is exactly how a duplicated artifact reaches the gate."""
            _m[0] += 1
            leaf = f"{crate}-shard-{k}-{n}" if k is not None else f"{crate}-run{_m[0]}"
            d = os.path.join(td, f"a{_m[0]}", leaf, "mutants.out")
            os.makedirs(d, exist_ok=True)
            p = os.path.join(d, "outcomes.json")
            outcomes = [{"scenario": "Baseline", "summary": "Success"}]
            for summary, count in (("MissedMutant", missed), ("CaughtMutant", caught),
                                   ("Timeout", timeout), ("Unviable", unviable)):
                outcomes += [{"scenario": {"Mutant": {"package": crate}},
                              "summary": summary}] * count
            with open(p, "w") as f: json.dump({"outcomes": outcomes}, f)
            if sidecar:
                with open(os.path.join(d, "mutants.json"), "w") as f:
                    json.dump([{"package": crate}] * (missed + caught + timeout + unviable), f)
            return p

        # the shard identity is read from the path, NOT inferred from the file count
        assert shard_of(shard("sparq-engine", 2, 24)) == (2, 24)
        assert shard_of(shard("sparq-engine")) is None, "unsharded run carries no identity"

        # three disjoint shards of one crate: 4+3+2 survivors == 9 whole-crate survivors
        s1 = shard("sparq-engine", 1, 3, missed=4, caught=10)
        s2 = shard("sparq-engine", 2, 3, missed=3, caught=20)
        s3 = shard("sparq-engine", 3, 3, missed=2, caught=30)
        rows = measured_rows([s1, s2, s3])
        assert set(rows) == {"sparq-engine"}, rows
        assert rows["sparq-engine"]["surviving"] == 9, rows
        assert rows["sparq-engine"]["caught"] == 60, rows
        assert rows["sparq-engine"]["shards"] == 3, rows

        # the seeded ceiling reflects the SUMMED survivors (9 + MARGIN), not the last
        # shard's 2 — the exact bug this guards.
        base2 = os.path.join(td, "baseline2.json")
        assert seed([s1, s2, s3], base2, allow_higher=False) == 0
        e = load(base2)["crates"]["sparq-engine"]
        assert e["ceiling"] == 9 + MARGIN, e
        assert e["measured_surviving"] == 9 and e["shards"] == 3, e

        # a duplicate path must NOT double-count
        assert measured_rows([s1, s2, s3, s2])["sparq-engine"]["surviving"] == 9

        # --check with ALL shards is exact, passes at the seeded level, and is NOT partial
        import contextlib, io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc_full = check([s1, s2, s3], base2, require_all=False)
        assert rc_full == 0 and "PARTIAL" not in buf.getvalue(), buf.getvalue()
        # ...and one shard alone under-reports (9 -> 4): tolerated (it can only under-report,
        # so it never spuriously FAILS) but it MUST be flagged PARTIAL rather than printed as
        # a clean "ok" that reads as whole-crate evidence.
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc_part = check([s1], base2, require_all=False)
        out = buf.getvalue()
        assert rc_part == 0, out
        assert "PARTIAL: 1/3 shard(s) measured" in out, out
        assert "LOWER BOUND" in out, out

        # a single-run crate records NO `shards` key, keeping existing entries unchanged
        solo = shard("sparq-hdt", missed=1, caught=5)
        assert seed([solo], base2, allow_higher=False) == 0
        assert "shards" not in load(base2)["crates"]["sparq-hdt"]

        # --- INCOMPLETE / OVERLAPPING SHARD SET: --seed must REFUSE ----------------------
        # Counting files is not a shard set. Each of these sums to LESS than the whole-crate
        # surviving count, and a lower count reads as BETTER test quality — so every one of
        # them would commit a too-tight ceiling that looks like an excellent result.
        def seed_refused(paths, why, **kw):
            b = os.path.join(td, f"refuse-{seed_refused.n}.json"); seed_refused.n += 1
            assert seed(paths, b, allow_higher=False, **kw) == 1, why
            assert not os.path.exists(b), f"a refused seed must not write a baseline ({why})"
            return b
        seed_refused.n = 0

        seed_refused([s1], "one shard of 3 is not the whole crate")
        seed_refused([s1, s2], "2 of 3 shards is not the whole crate")
        # the same slice twice under DIFFERENT paths (a re-download / a copy): the realpath
        # guard cannot see it, and it must not stand in for the missing 2/3 + 3/3.
        s1_copy = shard("sparq-engine", 1, 3, missed=4, caught=10)
        assert s1_copy != s1 and os.path.realpath(s1_copy) != os.path.realpath(s1)
        seed_refused([s1, s1_copy, s3], "a repeated shard cannot fill in for a missing one")
        # ...and it is not double-counted on the way to that refusal
        assert measured_rows([s1, s1_copy, s3])["sparq-engine"]["surviving"] == 6
        # slices of two DIFFERENT splits overlap, so their sum is meaningless
        seed_refused([s1, s2, shard("sparq-engine", 3, 4, missed=1, caught=1)],
                     "mixed shard denominators must be refused")
        # a sharded and an unsharded run of the same crate also overlap
        seed_refused([s1, s2, s3, shard("sparq-engine", missed=9, caught=60)],
                     "mixing a sharded set with a whole-crate run must be refused")
        # two whole-crate runs of one crate are not two shards either
        seed_refused([shard("sparq-hdt", missed=1, caught=5),
                      shard("sparq-hdt", missed=1, caught=5)],
                     "two unsharded runs of one crate must not be summed")

        # a COMPLETE set seeds — under either `--shard` index convention (0..n-1 or 1..n), so
        # the gate depends on the set being complete, not on cargo-mutants' numbering.
        base_zero = os.path.join(td, "baseline-zero.json")
        z = [shard("sparq-engine", k, 3, missed=1, caught=5) for k in (0, 1, 2)]
        assert seed(z, base_zero, allow_higher=False) == 0, "0-based complete set must seed"
        assert load(base_zero)["crates"]["sparq-engine"]["measured_surviving"] == 3

        # the escape hatch is deliberate, and STILL records `shards: n` from the identity
        # (not the file count) so --check keeps flagging later runs of that crate as PARTIAL.
        base_partial = os.path.join(td, "baseline-partial.json")
        assert seed([s1], base_partial, allow_higher=False, allow_partial_shards=True) == 0
        assert load(base_partial)["crates"]["sparq-engine"]["shards"] == 3

        # --- TRUNCATION: a run cut short by the job timeout must NOT seed a ceiling -----
        # cargo-mutants writes outcomes.json incrementally, so a killed job leaves a
        # well-formed PARTIAL artifact. mutants.json (the full planned list) reveals it.
        def plan(outcomes_path, n):
            with open(os.path.join(os.path.dirname(outcomes_path), "mutants.json"), "w") as f:
                json.dump([{"package": "sparq-engine"}] * n, f)

        t1 = shard("sparq-engine", missed=4, caught=10)   # 14 completed
        plan(t1, 14)                                       # ...of 14 planned -> COMPLETE
        assert not measured_rows([t1])["sparq-engine"]["truncated"]
        base3 = os.path.join(td, "baseline3.json")
        assert seed([t1], base3, allow_higher=False) == 0, "complete run must seed"

        t2 = shard("sparq-engine", missed=1, caught=3)     # only 4 completed
        plan(t2, 400)                                      # ...of 400 planned -> TRUNCATED
        assert measured_rows([t2])["sparq-engine"]["truncated"], "truncation must be seen"
        base4 = os.path.join(td, "baseline4.json")
        assert seed([t2], base4, allow_higher=False) == 1, "truncated run must NOT seed"
        assert not os.path.exists(base4), "a refused seed must not write a baseline"
        # the escape hatch still works, deliberately
        assert seed([t2], base4, allow_higher=False, allow_truncated=True) == 0
        assert load(base4)["crates"]["sparq-engine"]["ceiling"] == 1 + MARGIN

        # truncation in ANY contributing shard blocks the whole crate's seed — otherwise a
        # 7-of-8-shards-complete engine sweep would quietly commit a short count. The two
        # refusals are INDEPENDENT: each shard of an incomplete set can be individually
        # complete against its own (shard-scoped) mutants.json, so only the shard-set check
        # catches a missing shard, and only the truncation check catches a cut-short one.
        ok1 = shard("sparq-engine", 1, 2, missed=2, caught=8); plan(ok1, 10)
        ok2 = shard("sparq-engine", 2, 2, missed=1, caught=4); plan(ok2, 5)
        tr2 = shard("sparq-engine", 2, 2, missed=1, caught=3); plan(tr2, 400)
        seed_refused([ok1, tr2], "a truncated shard blocks the whole crate's seed")
        # the diagnostic must NAME the offending shard (every path's parent is "mutants.out")
        assert measured_rows([tr2])["sparq-engine"]["truncated"][0][0] == "shard 2/2"
        seed_refused([ok1], "shard 1/2 alone is not the whole-crate figure, however complete")
        base5 = os.path.join(td, "baseline5.json")
        assert seed([ok1, ok2], base5, allow_higher=False) == 0, "complete+untruncated seeds"
        assert load(base5)["crates"]["sparq-engine"]["measured_surviving"] == 3

        # --- UNVERIFIABLE COMPLETENESS: no usable mutants.json -> --seed FAILS CLOSED ----
        # sq-has2g review-2: `planned=None` is the ABSENCE of evidence, not
        # evidence of a whole run — a job killed at its timeout whose sidecar never reached the
        # artifact produces exactly this shape. Seeding it would leave the dishonest-ceiling
        # hole the truncation check exists to close, so all three shapes REFUSE: the sidecar
        # missing, unreadable, and present-but-not-the-planned-list.
        def unverifiable(body=None):
            """A run whose mutants.json is absent (body=None) or written verbatim (a string)."""
            p = shard("sparq-engine", missed=1, caught=1, sidecar=False)
            if body is not None:
                with open(os.path.join(os.path.dirname(p), "mutants.json"), "w") as f:
                    f.write(body)
            planned, done, why = completeness(p, 2)
            assert (planned, done) == (None, 2) and why, (planned, done, why)
            assert measured_rows([p])["sparq-engine"]["unverified"], "must be flagged"
            return p

        u_missing = unverifiable()
        u_corrupt = unverifiable("{not json")
        u_shape = unverifiable(json.dumps({"total_mutants": 400}))  # a dict, not the LIST
        seed_refused([u_missing], "a run with no planned-mutant list must not seed")
        seed_refused([u_corrupt], "an unreadable planned-mutant list must not seed")
        seed_refused([u_shape], "a wrong-shape planned-mutant list must not seed")
        # ...and the refusal is the COMPLETENESS one, not some other check firing by accident
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc_u = seed([u_missing], os.path.join(td, "never.json"), allow_higher=False)
        assert rc_u == 1 and "completeness cannot be established" in buf.getvalue(), buf.getvalue()

        # the escape hatch is deliberate and explicitly named, for legacy artifacts only
        base6 = os.path.join(td, "baseline6.json")
        assert seed([u_missing], base6, allow_higher=False, allow_unverified=True) == 0
        assert load(base6)["crates"]["sparq-engine"]["ceiling"] == 1 + MARGIN
        # --check stays backward-compatible: it can only under-report, never spuriously fail
        assert check([u_missing], base6, require_all=False) == 0

        # crate_of() strips the CI lane's -shard-k-n dir suffix when the doc carries no
        # package (all-unviable edge case), so shards still collapse to one key.
        d3 = os.path.join(td, "target", "mutants", "sparq-engine-shard-2-6", "mutants.out")
        os.makedirs(d3)
        p3 = os.path.join(d3, "outcomes.json")
        with open(p3, "w") as f: json.dump({"outcomes": []}, f)
        assert crate_of(p3, {"outcomes": []}) == "sparq-engine", crate_of(p3, {})

    print("mutants-gate self-test: ALL ASSERTIONS PASSED")
    return 0


if __name__ == "__main__":
    main()
