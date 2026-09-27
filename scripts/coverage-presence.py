#!/usr/bin/env python3
# [OPUS-4.8] Per-crate test-PRESENCE gate (sq-hbg7).
#
# A line-% ratchet can MISS a whole crate going untested: delete every test in a crate
# and its line% just drops a little (or, for a subprocess-driven crate like sparq-cli,
# not at all). This gate guards the COUNT of test functions per crate and whether the
# crate keeps its integration-tests dir — so losing a crate's tests fails CI even when
# the % gate wouldn't notice. Mirrors the conformance/coverage ratchet idiom: a checked
# in floor (bench/coverage-presence.json) that may only RISE.
#
# It counts `#[test]` / `#[tokio::test]` / `#[rstest]` attribute occurrences across each
# crate's *.rs (src + tests), purely from source — NO compile, so it is fast and runs
# every commit. The count is a FLOOR (>=), not an exact match, so adding tests is always
# fine; removing them below the recorded floor fails.
#
#   --seed                 (re)generate bench/coverage-presence.json from the tree
#   --check                FAIL if any crate dropped below its recorded test count, lost
#                          its integration-tests dir (had_integration_dir: true -> now
#                          absent), or is on disk with NO floor entry at all.
#   --allow-lower          permit --seed to LOWER a count (deliberate test removal).
#
# [OPUS-5] #5140: the UNTRACKED-crate arm of --check exists because the floor file had
# silently drifted 26 crates behind crates/ — --check only ever iterated the RECORDED
# floors, so a crate that was never seeded was not "passing" the gate, it was outside it,
# free to lose every test unnoticed. A crate joins the gate by being on disk, not by
# being remembered.
import argparse, json, os, re, sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
CRATES_DIR = os.path.join(ROOT, "crates")
DEFAULT = os.path.join(ROOT, "bench", "coverage-presence.json")

# Harness/binding crates that legitimately carry few/no #[test]s — recorded but their
# count floor is 0 (presence of the dir is still tracked where applicable).
NO_TEST_CRATES = {
    "sparq-bench": "perf-harness crate (benchmarks, not #[test]s)",
    "sparq-py": "pyo3 bindings; tested via pytest, not Rust #[test] (see crates/sparq-py/tests)",
}

TEST_ATTR = re.compile(r'#\[\s*(?:tokio::|async_std::)?test\s*\]|#\[\s*rstest\s*\]')

def count_tests(crate_dir):
    n = 0
    for base, _dirs, files in os.walk(crate_dir):
        # skip vendored/target artefacts if any slipped in
        if os.sep + "target" + os.sep in base + os.sep:
            continue
        for fn in files:
            if not fn.endswith(".rs"):
                continue
            try:
                with open(os.path.join(base, fn), encoding="utf-8", errors="replace") as f:
                    n += len(TEST_ATTR.findall(f.read()))
            except OSError:
                pass
    return n

def scan():
    out = {}
    for crate in sorted(os.listdir(CRATES_DIR)):
        cdir = os.path.join(CRATES_DIR, crate)
        if not os.path.isdir(cdir) or not os.path.exists(os.path.join(cdir, "Cargo.toml")):
            continue
        out[crate] = {
            "tests": count_tests(cdir),
            "integration_dir": os.path.isdir(os.path.join(cdir, "tests")),
        }
    return out

def seed(path, allow_lower):
    cur = scan()
    existing = json.load(open(path))["crates"] if os.path.exists(path) else {}
    res, raised, kept, lowered, new = {}, [], [], [], []
    for crate, info in cur.items():
        prev = existing.get(crate)
        floor = info["tests"]
        if crate in NO_TEST_CRATES:
            floor = 0
        if prev is not None:
            pf = prev.get("min_tests", 0)
            if floor > pf:
                raised.append(f"{crate} {pf}->{floor}")
            elif floor < pf and not allow_lower:
                floor = pf; kept.append(crate)
            elif floor < pf:
                lowered.append(f"{crate} {pf}->{floor}")
            else:
                kept.append(crate)
        else:
            new.append(f"{crate}={floor}")
        entry = {"min_tests": floor, "had_integration_dir": info["integration_dir"]}
        # [OPUS-5] #5140: carry a hand-written per-crate `note` across re-seeds. A note
        # explaining WHY a floor is what it is (e.g. a reserved seam crate legitimately
        # sitting at 0) is only useful if `--seed` cannot silently erase it.
        if prev is not None and prev.get("note"):
            entry["note"] = prev["note"]
        if crate in NO_TEST_CRATES:
            entry["note"] = NO_TEST_CRATES[crate]
        res[crate] = entry
    doc = {
        "_comment": [
            "[OPUS-4.8] TEST-PRESENCE gate (sq-hbg7). Per-crate FLOOR on the number of "
            "#[test]/#[tokio::test]/#[rstest] functions + whether the crate keeps its "
            "tests/ integration dir. Catches a whole crate losing its tests — which the "
            "line-% ratchet (bench/coverage-floor.json) can miss (esp. the subprocess-"
            "driven sparq-cli). Counts are FLOORS (>=); adding tests is always fine.",
            "Counted from source (no compile) by scripts/coverage-presence.py, run every "
            "commit by the .github/workflows/ci.yml coverage job. The floor only RISES "
            "(--seed will not lower without --allow-lower). Regenerate after adding tests: "
            "scripts/coverage-presence.py --seed.",
            "EVERY crates/<x> with a Cargo.toml must appear here: --check FAILS on a crate "
            "that is on disk with no entry (#5140 — the file had drifted 26 crates behind, "
            "and an unlisted crate is UN-GATED, free to lose every test unnoticed). Adding "
            "a crate therefore means re-running --seed. A per-crate `note` is preserved "
            "across re-seeds, so record WHY a floor is unusual (e.g. a 0) as a note.",
        ],
        "crates": res,
    }
    json.dump(doc, open(path, "w"), indent=2, sort_keys=True); open(path, "a").write("\n")
    print(f"seeded {path}: {len(res)} crates")
    if new:     print("  NEW:    " + ", ".join(new))
    if raised:  print("  RAISED: " + ", ".join(raised))
    if lowered: print("  LOWERED (--allow-lower): " + ", ".join(lowered))
    if kept:    print(f"  kept:   {len(kept)} unchanged")
    return 0

def check(path):
    floors = json.load(open(path))["crates"]
    cur = scan()
    fails, oks = [], []
    for crate, fentry in sorted(floors.items()):
        info = cur.get(crate)
        if info is None:
            fails.append(f"{crate}: crate DISAPPEARED from crates/ (was tracked)")
            continue
        mn = fentry.get("min_tests", 0)
        if info["tests"] < mn:
            fails.append(f"{crate}: {info['tests']} tests < floor {mn} "
                         f"(tests removed?)")
        elif fentry.get("had_integration_dir") and not info["integration_dir"]:
            fails.append(f"{crate}: lost its tests/ integration dir")
        else:
            oks.append(f"{crate}: {info['tests']} tests (floor {mn})"
                       + (" +integ" if info["integration_dir"] else ""))
    # [OPUS-5] #5140: a crate on disk with no floor entry is UNGATED, not passing.
    for crate in sorted(cur):
        if crate not in floors:
            fails.append(f"{crate}: on disk ({cur[crate]['tests']} tests) but has NO "
                         f"presence floor — un-gated; run: "
                         f"python3 scripts/coverage-presence.py --seed")
    for o in oks:   print(f"  ok   {o}")
    for fl in fails: print(f"  FAIL {fl}")
    if fails:
        print(f"::error::test-presence gate: {len(fails)} crate(s) regressed")
    print(f"\npresence gate: {len(oks)} ok / {len(fails)} fail")
    return 1 if fails else 0

def main():
    ap = argparse.ArgumentParser(description="per-crate test-presence ratchet gate")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--seed", action="store_true")
    g.add_argument("--check", action="store_true")
    ap.add_argument("--file", default=DEFAULT)
    ap.add_argument("--allow-lower", action="store_true")
    a = ap.parse_args()
    p = os.path.abspath(a.file)
    sys.exit(seed(p, a.allow_lower) if a.seed else check(p))

if __name__ == "__main__":
    main()
