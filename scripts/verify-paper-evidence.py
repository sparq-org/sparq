#!/usr/bin/env python3
# [OPUS-4.8] sq-gum8.13 (epic sq-gum8, paper factory F1) — the fail-closed
# EVIDENCE-BINDING verifier for the academic paper factory.
#
# WHAT (design record research/paper-factory-2026-07.md §3.1, option (c) tiered bindings):
# every record in site/src/data/paper-evidence.json carries a `value` a paper may headline
# and a free-text `source`. Before this gate the `source` was UNCHECKED — a rename orphaned
# the record silently, and a ratchet/floor rise republished a STALE hand-transcribed number.
# This verifier upgrades that trace from ASSERTED to MACHINE-VERIFIED by resolving a per-record
# `binding` object and checking the recorded `value` against the committed source:
#
#   json-pointer  {file, pointer}  — RFC-6901 pointer into a committed JSON artifact; the
#                                    recorded value MUST EQUAL the pointed scalar
#                                    (derivation-strength).
#   rust-anchor   {file, anchor}   — a test-fn / const NAME; the anchor MUST exist in the file
#                                    AND the value's literal MUST appear within the anchor's
#                                    window (honest limitation: LITERAL-ADJACENCY only — it can
#                                    in principle false-pass if the same literal sits nearby for
#                                    another reason; strictly weaker than json-pointer, strictly
#                                    stronger than unchecked free text).
#   doc-anchor    {file, quote}    — the exact `quote` MUST be present verbatim in the committed
#                                    doc (existence-strength only; does NOT parse the number).
#
# FAIL-CLOSED (the invariant): any environment=canonical or canonical-timing record WITHOUT a
# passing binding FAILS
# the paper build, UNLESS its key is listed in the shrink-only allowlist
# (scripts/paper-evidence-binding-allowlist.json). The verifier ALSO FAILS if the allowlist
# GROWS beyond its committed seed (the ratchet that drives migration — F3/sq-gum8.15 empties it)
# or lists a key that is NOT a canonical record / already has a passing binding (a stale entry).
#
# HONEST SCOPE (do NOT oversell): this gate is MECHANICAL — it checks value<->source EQUALITY /
# EXISTENCE only. A *semantic* overclaim (a true number framed misleadingly) is NOT caught here;
# that remains the human claims<->evidence review in skills/academic-paper/SKILL.md (sq-dxi3).
# rust-anchor is literal-adjacency strength ONLY. This gate does NOT touch the ZK/MPC posture
# (external cryptographer audit pending, sq-qhy4).
#
# USAGE:
#   python3 scripts/verify-paper-evidence.py                 # verify the real evidence file
#   python3 scripts/verify-paper-evidence.py --self-test     # run the fixture suite (CI gate)
#   python3 scripts/verify-paper-evidence.py --evidence P \
#       --supplement S --allowlist Q --root R                 # verify merged ledgers
#
# Stdlib-only, no third-party deps, no network — mirrors the other scripts/check-*.py gates.

from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
from typing import Any

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_EVIDENCE = os.path.join("site", "src", "data", "paper-evidence.json")
DEFAULT_TIMING_EVIDENCE = os.path.join(
    "site", "src", "data", "paper-evidence.canonical-timing.generated.json"
)
DEFAULT_ALLOWLIST = os.path.join("scripts", "paper-evidence-binding-allowlist.json")
CANONICAL_ENVIRONMENTS = frozenset({"canonical", "canonical-timing"})
TIMING_GENERATOR = "site/scripts/sync-canonical-timing.mjs"
TIMING_KINDS = frozenset({
    "canonical-timing", "canonical-timing-verdict", "canonical-timing-figure"
})
TIMING_PROVENANCE_FIELDS = frozenset({
    "study_id", "run_id", "collected_at_utc", "source_git_commit", "analysis_git_commit",
    "host_class", "host_label",
    "tenancy", "noise_limitation",
    "workload", "dataset", "query_scope", "protocol", "bootstrap_draws", "bootstrap_seed",
    "publisher_path", "publisher_sha256", "input_file_count",
    "raw_archive_kind", "raw_archive_location", "raw_archive_public_url",
    "raw_archive_sha256", "raw_archive_bytes",
    "raw_archive_build_verification",
    "raw_archive_member_verification_authority",
    "raw_archive_manifest_member", "raw_archive_manifest_sha256",
    "raw_archive_manifest_bytes", "raw_archive_manifest_entry_count",
    "raw_archive_regular_members", "raw_archive_all_members_rehashed",
    "raw_archive_exact_member_set", "raw_archive_sanitization_scan_passed",
    "raw_archive_deterministic_tar_headers", "raw_archive_zstd_version",
    "raw_archive_zstd_executable_sha256",
})

# How many lines after an anchor's declaration line count as the anchor "window" in which the
# value literal must appear. Kept small so the literal-adjacency claim stays honest: a const /
# test-fn declaration and the assertion that pins the number are within a handful of lines.
RUST_ANCHOR_WINDOW = 40

VALID_BINDING_KINDS = frozenset({"json-pointer", "rust-anchor", "doc-anchor"})


class VerifyError(Exception):
    """A record/allowlist problem that must fail the build (fail-closed)."""


# --------------------------------------------------------------------------------------------
# value <-> source-window matching for the rust-anchor tier (literal-adjacency, honest scope).
# --------------------------------------------------------------------------------------------
# A Rust numeric literal, possibly with underscore digit separators and an optional fraction /
# type suffix (e.g. 50, 1_229, 0.90, 0.5f64, 197usize). We extract these tokens from the anchor
# window and compare NUMERICALLY, so a range `0..50` yields the token "50" (not "0.50"'s
# fractional part), and `0.90` compares equal to a recorded 0.9. This is stronger and clearer
# than a raw substring/word-boundary regex, which mis-handled both `0..50` and `0.90`.
_NUM_TOKEN = re.compile(r"\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?")


def _rust_number_tokens(window: str) -> list[float]:
    """All numeric literals in a source window, parsed to float (underscores + suffix stripped)."""
    out: list[float] = []
    for m in _NUM_TOKEN.finditer(window):
        tok = m.group(0).replace("_", "")
        try:
            out.append(float(tok))
        except ValueError:
            continue
    return out


def value_describe(value: Any) -> str:
    """Human spelling of the value for an error message."""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def value_matches_window(value: Any, window: str) -> bool:
    """Does the recorded value appear in the anchor `window` (literal-adjacency, honest scope)?

    - bool  -> the Rust `true`/`false` keyword appears as a whole word.
    - int   -> a numeric token equal to the value appears (range-operator safe; `0..50` -> 50).
    - float -> a numeric token numerically equal appears (0.90 matches a recorded 0.9).
    - str   -> the exact string appears.
    Non-scalars are not rust-anchorable.
    """
    if isinstance(value, bool):
        kw = "true" if value else "false"
        return re.search(r"\b" + kw + r"\b", window) is not None
    if isinstance(value, (int, float)):
        target = float(value)
        for tok in _rust_number_tokens(window):
            if tok == target:
                return True
        return False
    if isinstance(value, str):
        return value in window
    return False


def value_rust_anchorable(value: Any) -> bool:
    return isinstance(value, (bool, int, float, str))


# --------------------------------------------------------------------------------------------
# json-pointer (RFC 6901)
# --------------------------------------------------------------------------------------------
def resolve_json_pointer(doc: Any, pointer: str) -> Any:
    """Resolve an RFC-6901 JSON pointer. Raises VerifyError if it does not resolve."""
    if pointer == "":
        return doc
    if not pointer.startswith("/"):
        raise VerifyError("json-pointer must start with '/' (RFC 6901): {}".format(pointer))
    cur = doc
    for raw in pointer.split("/")[1:]:
        token = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(cur, dict):
            if token not in cur:
                raise VerifyError("json-pointer key not found: {} (at token {})".format(pointer, token))
            cur = cur[token]
        elif isinstance(cur, list):
            if not re.fullmatch(r"\d+", token):
                raise VerifyError("json-pointer array index not an integer: {} (token {})".format(pointer, token))
            idx = int(token)
            if idx >= len(cur):
                raise VerifyError("json-pointer array index out of range: {} (token {})".format(pointer, token))
            cur = cur[idx]
        else:
            raise VerifyError("json-pointer descends into a scalar: {} (token {})".format(pointer, token))
    return cur


# --------------------------------------------------------------------------------------------
# the three binding verifiers — each returns None on success, raises VerifyError on failure.
# --------------------------------------------------------------------------------------------
def verify_json_pointer(key: str, value: Any, binding: dict, root: str) -> None:
    file_rel = binding.get("file")
    pointer = binding.get("pointer")
    if not isinstance(file_rel, str) or not isinstance(pointer, str):
        raise VerifyError("record '{}': json-pointer binding needs string 'file' and 'pointer'".format(key))
    path = os.path.join(root, file_rel)
    if not os.path.isfile(path):
        raise VerifyError("record '{}': json-pointer source missing/renamed: {}".format(key, file_rel))
    try:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except (OSError, json.JSONDecodeError) as e:
        raise VerifyError("record '{}': cannot read/parse json source {}: {}".format(key, file_rel, e)) from e
    pointed = resolve_json_pointer(doc, pointer)
    # Strict equality of the recorded value against the pointed scalar. bool is compared as bool
    # (Python's 1 == True is deliberately blocked: an int record must not silently satisfy a
    # boolean pointer, and vice versa).
    if isinstance(value, bool) != isinstance(pointed, bool):
        raise VerifyError(
            "record '{}': json-pointer type mismatch: value {} ({}) vs pointed {} ({}) at {}{}".format(
                key, value, type(value).__name__, pointed, type(pointed).__name__, file_rel, pointer
            )
        )
    if pointed != value:
        raise VerifyError(
            "record '{}': DRIFT — recorded value {} != pointed value {} at {}{}".format(
                key, value, pointed, file_rel, pointer
            )
        )


def verify_rust_anchor(key: str, value: Any, binding: dict, root: str) -> None:
    file_rel = binding.get("file")
    anchor = binding.get("anchor")
    if not isinstance(file_rel, str) or not isinstance(anchor, str):
        raise VerifyError("record '{}': rust-anchor binding needs string 'file' and 'anchor'".format(key))
    path = os.path.join(root, file_rel)
    if not os.path.isfile(path):
        raise VerifyError("record '{}': rust-anchor source missing/renamed: {}".format(key, file_rel))
    try:
        with open(path, encoding="utf-8") as fh:
            lines = fh.read().splitlines()
    except OSError as e:
        raise VerifyError("record '{}': cannot read rust source {}: {}".format(key, file_rel, e)) from e
    # The anchor must appear as a whole-word identifier somewhere in the file.
    anchor_re = re.compile(r"\b" + re.escape(anchor) + r"\b")
    anchor_lines = [i for i, ln in enumerate(lines) if anchor_re.search(ln)]
    if not anchor_lines:
        raise VerifyError(
            "record '{}': rust-anchor '{}' NOT FOUND in {} (renamed/deleted?)".format(key, anchor, file_rel)
        )
    if not value_rust_anchorable(value):
        raise VerifyError(
            "record '{}': value {} is not rust-anchorable (only scalar int/float/bool/str)".format(key, value)
        )
    # The value literal must appear within RUST_ANCHOR_WINDOW lines of SOME anchor occurrence.
    for a in anchor_lines:
        lo = a
        hi = min(len(lines), a + 1 + RUST_ANCHOR_WINDOW)
        window = "\n".join(lines[lo:hi])
        if value_matches_window(value, window):
            return
    raise VerifyError(
        "record '{}': rust-anchor DRIFT — value {} not within {} lines of anchor '{}' in {} "
        "(the anchor exists but the number moved; re-verify + update)".format(
            key, value_describe(value), RUST_ANCHOR_WINDOW, anchor, file_rel
        )
    )


def verify_doc_anchor(key: str, value: Any, binding: dict, root: str) -> None:
    file_rel = binding.get("file")
    quote = binding.get("quote")
    if not isinstance(file_rel, str) or not isinstance(quote, str) or quote == "":
        raise VerifyError("record '{}': doc-anchor binding needs string 'file' and non-empty 'quote'".format(key))
    path = os.path.join(root, file_rel)
    if not os.path.isfile(path):
        raise VerifyError("record '{}': doc-anchor source missing/renamed: {}".format(key, file_rel))
    try:
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
    except (OSError, UnicodeDecodeError) as e:
        raise VerifyError("record '{}': cannot read doc source {}: {}".format(key, file_rel, e)) from e
    if quote not in text:
        raise VerifyError(
            "record '{}': doc-anchor QUOTE not found verbatim in {} (edited/removed?): {!r}".format(
                key, file_rel, quote[:80]
            )
        )


BINDING_VERIFIERS = {
    "json-pointer": verify_json_pointer,
    "rust-anchor": verify_rust_anchor,
    "doc-anchor": verify_doc_anchor,
}


def verify_binding(key: str, value: Any, binding: dict, root: str) -> None:
    """Dispatch to the tier verifier. Raises VerifyError on any failure (fail-closed)."""
    kind = binding.get("kind")
    if kind not in VALID_BINDING_KINDS:
        raise VerifyError(
            "record '{}': binding.kind '{}' invalid — must be one of {}".format(
                key, kind, ", ".join(sorted(VALID_BINDING_KINDS))
            )
        )
    BINDING_VERIFIERS[kind](key, value, binding, root)


# --------------------------------------------------------------------------------------------
# top-level file verification
# --------------------------------------------------------------------------------------------
def load_json(path: str, what: str) -> Any:
    if not os.path.isfile(path):
        raise VerifyError("{} not found: {}".format(what, path))
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError) as e:
        raise VerifyError("{}: cannot read/parse {}: {}".format(what, path, e)) from e


def load_merged_records(evidence_paths: list[str]) -> dict[str, Any]:
    """Load ordered evidence ledgers, rejecting collisions instead of last-write-wins merge."""
    merged: dict[str, Any] = {}
    owners: dict[str, str] = {}
    for path in evidence_paths:
        data = load_json(path, "evidence file")
        records = data.get("records")
        if not isinstance(records, dict):
            raise VerifyError("evidence file {}: 'records' object missing".format(path))
        for key, record in records.items():
            if key in merged:
                raise VerifyError(
                    "duplicate evidence key '{}' in {} and {}".format(key, owners[key], path)
                )
            merged[key] = record
            owners[key] = path
    return merged


def timing_shape_problems(key: str, rec: dict) -> list[str]:
    """Structural requirements that make canonical timing provenance non-optional."""
    problems: list[str] = []
    kind = rec.get("kind")
    if kind not in TIMING_KINDS:
        problems.append(
            "canonical-timing record '{}': kind must be one of {}".format(
                key, ", ".join(sorted(TIMING_KINDS))
            )
        )
    if rec.get("_generated_by") != TIMING_GENERATOR:
        problems.append(
            "canonical-timing record '{}': must be generated by {}".format(key, TIMING_GENERATOR)
        )
    binding = rec.get("binding")
    if not isinstance(binding, dict) or binding.get("kind") != "json-pointer":
        problems.append(
            "canonical-timing record '{}': only a json-pointer binding is accepted".format(key)
        )
    provenance = rec.get("timing_provenance")
    if not isinstance(provenance, dict):
        problems.append("canonical-timing record '{}': timing_provenance is missing".format(key))
    else:
        missing = sorted(field for field in TIMING_PROVENANCE_FIELDS if provenance.get(field) is None)
        if missing:
            problems.append(
                "canonical-timing record '{}': timing_provenance lacks {}".format(
                    key, ", ".join(missing)
                )
            )
        for commit_field in ("source_git_commit", "analysis_git_commit"):
            commit = provenance.get(commit_field)
            if not isinstance(commit, str) or re.fullmatch(r"[0-9a-f]{40}", commit) is None:
                problems.append(
                    "canonical-timing record '{}': {} must be lowercase 40-hex".format(
                        key, commit_field
                    )
                )
        archive_kind = provenance.get("raw_archive_kind")
        archive_verification = provenance.get("raw_archive_build_verification")
        if archive_kind not in {"committed", "external"}:
            problems.append(
                "canonical-timing record '{}': raw_archive_kind must be committed or external".format(key)
            )
        elif archive_kind == "committed" and archive_verification != "local-rehash":
            problems.append(
                "canonical-timing record '{}': committed raw archive must be locally re-hashed".format(key)
            )
        elif archive_kind == "external" and archive_verification != "descriptor-only":
            problems.append(
                "canonical-timing record '{}': external raw archive must be labelled descriptor-only".format(key)
            )
        archive_sha = provenance.get("raw_archive_sha256")
        if not isinstance(archive_sha, str) or re.fullmatch(r"[0-9a-f]{64}", archive_sha) is None:
            problems.append(
                "canonical-timing record '{}': raw_archive_sha256 must be lowercase 64-hex".format(key)
            )
        if not isinstance(provenance.get("raw_archive_location"), str) \
                or not provenance.get("raw_archive_location"):
            problems.append(
                "canonical-timing record '{}': raw_archive_location must be non-empty".format(key)
            )
        if not isinstance(provenance.get("raw_archive_public_url"), str):
            problems.append(
                "canonical-timing record '{}': raw_archive_public_url must be a string (empty is allowed)".format(key)
            )
        if not isinstance(provenance.get("publisher_path"), str) \
                or not provenance.get("publisher_path"):
            problems.append(
                "canonical-timing record '{}': publisher_path must be non-empty".format(key)
            )
        publisher_sha = provenance.get("publisher_sha256")
        if not isinstance(publisher_sha, str) \
                or re.fullmatch(r"[0-9a-f]{64}", publisher_sha) is None:
            problems.append(
                "canonical-timing record '{}': publisher_sha256 must be lowercase 64-hex".format(key)
            )
        input_file_count = provenance.get("input_file_count")
        if isinstance(input_file_count, bool) or not isinstance(input_file_count, int) \
                or input_file_count < 1:
            problems.append(
                "canonical-timing record '{}': input_file_count must be a positive integer".format(key)
            )
        if provenance.get("raw_archive_member_verification_authority") != "publisher-recorded":
            problems.append(
                "canonical-timing record '{}': member verification must be labelled publisher-recorded".format(key)
            )
        manifest_member = provenance.get("raw_archive_manifest_member")
        manifest_segments = manifest_member.split("/") if isinstance(manifest_member, str) else []
        if not isinstance(manifest_member, str) or not manifest_member \
                or manifest_member.startswith("/") or "\\" in manifest_member \
                or re.search(r"[\x00-\x1f\x7f]", manifest_member) is not None \
                or any(segment in {"", ".", ".."} for segment in manifest_segments):
            problems.append(
                "canonical-timing record '{}': raw archive manifest member must be a safe relative path".format(key)
            )
        manifest_sha = provenance.get("raw_archive_manifest_sha256")
        if not isinstance(manifest_sha, str) \
                or re.fullmatch(r"[0-9a-f]{64}", manifest_sha) is None:
            problems.append(
                "canonical-timing record '{}': manifest SHA-256 must be lowercase 64-hex".format(key)
            )
        manifest_bytes = provenance.get("raw_archive_manifest_bytes")
        manifest_entries = provenance.get("raw_archive_manifest_entry_count")
        regular_members = provenance.get("raw_archive_regular_members")
        for field, value in (
            ("raw_archive_manifest_bytes", manifest_bytes),
            ("raw_archive_manifest_entry_count", manifest_entries),
            ("raw_archive_regular_members", regular_members),
        ):
            if isinstance(value, bool) or not isinstance(value, int) or value < 1:
                problems.append(
                    "canonical-timing record '{}': {} must be a positive integer".format(key, field)
                )
        if isinstance(manifest_entries, int) and not isinstance(manifest_entries, bool) \
                and isinstance(regular_members, int) and not isinstance(regular_members, bool) \
                and regular_members != manifest_entries + 1:
            problems.append(
                "canonical-timing record '{}': regular member count must equal manifest entries + 1".format(key)
            )
        for field in (
            "raw_archive_all_members_rehashed",
            "raw_archive_exact_member_set",
            "raw_archive_sanitization_scan_passed",
            "raw_archive_deterministic_tar_headers",
        ):
            if provenance.get(field) is not True:
                problems.append(
                    "canonical-timing record '{}': {} must be true".format(key, field)
                )
        if not isinstance(provenance.get("raw_archive_zstd_version"), str) \
                or not provenance.get("raw_archive_zstd_version"):
            problems.append(
                "canonical-timing record '{}': zstd version must be non-empty".format(key)
            )
        zstd_sha = provenance.get("raw_archive_zstd_executable_sha256")
        if not isinstance(zstd_sha, str) \
                or re.fullmatch(r"[0-9a-f]{64}", zstd_sha) is None:
            problems.append(
                "canonical-timing record '{}': zstd executable SHA-256 must be lowercase 64-hex".format(key)
            )
        if provenance.get("host_class") == "controlled-single-process-ec2" \
                and provenance.get("tenancy") != "shared":
            problems.append(
                "canonical-timing record '{}': controlled EC2 provenance must declare shared tenancy".format(key)
            )
    papers = rec.get("papers")
    if not isinstance(papers, list) or not papers or not all(isinstance(p, str) and p for p in papers):
        problems.append("canonical-timing record '{}': papers must be a non-empty slug list".format(key))

    value = rec.get("value")
    if kind == "canonical-timing":
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
            problems.append("canonical-timing value '{}': value must be finite numeric".format(key))
    elif kind == "canonical-timing-verdict":
        if not isinstance(value, bool) or rec.get("unit") != "boolean" \
                or rec.get("hypothesis") not in {"H1", "H2"}:
            problems.append(
                "canonical timing verdict '{}': value/unit must be boolean and hypothesis H1/H2".format(key)
            )
    elif kind == "canonical-timing-figure":
        figure = rec.get("figure")
        if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
            problems.append("canonical timing figure '{}': value must be its SHA-256".format(key))
        if rec.get("unit") != "sha256":
            problems.append("canonical timing figure '{}': unit must be sha256".format(key))
        if not isinstance(figure, dict):
            problems.append("canonical timing figure '{}': figure descriptor is missing".format(key))
        else:
            path = figure.get("typst_path")
            if not isinstance(path, str) or not path.startswith("/papers/figures/canonical-timing/") \
                    or not path.endswith(".svg"):
                problems.append(
                    "canonical timing figure '{}': typst_path must use the reserved SVG tree".format(key)
                )
            if figure.get("media_type") != "image/svg+xml":
                problems.append("canonical timing figure '{}': media_type must be image/svg+xml".format(key))
            if not isinstance(figure.get("alt"), str) or not figure.get("alt"):
                problems.append("canonical timing figure '{}': non-empty alt text is required".format(key))
            if not isinstance(figure.get("output_name"), str) or not figure.get("output_name"):
                problems.append("canonical timing figure '{}': analyzer output_name is required".format(key))
            if not isinstance(figure.get("bytes"), int) or figure.get("bytes", 0) < 1:
                problems.append("canonical timing figure '{}': positive byte length is required".format(key))
    if kind in {"canonical-timing", "canonical-timing-verdict"}:
        result_bindings = rec.get("result_bindings")
        if not isinstance(result_bindings, list) or not result_bindings:
            problems.append(
                "canonical timing value '{}': non-empty result_bindings provenance is required".format(key)
            )
        else:
            seen_bindings: set[tuple[str, str, str]] = set()
            for index, result_binding in enumerate(result_bindings):
                label = "canonical timing value '{}': result_bindings[{}]".format(key, index)
                if not isinstance(result_binding, dict):
                    problems.append("{} must be an object".format(label))
                    continue
                if set(result_binding) != {"artifact", "sha256", "locator"}:
                    problems.append("{} must contain exactly artifact, sha256, locator".format(label))
                    continue
                artifact = result_binding.get("artifact")
                digest = result_binding.get("sha256")
                locator = result_binding.get("locator")
                if not isinstance(artifact, str) or not artifact:
                    problems.append("{}.artifact must be non-empty".format(label))
                if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
                    problems.append("{}.sha256 must be lowercase 64-hex".format(label))
                if not isinstance(locator, str) or not locator:
                    problems.append("{}.locator must be non-empty".format(label))
                if isinstance(artifact, str) and isinstance(digest, str) and isinstance(locator, str):
                    identity = (artifact, digest, locator)
                    if identity in seen_bindings:
                        problems.append("{} duplicates an earlier source-artifact binding".format(label))
                    seen_bindings.add(identity)
    elif kind == "canonical-timing-figure" and "result_bindings" in rec:
        problems.append(
            "canonical timing figure '{}': scalar result_bindings do not belong on figure records".format(key)
        )
    return problems


def verify_evidence(
    evidence_path: str,
    allowlist_path: str,
    root: str,
    supplements: list[str] | None = None,
) -> list[str]:
    """Verify merged evidence ledgers against one allowlist, resolving sources under `root`.

    Returns a list of human-readable problem strings; an empty list means PASS. Raises
    VerifyError only for a structural problem that prevents inspection at all (unparseable
    file), which the caller treats as a hard failure too.
    """
    problems: list[str] = []
    evidence_paths = [evidence_path, *(supplements or [])]
    records = load_merged_records(evidence_paths)

    allow_raw = load_json(allowlist_path, "allowlist")
    allow_seed = allow_raw.get("seed_count")
    allow_keys_list = allow_raw.get("keys")
    if not isinstance(allow_keys_list, list):
        raise VerifyError("allowlist {}: 'keys' array missing".format(allowlist_path))
    allow_keys = set(allow_keys_list)
    if len(allow_keys) != len(allow_keys_list):
        problems.append("allowlist has DUPLICATE keys (each key must appear once)")
    # SHRINK-ONLY ratchet: the live key count must never exceed the committed seed_count.
    if isinstance(allow_seed, int) and len(allow_keys) > allow_seed:
        problems.append(
            "allowlist GREW: {} keys > committed seed_count {} — the allowlist is SHRINK-ONLY "
            "(migrate the record to a binding instead of adding it here; F3/sq-gum8.15)".format(
                len(allow_keys), allow_seed
            )
        )

    canonical_keys = {k for k, r in records.items() if isinstance(r, dict) and r.get("environment") == "canonical"}

    # An allowlist key must name a real canonical record; a stale key (renamed/removed record,
    # or a record that now HAS a passing binding) must be pruned, not left to rot.
    for ak in sorted(allow_keys):
        if ak not in records:
            problems.append("allowlist key '{}' is not a record in the evidence file (stale; prune it)".format(ak))
        elif ak not in canonical_keys:
            problems.append(
                "allowlist key '{}' is not environment=canonical (only canonical records need "
                "an allowlist entry; prune it)".format(ak)
            )

    for key, rec in records.items():
        if not isinstance(rec, dict):
            problems.append("record '{}' is not an object".format(key))
            continue
        environment = rec.get("environment")
        is_canonical = environment in CANONICAL_ENVIRONMENTS
        is_timing = environment == "canonical-timing"
        binding = rec.get("binding")
        value = rec.get("value")

        if is_timing:
            problems.extend(timing_shape_problems(key, rec))
            if key in allow_keys:
                problems.append(
                    "canonical-timing record '{}' may never use the transitional allowlist".format(key)
                )

        if binding is None:
            # Fail-closed applies ONLY to canonical records (indicative ones are never
            # headline-cited, so an unbound indicative record is fine).
            if is_timing or (is_canonical and key not in allow_keys):
                problems.append(
                    "canonical record '{}' has NO binding{} — fail-closed. "
                    "Add a machine-verified binding (json-pointer/rust-anchor/doc-anchor) or, only "
                    "for deterministic canonical records, use the shrink-only allowlist.".format(
                        key, " (canonical timing cannot be allowlisted)" if is_timing else " and is not allowlisted"
                    )
                )
            continue

        if not isinstance(binding, dict):
            problems.append("record '{}': binding must be an object".format(key))
            continue
        # A record WITH a binding is verified regardless of environment — if you bother to bind
        # an indicative record too, the binding must still resolve + match (defence-in-depth).
        try:
            verify_binding(key, value, binding, root)
        except VerifyError as e:
            problems.append(str(e))
        else:
            if key in allow_keys:
                problems.append(
                    "record '{}' has a PASSING binding but is still on the allowlist — prune it "
                    "(the allowlist is only for un-migrated records)".format(key)
                )

    return problems


def run_verify(
    evidence_path: str,
    allowlist_path: str,
    root: str,
    supplements: list[str] | None = None,
    quiet: bool = False,
    silent: bool = False,
) -> int:
    """Verify one (evidence, allowlist) pair. quiet=True suppresses the PASS summary; silent=True
    additionally suppresses the FAILURE detail (used by the self-test's expected-FAIL cases,
    which assert the exit code, not the message)."""
    try:
        problems = verify_evidence(evidence_path, allowlist_path, root, supplements=supplements)
    except VerifyError as e:
        if not silent:
            print("[paper-evidence] VERIFY FAILED (structural): {}".format(e), file=sys.stderr)
        return 1
    if problems:
        if not silent:
            print("\n[paper-evidence] EVIDENCE-BINDING VERIFY FAILED:", file=sys.stderr)
            for p in problems:
                print("  - {}".format(p), file=sys.stderr)
            print(
                "\n  A canonical paper-evidence value no longer matches its committed source, or a\n"
                "  bound source was renamed/removed, or the shrink-only allowlist grew. Fix the value\n"
                "  at its source-of-truth (do not hand-edit the paper number), re-point the binding,\n"
                "  or shrink the allowlist. This gate is MECHANICAL (value<->source match only);\n"
                "  semantic framing stays the human review in skills/academic-paper/SKILL.md.\n",
                file=sys.stderr,
            )
        return 1
    if not quiet:
        recs = load_merged_records([evidence_path, *(supplements or [])])
        n_canon = sum(1 for r in recs.values() if isinstance(r, dict) and r.get("environment") == "canonical")
        n_timing = sum(
            1 for r in recs.values()
            if isinstance(r, dict) and r.get("environment") == "canonical-timing"
        )
        n_bound = sum(
            1
            for r in recs.values()
            if isinstance(r, dict) and r.get("environment") == "canonical" and isinstance(r.get("binding"), dict)
        )
        allow = load_json(allowlist_path, "allowlist")
        n_allow = len(set(allow.get("keys", [])))
        print(
            "[paper-evidence] verify passed: {} deterministic canonical records — {} "
            "machine-bound, {} on the shrink-only allowlist; {} canonical-timing records, "
            "all JSON-bound and sync-generated.".format(n_canon, n_bound, n_allow, n_timing)
        )
    return 0


# --------------------------------------------------------------------------------------------
# --self-test: drive the fixture suite (non-vacuous: drift + missing-anchor must FAIL).
# --------------------------------------------------------------------------------------------
def run_self_test() -> int:
    fx = os.path.join(REPO_ROOT, "scripts", "tests", "paper-evidence-fixtures")
    if not os.path.isdir(fx):
        print("[self-test] fixtures dir missing: {}".format(fx), file=sys.stderr)
        return 1
    src_root = os.path.join(fx, "sources_root")

    # (label, evidence, allowlist, root, expect_pass)
    cases = [
        ("pass/all-tiers-pass", "pass/evidence.json", "pass/allowlist.json", src_root, True),
        ("drift/json-pointer-value-drift", "drift/evidence.json", "drift/allowlist.json", src_root, False),
        ("missing-anchor/renamed-source", "missing-anchor/evidence.json", "missing-anchor/allowlist.json", src_root, False),
        ("allowlist-grew", "allowlist-grew/evidence.json", "allowlist-grew/allowlist.json", src_root, False),
        ("stale-allowlist-entry", "stale-allowlist/evidence.json", "stale-allowlist/allowlist.json", src_root, False),
        ("rust-anchor-literal-drift", "rust-drift/evidence.json", "rust-drift/allowlist.json", src_root, False),
        ("unbound-not-allowlisted", "unbound/evidence.json", "unbound/allowlist.json", src_root, False),
    ]

    failures = 0
    for label, ev, al, root, expect_pass in cases:
        ev_p = os.path.join(fx, ev)
        al_p = os.path.join(fx, al)
        # Expected-FAIL cases run silent (we assert the exit code, not the message); an
        # UNEXPECTED result re-runs verbose so the failure detail is visible.
        rc = run_verify(ev_p, al_p, root, quiet=True, silent=not expect_pass)
        got_pass = rc == 0
        ok = got_pass == expect_pass
        status = "OK " if ok else "FAIL"
        print(
            "[self-test] {} {}: expected {}, got {}".format(
                status, label, "PASS" if expect_pass else "FAIL", "PASS" if got_pass else "FAIL"
            )
        )
        if not ok:
            failures += 1
            # surface WHY it misbehaved (a case expected to pass that failed, or vice versa).
            run_verify(ev_p, al_p, root, quiet=True, silent=False)

    # Also verify the REAL committed evidence file against the REAL allowlist (must PASS).
    real_ev = os.path.join(REPO_ROOT, DEFAULT_EVIDENCE)
    real_al = os.path.join(REPO_ROOT, DEFAULT_ALLOWLIST)
    real_timing = os.path.join(REPO_ROOT, DEFAULT_TIMING_EVIDENCE)
    real_supplements = [real_timing] if os.path.isfile(real_timing) else []
    rc_real = run_verify(real_ev, real_al, REPO_ROOT, supplements=real_supplements, quiet=True)
    if rc_real == 0:
        print("[self-test] OK  real-evidence-file: the committed paper-evidence.json passes.")
    else:
        print("[self-test] FAIL real-evidence-file: the committed paper-evidence.json FAILED verify.")
        failures += 1

    if failures:
        print("\n[self-test] {} case(s) FAILED — the verifier is not sound.".format(failures), file=sys.stderr)
        return 1
    print("\n[self-test] all cases behaved as expected (drift + missing-anchor fixtures FAIL; real evidence PASSES).")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Fail-closed evidence-binding verifier for the paper factory.")
    ap.add_argument("--self-test", action="store_true", help="run the fixture suite (CI gate); non-vacuous.")
    ap.add_argument("--evidence", default=None, help="path to the evidence JSON (default: the committed file).")
    ap.add_argument(
        "--supplement",
        action="append",
        default=None,
        help="additional evidence ledger to merge (repeatable; duplicate keys fail closed)",
    )
    ap.add_argument("--allowlist", default=None, help="path to the binding allowlist JSON (default: committed).")
    ap.add_argument("--root", default=None, help="root under which binding source paths resolve (default: repo root).")
    args = ap.parse_args()

    if args.self_test:
        return run_self_test()

    root = args.root if args.root is not None else REPO_ROOT
    evidence = args.evidence if args.evidence is not None else os.path.join(REPO_ROOT, DEFAULT_EVIDENCE)
    allowlist = args.allowlist if args.allowlist is not None else os.path.join(REPO_ROOT, DEFAULT_ALLOWLIST)
    if args.supplement is not None:
        supplements = args.supplement
    else:
        default_timing = os.path.join(REPO_ROOT, DEFAULT_TIMING_EVIDENCE)
        supplements = [default_timing] if os.path.isfile(default_timing) else []
    return run_verify(evidence, allowlist, root, supplements=supplements)


if __name__ == "__main__":
    sys.exit(main())
