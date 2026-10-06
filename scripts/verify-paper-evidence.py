#!/usr/bin/env python3
# sq-gum8.13 (epic sq-gum8, paper factory F1) — the fail-closed
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
# FAIL-CLOSED (the invariant): any environment=canonical record WITHOUT a passing binding FAILS
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
#       --allowlist Q --root R                                # verify an explicit triple
#
# Stdlib-only, no third-party deps, no network — mirrors the other scripts/check-*.py gates.

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from typing import Any

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_EVIDENCE = os.path.join("site", "src", "data", "paper-evidence.json")
DEFAULT_ALLOWLIST = os.path.join("scripts", "paper-evidence-binding-allowlist.json")

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


def verify_evidence(evidence_path: str, allowlist_path: str, root: str) -> list[str]:
    """Verify one evidence file against one allowlist, resolving sources under `root`.

    Returns a list of human-readable problem strings; an empty list means PASS. Raises
    VerifyError only for a structural problem that prevents inspection at all (unparseable
    file), which the caller treats as a hard failure too.
    """
    problems: list[str] = []
    data = load_json(evidence_path, "evidence file")
    records = data.get("records")
    if not isinstance(records, dict):
        raise VerifyError("evidence file {}: 'records' object missing".format(evidence_path))

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
        is_canonical = rec.get("environment") == "canonical"
        binding = rec.get("binding")
        value = rec.get("value")

        if binding is None:
            # Fail-closed applies ONLY to canonical records (indicative ones are never
            # headline-cited, so an unbound indicative record is fine).
            if is_canonical and key not in allow_keys:
                problems.append(
                    "canonical record '{}' has NO binding and is NOT allowlisted — fail-closed. "
                    "Add a machine-verified binding (json-pointer/rust-anchor/doc-anchor) or, only "
                    "transitionally, add it to the shrink-only allowlist.".format(key)
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


def run_verify(evidence_path: str, allowlist_path: str, root: str, quiet: bool = False, silent: bool = False) -> int:
    """Verify one (evidence, allowlist) pair. quiet=True suppresses the PASS summary; silent=True
    additionally suppresses the FAILURE detail (used by the self-test's expected-FAIL cases,
    which assert the exit code, not the message)."""
    try:
        problems = verify_evidence(evidence_path, allowlist_path, root)
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
        data = load_json(evidence_path, "evidence file")
        recs = data.get("records", {})
        n_canon = sum(1 for r in recs.values() if isinstance(r, dict) and r.get("environment") == "canonical")
        n_bound = sum(
            1
            for r in recs.values()
            if isinstance(r, dict) and r.get("environment") == "canonical" and isinstance(r.get("binding"), dict)
        )
        allow = load_json(allowlist_path, "allowlist")
        n_allow = len(set(allow.get("keys", [])))
        print(
            "[paper-evidence] verify passed: {} canonical records — {} machine-bound, "
            "{} on the shrink-only allowlist.".format(n_canon, n_bound, n_allow)
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
    rc_real = run_verify(real_ev, real_al, REPO_ROOT, quiet=True)
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
    ap.add_argument("--allowlist", default=None, help="path to the binding allowlist JSON (default: committed).")
    ap.add_argument("--root", default=None, help="root under which binding source paths resolve (default: repo root).")
    args = ap.parse_args()

    if args.self_test:
        return run_self_test()

    root = args.root if args.root is not None else REPO_ROOT
    evidence = args.evidence if args.evidence is not None else os.path.join(REPO_ROOT, DEFAULT_EVIDENCE)
    allowlist = args.allowlist if args.allowlist is not None else os.path.join(REPO_ROOT, DEFAULT_ALLOWLIST)
    return run_verify(evidence, allowlist, root)


if __name__ == "__main__":
    sys.exit(main())
