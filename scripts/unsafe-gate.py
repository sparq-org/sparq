#!/usr/bin/env python3
# sq-toze.6 (cert gap GX-5) — UNSAFE-COUNT RATCHET gate.
#
# Promotes the previously-INFORMATIONAL cargo-geiger report (ci.yml `geiger`, bead
# sq-emay — whose own comment said "Promote to a gate later only with a checked-in
# unsafe-count ratchet") into a failing CI job, mirroring the conformance /
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
# --verify-register separately checks a one-to-one match between those live sites
# and the register's File:line cells; equal counts alone cannot establish coverage.
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
#   --verify-register [register.md]
#                        FAIL on every stale cell, missing row or duplicate site.
#                        Defaults to compliance/memsafety/unsafe-register.md.
#   --self-test          Run fixtures and require the real register to pass.
#
# Exit 0 = pass, 1 = ratchet / register mismatch, 2 = usage / input error.
import json
import os
import re
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_SNAPSHOT = os.path.join(REPO_ROOT, "bench", "unsafe-snapshot.json")
DEFAULT_REGISTER = os.path.join(REPO_ROOT, "compliance", "memsafety", "unsafe-register.md")
SCAN_ROOT = os.path.join(REPO_ROOT, "crates")

# The `unsafe` KEYWORD in statement/item position (not the `unsafe_code` lint name,
# which has no following `{`/`fn`/`impl`/`trait`/`extern`).
_KW = re.compile(r"\bunsafe\b(\s*\{|\s+fn\b|\s+impl\b|\s+trait\b|\s+extern\b)")
_CRATE_HEADING = re.compile(r"^#{1,6}\s+`(sparq-[\w-]+)`")
# The backticks delimit ONLY the location; a cell may also carry a feature suffix,
# e.g. `src/simd.rs:98` (`approx-ann`). Do not full-match the entire cell.
_REGISTER_CELL = re.compile(r"^`([^`:\s]+):([0-9]+)`(?:\s|$)")


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


def register_mismatches(source, sites):
    """Compare File:line table cells to the SAME census used by --list.

    Locations beginning with crates/ are repository-relative; all others are
    relative to the crate named by the enclosing Markdown heading. Row numbers in
    diagnostics refer to the register, not to the Rust source. Return ALL problems,
    including uncovered live sites even when other cells are malformed or stale.
    """
    live = {(rel, ln): txt.strip() for _crate, rel, ln, txt in sites}
    rows = {}
    problems = []
    crate = None
    in_table = False
    for row_number, raw in enumerate(source.splitlines(), 1):
        line = raw.strip()
        if line.startswith("#"):
            heading = _CRATE_HEADING.match(line)
            crate = heading.group(1) if heading else None
        if not line.startswith("|"):
            in_table = False
            continue
        cell = line.split("|", 2)[1].strip()
        if cell == "File:line":
            in_table = True
            continue
        if not in_table or re.fullmatch(r":?-+:?", cell):
            continue
        match = _REGISTER_CELL.match(cell)
        if not match:
            problems.append(f"register row {row_number}: malformed File:line cell: {cell}")
            continue
        path, ln = match.group(1), int(match.group(2))
        if not path.startswith("crates/"):
            if crate is None:
                problems.append(
                    f"register row {row_number}: crate-relative cell has no crate heading: {cell}"
                )
                continue
            path = f"crates/{crate}/{path}"
        path = os.path.normpath(path)
        key = path, ln
        rows.setdefault(key, []).append(row_number)
        if key not in live:
            problems.append(f"register row {row_number}: no live unsafe site at {path}:{ln}")
    for (path, ln), row_numbers in sorted(rows.items()):
        if len(row_numbers) > 1:
            labels = ", ".join(map(str, row_numbers))
            problems.append(f"duplicate register rows {labels} name {path}:{ln}")
    for (path, ln), txt in sorted(live.items()):
        if (path, ln) not in rows:
            problems.append(f"live unsafe site has no register row: {path}:{ln}: {txt}")
    return problems


def cmd_verify_register(path):
    try:
        with open(path, encoding="utf-8") as fh:
            source = fh.read()
    except (OSError, UnicodeError) as error:
        print(f"::error::cannot read unsafe register {path}: {error}", file=sys.stderr)
        return 2
    sites = list(enumerate_sites())
    problems = register_mismatches(source, sites)
    if problems:
        print(f"::error::unsafe-register: {len(problems)} mismatch(es) in {path}:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(f"unsafe-register: PASS ({len(sites)} live sites each have exactly one register row).")
    return 0


def self_test():
    """Build-free fixtures exercise the CLI, then check the real register too."""
    import contextlib
    import io
    import tempfile
    from unittest.mock import patch

    header = "| File:line | Kind | Invariant relied on | Why sound / how bounded |\n|---|---|---|---|\n"
    fn_row = "| `src/lib.rs:3` | unsafe fn | fixture | fixture |\n"
    impl_row = "| `src/lib.rs:4` | unsafe impl | fixture | fixture |\n"
    block_row = "| `crates/sparq-fixture/src/lib.rs:6` (`optional`) | block | fixture | fixture |\n"
    baseline = (
        "### `sparq-fixture` — fixture sites\n\n" + header + fn_row + impl_row
        + "\nA second table in the same crate.\n\n" + header + block_row
        + "\n### `sparq-other` — fixture sites\n\n" + header
        + "| `src/lib.rs:1` | unsafe trait | fixture | fixture |\n"
        + '| `crates/sparq-other/src/lib.rs:2` | unsafe extern | fixture | fixture |\n'
    )
    # Each case asserts the exit status AND its diagnostic(s), so an unrelated
    # failure cannot masquerade as detection. The mixed case pins full reporting.
    cases = [
        ("valid/crate-relative-root-relative-and-suffix", baseline, 0,
         ["PASS (5 live sites each have exactly one register row)"], 0),
        ("stale-line", baseline.replace("src/lib.rs:3`", "src/lib.rs:30`"), 1,
         ["no live unsafe site at crates/sparq-fixture/src/lib.rs:30",
          "live unsafe site has no register row: crates/sparq-fixture/src/lib.rs:3:"], 2),
        ("missing-row", baseline.replace(impl_row, ""), 1,
         ["live unsafe site has no register row: crates/sparq-fixture/src/lib.rs:4:"], 1),
        ("duplicate-row", baseline.replace(block_row, block_row + block_row), 1,
         ["duplicate register rows", "name crates/sparq-fixture/src/lib.rs:6"], 1),
        ("all-mismatches", baseline.replace("src/lib.rs:3`", "src/lib.rs:30`")
         .replace(impl_row, "").replace(block_row, block_row + block_row), 1,
         ["no live unsafe site at crates/sparq-fixture/src/lib.rs:30",
          "live unsafe site has no register row: crates/sparq-fixture/src/lib.rs:3:",
          "live unsafe site has no register row: crates/sparq-fixture/src/lib.rs:4:",
          "duplicate register rows"], 4),
        ("malformed-cell", baseline.replace("`src/lib.rs:3`", "`src/lib.rs:typo`"), 1,
         ["malformed File:line cell", "live unsafe site has no register row:"], 2),
        ("missing-crate-heading", baseline.replace("### `sparq-fixture`", "### Fixture"), 1,
         ["crate-relative cell has no crate heading", "live unsafe site has no register row:"], 4),
        ("missing-file", baseline.replace("src/lib.rs:3`", "src/gone.rs:3`"), 1,
         ["no live unsafe site at crates/sparq-fixture/src/gone.rs:3"], 2),
        ("empty-register", "", 1, ["live unsafe site has no register row:"], 5),
        ("unreadable-register", None, 2, ["cannot read unsafe register"], 0),
    ]
    failures = 0
    with tempfile.TemporaryDirectory(prefix="unsafe-register-test-") as root:
        for crate, contents in (
            ("sparq-fixture", '// unsafe {}\nconst TEXT: &str = "unsafe {}";\n'
             'unsafe fn allocate() {}\nunsafe impl Send for Marker {}\nfn sample() {\n    unsafe {}\n}\n'),
            ("sparq-other", 'unsafe trait Contract {}\nunsafe extern "C" {}\n'),
        ):
            directory = os.path.join(root, "crates", crate, "src")
            os.makedirs(directory)
            with open(os.path.join(directory, "lib.rs"), "w", encoding="utf-8") as fh:
                fh.write(contents)
        register = os.path.join(root, "register.md")
        # Only scan the synthetic tree while driving the real CLI; restore both
        # roots before the final case reads the repository's actual census.
        with patch.dict(globals(), REPO_ROOT=root, SCAN_ROOT=os.path.join(root, "crates")):
            for label, source, expected, messages, problem_count in cases:
                if source is None:
                    os.remove(register)
                else:
                    with open(register, "w", encoding="utf-8") as fh:
                        fh.write(source)
                output = io.StringIO()
                with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
                    rc = main(["unsafe-gate.py", "--verify-register", register])
                diagnostic = output.getvalue()
                ok = (rc == expected and all(message in diagnostic for message in messages)
                      and diagnostic.count("  - ") == problem_count)
                print(f"[self-test] {'OK' if ok else 'FAIL'} {label} (exit {rc}, expected {expected})")
                if not ok:
                    failures += 1
                    print(diagnostic)
            # Pin the handler itself as well as its CLI dispatch. This direct
            # negative assertion must fail if cmd_verify_register stops checking.
            with open(register, "w", encoding="utf-8") as fh:
                fh.write(baseline.replace("src/lib.rs:3`", "src/lib.rs:30`"))
            output = io.StringIO()
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
                rc = cmd_verify_register(register)
            ok = (rc == 1 and "no live unsafe site at crates/sparq-fixture/src/lib.rs:30"
                  in output.getvalue() and output.getvalue().count("  - ") == 2)
            print(f"[self-test] {'OK' if ok else 'FAIL'} direct-verifier-stale-line")
            if not ok:
                failures += 1
                print(output.getvalue())
    # No expected-failure exception or crate allow-list: this must go green when
    # the real register is reconciled, and red on any subsequent source drift.
    rc = main(["unsafe-gate.py", "--verify-register"])
    print(f"[self-test] {'OK' if rc == 0 else 'FAIL'} real-register (expected PASS)")
    if rc != 0:
        failures += 1
    print(f"[self-test] {'FAIL' if failures else 'PASS'}: {len(cases) + 2} cases, {failures} failed")
    return 1 if failures else 0


def main(argv):
    modes = ("--check", "--seed", "--list", "--verify-register", "--self-test")
    if (len(argv) < 2 or argv[1] not in modes or len(argv) > 3
            or (argv[1] in ("--list", "--self-test") and len(argv) != 2)):
        print("usage: unsafe-gate.py {--check|--seed} [snapshot.json] | --list | "
              "--verify-register [register.md] | --self-test", file=sys.stderr)
        return 2
    mode = argv[1]
    if mode == "--self-test":
        return self_test()
    if mode == "--list":
        return cmd_list()
    if mode == "--verify-register":
        return cmd_verify_register(argv[2] if len(argv) > 2 else DEFAULT_REGISTER)
    path = argv[2] if len(argv) > 2 else DEFAULT_SNAPSHOT
    return cmd_seed(path) if mode == "--seed" else cmd_check(path)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
