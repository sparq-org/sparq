#!/usr/bin/env python3
# [OPUS-4.8] Hermetic unit tests for the ci/path-ownership.toml map + the
# out-of-crate-input audit (bead sq-fmx4u.2, epic sq-fmx4u). Authored by Opus
# 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# Covers the design §4 acceptance criteria:
#   * map-validity: every `crates` name is a live member, every pattern root
#     exists OR is a FETCHED/GENERATED suite (allowlist, #1392 — not brittle on
#     a clean clone);
#   * the [triggers] mirror stays == ci_select._FULL_TRIGGERS (drift guard);
#   * one test per §4.1 trigger class => mode=full (the map cannot shadow a
#     trigger);
#   * the map never WEAKENS the fail-closed set (no safe/attribution entry
#     matches a trigger path);
#   * NEGATIVE: an AGENTS.md / skills/** change SELECTS sparq-kb (not skipped);
#   * the audit itself: mapped/unmapped/cross-crate/acknowledged/safe-read/stale
#     classification, plus a live audit of the REAL workspace (self-skips when
#     cargo metadata is unavailable, so the suite never REQUIRES a live cargo).
#
# Disjoint from bead .1's files: this module imports ci_select.py + the sibling
# ci_audit_inputs.py; it edits neither.
#
# Run:  python3 scripts/tests/test_path_ownership.py   (stdlib only; no pytest)

from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = REPO_ROOT / "scripts"
MAP_FILE = REPO_ROOT / "ci" / "path-ownership.toml"


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


cs = _load("ci_select", SCRIPTS / "ci_select.py")
audit = _load("ci_audit_inputs", SCRIPTS / "ci_audit_inputs.py")


# --- shared synthetic metadata ----------------------------------------------
def _dep(name):
    return {"name": name, "kind": None, "optional": False, "target": None, "path": None}


def _pkg(name, deps=()):
    return {
        "id": f"path+file:///repo/crates/{name}#0.1.0",
        "name": name,
        "source": None,
        "manifest_path": f"/repo/crates/{name}/Cargo.toml",
        "dependencies": [_dep(d) for d in deps],
    }


# A synthetic workspace whose members are exactly the crate names the REAL map
# references (so cs.select accepts the map), plus a leaf `sparq-leaf`. sparq-kb
# is a leaf here (no in-repo dependents) so an AGENTS.md change selects exactly
# {sparq-kb}.
_MAP_CRATES = [
    "sparq-kb", "sparq-conformance", "sparq-policy", "sparq-engine",
    "sparq-introspect", "sparq-nlq", "sparq-sim", "sparq-vectors", "sparq-zk-compose",
]


def _map_meta():
    pkgs = [_pkg(n) for n in _MAP_CRATES] + [_pkg("sparq-leaf", deps=["sparq-engine"])]
    return {
        "workspace_root": "/repo",
        "packages": pkgs,
        "workspace_members": [p["id"] for p in pkgs],
    }


def _real_map_entries():
    return cs.load_ownership_map(str(MAP_FILE))


def _raw_toml():
    with open(MAP_FILE, "rb") as fh:
        return tomllib.load(fh)


# §4.1 trigger rows — one representative changed path per class.
_TRIGGER_PATHS = {
    "Cargo.lock": "Cargo.lock",
    "root Cargo.toml": "Cargo.toml",
    "rust-toolchain": "rust-toolchain",
    "rust-toolchain.toml": "rust-toolchain.toml",
    ".cargo/**": ".cargo/config.toml",
    ".github/**": ".github/workflows/ci.yml",
    "scripts/**": "scripts/ci_select.py",
    "deny.toml": "deny.toml",
    "supply-chain/**": "supply-chain/audits.toml",
    "ci/path-ownership.toml": "ci/path-ownership.toml",
}


class MapValidityTests(unittest.TestCase):
    """Every crates name is a live member; every pattern root exists or is fetched."""

    def _members(self):
        # Prefer real cargo metadata; fall back to the on-disk crate names so the
        # test stays hermetic when cargo is unavailable.
        if shutil.which("cargo") is not None:
            try:
                out = subprocess.run(
                    ["cargo", "metadata", "--no-deps", "--format-version", "1"],
                    cwd=str(REPO_ROOT), check=True, capture_output=True, text=True, timeout=180,
                )
                ws = cs.parse_workspace(json.loads(out.stdout))
                return ws.members
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError):
                pass
        return set(audit.enumerate_crates(str(REPO_ROOT)).values())

    def test_map_is_valid_against_real_workspace(self):
        entries = _real_map_entries()
        self.assertTrue(entries, "the map should ship at least the audited entries")
        allow = audit.fetched_roots(str(REPO_ROOT))
        problems = cs.validate_map(entries, self._members(),
                                   known_generated_roots=allow, repo_root=str(REPO_ROOT))
        self.assertEqual(problems, [], f"map-validity problems: {problems}")

    def test_fetched_allowlist_covers_missing_roots(self):
        # The two fetched suites the map references must be in the allowlist so
        # they do not falsely fail on a clean clone (design §4.2 / #1392).
        allow = audit.fetched_roots(str(REPO_ROOT))
        self.assertIn("tests/w3c", allow)
        self.assertIn("tests/odrl-test-suite", allow)

    def test_every_crates_name_is_a_real_member(self):
        members = self._members()
        for e in _real_map_entries():
            for c in e.get("crates", []) or []:
                self.assertIn(c, members, f"map references unknown crate {c!r}")


class TriggerMirrorTests(unittest.TestCase):
    """The informational [triggers] table must not drift from _FULL_TRIGGERS."""

    def test_triggers_mirror_matches_selector(self):
        declared = _raw_toml()["triggers"]["patterns"]
        builtin = [pat for pat, _reason in cs._FULL_TRIGGERS]
        self.assertEqual(declared, builtin,
                         "ci/path-ownership.toml [triggers] drifted from ci_select._FULL_TRIGGERS")


class TriggerClassTests(unittest.TestCase):
    """One test per §4.1 row: a change there forces mode=full, map notwithstanding."""

    def setUp(self):
        self.meta = _map_meta()
        self.map = _real_map_entries()

    def test_each_trigger_class_forces_full(self):
        for label, path in _TRIGGER_PATHS.items():
            with self.subTest(trigger=label):
                # Pair the trigger with an ordinary crate change to prove the
                # trigger dominates rather than the crate narrowing the run.
                sel = cs.select([path, "crates/sparq-kb/src/lib.rs"], self.meta, self.map)
                self.assertEqual(sel.mode, "full", f"{label} ({path}) should force full")


class MapNeverWeakensTests(unittest.TestCase):
    """No map entry may match a trigger path (it can only EXTEND the fail set)."""

    def test_no_map_entry_shadows_a_trigger(self):
        entries = _real_map_entries()
        for label, path in _TRIGGER_PATHS.items():
            with self.subTest(trigger=label):
                verdict = cs.apply_ownership_map(path, entries)
                self.assertIsNone(
                    verdict,
                    f"map entry matches trigger path {path!r} ({verdict}); it must not — "
                    f"triggers are the fail-closed backbone",
                )


class NegativeKbSelectionTests(unittest.TestCase):
    """AGENTS.md / skills/** changes SELECT sparq-kb — they are never skipped."""

    def setUp(self):
        self.meta = _map_meta()
        self.map = _real_map_entries()

    def test_agents_md_change_selects_kb(self):
        sel = cs.select(["AGENTS.md"], self.meta, self.map)
        self.assertEqual(sel.mode, "selected")
        self.assertIn("sparq-kb", sel.affected)

    def test_skill_change_selects_kb(self):
        sel = cs.select(["skills/query-pkg/SKILL.md"], self.meta, self.map)
        self.assertEqual(sel.mode, "selected")
        self.assertIn("sparq-kb", sel.affected)

    def test_kb_is_not_silently_skipped(self):
        # The failure mode the negative test guards against: a SAFE-list mistake
        # would make the change select the EMPTY set (kb skipped). Prove not.
        sel = cs.select(["AGENTS.md"], self.meta, self.map)
        self.assertNotEqual(sel.affected, [], "AGENTS.md must not resolve to an empty (all-skipped) set")

    def test_research_change_is_safe_and_skips_all(self):
        # Contrast: a proven-inert SAFE path selects nobody (design intent).
        sel = cs.select(["research/change-based-test-selection.md"], self.meta, self.map)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])


# --- audit fixtures ---------------------------------------------------------
_CRATE_ROOTS = {
    "crates/a": "crate-a",
    "crates/b": "crate-b",
    "crates/c": "crate-c",
}
# reverse closures: crate-b depends on crate-c (so c-change runs b).
_CLOSURES = {"crate-a": {"crate-a"}, "crate-b": {"crate-b"}, "crate-c": {"crate-c", "crate-b"}}


def _ref(crate, resolved, kind="manifest_join", src="crates/x/tests/t.rs", dynamic=False):
    return audit.Ref(crate=crate, src=src, resolved=resolved, kind=kind, dynamic=dynamic)


class AuditUnitTests(unittest.TestCase):
    """Hermetic classification tests — no cargo, no real tree."""

    def _audit(self, refs, entries, acks=()):
        return audit.audit_refs(refs, _CRATE_ROOTS, entries, _CLOSURES, acks=acks)

    def test_mapped_repo_level_input_is_covered(self):
        entries = [{"pattern": "data/**", "crates": ["crate-a"]}]
        r = self._audit([_ref("crate-a", "data/x.ttl")], entries)
        self.assertTrue(r.ok, r.findings)

    def test_unmapped_repo_level_input_is_a_finding(self):
        r = self._audit([_ref("crate-a", "data/x.ttl")], [])
        self.assertFalse(r.ok)
        self.assertTrue(any("UNMAPPED" in f for f in r.findings))

    def test_trigger_path_is_covered(self):
        r = self._audit([_ref("crate-a", "scripts/gen.sh")], [])
        self.assertTrue(r.ok, r.findings)

    def test_cross_crate_read_in_closure_is_covered(self):
        # crate-b reads a file inside crate-c; b in closure(c) -> covered.
        r = self._audit([_ref("crate-b", "crates/c/data/x")], [])
        self.assertTrue(r.ok, r.findings)

    def test_cross_crate_read_not_in_closure_is_a_finding(self):
        # crate-a reads a file inside crate-c; a NOT in closure(c) -> finding.
        r = self._audit([_ref("crate-a", "crates/c/data/x")], [])
        self.assertFalse(r.ok)
        self.assertTrue(any("reverse-dependency closure" in f for f in r.findings))

    def test_acknowledged_residual_passes(self):
        acks = (audit.Residual("crate-a", "crates/c/data", "known; backstopped"),)
        r = self._audit([_ref("crate-a", "crates/c/data/x")], [], acks=acks)
        self.assertTrue(r.ok, r.findings)
        self.assertEqual(r.acknowledged, [("crate-a", "crates/c/data/x")])

    def test_shipped_residual_list_is_empty(self):
        # [SONNET-4.6] sq-z1xv8 — residual 3 (sparq-conformance's scoreboard_floors.rs
        # reading sibling test sources at a runtime-built workspace path) was the last
        # acknowledged residual; the shared sparq-conformance-floors crate relocated the
        # input so the read is gone. Nothing is acknowledged any more, and this pins that
        # posture: a NEW out-of-crate read must be fixed or attributed in
        # ci/path-ownership.toml, not parked on the nightly full-run backstop. Removing
        # this assertion is the deliberate act of re-opening that door.
        self.assertEqual(
            audit.KNOWN_RESIDUALS, (),
            "every out-of-crate input is covered; prefer relocation/attribution over a "
            "new acknowledged residual (sq-z1xv8 closed the last one)",
        )

    def test_stale_acknowledgement_is_a_finding(self):
        acks = (audit.Residual("crate-a", "crates/nope/data", "matches nothing"),)
        r = self._audit([_ref("crate-a", "data/x")], [{"pattern": "data/**", "crates": ["crate-a"]}], acks=acks)
        self.assertFalse(r.ok)
        self.assertTrue(any("STALE" in f for f in r.findings))

    def test_crate_reading_safe_path_is_a_finding(self):
        entries = [{"pattern": "docs/**", "safe": True}]
        r = self._audit([_ref("crate-a", "docs/x.md")], entries)
        self.assertFalse(r.ok)
        self.assertTrue(any("SAFE-listed" in f for f in r.findings))

    def test_mapped_but_reader_not_listed_is_a_finding(self):
        entries = [{"pattern": "data/**", "crates": ["crate-b"]}]  # a reads it but not listed
        r = self._audit([_ref("crate-a", "data/x")], entries)
        self.assertFalse(r.ok)
        self.assertTrue(any("NOT to 'crate-a'" in f for f in r.findings))

    def test_sibling_read_covered_by_additional_readers(self):
        # [FABLE-5] sq-m4bxc: crate-a reads a file inside crate-c and is NOT in
        # closure(c) — normally a finding — but a `readers` map entry declaring
        # crate-a covers it (the selector unions crate-a into the affected set).
        entries = [{"pattern": "crates/c/**", "readers": ["crate-a"]}]
        r = self._audit([_ref("crate-a", "crates/c/rules/x.n3")], entries)
        self.assertTrue(r.ok, r.findings)
        self.assertIn(("crate-a", "crates/c/rules/x.n3", "additional-readers map"), r.covered)

    def test_sibling_read_not_in_readers_is_still_a_finding(self):
        # A `readers` entry that does NOT name the reader does not cover it.
        entries = [{"pattern": "crates/c/**", "readers": ["crate-b"]}]
        r = self._audit([_ref("crate-a", "crates/c/rules/x.n3")], entries)
        self.assertFalse(r.ok)
        self.assertTrue(any("reverse-dependency closure" in f for f in r.findings))


class AdditionalReadersMapTests(unittest.TestCase):
    """[FABLE-5] sq-m4bxc: the shipped map's `readers` entries validate + resolve.

    The residual sibling-read (sparq-reason -> sparq-solid/rules) is closed by an
    additional-readers entry; assert it is present, valid, and MONOTONE (never a
    trigger-shadow, never an ownership verdict).

    [OPUS-5] sq-3705: the secprop-vocabulary readers entries are GONE, and their
    absence is asserted below. They patched a real residual — sparq-zk and
    sparq-policy `include_str!`d sparq-trust's `secprop-ext.ttl` across package
    boundaries because a sparq-zk -> sparq-trust edge would be a cycle — but the
    reads themselves were removed: the vocabulary moved to the zero-dependency
    `sparq-secprop-vocab` leaf all three now depend on, so the ORDINARY reverse
    closure attributes it (see `test_secprop_vocab_change_selects_its_consumers`
    in test_ci_select.py)."""

    def test_shipped_map_has_the_solid_rules_readers_entry(self):
        readers_map = {
            e["pattern"]: e["readers"]
            for e in _real_map_entries() if "readers" in e
        }
        self.assertEqual(
            readers_map.get("crates/sparq-solid/rules/**"), ["sparq-reason"],
        )

    def test_secprop_readers_entries_are_retired(self):
        # [OPUS-5] sq-3705. A `readers` entry for either path would mean a
        # cross-package `include_str!` of the secprop vocabulary / annotation
        # graph came back — the thing #3705 deleted (it also broke `cargo package`
        # file inclusion for sparq-zk). Real cargo edges attribute these now.
        readers_map = {
            e["pattern"]: e["readers"]
            for e in _real_map_entries() if "readers" in e
        }
        for retired in (
            "crates/sparq-trust/ontologies/zkp-sparql/**",
            "crates/sparq-zk/ontologies/**",
            "crates/sparq-secprop-vocab/**",
        ):
            self.assertIsNone(
                readers_map.get(retired),
                f"{retired} has a readers entry again — a cross-package "
                "include_str! of the secprop vocabulary is back (sq-3705)",
            )

    def test_every_readers_name_is_a_real_member(self):
        members = MapValidityTests()._members()
        for e in _real_map_entries():
            for c in e.get("readers", []) or []:
                self.assertIn(c, members, f"map references unknown reader crate {c!r}")

    def test_additional_readers_resolves_the_residual_path(self):
        entries = _real_map_entries()
        self.assertIn(
            "sparq-reason",
            cs.additional_readers("crates/sparq-solid/rules", entries),
        )

    def test_readers_entry_is_not_an_ownership_verdict(self):
        # A readers entry must never resolve as a `crates`/`safe` verdict.
        entries = _real_map_entries()
        self.assertIsNone(cs.apply_ownership_map("crates/sparq-solid/rules/wac.n3", entries))

    def test_validate_map_rejects_unknown_reader(self):
        problems = cs.validate_map(
            [{"pattern": "crates/x/**", "readers": ["nope"]}],
            members={"sparq-core"}, known_generated_roots={"crates/x"},
        )
        self.assertTrue(any("unknown readers crate" in p for p in problems))


class ScanRefsTests(unittest.TestCase):
    """The scanner finds escaping include_str! + manifest-join reads in a tempdir."""

    def test_scan_finds_escaping_reads(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # crate-a includes a sibling crate-b file and reads a repo-level dir.
            (root / "crates" / "crate-a" / "src").mkdir(parents=True)
            (root / "crates" / "crate-a" / "Cargo.toml").write_text('name = "crate-a"\n')
            (root / "crates" / "crate-b").mkdir(parents=True)
            (root / "crates" / "crate-b" / "Cargo.toml").write_text('name = "crate-b"\n')
            (root / "crates" / "crate-a" / "src" / "lib.rs").write_text(
                'const X: &str = include_str!("../../crate-b/data.ttl");\n'
                'const README: &str = include_str!("../README.md");\n'  # in-crate: ignored
            )
            (root / "crates" / "crate-a" / "tests").mkdir()
            (root / "crates" / "crate-a" / "tests" / "t.rs").write_text(
                'let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/data");\n'
            )
            roots = audit.enumerate_crates(str(root))
            refs = audit.scan_refs(str(root), roots)
            resolved = {(r.crate, r.resolved) for r in refs}
            self.assertIn(("crate-a", "crates/crate-b/data.ttl"), resolved)
            self.assertIn(("crate-a", "corpus/data"), resolved)
            # The in-crate README include must NOT be reported.
            self.assertNotIn(("crate-a", "crates/crate-a/README.md"), resolved)


class RealWorkspaceAuditTests(unittest.TestCase):
    """Live audit of the REAL workspace: the shipped map must fully cover it."""

    @classmethod
    def setUpClass(cls):
        cls.meta = None
        if shutil.which("cargo") is None:
            return
        try:
            out = subprocess.run(
                ["cargo", "metadata", "--format-version", "1"],
                cwd=str(REPO_ROOT), check=True, capture_output=True, text=True, timeout=300,
            )
            cls.meta = json.loads(out.stdout)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError):
            cls.meta = None

    def setUp(self):
        if self.meta is None:
            self.skipTest("cargo metadata unavailable (hermetic skip)")

    def test_real_workspace_audit_passes(self):
        result = audit.run_audit(str(REPO_ROOT), str(MAP_FILE), self.meta)
        self.assertTrue(
            result.ok,
            "out-of-crate-input audit found UNCOVERED inputs — complete the map "
            "or acknowledge a residual:\n" + "\n".join(result.findings),
        )
        # Sanity: the shipped map is actually doing work (not vacuously empty).
        self.assertGreater(len(result.covered), 5)


if __name__ == "__main__":
    unittest.main(verbosity=2)
