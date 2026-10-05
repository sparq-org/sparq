#!/usr/bin/env python3
# sq-toze.6 (cert gap GX-5) — UNSAFE-COUNT RATCHET gate.
#
# Promotes the previously-INFORMATIONAL cargo-geiger report (ci.yml `geiger`, bead
# sq-emay — whose own comment said "Promote to a gate later only with a checked-in
# unsafe-count ratchet") into a TRUE merge-blocking gate, mirroring the conformance /
# coverage / perf ratchet idiom already in this repo (a committed snapshot, reviewed in
# diffs, that a PR cannot silently raise above).
#
# WHAT IT GATES
# -------------
# Every `unsafe` block / fn / impl / trait / extern in `crates/` is a row in
# compliance/memsafety/unsafe-register.md (the justification register, GX-5). This
# script counts those sites PER CRATE and FAILS (exit 1) if any crate's count RISES
# above its committed snapshot in bench/unsafe-snapshot.json. A PR that adds an
# `unsafe` site must therefore EITHER reduce one elsewhere OR re-seed the snapshot —
# and re-seeding shows up in the diff next to the register row the author must add.
# This is the mechanical backstop that keeps the register at 100% coverage.
#
# WHY A GREP-STYLE COUNTER, NOT `cargo geiger`
# --------------------------------------------
# cargo-geiger cannot run against this workspace's VIRTUAL root manifest (it errors
# "is a virtual manifest, but this command requires running against an actual
# package") and pulls the whole dep tree + a full build — too heavy and too flaky for
# a per-PR GATING lane (the informational geiger job already carries `|| true` for
# exactly this reason). A deterministic source scan over the FIRST-PARTY crates is
# what we actually want to ratchet: it is hermetic, fast, build-free, and counts only
# OUR unsafe (third-party crate unsafe — e.g. `memmap2`, `libc`, the `hdt` dep — is
# out of scope for the register and is governed by cargo-deny/the supply-chain lane).
#
# COUNTING RULE (must match the register + scripts behaviour exactly)
# -------------------------------------------------------------------
# A site is the `unsafe` KEYWORD used as: `unsafe {`, `unsafe fn`, `unsafe impl`,
# `unsafe trait`, or `unsafe extern`. We strip `//` line comments, `/* */` block
# comments, and `"..."` string literals before matching, so the word `unsafe` inside
# a comment (e.g. a `// SAFETY:` note that mentions "unsafe") or a string is NOT
# counted. `unsafe_code` (the lint, as in `#![forbid(unsafe_code)]`) is NOT the bare
# keyword and is not matched.
#
# MODES
# -----
#   --check [snapshot]   FAIL if any crate's live count exceeds its snapshot count, OR
#                        if a crate with unsafe is missing from the snapshot. A crate
#                        whose live count is BELOW snapshot is fine (ratchet only ever
#                        tightens) but is reported so the snapshot can be re-seeded.
#   --seed  [snapshot]   Rewrite the snapshot from the live counts (run after a
#                        reviewed register update). Default snapshot path is
#                        bench/unsafe-snapshot.json.
#   --list               Print every enumerated site as file:line:text (for the
#                        register / a count cross-check). No snapshot needed.
#
# Exit 0 = pass, 1 = ratchet regression / missing crate, 2 = usage error.
import json
import os
import re
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_SNAPSHOT = os.path.join(REPO_ROOT, "bench", "unsafe-snapshot.json")
SCAN_ROOT = os.path.join(REPO_ROOT, "crates")

# The `unsafe` KEYWORD in statement/item position (not the `unsafe_code` lint name,
# which has no following `{`/`fn`/`impl`/`trait`/`extern`).
_KW = re.compile(r"\bunsafe\b(\s*\{|\s+fn\b|\s+impl\b|\s+trait\b|\s+extern\b)")


def _mask_line(line: str, in_block: bool):
    """Single-pass blank-out of comments and string contents on one line.

    Returns (masked_line, still_in_block). All four lexical contexts are tracked
    SIMULTANEOUSLY in source order, so the bugs of order-dependent stripping are
    avoided: a `/*` inside a `//` line comment (e.g. `// a/* = ...`) does NOT open
    a block comment; a `//` or `/*` inside a `"..."` string does NOT start a
    comment; and `*/` only closes a block when we are actually in one. Masked-out
    characters become spaces so column positions (and thus the `unsafe` keyword
    regex) stay faithful to the surviving code. Block-comment state carries across
    lines via `in_block`.

    Rust raw strings (r"...", r#"..."#) are NOT specially handled, but the keyword
    regex only fires on `unsafe` directly followed by `{`/`fn`/`impl`/`trait`/
    `extern`, which raw-string payloads in this codebase do not contain, so this is
    conservative for the count.
    """
    out = []
    in_str = esc = False
    i = 0
    n = len(line)
    while i < n:
        c = line[i]
        if in_block:
            if c == "*" and i + 1 < n and line[i + 1] == "/":
                in_block = False
                out.append("  ")
                i += 2
                continue
            out.append(" ")
            i += 1
            continue
        if in_str:
            out.append(" ")  # blank out string contents (incl. any // or /*)
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(" ")
            i += 1
            continue
        if c == "/" and i + 1 < n and line[i + 1] == "/":
            break  # rest of line is a `//` comment — drop it (even if it has /*)
        if c == "/" and i + 1 < n and line[i + 1] == "*":
            in_block = True
            out.append("  ")
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out), in_block


def enumerate_sites():
    """Yield (crate, relpath, lineno, text) for every first-party unsafe site."""
    for dirpath, _dirs, files in os.walk(SCAN_ROOT):
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(dirpath, name)
            rel = os.path.relpath(path, REPO_ROOT)
            crate = rel.split(os.sep)[1]  # crates/<crate>/...
            in_block = False
            with open(path, errors="replace") as fh:
                for lineno, raw in enumerate(fh, 1):
                    masked, in_block = _mask_line(raw, in_block)
                    if _KW.search(masked):
                        yield crate, rel, lineno, raw.rstrip()


def per_crate_counts():
    counts = {}
    for crate, _rel, _ln, _txt in enumerate_sites():
        counts[crate] = counts.get(crate, 0) + 1
    return counts


def load_snapshot(path):
    with open(path) as fh:
        doc = json.load(fh)
    return doc.get("crates", {})


def cmd_list():
    n = 0
    for _crate, rel, ln, txt in sorted(enumerate_sites()):
        print(f"{rel}:{ln}:{txt.strip()}")
        n += 1
    print(f"\nTOTAL={n}", file=sys.stderr)
    return 0


def cmd_seed(path):
    counts = per_crate_counts()
    doc = {
        "_comment": [
            "sq-toze.6 / cert gap GX-5 — UNSAFE-COUNT RATCHET snapshot.",
            "One entry per first-party crate that contains `unsafe`, with the count of",
            "unsafe sites (block / fn / impl / trait / extern) the crate currently holds.",
            "scripts/unsafe-gate.py --check FAILS CI when a crate's live count RISES above",
            "its count here without re-seeding. Every counted site MUST have a row in",
            "compliance/memsafety/unsafe-register.md and a `// SAFETY:` comment in source.",
            "Re-seed (scripts/unsafe-gate.py --seed) ONLY alongside a reviewed register",
            "update — the diff to this file is the review hook. Smaller is always allowed.",
        ],
        "total": sum(counts.values()),
        "crates": dict(sorted(counts.items())),
    }
    with open(path, "w") as fh:
        json.dump(doc, fh, indent=2)
        fh.write("\n")
    print(f"seeded {path}: total={doc['total']} across {len(counts)} crate(s)")
    for c, n in doc["crates"].items():
        print(f"  {c}: {n}")
    return 0


def cmd_check(path):
    if not os.path.exists(path):
        print(f"::error::unsafe snapshot not found: {path} (run --seed)", file=sys.stderr)
        return 2
    snap = load_snapshot(path)
    live = per_crate_counts()
    regressed = []
    below = []
    crates = sorted(set(snap) | set(live))
    print(f"{'crate':24} {'snapshot':>9} {'live':>6}")
    for c in crates:
        s = snap.get(c)
        v = live.get(c, 0)
        s_disp = "—" if s is None else str(s)
        flag = ""
        if s is None and v > 0:
            regressed.append(f"{c}: not in snapshot but has {v} unsafe site(s) — re-seed + register them")
            flag = "  NEW (FAIL)"
        elif s is not None and v > s:
            regressed.append(f"{c}: {v} unsafe site(s) > snapshot {s} (+{v - s}) — add register rows + re-seed")
            flag = "  RISE (FAIL)"
        elif s is not None and v < s:
            below.append(f"{c}: {v} < snapshot {s} (re-seed to tighten)")
            flag = "  below"
        print(f"{c:24} {s_disp:>9} {v:>6}{flag}")
    print(f"\nlive total = {sum(live.values())}, snapshot total = {sum(snap.values())}")
    for b in below:
        print(f"note: {b}")
    if regressed:
        print("\n::error::unsafe-count ratchet REGRESSED:", file=sys.stderr)
        for r in regressed:
            print(f"  - {r}", file=sys.stderr)
        print(
            "\nAdd a row per new `unsafe` site to compliance/memsafety/unsafe-register.md\n"
            "with a `// SAFETY:` comment in source, then run: scripts/unsafe-gate.py --seed",
            file=sys.stderr,
        )
        return 1
    print("\nunsafe-count ratchet: PASS (no crate rose above its snapshot).")
    return 0


def main(argv):
    if len(argv) < 2 or argv[1] not in ("--check", "--seed", "--list"):
        print("usage: unsafe-gate.py {--check|--seed} [snapshot.json] | --list", file=sys.stderr)
        return 2
    mode = argv[1]
    if mode == "--list":
        return cmd_list()
    path = argv[2] if len(argv) > 2 else DEFAULT_SNAPSHOT
    return cmd_seed(path) if mode == "--seed" else cmd_check(path)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
