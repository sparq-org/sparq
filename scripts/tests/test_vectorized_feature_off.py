#!/usr/bin/env python3
"""
sq-pntvh.8 / sq-v3nel: standalone test suite for
check-vectorized-feature-off.py.

Covers leg1 (feature-resolution guard) and the RE-DESIGNED leg2 dynamic byte-identity
check (base-tree vs head-tree feature-OFF wasm, declaration-token gated). No external
frameworks — stdlib only.

Usage:
    python3 scripts/tests/test_vectorized_feature_off.py
"""

from __future__ import annotations

import json
import os
import sys
import tempfile

# Locate the script under test relative to this test file.
_SCRIPT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_SCRIPT_PATH = os.path.join(_SCRIPT_DIR, "check-vectorized-feature-off.py")

# Import the module by file path (name has hyphens, so importlib.util is required).
import importlib.util as _ilu  # noqa: E402

_spec = _ilu.spec_from_file_location("check_vectorized_feature_off", _SCRIPT_PATH)
_check = _ilu.module_from_spec(_spec)  # type: ignore[arg-type]
_spec.loader.exec_module(_check)  # type: ignore[union-attr]


def _leg1_on_dict(meta: dict) -> int:
    """Run check_leg1 on an in-memory dict via a temp file."""
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as tmp:
        json.dump(meta, tmp)
        tmp_path = tmp.name
    try:
        return _check.check_leg1(tmp_path)
    finally:
        os.unlink(tmp_path)


def _leg2_dynamic(base_bytes: bytes, head_bytes: bytes,
                  base_decl: dict | None, head_decl: dict | None,
                  base_decl_names: list[str] | None = None,
                  head_decl_names: list[str] | None = None) -> int:
    """Run check_leg2_dynamic on in-memory wasm bytes + declaration dicts via temp files.

    A None declaration writes NO file (missing path) to exercise the fail-safe default
    (an absent declaration reads change_token as 0).

    base_decl_names / head_decl_names exercise MECHANISM V2 (sq-v3nel-v2): each is a list
    of file names materialised in a temp declarations directory (e.g. ["1720.json",
    "README.md"]). None => no directory is passed for that side (V2 disabled).
    """
    paths: list[str] = []
    dirs: list[str] = []

    def _write_bytes(data: bytes) -> str:
        with tempfile.NamedTemporaryFile(mode="wb", suffix=".wasm", delete=False) as tmp:
            tmp.write(data)
            paths.append(tmp.name)
            return tmp.name

    def _write_decl(data: dict | None) -> str:
        if data is None:
            # Return a path that does not exist -> _read_change_token defaults to 0.
            return os.path.join(tempfile.gettempdir(), "sq-v3nel-nonexistent-decl.json")
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as tmp:
            json.dump(data, tmp)
            paths.append(tmp.name)
            return tmp.name

    def _mkdir(names: list[str] | None) -> str | None:
        if names is None:
            return None
        d = tempfile.mkdtemp()
        dirs.append(d)
        for n in names:
            with open(os.path.join(d, n), "w") as fh:
                fh.write("{}")
        return d

    base_wasm = _write_bytes(base_bytes)
    head_wasm = _write_bytes(head_bytes)
    base_decl_p = _write_decl(base_decl)
    head_decl_p = _write_decl(head_decl)
    base_dir = _mkdir(base_decl_names)
    head_dir = _mkdir(head_decl_names)
    try:
        return _check.check_leg2_dynamic(base_wasm, head_wasm, base_decl_p, head_decl_p,
                                         base_dir, head_dir)
    finally:
        for p in paths:
            try:
                os.unlink(p)
            except OSError:
                pass
        for d in dirs:
            for n in os.listdir(d):
                os.unlink(os.path.join(d, n))
            os.rmdir(d)


# ---------------------------------------------------------------------------
# Leg 1 tests (unchanged behaviour)
# ---------------------------------------------------------------------------

def test_leg1_guard_rejects_vectorized_in_resolve() -> bool:
    bad_meta = {
        "packages": [
            {"name": "sparq-engine", "features": {"default": [], "vectorized": []}}
        ],
        "resolve": {
            "nodes": [
                {
                    "id": "sparq-engine 0.1.0 (path+file:///repo/crates/sparq-engine)",
                    "features": ["vectorized"],
                }
            ]
        },
    }
    rc = _leg1_on_dict(bad_meta)
    if rc != 0:
        print("  PASS — leg1 guard correctly rejected 'vectorized' in sparq-engine resolved node")
        return True
    print("  FAIL — leg1 guard did NOT reject 'vectorized'; the guard is broken")
    return False


def test_leg1_guard_rejects_vectorized_in_default_features() -> bool:
    bad_meta = {
        "packages": [
            {"name": "sparq-engine", "features": {"default": ["vectorized"], "vectorized": []}}
        ],
        "resolve": {
            "nodes": [
                {
                    "id": "sparq-engine 0.1.0 (path+file:///repo/crates/sparq-engine)",
                    "features": [],
                }
            ]
        },
    }
    rc = _leg1_on_dict(bad_meta)
    if rc != 0:
        print("  PASS — leg1 guard correctly rejected 'vectorized' in sparq-engine default features")
        return True
    print("  FAIL — leg1 guard did NOT reject 'vectorized' in default features; the guard is broken")
    return False


def test_leg1_accepts_clean_metadata() -> bool:
    clean_meta = {
        "packages": [
            {"name": "sparq-engine", "features": {"default": [], "vectorized": []}}
        ],
        "resolve": {
            "nodes": [
                {
                    "id": "sparq-engine 0.1.0 (path+file:///repo/crates/sparq-engine)",
                    "features": [],
                }
            ]
        },
    }
    rc = _leg1_on_dict(clean_meta)
    if rc == 0:
        print("  PASS — leg1 correctly accepted clean metadata (vectorized absent from defaults)")
        return True
    print("  FAIL — leg1 incorrectly rejected clean metadata; false positive")
    return False


# ---------------------------------------------------------------------------
# Leg 2 (dynamic byte-identity) tests — sq-v3nel
# ---------------------------------------------------------------------------

_BASE = b"\x00asm\x01\x00\x00\x00feature-off-base-bundle"
_HEAD_DIFF = _BASE + b"X"          # different length + content
_HEAD_SAME_LEN = b"\x00asm\x01\x00\x00\x00feature-off-Xead-bundle"  # same length, diff content


def test_leg2_identical_bytes_pass() -> bool:
    """Byte-identical base/head builds => PASS regardless of declaration token."""
    rc = _leg2_dynamic(_BASE, _BASE, {"change_token": 0}, {"change_token": 0})
    if rc == 0:
        print("  PASS — identical feature-OFF bytes accepted (deterministic, no static pin)")
        return True
    print("  FAIL — identical bytes were rejected; false positive")
    return False


def test_leg2_undeclared_change_rejected() -> bool:
    """Bytes differ + token un-bumped => FAIL (the core tripwire)."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 3}, {"change_token": 3})
    if rc != 0:
        print("  PASS — undeclared feature-OFF byte change rejected (token unchanged)")
        return True
    print("  FAIL — undeclared change was NOT rejected; the gate is disabled")
    return False


def test_leg2_declared_change_accepted() -> bool:
    """Bytes differ + token bumped => PASS (declared intentional change)."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 3}, {"change_token": 4})
    if rc == 0:
        print("  PASS — declared feature-OFF byte change accepted (token 3 -> 4)")
        return True
    print("  FAIL — declared change was rejected; false positive")
    return False


def test_leg2_same_length_content_change_caught() -> bool:
    """Same length but different content, undeclared => FAIL (byte-for-byte, not size)."""
    assert len(_HEAD_SAME_LEN) == len(_BASE), "fixture must be equal length"
    rc = _leg2_dynamic(_BASE, _HEAD_SAME_LEN, {"change_token": 0}, {"change_token": 0})
    if rc != 0:
        print("  PASS — same-length content change caught (comparison is byte-for-byte)")
        return True
    print("  FAIL — same-length content change slipped through; the comparison is size-only")
    return False


def test_leg2_missing_base_decl_defaults_zero() -> bool:
    """Absent base declaration file reads token 0; a head bump to 1 => declared => PASS.

    Mirrors THIS PR's own situation: base main has no declaration file yet.
    """
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, None, {"change_token": 1})
    if rc == 0:
        print("  PASS — missing base decl defaults to 0; head token 1 counts as declared")
        return True
    print("  FAIL — missing base decl mishandled; false negative")
    return False


def test_leg2_both_decls_missing_undeclared_rejected() -> bool:
    """Both declaration files absent (both read 0) + bytes differ => FAIL (fail-safe)."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, None, None)
    if rc != 0:
        print("  PASS — both decls absent => tokens 0==0 => undeclared change rejected")
        return True
    print("  FAIL — absent decls silently passed a byte change; fail-safe broken")
    return False


# ---------------------------------------------------------------------------
# Leg 2 MECHANISM V2 (per-PR declaration files) tests — sq-v3nel-v2
# ---------------------------------------------------------------------------

def test_leg2_v2_new_file_accepted() -> bool:
    """Bytes differ + head ADDS a per-PR file not in base => PASS (V2 declaration)."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 0}, {"change_token": 0},
                       base_decl_names=["README.md"],
                       head_decl_names=["README.md", "1720.json"])
    if rc == 0:
        print("  PASS — new per-PR file (1720.json) accepted as a V2 declaration")
        return True
    print("  FAIL — V2 file declaration rejected; false positive")
    return False


def test_leg2_v2_no_new_file_rejected() -> bool:
    """Bytes differ + NO new file (README only, both sides) + token equal => FAIL."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 0}, {"change_token": 0},
                       base_decl_names=["README.md"],
                       head_decl_names=["README.md"])
    if rc != 0:
        print("  PASS — no new per-PR file => undeclared change rejected (fail-closed)")
        return True
    print("  FAIL — undeclared change slipped through V2; the gate is disabled")
    return False


def test_leg2_v2_readme_not_a_declaration() -> bool:
    """A NEW README.md (not <digits>.json|md) is NOT a declaration => FAIL when undeclared."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 0}, {"change_token": 0},
                       base_decl_names=[],
                       head_decl_names=["README.md", ".gitkeep"])
    if rc != 0:
        print("  PASS — README.md/.gitkeep are not declarations; change stays rejected")
        return True
    print("  FAIL — a non-PR-numbered file was mistaken for a declaration")
    return False


def test_leg2_v2_md_declaration_accepted() -> bool:
    """A <digits>.md declaration file is also accepted (json or md permitted)."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 0}, {"change_token": 0},
                       base_decl_names=[],
                       head_decl_names=["1718.md"])
    if rc == 0:
        print("  PASS — <PR-number>.md accepted as a V2 declaration")
        return True
    print("  FAIL — .md declaration rejected; false positive")
    return False


def test_leg2_legacy_scalar_still_accepted_in_transition() -> bool:
    """Transition window: scalar token inequality still declares even with V2 dirs present.

    Mirrors a pre-V2 in-flight branch that bumped the scalar token; head added no new file.
    """
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 1726}, {"change_token": 1720},
                       base_decl_names=["README.md"],
                       head_decl_names=["README.md"])
    if rc == 0:
        print("  PASS — legacy scalar inequality still accepted during transition window")
        return True
    print("  FAIL — transition window broken; a pre-V2 declared branch would falsely fail")
    return False


def test_leg2_no_dirs_falls_back_to_scalar() -> bool:
    """No declarations dirs passed at all (None) => V2 disabled, scalar path governs."""
    rc = _leg2_dynamic(_BASE, _HEAD_DIFF, {"change_token": 0}, {"change_token": 0})
    if rc != 0:
        print("  PASS — no dirs + equal scalar token => undeclared change rejected")
        return True
    print("  FAIL — missing dirs mishandled")
    return False


def test_leg3_ungated_child_rejected() -> bool:
    """An ungated vectorized reference in an exec/ child module must fail leg 3."""
    if _check._leg3_on_tree(_check._LEG3_TREE_UNGATED_CHILD) != 0:
        print("  PASS — ungated reference in exec/child.rs rejected")
        return True
    print("  FAIL — exec child modules are not audited")
    return False


def test_leg3_unscanned_child_rejected() -> bool:
    """exec.rs declaring a child module outside the scanned set must fail leg 3."""
    if _check._leg3_on_tree(_check._LEG3_TREE_UNSCANNED_CHILD) != 0:
        print("  PASS — #[path] child outside the scanned set rejected")
        return True
    print("  FAIL — unscanned child module accepted")
    return False


def test_leg3_gated_tree_accepted() -> bool:
    """Item-level and module-level `vectorized` gates both satisfy leg 3."""
    if _check._leg3_on_tree(_check._LEG3_TREE_OK) == 0:
        print("  PASS — gated references accepted")
        return True
    print("  FAIL — gated references rejected")
    return False


def _all_rc(trees, want_fail: bool) -> list[int]:
    """Indices of trees whose leg-3 result is NOT the wanted one."""
    return [i for i, t in enumerate(trees)
            if (_check._leg3_on_tree(t) != 0) != want_fail]


def test_leg3_same_line_path_attr_resolved() -> bool:
    """`#[path]` on the same line as `mod` (several spellings) is resolved, not skipped."""
    bad = _all_rc(_check._LEG3_TREES_SAME_LINE_ATTR, want_fail=True)
    if not bad:
        print("  PASS — every same-line #[path] spelling rejected as an unscanned child")
        return True
    print(f"  FAIL — same-line #[path] spellings {bad} skipped")
    return False


def test_leg3_non_gating_cfg_rejected() -> bool:
    """not(...), any(...) and other-feature cfgs never count as a vectorized gate."""
    bad = _all_rc(_check._LEG3_TREES_NON_GATING_CFG, want_fail=True)
    if not bad:
        print("  PASS — non-gating cfgs do not gate the module")
        return True
    print(f"  FAIL — non-gating cfgs {bad} accepted as gates")
    return False


def test_leg3_unresolvable_mod_rejected() -> bool:
    """A `mod x;` whose file does not exist fails closed."""
    if _check._leg3_on_tree(_check._LEG3_TREE_UNRESOLVABLE) != 0:
        print("  PASS — unresolvable child module rejected")
        return True
    print("  FAIL — unresolvable child module accepted")
    return False


def test_leg3_gating_cfg_spellings_accepted() -> bool:
    """cfgs that require vectorized are accepted with any whitespace / nesting."""
    bad = _all_rc(_check._LEG3_TREES_GATING_CFG, want_fail=False)
    if not bad:
        print("  PASS — gating cfg spellings accepted")
        return True
    print(f"  FAIL — gating cfg spellings {bad} rejected")
    return False


def test_leg3_cfg_parser() -> bool:
    """Direct checks of the cfg predicate parser."""
    req = _check._cfg_requires_vectorized
    parse = _check._parse_cfg
    cases = [
        ('feature = "vectorized"', True),
        ('all( test ,feature="vectorized" )', True),
        ('any(feature = "vectorized", all(feature = "vectorized", test))', True),
        ('not (feature = "vectorized")', False),
        ('all(not (feature = "vectorized"))', False),
        ('not(not(feature = "vectorized"))', False),
        ('any(feature = "vectorized", test)', False),
        ('any()', False),
        ('feature = "vectorized-lite"', False),
        ('all(feature = "vectorized"', False),
        ('feature = vectorized', False),
    ]
    bad = [p for p, want in cases if req(parse(p)) != want]
    if not bad:
        print(f"  PASS — {len(cases)} cfg predicates classified correctly")
        return True
    print(f"  FAIL — misclassified: {bad}")
    return False


def test_leg3_cfg_attr_path_rejected() -> bool:
    """cfg_attr(..., path = ...) on a mod declaration fails as unsupported."""
    bad = _all_rc(_check._LEG3_TREES_CFG_ATTR_PATH, want_fail=True)
    if not bad:
        print("  PASS — every cfg_attr(path) spelling rejected")
        return True
    print(f"  FAIL — cfg_attr(path) spellings {bad} accepted")
    return False


def test_leg3_cfg_attr_without_path_accepted() -> bool:
    """cfg_attr without a path argument does not trip the audit."""
    bad = _all_rc(_check._LEG3_TREES_CFG_ATTR_OK, want_fail=False)
    if not bad:
        print("  PASS — path-free cfg_attr accepted")
        return True
    print(f"  FAIL — path-free cfg_attr {bad} rejected")
    return False


def test_leg3_mixed_declarations_not_gated() -> bool:
    """A file declared both gated and ungated (and its descendants) is not gated."""
    bad = _all_rc(_check._LEG3_TREES_MIXED_DECLS, want_fail=True)
    if not bad:
        print("  PASS — mixed declarations leave the file and its descendants ungated")
        return True
    print(f"  FAIL — mixed declarations {bad} exempted the file")
    return False


def test_leg3_all_declarations_gated_accepted() -> bool:
    """A file whose every declaration is gated (directly or via its parent) is gated."""
    bad = _all_rc(_check._LEG3_TREES_ALL_DECLS_GATED, want_fail=False)
    if not bad:
        print("  PASS — all-gated declarations accepted")
        return True
    print(f"  FAIL — all-gated declarations {bad} rejected")
    return False


def test_leg3_unsupported_path_spellings_rejected() -> bool:
    """r"..", r#".."#, escaped, byte-string, macro and duplicate #[path] all fail."""
    bad = _all_rc(_check._LEG3_TREES_PATH_UNSUPPORTED, want_fail=True)
    if not bad:
        print("  PASS — every non-simple #[path] spelling rejected as unsupported")
        return True
    print(f"  FAIL — non-simple #[path] spellings {bad} accepted")
    return False


def test_leg3_real_tree_passes() -> bool:
    """The repo's own exec tree passes leg 3."""
    root = os.path.dirname(_SCRIPT_DIR)
    if _check.check_leg3(repo_root=root) == 0:
        print("  PASS — repo exec tree is clean")
        return True
    print("  FAIL — repo exec tree has violations")
    return False


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

def main() -> int:
    tests = [
        ("leg1 rejects vectorized in resolve node", test_leg1_guard_rejects_vectorized_in_resolve),
        ("leg1 rejects vectorized in default features", test_leg1_guard_rejects_vectorized_in_default_features),
        ("leg1 accepts clean metadata (sanity)", test_leg1_accepts_clean_metadata),
        ("leg2 identical bytes pass (sanity)", test_leg2_identical_bytes_pass),
        ("leg2 undeclared change rejected (tripwire)", test_leg2_undeclared_change_rejected),
        ("leg2 declared change accepted (sanity)", test_leg2_declared_change_accepted),
        ("leg2 same-length content change caught (byte-for-byte)", test_leg2_same_length_content_change_caught),
        ("leg2 missing base decl defaults to 0", test_leg2_missing_base_decl_defaults_zero),
        ("leg2 both decls absent => undeclared rejected", test_leg2_both_decls_missing_undeclared_rejected),
        ("leg2 V2 new per-PR file accepted", test_leg2_v2_new_file_accepted),
        ("leg2 V2 no new file => undeclared rejected", test_leg2_v2_no_new_file_rejected),
        ("leg2 V2 README/.gitkeep not a declaration", test_leg2_v2_readme_not_a_declaration),
        ("leg2 V2 <PR>.md declaration accepted", test_leg2_v2_md_declaration_accepted),
        ("leg2 legacy scalar accepted in transition window", test_leg2_legacy_scalar_still_accepted_in_transition),
        ("leg2 no dirs falls back to scalar path", test_leg2_no_dirs_falls_back_to_scalar),
        ("leg3 ungated reference in exec child rejected", test_leg3_ungated_child_rejected),
        ("leg3 unscanned exec child module rejected", test_leg3_unscanned_child_rejected),
        ("leg3 item- and module-level gates accepted", test_leg3_gated_tree_accepted),
        ("leg3 same-line #[path] attributes resolved", test_leg3_same_line_path_attr_resolved),
        ("leg3 not/any/other-feature cfgs do not gate", test_leg3_non_gating_cfg_rejected),
        ("leg3 unresolvable mod fails closed", test_leg3_unresolvable_mod_rejected),
        ("leg3 gating cfg spellings accepted", test_leg3_gating_cfg_spellings_accepted),
        ("leg3 cfg predicate parser", test_leg3_cfg_parser),
        ("leg3 cfg_attr(path) on mod rejected", test_leg3_cfg_attr_path_rejected),
        ("leg3 cfg_attr without path accepted", test_leg3_cfg_attr_without_path_accepted),
        ("leg3 gated+ungated declarations do not exempt a file", test_leg3_mixed_declarations_not_gated),
        ("leg3 all-gated declarations accepted", test_leg3_all_declarations_gated_accepted),
        ("leg3 raw/escaped/byte/macro #[path] rejected", test_leg3_unsupported_path_spellings_rejected),
        ("leg3 repo exec tree passes", test_leg3_real_tree_passes),
    ]

    passed = 0
    failed = 0
    print("=== test_vectorized_feature_off.py ===\n")
    for name, fn in tests:
        print(f"[TEST] {name}")
        ok = fn()
        if ok:
            passed += 1
        else:
            failed += 1
        print()

    print(f"=== Results: {passed} passed, {failed} failed ===")
    if failed:
        print("FAIL")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
