#!/usr/bin/env python3
"""
scripts/export-kb-dump.py — tier-aware KB dump assembler (sq-tzars.8)

Assembles a versioned dump from per-tier KB artifacts into dumps/YYYY-MM-DD/:

  pkg-ontology.ttl.gz               — the PKG vocabulary (pkg.ttl)
  pkg-hand-authored.ttl.gz          — hand-authored tier (ingest_pkg.py output)
  pkg-machine.ttl.gz                — machine tier (literature pipeline, if present)
  pkg-restricted-projection.ttl.gz  — metadata-only public view of restricted tier (if present)
  manifest.json                     — per-file tier + license class + triple counts + commit
  dump-provenance.ttl               — PROV-O activity for this dump (prov:generatedAtTime)

The LICENSE-RESTRICTED tier is NEVER exported; only its metadata-only projection.
A mandatory leak check scans all dump files for restricted-tier markers and secrets.

Usage:
  # Self-test (writes to a temp dir + runs the injected-leak negative):
  python3 scripts/export-kb-dump.py --dry-run

  # Full dump run:
  python3 scripts/export-kb-dump.py [--out-dir dumps/] [--commit <sha>]
  [--hand-authored PATH] [--machine-tier PATH] [--restricted-projection PATH]

The GitHub Actions workflow (kb-dump.yml) calls this script, then pushes the
output directory to sparq-org/research-kb.

(sq-tzars.8) 🤖 SPARQ agent — sparq-org/research-kb dump assembler.
"""

import argparse
import gzip
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

# ── Tier graph IRIs (byte-pinned in crates/sparq-kb/src/vocab.rs) ─────────────
TIER_HAND_AUTHORED_GRAPH = "https://sparq.dev/ns/pkg/graph#hand-authored"
TIER_MACHINE_GRAPH = "https://sparq.dev/ns/pkg/graph#machine"
TIER_LICENSE_RESTRICTED_GRAPH = "https://sparq.dev/ns/pkg/graph#license-restricted"

# ── Leak-check patterns ───────────────────────────────────────────────────────
# Each entry is (compiled_regex_or_None, plain_string_or_None, human-readable reason).
# Exactly one of regex/string is non-None per entry.
# String markers use exact substring matching; regex markers use re.search per line.

# Global: applies to ALL dump files.
#
# Note on `Bearer`: task titles in the PKG legitimately contain the English word "Bearer"
# (e.g., "Gate WebSocket with the Bearer read-token") — an exact-string match would
# produce false positives.  We instead require `Bearer ` followed by ≥ 32 alphanumeric
# characters (the shortest realistic credential), which catches real token leaks without
# flagging natural language.
LEAK_PATTERNS_GLOBAL: list[tuple[re.Pattern | None, str | None, str]] = [
    (
        re.compile(r"Bearer\s+[A-Za-z0-9_\-]{32,}"),
        None,
        "Bearer auth-token (≥32 alphanumeric chars after 'Bearer ') — "
        "a real credential that must never appear in a dump",
    ),
    (
        None,
        "CORE_API_KEY",
        "CORE API key env-var name — must never appear in a dump",
    ),
    (
        None,
        "machine findings from unknown/absent/non-redistributable sources (PRIVATE)",
        "restricted FULL-tier graph label — indicates the private artifact was "
        "accidentally included instead of the public projection",
    ),
]

# Restricted-projection specific: the metadata-only PUBLIC projection must carry
# ZERO abstract-derived text.  These markers are only checked on the projection file.
#
# Two defence layers per predicate/class:
#   1. Prefixed-name string  (e.g. "sigimpl:justification") — catches common serialisations.
#   2. IRI-substring string  (e.g. "sig-impl#justification") — catches full-IRI and any
#      non-standard prefix serialisations that evade the prefixed form.
# Additionally, _rdflib_check_restricted_projection() parses the Turtle graph with rdflib
# and asserts zero matching triples by full IRI — catching rdflib's auto-prefixed forms
# (e.g. "ns1:justification") that evade BOTH string patterns above. rdflib is REQUIRED:
# without it run_leak_check refuses the export (fail closed).
LEAK_PATTERNS_RESTRICTED_PROJECTION: list[tuple[re.Pattern | None, str | None, str]] = [
    # ── sigimpl:justification ─────────────────────────────────────────────────
    (
        None,
        "sigimpl:justification",
        "abstract-derived justification text (prefixed form) — restricted full tier only; "
        "the public projection must not carry any sigimpl:justification triple",
    ),
    (
        None,
        "sig-impl#justification",
        "abstract-derived justification text (IRI-substring form) — catches full-IRI "
        "<https://w3id.org/zkp-sparql/sig-impl#justification> serialisation",
    ),
    # ── dcterms:abstract ─────────────────────────────────────────────────────
    (
        None,
        "dcterms:abstract",
        "raw abstract text (prefixed form) — restricted full tier only; "
        "the public projection must not carry any dcterms:abstract triple",
    ),
    (
        None,
        "dc/terms/abstract",
        "raw abstract text (IRI-substring form) — catches full-IRI "
        "<http://purl.org/dc/terms/abstract> serialisation",
    ),
    # ── pkg:Finding ──────────────────────────────────────────────────────────
    (
        None,
        "a pkg:Finding",
        "full Finding triple (prefixed form) — the restricted projection carries only "
        "source metadata, never Finding nodes",
    ),
    (
        None,
        "pkg#Finding",
        "full Finding triple (IRI-substring form) — catches full-IRI "
        "<https://sparq.dev/ns/pkg#Finding> serialisation",
    ),
]

# Self-test injected-leak marker: a sentinel value used ONLY in --dry-run.
# It is inserted as a sigimpl:justification value to simulate a leak of restricted-tier
# abstract-derived text, then the scanner must catch it (the negative self-test).
INJECTED_LEAK_MARKER = "SPARQ_KB_RESTRICTED_TIER_LEAK_MARKER_DO_NOT_EXPORT"

# ── Helpers ───────────────────────────────────────────────────────────────────


def _now_iso() -> str:
    """UTC ISO 8601 xsd:dateTime string (e.g. '2026-07-05T12:34:56Z')."""
    return datetime.now(tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _today() -> str:
    """UTC date string (e.g. '2026-07-05')."""
    return datetime.now(tz=timezone.utc).strftime("%Y-%m-%d")


def _sha256(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _count_triples(turtle_text: str) -> int:
    """
    Approximate triple count for a Turtle document.
    Uses rdflib if available; otherwise falls back to a heuristic that counts
    statement terminators (`.` for new subject, `;` for additional predicate,
    `,` for additional object) on non-comment, non-prefix lines.
    """
    try:
        import rdflib  # noqa: PLC0415

        g = rdflib.Graph()
        g.parse(data=turtle_text, format="turtle")
        return len(g)
    except Exception:
        count = 0
        for line in turtle_text.splitlines():
            s = line.strip()
            if not s or s.startswith("#") or s.startswith("@prefix") or s.startswith("@base"):
                continue
            # Count each statement terminator as one (or more) triples
            count += s.count(" .") + s.count("\t.")
            count += s.count(" ;") + s.count("\t;")
            count += s.count(" ,") + s.count("\t,")
            # A bare `.` at the end of a short line
            if s == ".":
                count += 1
        return max(count, 0)


def _git_head(repo_root: Path) -> str:
    """Return the current git HEAD SHA, or 'unknown' if git is unavailable."""
    try:
        result = subprocess.run(
            ["git", "-C", str(repo_root), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode == 0:
            return result.stdout.strip()
    except Exception:
        pass
    return "unknown"


def _compress(content: bytes) -> bytes:
    """gzip-compress *content* at default compression level."""
    import io

    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as gz:
        gz.write(content)
    return buf.getvalue()


# ── Leak check ────────────────────────────────────────────────────────────────


class LeakViolation:
    def __init__(self, filename: str, marker: str, reason: str, line_no: int, snippet: str):
        self.filename = filename
        self.marker = marker
        self.reason = reason
        self.line_no = line_no
        self.snippet = snippet

    def __str__(self) -> str:
        return (
            f"  LEAK in {self.filename!r} at line {self.line_no}: "
            f"marker={self.marker!r}\n"
            f"    reason: {self.reason}\n"
            f"    snippet: {self.snippet!r}"
        )


def leak_check_file(
    filename: str,
    content: str,
    extra_patterns: list[tuple[re.Pattern | None, str | None, str]] | None = None,
) -> list[LeakViolation]:
    """
    Scan *content* for LEAK_PATTERNS_GLOBAL (and optional *extra_patterns*).
    Returns a list of LeakViolation for every hit.
    """
    violations: list[LeakViolation] = []
    all_patterns = list(LEAK_PATTERNS_GLOBAL)
    if extra_patterns:
        all_patterns.extend(extra_patterns)

    lines = content.splitlines()
    for lineno, line in enumerate(lines, start=1):
        for pat, literal, reason in all_patterns:
            hit = False
            marker_display: str
            if literal is not None:
                if literal in line:
                    hit = True
                    marker_display = literal
            elif pat is not None:
                m = pat.search(line)
                if m:
                    hit = True
                    marker_display = m.group(0)[:60]
            if hit:
                snippet = line.strip()[:120]
                violations.append(
                    LeakViolation(
                        filename=filename,
                        marker=marker_display,
                        reason=reason,
                        line_no=lineno,
                        snippet=snippet,
                    )
                )
    return violations


# Marker of the fail-closed refusal emitted when rdflib is missing (#6037/#6230/#6383).
RDFLIB_REQUIRED_MARKER = "rdflib-required"
RDFLIB_INSTALL_HINT = "python3 -m pip install rdflib"
# Marker of the fail-closed refusal when rdflib cannot parse the projection.
PROJECTION_UNPARSEABLE_MARKER = "projection-unparseable"


def _rdflib_available() -> bool:
    """True iff rdflib can be imported. The serialisation-independent restricted-term
    check needs it: without a real Turtle parser, prefixed forms such as
    "ns1:justification" cannot be expanded, so the export must be refused."""
    try:
        import rdflib  # noqa: F401, PLC0415
    except ImportError:
        return False
    return True


def _rdflib_check_restricted_projection(
    filename: str, content: str
) -> list[LeakViolation]:
    """
    Parse *content* as Turtle using rdflib and assert zero restricted-tier triples
    by full IRI — serialisation-independent check that catches auto-prefixed forms
    (e.g. "ns1:justification") which evade both the prefixed-name and IRI-substring
    string patterns.

    Returns an empty list if rdflib is unavailable (run_leak_check refuses the export
    in that case). Unparseable Turtle yields a PROJECTION_UNPARSEABLE_MARKER violation
    (fail closed).
    """
    try:
        import rdflib  # noqa: PLC0415
    except ImportError:
        return []

    RESTRICTED_PREDICATES = [
        rdflib.URIRef("https://w3id.org/zkp-sparql/sig-impl#justification"),
        rdflib.URIRef("http://purl.org/dc/terms/abstract"),
    ]
    FINDING_CLASS = rdflib.URIRef("https://sparq.dev/ns/pkg#Finding")

    violations: list[LeakViolation] = []
    try:
        g = rdflib.Graph()
        g.parse(data=content, format="turtle")
    except Exception as exc:
        # FAIL CLOSED: a projection rdflib cannot parse cannot be verified free of
        # restricted-tier triples, and the string patterns miss prefixed forms.
        msg = (
            f"restricted projection is not parseable Turtle ({type(exc).__name__}: "
            f"{str(exc)[:200]}) — it cannot be verified free of restricted-tier terms; "
            f"refusing the export"
        )
        print(f"::error::{msg}", file=sys.stderr)
        return [
            LeakViolation(
                filename=filename,
                marker=PROJECTION_UNPARSEABLE_MARKER,
                reason=msg,
                line_no=0,
                snippet="",
            )
        ]

    for pred in RESTRICTED_PREDICATES:
        for subj, _, obj in g.triples((None, pred, None)):
            snippet = f"<{subj}> <{pred}> {obj!r}"[:120]
            violations.append(
                LeakViolation(
                    filename=filename,
                    marker=str(pred),
                    reason=(
                        f"rdflib IRI-match: predicate <{pred}> found in restricted "
                        f"projection — serialisation-independent check"
                    ),
                    line_no=0,
                    snippet=snippet,
                )
            )

    for subj, _, _ in g.triples((None, rdflib.RDF.type, FINDING_CLASS)):
        snippet = f"<{subj}> a <{FINDING_CLASS}>"[:120]
        violations.append(
            LeakViolation(
                filename=filename,
                marker=str(FINDING_CLASS),
                reason=(
                    f"rdflib IRI-match: pkg:Finding instance <{subj}> found in restricted "
                    f"projection — serialisation-independent check"
                ),
                line_no=0,
                snippet=snippet,
            )
        )

    return violations


def run_leak_check(
    dump_files: dict[str, str],
    restricted_projection_key: str | None = None,
) -> tuple[bool, list[LeakViolation]]:
    """
    Run the full leak check over *dump_files* (mapping filename → text content).

    *restricted_projection_key* names the file that must pass the additional
    restricted-projection patterns — checked via both string patterns (prefixed and
    IRI-substring forms) AND rdflib graph parsing (serialisation-independent).

    Returns (passed, violations). *passed* is True iff the violation list is empty.
    """
    all_violations: list[LeakViolation] = []

    for filename, content in dump_files.items():
        extra: list[tuple[re.Pattern | None, str | None, str]] | None = None
        if filename == restricted_projection_key:
            extra = list(LEAK_PATTERNS_RESTRICTED_PROJECTION)
        violations = leak_check_file(filename, content, extra_patterns=extra)
        all_violations.extend(violations)

        # Additional rdflib-based IRI check for the restricted projection.
        # FAIL CLOSED without rdflib: the string patterns cannot see prefixed forms
        # such as "ns1:justification", so a clean string scan proves nothing there.
        if filename == restricted_projection_key:
            if not _rdflib_available():
                msg = (
                    "rdflib is required to verify the restricted projection is free of "
                    "restricted-tier terms (prefixed forms such as 'ns1:justification' "
                    f"evade the string patterns) — refusing the export. Install it: "
                    f"{RDFLIB_INSTALL_HINT}"
                )
                print(f"::error::{msg}", file=sys.stderr)
                all_violations.append(
                    LeakViolation(
                        filename=filename,
                        marker=RDFLIB_REQUIRED_MARKER,
                        reason=msg,
                        line_no=0,
                        snippet="",
                    )
                )
            else:
                all_violations.extend(
                    _rdflib_check_restricted_projection(filename, content)
                )

    return (len(all_violations) == 0), all_violations


# ── PROV-O dump-provenance.ttl ────────────────────────────────────────────────


def make_dump_provenance(
    dump_date: str,
    generated_at: str,
    source_commit: str,
    tier_file_iris: list[str],
) -> str:
    """
    Produce a PROV-O Turtle document for this dump.

    Domain correctness:
      prov:generatedAtTime  — domain prov:Entity  → on the dump Entity
      prov:startedAtTime /
      prov:endedAtTime      — domain prov:Activity → on the dump Activity

    The Entity represents the dump artifact; the Activity represents the assembly run.
    """
    dump_entity_iri = f"https://sparq.dev/ns/kb/dump/{dump_date}"
    dump_activity_iri = f"https://sparq.dev/ns/kb/dump/{dump_date}/activity"
    used_triples = "\n".join(
        f"  prov:used <{iri}> ;" for iri in tier_file_iris
    )
    comment_text = (
        "Assembled by scripts/export-kb-dump.py. "
        "The LICENSE-RESTRICTED tier is never exported; "
        "only its metadata-only projection is included."
    )
    return f"""\
# dump-provenance.ttl — PROV-O record for the KB dump of {dump_date}
# Generated by scripts/export-kb-dump.py (sq-tzars.8)
# 🤖 SPARQ agent — do not hand-edit.

@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix xsd:     <http://www.w3.org/2001/XMLSchema#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix dcterms: <http://purl.org/dc/terms/> .

# The dump artifact (Entity) — prov:generatedAtTime has domain prov:Entity.
<{dump_entity_iri}> a prov:Entity ;
  rdfs:label "KB dump for {dump_date}"@en ;
  prov:generatedAtTime "{generated_at}"^^xsd:dateTime ;
  prov:wasGeneratedBy <{dump_activity_iri}> .

# The assembly run (Activity) — startedAtTime/endedAtTime stay on the Activity.
<{dump_activity_iri}> a prov:Activity ;
  rdfs:label "KB dump activity for {dump_date}"@en ;
  prov:startedAtTime "{generated_at}"^^xsd:dateTime ;
  prov:endedAtTime "{generated_at}"^^xsd:dateTime ;
{used_triples}
  dcterms:source <https://github.com/sparq-org/sparq/commit/{source_commit}> ;
  rdfs:comment "{comment_text}"@en .
"""


# ── Manifest ─────────────────────────────────────────────────────────────────


def make_manifest(
    dump_date: str,
    generated_at: str,
    source_commit: str,
    file_entries: list[dict],
) -> dict:
    """Build the manifest.json dict."""
    return {
        "dump_version": dump_date,
        "source_commit": source_commit,
        "generated_at": generated_at,
        "generator": "scripts/export-kb-dump.py (sq-tzars.8)",
        "note": (
            "The LICENSE-RESTRICTED tier is never exported. "
            "Only its metadata-only public projection (source IRI + title + year "
            "+ licence status; no abstract-derived text) is included. "
            "This repo is PRIVATE until the maintainer reviews the tier-enforcement "
            "guarantees and explicitly flips visibility."
        ),
        "tier_semantics": {
            "hand-authored": (
                "Human-curated PKG data: task projections from bd + skill front-matter "
                "+ hand-authored AGENTS.md findings (ingest_pkg.py output)."
            ),
            "machine": (
                "Literature pipeline output for sources with a known redistributable "
                "licence (CC-BY, CC0, public-domain). Machine-tier findings are capped "
                "at confidence ≤ 0.7 and assurance ≤ Conjectured."
            ),
            "restricted-projection": (
                "Metadata-only public view of the license-restricted tier: source IRI, "
                "title, year, and licence status only. No abstract-derived text."
            ),
        },
        "files": file_entries,
    }


# ── Core assembly ─────────────────────────────────────────────────────────────


def assemble_dump(
    out_dir: Path,
    ontology_content: str | None,
    hand_authored_content: str | None,
    machine_content: str | None,
    restricted_projection_content: str | None,
    source_commit: str,
    dump_date: str | None = None,
    generated_at: str | None = None,
) -> dict:
    """
    Assemble ALL dump content in memory, run the leak check over EVERYTHING
    (tier TTL files + manifest.json + dump-provenance.ttl), and only THEN write
    to *out_dir*.  No file is written before the leak check passes — fail-closed
    is guaranteed.

    Raises SystemExit(1) on any leak check failure.
    """
    dump_date = dump_date or _today()
    generated_at = generated_at or _now_iso()
    # out_dir is NOT created here — only after the leak check passes.

    file_entries: list[dict] = []
    dump_texts: dict[str, str] = {}  # filename → text (for leak check)
    pending_gz: dict[str, bytes] = {}  # filename → compressed bytes (deferred write)
    restricted_proj_key: str | None = None
    used_tier_iris: list[str] = []

    def _collect_tier(
        filename: str,
        content: str,
        tier: str,
        license_class: str,
        graph_iri: str,
    ) -> None:
        """Compress and record a tier artifact — does NOT write to disk yet."""
        nonlocal restricted_proj_key
        compressed = _compress(content.encode("utf-8"))
        pending_gz[filename] = compressed  # deferred until after leak check
        triple_count = _count_triples(content)
        raw_sha = _sha256(content.encode("utf-8"))
        file_entries.append(
            {
                "filename": filename,
                "tier": tier,
                "license_class": license_class,
                "graph_iri": graph_iri,
                "triple_count": triple_count,
                "source_sha256": raw_sha,
                "compressed_bytes": len(compressed),
            }
        )
        dump_texts[filename] = content
        if tier == "restricted-projection":
            restricted_proj_key = filename
        used_tier_iris.append(graph_iri)

    # 1. Ontology (optional — present if the ontology file exists)
    if ontology_content is not None:
        _collect_tier(
            "pkg-ontology.ttl.gz",
            ontology_content,
            tier="ontology",
            license_class="public",
            graph_iri="https://sparq.dev/ns/pkg#",
        )

    # 2. Hand-authored tier
    if hand_authored_content is not None:
        _collect_tier(
            "pkg-hand-authored.ttl.gz",
            hand_authored_content,
            tier="hand-authored",
            license_class="public",
            graph_iri=TIER_HAND_AUTHORED_GRAPH,
        )

    # 3. Machine tier (optional — may not exist yet)
    if machine_content is not None:
        _collect_tier(
            "pkg-machine.ttl.gz",
            machine_content,
            tier="machine",
            license_class="public",
            graph_iri=TIER_MACHINE_GRAPH,
        )

    # 4. Restricted projection (optional — may not exist yet)
    if restricted_projection_content is not None:
        _collect_tier(
            "pkg-restricted-projection.ttl.gz",
            restricted_projection_content,
            tier="restricted-projection",
            license_class="public",
            graph_iri=TIER_LICENSE_RESTRICTED_GRAPH,
        )

    # ── Build manifest + provenance in memory (included in leak check) ────────
    manifest = make_manifest(dump_date, generated_at, source_commit, file_entries)
    manifest_text = json.dumps(manifest, indent=2) + "\n"
    dump_texts["manifest.json"] = manifest_text

    prov_ttl = make_dump_provenance(dump_date, generated_at, source_commit, used_tier_iris)
    dump_texts["dump-provenance.ttl"] = prov_ttl

    # ── Leak check (mandatory, fail-closed, before ANY write) ─────────────────
    # Scans ALL content: tier TTL files + manifest.json + dump-provenance.ttl.
    # No file is written until this passes.
    passed, violations = run_leak_check(dump_texts, restricted_proj_key)
    if not passed:
        print("\n[FAIL] Leak check detected restricted-tier content or secrets:", file=sys.stderr)
        for v in violations:
            print(str(v), file=sys.stderr)
        print(
            "\nABORTING: the assembled dump contains forbidden content. "
            "Do NOT push this dump.",
            file=sys.stderr,
        )
        sys.exit(1)

    print(
        f"[OK]   Leak check passed ({len(dump_texts)} file(s) scanned, 0 violations).",
        file=sys.stdout,
    )

    # ── Write everything only after leak check passes ─────────────────────────
    out_dir.mkdir(parents=True, exist_ok=True)

    for fname, compressed_bytes in pending_gz.items():
        (out_dir / fname).write_bytes(compressed_bytes)

    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(manifest_text, encoding="utf-8")
    print(f"[OK]   manifest.json written ({len(file_entries)} file(s)).", file=sys.stdout)

    (out_dir / "dump-provenance.ttl").write_text(prov_ttl, encoding="utf-8")
    print("[OK]   dump-provenance.ttl written.", file=sys.stdout)

    return manifest


# ── Dry-run self-test ─────────────────────────────────────────────────────────


def run_dry_run_self_test(repo_root: Path) -> bool:
    """
    Run the --dry-run self-test:
    1. Assemble a synthetic dump with no restricted content → leak check PASSES.
    2. Inject a restricted-tier marker into the restricted projection file and
       re-assemble → leak check FAILS (the injected-leak negative).

    Returns True if the self-test passes (both assertions hold), False otherwise.
    """
    print("\n── Dry-run self-test ──────────────────────────────────────────────────", flush=True)

    # ── Synthetic tier content ─────────────────────────────────────────────────
    SYNTHETIC_HAND_AUTHORED = """\
# SYNTHETIC hand-authored tier (self-test only)
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix xsd:     <http://www.w3.org/2001/XMLSchema#> .
@prefix kb:      <https://sparq.dev/ns/pkg/kb#> .

# Tier stamp
<https://sparq.dev/ns/pkg/graph#hand-authored> <http://www.w3.org/2000/01/rdf-schema#label>
  "hand-authored tier -- self-test synthetic data" .

kb:finding-selftest-1 a pkg:Finding ;
  pkg:about kb:topic-test ;
  pkg:confidence "0.90"^^xsd:decimal ;
  pkg:assurance secx:Claimed ;
  sigimpl:justification "This is a hand-authored justification — legitimately in the public dump."@en ;
  prov:wasDerivedFrom kb:source-selftest-1 .
"""

    SYNTHETIC_MACHINE = """\
# SYNTHETIC machine tier (self-test only)
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix xsd:     <http://www.w3.org/2001/XMLSchema#> .
@prefix kb:      <https://sparq.dev/ns/pkg/kb#> .

# Tier stamp
<https://sparq.dev/ns/pkg/graph#machine> <http://www.w3.org/2000/01/rdf-schema#label>
  "machine tier -- self-test synthetic data" .

kb:finding-machine-1 a pkg:Finding ;
  pkg:about kb:topic-query-opt ;
  pkg:confidence "0.65"^^xsd:decimal ;
  pkg:assurance secx:Conjectured ;
  sigimpl:justification "Machine-extracted justification for a CC-BY source — legitimately public."@en ;
  prov:wasDerivedFrom kb:source-openalex-1 .
"""

    # The restricted PROJECTION carries ONLY source metadata — NO justification,
    # NO abstract, NO pkg:Finding triples.
    SYNTHETIC_RESTRICTED_PROJECTION = """\
# SYNTHETIC restricted-tier public projection (self-test only)
# Metadata-only: DOI + title + year + licence status. NO abstract-derived text.
@prefix pkg:     <https://sparq.dev/ns/pkg#> .
@prefix dcterms: <http://purl.org/dc/terms/> .
@prefix schema:  <http://schema.org/> .
@prefix xsd:     <http://www.w3.org/2001/XMLSchema#> .
@prefix kb:      <https://sparq.dev/ns/pkg/kb#> .

# Tier stamp (metadata-only PUBLIC projection)
<https://sparq.dev/ns/pkg/graph#license-restricted>
  <http://www.w3.org/2000/01/rdf-schema#label>
  "license-restricted tier -- metadata-only PUBLIC projection (DOI + title + year + licence status; NO abstract-derived text)" .

# Restricted source metadata (no abstract, no findings, no justification)
<https://doi.org/10.5555/selftest.restricted.1> a pkg:Source ;
  dcterms:title "A Restricted Source (self-test)" ;
  schema:datePublished "2025"^^xsd:gYear ;
  pkg:exploredStatus pkg:Explored ;
  <https://sparq.dev/ns/pkg#licenseStatus> "unknown" .
"""

    SYNTHETIC_ONTOLOGY = """\
# SYNTHETIC ontology stub (self-test only)
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

<https://sparq.dev/ns/pkg#> a owl:Ontology ;
  rdfs:label "PKG vocabulary (self-test stub)" .
"""

    import tempfile

    with tempfile.TemporaryDirectory(prefix="sparq-kb-dump-selftest-") as tmpdir:
        out_path = Path(tmpdir) / "dump"

        have_rdflib = _rdflib_available()
        if not have_rdflib:
            # ── Step 0: without rdflib a real export must be REFUSED ─────────
            print(
                "\n[self-test] Step 0: rdflib NOT available — a real leak check must "
                "REFUSE the export (fail closed)"
            )
            import contextlib as _ctx  # noqa: PLC0415
            import io as _io  # noqa: PLC0415

            _err = _io.StringIO()
            refused = False
            with _ctx.redirect_stderr(_err):
                try:
                    assemble_dump(
                        out_dir=Path(tmpdir) / "dump-no-rdflib",
                        ontology_content=SYNTHETIC_ONTOLOGY,
                        hand_authored_content=SYNTHETIC_HAND_AUTHORED,
                        machine_content=SYNTHETIC_MACHINE,
                        restricted_projection_content=SYNTHETIC_RESTRICTED_PROJECTION,
                        source_commit="selftest-no-rdflib",
                        dump_date="1970-01-01",
                        generated_at="1970-01-01T00:00:00Z",
                    )
                except SystemExit as e:
                    refused = e.code == 1
            if not refused or RDFLIB_REQUIRED_MARKER not in _err.getvalue():
                print(
                    "[FAIL] self-test step 0: without rdflib the export was NOT refused.",
                    file=sys.stderr,
                )
                return False
            if (Path(tmpdir) / "dump-no-rdflib").exists():
                print(
                    "[FAIL] self-test step 0: the refused export still wrote files.",
                    file=sys.stderr,
                )
                return False
            print("[self-test] Step 0 PASSED — export refused without rdflib.")
            print(
                f"[self-test] Steps 1 and 1b SKIPPED — rdflib not available "
                f"(a clean export requires it; install: {RDFLIB_INSTALL_HINT})."
            )

        if have_rdflib:
            # ── Step 1: clean assembly — expect PASS ──────────────────────────────
            print("\n[self-test] Step 1: assemble clean dump — leak check must PASS")
            try:
                assemble_dump(
                    out_dir=out_path,
                    ontology_content=SYNTHETIC_ONTOLOGY,
                    hand_authored_content=SYNTHETIC_HAND_AUTHORED,
                    machine_content=SYNTHETIC_MACHINE,
                    restricted_projection_content=SYNTHETIC_RESTRICTED_PROJECTION,
                    source_commit="selftest-0000000",
                    dump_date="1970-01-01",
                    generated_at="1970-01-01T00:00:00Z",
                )
            except SystemExit:
                print(
                    "[FAIL] self-test step 1: clean dump UNEXPECTEDLY failed the leak check.",
                    file=sys.stderr,
                )
                return False

            print("[self-test] Step 1 PASSED — clean dump clears the leak check.")

            # ── Step 1b: prov round-trip — parse the generated dump-provenance.ttl ─
            print("\n[self-test] Step 1b: parse dump-provenance.ttl with rdflib")
            prov_file = out_path / "dump-provenance.ttl"
            try:
                import rdflib as _rdflib  # noqa: PLC0415

                _g = _rdflib.Graph()
                _g.parse(str(prov_file), format="turtle")
                print(
                    f"[self-test] Step 1b PASSED — dump-provenance.ttl is valid Turtle "
                    f"({len(_g)} triples)."
                )
            except ImportError:
                print(
                    "[self-test] Step 1b SKIPPED — rdflib not available; "
                    "string-scan only for provenance validation."
                )
            except Exception as exc:
                print(
                    f"[FAIL] self-test step 1b: dump-provenance.ttl is not valid Turtle: {exc}",
                    file=sys.stderr,
                )
                return False

        # ── Step 2: injected-leak negative — expect FAIL ──────────────────────
        print(
            "\n[self-test] Step 2: inject bare full-IRI restricted-tier triple into the "
            "projection — leak check must FAIL and report the injected value.\n"
            "  (No comment naming the predicate; scanner must catch the TRIPLE itself.)"
        )

        # Inject the BARE full-IRI triple — no comment naming sigimpl:justification.
        # The scanner must detect this via IRI-substring or rdflib graph parsing,
        # NOT via the comment line (which was the previous vacuous pass mechanism).
        INJECTED_PROJECTION = SYNTHETIC_RESTRICTED_PROJECTION + (
            f"\n<https://doi.org/10.5555/selftest.restricted.1>"
            f"\n  <https://w3id.org/zkp-sparql/sig-impl#justification>"
            f' "{INJECTED_LEAK_MARKER}" .\n'
        )

        # Redirect stderr to capture the failure message, then verify it
        import io  # noqa: PLC0415
        import contextlib  # noqa: PLC0415

        captured_err = io.StringIO()

        # We WANT assemble_dump to call sys.exit(1) here; catch it.
        exited_with_failure = False
        with contextlib.redirect_stderr(captured_err):
            try:
                # Use a fresh subdirectory for the injected-leak run
                injected_out = Path(tmpdir) / "dump-injected"
                assemble_dump(
                    out_dir=injected_out,
                    ontology_content=SYNTHETIC_ONTOLOGY,
                    hand_authored_content=SYNTHETIC_HAND_AUTHORED,
                    machine_content=SYNTHETIC_MACHINE,
                    restricted_projection_content=INJECTED_PROJECTION,
                    source_commit="selftest-injected",
                    dump_date="1970-01-01",
                    generated_at="1970-01-01T00:00:00Z",
                )
            except SystemExit as e:
                if e.code == 1:
                    exited_with_failure = True

        err_output = captured_err.getvalue()

        if not exited_with_failure:
            print(
                "[FAIL] self-test step 2: injected-leak dump did NOT fail the leak check "
                "— the scanner is not catching the bare full-IRI sigimpl:justification "
                "triple in the projection.",
                file=sys.stderr,
            )
            print(f"  stderr was: {err_output!r}", file=sys.stderr)
            return False

        # Require INJECTED_LEAK_MARKER itself to appear in the violation report.
        # This proves the scanner hit the actual triple value — not merely a comment
        # that names the predicate (the previous vacuous pass mechanism).
        if INJECTED_LEAK_MARKER not in err_output:
            print(
                "[FAIL] self-test step 2: leak check exited 1 as expected but did not "
                f"report the injected value {INJECTED_LEAK_MARKER!r} in stderr.\n"
                "  The scanner may have matched a comment rather than the actual triple.",
                file=sys.stderr,
            )
            print(f"  stderr was: {err_output!r}", file=sys.stderr)
            return False

        print("[self-test] Step 2 PASSED — bare full-IRI injected triple correctly caught.")
        print(
            "  (INJECTED_LEAK_MARKER appeared in the violation report — "
            "scanner hit the actual triple, not a comment.)"
        )

        # ── Step 3: evasion probe verification ────────────────────────────────
        # Re-run all four of the reviewer's evasion probes against run_leak_check
        # directly to confirm each is now caught.
        print(
            "\n[self-test] Step 3: evasion probe verification — "
            "all four reviewer probes must be CAUGHT"
        )

        PROJ_KEY = "pkg-restricted-projection.ttl.gz"

        def _probe(label: str, content: str, rdflib_only: bool = False) -> bool:
            """Run run_leak_check on *content* as the restricted projection; return True if caught.

            The fail-closed rdflib refusal is NOT counted as catching a probe: only a
            violation naming the restricted content does. A probe that only the rdflib
            check can catch is SKIPPED (not EVADED) when rdflib is absent, because the
            exporter then refuses every export anyway (Step 0)."""
            if rdflib_only and not have_rdflib:
                print(
                    f"  [PROBE {label}] SKIPPED — only the rdflib check can catch this "
                    f"form and rdflib is not available (exports are refused without it; "
                    f"install: {RDFLIB_INSTALL_HINT})."
                )
                return True
            import contextlib as _ctx  # noqa: PLC0415
            import io as _io  # noqa: PLC0415

            with _ctx.redirect_stderr(_io.StringIO()):
                _, all_viols = run_leak_check(
                    {PROJ_KEY: content},
                    restricted_projection_key=PROJ_KEY,
                )
            viols = [v for v in all_viols if v.marker != RDFLIB_REQUIRED_MARKER]
            if not viols:
                print(
                    f"  [PROBE {label}] EVADED — scanner did NOT catch this form.",
                    file=sys.stderr,
                )
                return False
            print(f"  [PROBE {label}] CAUGHT — {len(viols)} violation(s).")
            return True

        # Probe A: full-IRI sigimpl:justification predicate (no prefix declaration)
        probe_a_ok = _probe(
            "A (full-IRI sigimpl)",
            "<https://doi.org/10.5555/probe.a>"
            " <https://w3id.org/zkp-sparql/sig-impl#justification>"
            ' "abstract-derived text" .\n',
        )

        # Probe B: rdflib auto-prefix form (ns1: with explicit @prefix binding)
        probe_b_ok = _probe(
            "B (ns1: auto-prefix)",
            "@prefix ns1: <https://w3id.org/zkp-sparql/sig-impl#> .\n"
            '<https://doi.org/10.5555/probe.b> ns1:justification "auto-prefix text" .\n',
            rdflib_only=True,
        )

        # Probe C: bare full-IRI triple (no comment; the previous self-test blind-spot)
        probe_c_ok = _probe(
            "C (bare triple, no comment)",
            "<https://doi.org/10.5555/probe.c>"
            " <https://w3id.org/zkp-sparql/sig-impl#justification>"
            f' "{INJECTED_LEAK_MARKER}" .\n',
        )

        # Probe D: full-IRI pkg#Finding + full-IRI dcterms:abstract
        probe_d_ok = _probe(
            "D (full-IRI pkg#Finding + dcterms:abstract)",
            "<https://doi.org/10.5555/probe.d>"
            " a <https://sparq.dev/ns/pkg#Finding> ;\n"
            "  <http://purl.org/dc/terms/abstract>"
            ' "raw abstract text" .\n',
        )

        if not all([probe_a_ok, probe_b_ok, probe_c_ok, probe_d_ok]):
            print(
                "[FAIL] self-test step 3: one or more evasion probes were NOT caught.",
                file=sys.stderr,
            )
            return False

        if have_rdflib:
            print("[self-test] Step 3 PASSED — all four evasion probes caught.")
        else:
            print(
                "[self-test] Step 3 PASSED — probes A/C/D caught; PROBE B SKIPPED "
                "(rdflib not available, so the exporter refuses every export)."
            )

    print("\n── Self-test result: ALL PASSED ────────────────────────────────────────")
    return True


# ── Argument parsing and main ─────────────────────────────────────────────────


def _find_repo_root(start: Path) -> Path:
    """Walk up from *start* looking for the .git directory."""
    p = start.resolve()
    while p != p.parent:
        if (p / ".git").exists():
            return p
        p = p.parent
    return start.resolve()


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Assemble a tier-aware sparq KB dump (sq-tzars.8).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help=(
            "Run the self-test: assemble a synthetic dump (temp dir) + run the "
            "injected-leak negative test. Does not modify the repository or push anything."
        ),
    )
    ap.add_argument(
        "--repo-root",
        type=Path,
        default=None,
        help="Path to the sparq repo root (auto-detected if omitted).",
    )
    ap.add_argument(
        "--out-dir",
        type=Path,
        default=None,
        help=(
            "Output directory. A dated subdirectory (YYYY-MM-DD) is created inside it. "
            "Defaults to <repo-root>/dumps/."
        ),
    )
    ap.add_argument(
        "--commit",
        type=str,
        default=None,
        help="Source commit SHA to embed in the manifest (auto-detected from git HEAD).",
    )
    ap.add_argument(
        "--hand-authored",
        type=Path,
        default=None,
        help=(
            "Path to the hand-authored tier TTL artifact "
            "(default: crates/sparq-kb/ingest/pkg-instances.ttl relative to repo root)."
        ),
    )
    ap.add_argument(
        "--machine-tier",
        type=Path,
        default=None,
        help=(
            "Path to the machine-tier TTL artifact (optional; omitted if the file does "
            "not exist)."
        ),
    )
    ap.add_argument(
        "--restricted-projection",
        type=Path,
        default=None,
        help=(
            "Path to the metadata-only public projection of the restricted tier (optional; "
            "omitted if the file does not exist)."
        ),
    )
    args = ap.parse_args()

    # ── Dry-run self-test ─────────────────────────────────────────────────────
    if args.dry_run:
        repo_root = args.repo_root or _find_repo_root(Path(__file__))
        ok = run_dry_run_self_test(repo_root)
        if ok:
            print("\n[PASS] --dry-run self-test GREEN.", flush=True)
            return 0
        else:
            print("\n[FAIL] --dry-run self-test FAILED.", file=sys.stderr, flush=True)
            return 1

    # ── Normal assembly run ───────────────────────────────────────────────────
    repo_root = args.repo_root or _find_repo_root(Path(__file__))

    # Resolve tier artifact paths
    hand_authored_path = args.hand_authored or (
        repo_root / "crates" / "sparq-kb" / "ingest" / "pkg-instances.ttl"
    )
    ontology_path = (
        repo_root / "crates" / "sparq-kb" / "ontology" / "pkg" / "pkg.ttl"
    )

    # Machine tier and restricted projection: optional
    machine_tier_path = args.machine_tier
    restricted_proj_path = args.restricted_projection

    # Output directory
    out_base = args.out_dir or (repo_root / "dumps")
    dump_date = _today()
    out_dir = out_base / dump_date

    # Source commit
    source_commit = args.commit or _git_head(repo_root)

    # Read tier artifacts
    def _read_opt(path: Path | None, label: str) -> str | None:
        if path is None:
            return None
        if not path.exists():
            print(f"[SKIP] {label}: {path} not found — skipping tier.", file=sys.stdout)
            return None
        content = path.read_text(encoding="utf-8")
        print(f"[READ] {label}: {path} ({len(content):,} chars).", file=sys.stdout)
        return content

    ontology_content = _read_opt(ontology_path, "ontology")
    hand_authored_content = _read_opt(hand_authored_path, "hand-authored tier")
    machine_content = _read_opt(machine_tier_path, "machine tier")
    restricted_projection_content = _read_opt(restricted_proj_path, "restricted projection")

    if hand_authored_content is None:
        print(
            f"[ERROR] Hand-authored tier artifact not found: {hand_authored_path}",
            file=sys.stderr,
        )
        print("  Run crates/sparq-kb/ingest/ingest_pkg.py first.", file=sys.stderr)
        return 1

    print(f"\nAssembling KB dump → {out_dir}", flush=True)

    assemble_dump(
        out_dir=out_dir,
        ontology_content=ontology_content,
        hand_authored_content=hand_authored_content,
        machine_content=machine_content,
        restricted_projection_content=restricted_projection_content,
        source_commit=source_commit,
        dump_date=dump_date,
    )

    print(f"\n[DONE] Dump written to: {out_dir}")
    print("  Review the manifest, then push to sparq-org/research-kb (kb-dump.yml).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
