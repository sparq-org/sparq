#!/usr/bin/env python3
"""Upstream-release watch for the vendored `spargebra` (bead sq-98w7z.8).

`vendor/spargebra` + the root `[patch.crates-io]` entry exist because six W3C
conformance parser fixes are on oxigraph main but have never shipped in a
PUBLISHED spargebra (crates.io still tops out at 0.4.6). This script answers the
one question that gates retiring that tree — "has upstream released above 0.4.6
yet?" — against the crates.io sparse index, so a re-check is one command instead
of a research session.

    python3 scripts/check-spargebra-release.py           # query the index
    python3 scripts/check-spargebra-release.py --self-test   # offline, no network

Exit codes (distinct on purpose — a wrapper must never read a network failure as
"still blocked"):

    0   still at the baseline; the vendor tree stays. Re-defer with a dated note.
    10  a stable release ABOVE the baseline exists — retirement is unblocked;
        the printed checklist is the actual scope of that work.
    2   indeterminate (network/parse failure). Not an answer; do not act on it.

This is a DEVELOPER tool, not a CI gate: it depends on a live network call to
crates.io, so wiring it into a required check would make the gate flaky on an
upstream outage. Run it when the bead is picked up.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

# The version this tree was forked from (see vendor/spargebra/SPARQ-PATCHES.md).
BASELINE = (0, 4, 6)
# crates.io sparse index. Path layout for a >=4-char crate name is `<a><b>/<c><d>/<name>`.
INDEX_URL = "https://index.crates.io/sp/ar/spargebra"
USER_AGENT = "sparq-upstream-watch (https://github.com/sparq-org/sparq)"
TIMEOUT_S = 30


def parse_version(vers: str) -> tuple[tuple[int, int, int], bool]:
    """Return ((major, minor, patch), is_prerelease) for a semver string.

    Build metadata (`+...`) is stripped; a pre-release (`-beta.1`) is flagged so
    callers can ignore it — a beta is not a retirement target.
    """
    core = vers.split("+", 1)[0]
    core, _, pre = core.partition("-")
    parts = core.split(".")
    if len(parts) != 3:
        raise ValueError(f"not a semver core: {vers!r}")
    major, minor, patch = (int(p) for p in parts)
    return (major, minor, patch), bool(pre)


def newest_stable(index_body: str) -> tuple[tuple[int, int, int] | None, list[str]]:
    """Highest non-yanked, non-prerelease version in a sparse-index response.

    The index is newline-delimited JSON, one object per published version, and is
    NOT guaranteed sorted — take the max rather than the last line.
    """
    best: tuple[int, int, int] | None = None
    skipped: list[str] = []
    for line in index_body.splitlines():
        line = line.strip()
        if not line:
            continue
        entry = json.loads(line)
        vers = entry["vers"]
        if entry.get("yanked"):
            skipped.append(f"{vers} (yanked)")
            continue
        try:
            triple, is_pre = parse_version(vers)
        except ValueError:
            skipped.append(f"{vers} (unparseable)")
            continue
        if is_pre:
            skipped.append(f"{vers} (prerelease)")
            continue
        if best is None or triple > best:
            best = triple
    return best, skipped


def fmt(v: tuple[int, int, int]) -> str:
    return ".".join(str(p) for p in v)


RETIREMENT_CHECKLIST = """\
Retirement is NOT just "drop the patch entry and bump". Verified scope:

  1. 13 manifests carry a dependency on the vendored tree, not 1. The root
     Cargo.toml uses `[patch.crates-io]`, but bench/* (11) and
     zk/xpath/differential (1) are SEPARATE workspaces that the root patch table
     never reaches — each pins `path = ".../vendor/spargebra"` directly and must
     be repointed at the registry version by hand.
  2. The next release is 0.5.0, not 0.4.7 (upstream main is `0.5.0-dev`). That is
     a semver-MAJOR bump: the root `spargebra = { version = "0.4", ... }`
     requirement will not resolve it, and the Chumsky/Logos parser rewrite
     (dabda10) changed the crate internals wholesale.
  3. spargebra 0.5.0 will pull a newer `oxrdf` than the vendored `=0.3.3` pin.
     `spargebra::Query` embeds oxrdf term types across every crate seam, so a
     duplicate oxrdf in the lock is a hard type error, not a warning. Check the
     oxrdf major before assuming the bump is mechanical.
  4. FOUR of the ten vendored patches are sparq-local and have NO upstream home,
     so an upstream release does NOT make the tree droppable on its own:
       §7  `rand` 0.9 pin (wasm build guard)
       §8  parser recursion-depth cap — DoS hardening for the unauthenticated
           /sparql endpoint (threat-model B2 / T-PARSE-DoS, bead sq-v5dg)
       §9  custom-aggregate DISTINCT reachability fix
       §10 `MULTIPLICITY()` reserved-IRI extension — explicitly "not upstream";
           sparq-engine evaluation depends on it
     Each must be re-landed on the release, upstreamed, or consciously dropped
     with its dependents. §8 in particular is a SECURITY regression if dropped
     silently.
  5. Only then: drop the patch entry + vendor tree, bump the requirement, and
     re-run the FULL W3C conformance suite. The ratchet (ci.yml `conformance`,
     currently 0-fail) is the invariant — any new failure means the release is
     missing a fix, so KEEP the vendor tree and report it upstream."""


def selftest() -> int:
    """Exercise the version logic offline (no network)."""
    cases = [
        ("0.4.6", ((0, 4, 6), False)),
        ("0.5.0", ((0, 5, 0), False)),
        ("0.4.0-beta.1", ((0, 4, 0), True)),
        ("0.2.0-rc.1", ((0, 2, 0), True)),
        ("1.0.0+build.5", ((1, 0, 0), False)),
    ]
    for vers, want in cases:
        got = parse_version(vers)
        assert got == want, f"parse_version({vers!r}) = {got}, want {want}"

    # Unsorted, with a yanked and a prerelease entry above the stable max.
    body = "\n".join(
        json.dumps(d)
        for d in (
            {"vers": "0.4.5", "yanked": False},
            {"vers": "0.5.0-beta.1", "yanked": False},
            {"vers": "0.4.6", "yanked": False},
            {"vers": "0.4.4", "yanked": False},
            {"vers": "0.9.9", "yanked": True},
        )
    )
    best, skipped = newest_stable(body)
    assert best == (0, 4, 6), f"newest_stable = {best}, want (0, 4, 6)"
    assert len(skipped) == 2, f"skipped = {skipped}"

    # A genuine release above the baseline must be detected.
    best, _ = newest_stable(json.dumps({"vers": "0.5.0", "yanked": False}))
    assert best == (0, 5, 0) and best > BASELINE

    # An empty/blank body is indeterminate, not "no release".
    assert newest_stable("\n  \n")[0] is None

    print("selftest: ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="verify the version-comparison logic offline and exit",
    )
    args = ap.parse_args()
    if args.self_test:
        return selftest()

    req = urllib.request.Request(INDEX_URL, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_S) as resp:
            body = resp.read().decode("utf-8")
    except (urllib.error.URLError, OSError, TimeoutError) as exc:
        print(f"INDETERMINATE: could not reach the crates.io index: {exc}", file=sys.stderr)
        print("This is NOT evidence that upstream has not released.", file=sys.stderr)
        return 2

    try:
        best, skipped = newest_stable(body)
    except (json.JSONDecodeError, KeyError) as exc:
        print(f"INDETERMINATE: could not parse the index response: {exc}", file=sys.stderr)
        return 2

    if best is None:
        print("INDETERMINATE: the index returned no usable stable version.", file=sys.stderr)
        return 2

    print(f"baseline (vendored): {fmt(BASELINE)}")
    print(f"newest stable on crates.io: {fmt(best)}")
    if skipped:
        print(f"ignored: {', '.join(skipped)}")

    if best <= BASELINE:
        print()
        print(
            f"STILL BLOCKED: no spargebra release above {fmt(BASELINE)}. The vendor tree "
            "and the [patch.crates-io] entry stay."
        )
        print("Add a dated note to vendor/spargebra/SPARQ-PATCHES.md and re-defer the bead.")
        return 0

    print()
    print(f"RELEASED: spargebra {fmt(best)} > {fmt(BASELINE)} — retirement is unblocked.")
    print()
    print(RETIREMENT_CHECKLIST)
    return 10


if __name__ == "__main__":
    sys.exit(main())
