#!/usr/bin/env python3
# Terminology HARD gate.
#   original RDF-star/SPARQL-star wording gate (bead sq-2p8z / issue #812).
#     made DATA-DRIVEN + given per-term file surfaces (issue #3811).
#
# WHAT: scans the repo for banned terminology and FAILS the build on a hit. The banned
# list is DATA, not code — `scripts/banned-terminology.json`. Adding a future banned
# term is ONE object in that file's `terms` array; this script does not change.
#
# WHY THE REWRITE (issue #3811): the original gate hard-coded its list AND scanned only
# `*.md`. A term banned by maintainer directive therefore reached PR #3451 as a `pub`
# Rust type name and as an `rdfs:comment` inside a PUBLISHED `.ttl` vocabulary with a
# green `ci-summary / gate` — neither extension was in the gate's surface, so nothing
# mechanical could have caught it. Terms now declare their own SURFACE, so a term that
# must not appear in code or vocabulary is checked in code and vocabulary.
#
# ESCAPES (all narrow, all reviewed in the diff — never a broad path exclusion):
#   1. inline `terminology-allow: <why>` on the offending line — the text after the
#      colon is the required human justification (the audit trail);
#   2. per-term `allowPatterns` — proper nouns, paper titles, third-party doc names,
#      URLs (e.g. the W3C "RDF-star Working Group" is a real body and must be nameable);
#   3. per-term `exemptPaths` — an ENUMERATED list of exact paths/globs, each carrying a
#      `why`, for files that must quote the term to define or test the ban.
#
# Deterministic, grep-only, no network, no build. HARD: the docs-quality `quick-gates`
# job name carries no advisory/informational token, so `ci-summary / gate` gates on it.
#
# Usage:
#   python3 scripts/check-terminology.py            # scan every term's declared surface
#   python3 scripts/check-terminology.py FILE ...   # scan explicit files (every term)
#   python3 scripts/check-terminology.py --self-test # hermetic both-direction self-test

from __future__ import annotations

import fnmatch
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CONFIG_PATH = REPO_ROOT / "scripts" / "banned-terminology.json"


class Term:
    """One banned term, compiled from its JSON declaration."""

    def __init__(self, raw: dict, allow_patterns: list[str], surfaces: dict):
        self.id: str = raw["id"]
        self.label: str = raw["label"]
        flags = re.I if raw.get("ignoreCase", True) else 0
        self.patterns = [re.compile(p, flags) for p in raw["patterns"]]
        self.surface_name: str = raw["surface"]
        if self.surface_name not in surfaces:
            raise KeyError(
                "term %r references undeclared surface %r" % (self.id, self.surface_name)
            )
        self.surface = surfaces[self.surface_name]
        self.allow_patterns = [re.compile(p, re.I) for p in allow_patterns]
        self.exempt_paths = [e["path"] for e in raw.get("exemptPaths", [])]

    def exempt_path(self, path: str) -> bool:
        """Enumerated per-term exemption: exact path OR an explicit `dir/**` glob."""
        return any(path == e or fnmatch.fnmatch(path, e) for e in self.exempt_paths)

    def line_allowed(self, line: str) -> bool:
        return any(p.search(line) for p in self.allow_patterns)

    def matches(self, line: str) -> bool:
        return any(p.search(line) for p in self.patterns)


def load_config(path: Path = CONFIG_PATH) -> tuple[str, list[Term], dict]:
    with open(path, encoding="utf-8") as fh:
        cfg = json.load(fh)
    allow_marker: str = cfg["allowMarker"]
    surfaces: dict = cfg["surfaces"]
    per_term_allow: dict = cfg.get("allowPatterns", {})
    marker_rx = re.escape(allow_marker) + r"\s*:"
    terms = [
        Term(raw, list(per_term_allow.get(raw["id"], [])) + [marker_rx], surfaces)
        for raw in cfg["terms"]
    ]
    return allow_marker, terms, cfg


def surface_files(surface: dict) -> list[str]:
    """Resolve a surface's include/exclude globs through `git ls-files`."""
    args = ["git", "ls-files"] + list(surface["include"]) + list(surface.get("exclude", []))
    out = subprocess.run(args, capture_output=True, text=True, check=True, cwd=REPO_ROOT)
    return sorted({p for p in out.stdout.splitlines() if p})


def scan_file(
    path: str, terms: list[Term], honor_exempt_paths: bool = True
) -> list[tuple[int, str, str]]:
    """Report (lineno, label, line) for every banned-term hit in `path`.

    `honor_exempt_paths=False` is used by the EXPLICIT-file mode: when a caller names a
    file outright ("scan exactly this"), the per-term path exemptions are bypassed. That
    is what makes the violation FIXTURES real evidence — they are exempt from the
    repo-wide sweep (they exist to contain violations), so if explicit mode also skipped
    them the mutation test would be vacuously green, which is precisely the defect class
    this gate exists to prevent (issue #3811). Line-level escapes (`allowPatterns` and
    the inline `terminology-allow:` marker) still apply in both modes.
    """
    findings: list[tuple[int, str, str]] = []
    try:
        with open(REPO_ROOT / path, encoding="utf-8") as fh:
            lines = fh.readlines()
    except (OSError, UnicodeDecodeError):
        return findings
    for n, line in enumerate(lines, 1):
        for term in terms:
            if honor_exempt_paths and term.exempt_path(path):
                continue
            if term.line_allowed(line):
                continue
            if term.matches(line):
                findings.append((n, term.label, line.rstrip("\n")))
                break
    return findings


def run(explicit_paths: list[str]) -> int:
    allow_marker, terms, _ = load_config()

    # Build the file -> applicable-terms map. With explicit paths, EVERY term applies
    # (that is how the fixtures are exercised); otherwise each term contributes its own
    # declared surface, so a code/vocabulary term is checked in code and vocabulary.
    per_file: dict[str, list[Term]] = {}
    if explicit_paths:
        for p in explicit_paths:
            per_file[p] = list(terms)
    else:
        cache: dict[str, list[str]] = {}
        for term in terms:
            if term.surface_name not in cache:
                cache[term.surface_name] = surface_files(term.surface)
            for p in cache[term.surface_name]:
                per_file.setdefault(p, []).append(term)

    total = 0
    honor_exempt = not explicit_paths
    for path in sorted(per_file):
        for n, label, line in scan_file(path, per_file[path], honor_exempt):
            total += 1
            print(f"{path}:{n}: banned terminology — {label}")
            print(f"    {line.strip()}")
    if total:
        print(
            f"\n{total} banned-terminology hit(s). Use the approved wording (the `label` "
            "above says what to say instead; the RDF/SPARQL wording rules live in "
            "skills/terminology/SKILL.md). If a line is a legitimate proper-noun, "
            "citation, or definition the gate over-matched, append an inline "
            f"`{allow_marker}: <one-line justification>` on that line. Do NOT widen a "
            "path exclusion to make a violation pass — that defeats the gate.",
            file=sys.stderr,
        )
        return 1
    scanned = len(per_file)
    print(
        f"terminology: clean — {len(terms)} banned term(s) checked over {scanned} file(s)."
    )
    return 0


# --------------------------------------------------------------------------- #
# Hermetic both-direction self-test. Pins the CLASSIFICATION of every term before
# the gate is trusted to guard the tree — no git, no network, no file I/O beyond
# the JSON. Cases live next to the term they exercise.
# --------------------------------------------------------------------------- #
MUST_FAIL = [
    # RDF 1.2 / SPARQL 1.2 wording (the original sq-2p8z coverage — must not regress).
    "sparq supports RDF-star quoted triples.",
    "Run SPARQL-star queries over the store.",
    "Hartig co-originated RDF*/SPARQL* (statement-level metadata).",
    "We store embedded triples in a side table.",
    "The RDF star feature is first-class.",
    # issue #3811 — the trust-container term, in every spelling that reached
    # PR #3451. The Rust and Turtle shapes are the ones the md-only gate could not see.
    "pub struct TrustEnvelope {",
    "pub fn parse_envelope(query: &str) -> Result<TrustEnvelope, ExpressionError> {",
    "let e: trust_envelope::Parsed = decode(bytes)?;",
    'rdfs:comment "…owned by whoever authenticates the trust envelope."@en ;',
    "The verifier sends a Trust Envelope containing the query.",
    "See the trust-envelope contract in §3.1.",
    "# parse the verifier->holder trust  envelope (query + TR + nonce)",
]

MUST_PASS = [
    # RDF 1.2 / SPARQL 1.2 — approved wording + the documented exceptions.
    "sparq supports RDF 1.2 triple terms.",
    "Run SPARQL 1.2 queries over the store.",
    "The W3C RDF-star Working Group standardised triple terms.",
    "Hartig & Thompson, *Foundations of RDF★ and SPARQL★* (2014).",
    "See GraphDB's rdf-sparql-star docs for their encoding.",
    "Historically nicknamed RDF-star. terminology-allow: glossary note",
    "GeoSPARQL spatial filters and Text-to-SPARQL are unrelated.",
    "The *engine runs real SPARQL* against a graph.",
    "It loads the operator's own RDF**, then indexes it.",
    # #3811 — the APPROVED replacement wording must pass.
    "pub struct ContractRequest {",
    "pub fn parse_request(query: &str) -> Result<ContractRequest, ExpressionError> {",
    "The verifier sends a trust-requirements document (TR) plus a nonce.",
    "pub struct TrustRequirements {",
    # Bare "envelope" is legitimate elsewhere and must NOT be caught — the term is the
    # COMPOUND. These are real in-repo usages (sparq-crdt, sparq-fedplan-mpc, ZK docs).
    "pub struct Envelope { pub payload: Vec<u8> }",
    "the leakage envelope of the MPC routing seam",
    "Salted-hash selective-disclosure envelope over a JWT.",
    "bench/lws-core-readpath/emit_envelope.py writes the console envelope",
    # The inline marker exempts a deliberate mention (e.g. a changelog recording the rename).
    "renamed TrustEnvelope -> ContractRequest. terminology-allow: CHANGELOG record of the rename",
]


def self_test() -> int:
    _, terms, cfg = load_config()

    def caught(line: str) -> bool:
        return any(t.matches(line) and not t.line_allowed(line) for t in terms)

    failures: list[str] = []
    for s in MUST_FAIL:
        if not caught(s):
            failures.append(f"  should-FAIL but PASSED: {s!r}")
    for s in MUST_PASS:
        if caught(s):
            failures.append(f"  should-PASS but FAILED: {s!r}")

    # Schema invariants — a malformed term must not silently disable the gate.
    ids = [t.id for t in terms]
    if len(ids) != len(set(ids)):
        failures.append("  duplicate term id(s) in banned-terminology.json")
    if not any(t.id == "trust-envelope" for t in terms):
        failures.append("  the #3811 term declaration is MISSING from banned-terminology.json")
    for t in terms:
        if not t.patterns:
            failures.append(f"  term {t.id!r} declares no patterns (vacuous)")
    for name, surf in cfg["surfaces"].items():
        if not surf.get("include"):
            failures.append(f"  surface {name!r} declares no include globs (vacuous)")

    if failures:
        print("check-terminology self-test FAILED:")
        print("\n".join(failures))
        return 1
    print(
        f"check-terminology self-test: OK — {len(MUST_FAIL)} should-FAIL + "
        f"{len(MUST_PASS)} should-PASS cases across {len(terms)} term(s)."
    )
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    return run([a for a in argv if not a.startswith("--")])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
