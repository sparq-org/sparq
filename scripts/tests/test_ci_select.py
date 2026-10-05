#!/usr/bin/env python3
# [OPUS-4.8] Hermetic unit tests for the change-based CI test-selector
# (bead sq-fmx4u.1, epic sq-fmx4u). Authored by Opus 4.8 (Fable unavailable;
# flag for re-review when Fable returns).
#
# Covers the design §3/§4 golden cases + the bead acceptance criteria (a)-(i):
#   (a) leaf-crate change   => exactly that crate
#   (b) sparq-core change    => all workspace members
#   (c) dev-dep edge         propagates to the dependent's tests
#   (d) optional-dep edge    propagates
#   (e) file move            marks BOTH crates
#   (f) unowned/unmapped     => full  (fail-safe)
#   (g) every §4.1 trigger   => full  (fail-safe)
#   (h) forced internal error=> mode=full, exit 0  (fail-closed)
#   (i) REAL cargo-metadata  closure shape (core root-like, geo leaf-like)
#
# Almost fully hermetic: the synthetic-graph cases build `cargo metadata`-shaped
# dicts in-process (no workspace resolve). Case (i) shells out to
# `cargo metadata --no-deps` and self-SKIPS when cargo is unavailable, so the
# suite never REQUIRES a live cargo (task hermeticity rule).
#
# Run:  python3 scripts/tests/test_ci_select.py   (stdlib only; no pytest needed)

from __future__ import annotations

import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CI_SELECT = REPO_ROOT / "scripts" / "ci_select.py"
MAP_FILE = REPO_ROOT / "ci" / "path-ownership.toml"


def _load_module():
    spec = importlib.util.spec_from_file_location("ci_select", CI_SELECT)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ci_select"] = mod
    spec.loader.exec_module(mod)
    return mod


cs = _load_module()

ROOT = "/repo"


def _dep(name, kind=None, optional=False, target=None):
    return {"name": name, "kind": kind, "optional": optional, "target": target, "path": None}


def _pkg(name, reldir, deps=(), source=None):
    return {
        "id": f"path+file://{ROOT}/{reldir}#0.1.0",
        "name": name,
        "source": source,
        "manifest_path": f"{ROOT}/{reldir}/Cargo.toml",
        "dependencies": list(deps),
    }


def _synthetic_meta():
    """A small DAG rooted at `core` (everything depends on core), with `app` a
    reverse-graph leaf. Exercises normal, dev, build and optional edges.

        core  <- parse <- engine <- app
        core  <- devlib (dev-dep of engine)
        core  <- optlib (optional dep of engine)
        core  <- buildlib (build-dep of parse)
    """
    pkgs = [
        _pkg("core", "crates/core", []),
        _pkg("parse", "crates/parse", [_dep("core"), _dep("buildlib", kind="build")]),
        _pkg("engine", "crates/engine",
             [_dep("core"), _dep("parse"), _dep("devlib", kind="dev"),
              _dep("optlib", optional=True)]),
        _pkg("app", "crates/app", [_dep("core"), _dep("engine")]),
        _pkg("devlib", "crates/devlib", [_dep("core")]),
        _pkg("optlib", "crates/optlib", [_dep("core")]),
        _pkg("buildlib", "crates/buildlib", [_dep("core")]),
        # A registry dep sharing NOTHING in-repo: must be ignored as an owner/edge.
        {"id": "registry+x#1.0", "name": "serde", "source": "registry+https://x",
         "manifest_path": "/reg/serde/Cargo.toml", "dependencies": []},
    ]
    member_ids = [p["id"] for p in pkgs if p["source"] is None]
    return {"workspace_root": ROOT, "packages": pkgs, "workspace_members": member_ids}


ALL_MEMBERS = ["app", "buildlib", "core", "devlib", "engine", "optlib", "parse"]


class SyntheticGraphTests(unittest.TestCase):
    def setUp(self):
        self.meta = _synthetic_meta()

    def _select(self, paths, map_entries=None):
        return cs.select(paths, self.meta, map_entries)

    def test_a_leaf_crate_change_is_exactly_that_crate(self):
        sel = self._select(["crates/app/src/lib.rs"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app"])

    def test_b_core_change_is_all_members(self):
        sel = self._select(["crates/core/src/dict.rs"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ALL_MEMBERS)

    def test_c_dev_dep_edge_propagates(self):
        # devlib is a DEV-dep of engine; a devlib change must pull engine (+ app).
        sel = self._select(["crates/devlib/src/lib.rs"])
        self.assertEqual(sel.mode, "selected")
        self.assertIn("engine", sel.affected)
        self.assertEqual(sel.affected, ["app", "devlib", "engine"])

    def test_d_optional_dep_edge_propagates(self):
        # optlib is an OPTIONAL dep of engine; included regardless of features.
        sel = self._select(["crates/optlib/src/lib.rs"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app", "engine", "optlib"])

    def test_build_dep_edge_propagates(self):
        # buildlib is a BUILD-dep of parse; build kind is included.
        sel = self._select(["crates/buildlib/build_input.rs"])
        self.assertEqual(sel.affected, ["app", "buildlib", "engine", "parse"])

    def test_e_file_move_marks_both_crates(self):
        # --no-renames reports a move as delete+add; BOTH crate dirs attributed.
        sel = self._select(["crates/devlib/moved.rs", "crates/buildlib/moved.rs"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sorted(sel.changed_crates), ["buildlib", "devlib"])
        # parse is reachable ONLY via buildlib, engine via both -> proves union.
        self.assertIn("parse", sel.affected)
        self.assertIn("devlib", sel.affected)

    def test_f_unowned_unmapped_path_is_full(self):
        sel = self._select(["docs/whatever.md"])
        self.assertEqual(sel.mode, "full")
        self.assertIn("unowned", sel.reason)
        self.assertEqual(sel.affected, ALL_MEMBERS)  # full => run ALL crates

    def test_g_every_full_run_trigger_is_full(self):
        triggers = [
            "Cargo.lock",
            "Cargo.toml",
            "rust-toolchain",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            ".github/workflows/ci.yml",
            "scripts/ci_select.py",
            "deny.toml",
            "supply-chain/audits.toml",
            "ci/path-ownership.toml",
        ]
        for path in triggers:
            with self.subTest(path=path):
                sel = self._select([path, "crates/app/src/lib.rs"])
                self.assertEqual(sel.mode, "full", f"{path} should force full")
                self.assertEqual(sel.affected, ALL_MEMBERS)

    def test_crate_cargo_toml_is_owned_not_a_root_trigger(self):
        # A crate manifest is owned by that crate (NOT caught by root Cargo.toml).
        sel = self._select(["crates/app/Cargo.toml"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app"])

    def test_longest_prefix_ownership(self):
        # Nested-crate safety: prefix match must not leak across sibling dirs.
        sel = self._select(["crates/engine/benches/x.rs"])
        self.assertEqual(sel.changed_crates, ["engine"])

    def test_json_contract_keys(self):
        # [OPUS-4.8] `change_class` added to the JSON contract (audit-trail label; not
        # a gating input — the downstream guards still read only mode/affected).
        sel = self._select(["crates/app/src/lib.rs"])
        obj = sel.to_json_obj()
        self.assertEqual(set(obj), {"mode", "reason", "affected", "change_class"})


class OwnershipMapTests(unittest.TestCase):
    def setUp(self):
        self.meta = _synthetic_meta()

    def test_safe_entry_is_ignored(self):
        # research/** SAFE + a real crate change => only the crate counts.
        m = [{"pattern": "research/**", "safe": True}]
        sel = cs.select(["research/foo.md", "crates/app/src/lib.rs"], self.meta, m)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app"])

    def test_safe_only_change_selects_empty(self):
        m = [{"pattern": "research/**", "safe": True}]
        sel = cs.select(["research/foo.md"], self.meta, m)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])

    def test_map_attributes_out_of_crate_path(self):
        # The #1392 refinement: the REAL fetched W3C path is tests/w3c/** and it
        # is read by the conformance crate (here mapped to `engine`).
        m = [{"pattern": "tests/w3c/**", "crates": ["engine"]}]
        sel = cs.select(["tests/w3c/rdf-tests/data.ttl"], self.meta, m)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app", "engine"])

    def test_map_unknown_crate_fails_safe(self):
        m = [{"pattern": "tests/w3c/**", "crates": ["ghost-crate"]}]
        sel = cs.select(["tests/w3c/x"], self.meta, m)
        self.assertEqual(sel.mode, "full")
        self.assertIn("unknown crate", sel.reason)

    def test_map_malformed_entry_raises(self):
        m = [{"pattern": "tests/w3c/**"}]  # neither safe, crates, nor readers
        with self.assertRaises(cs.SelectorError):
            cs.select(["tests/w3c/x"], self.meta, m)


class AdditionalReadersTests(unittest.TestCase):
    """[FABLE-5] sq-m4bxc: the additional-readers (monotone union) mechanism.

    A `readers` entry unions extra reader crates into the affected set EVEN FOR
    A CRATE-OWNED path, closing a sibling read where a real dep edge would be a
    forbidden cycle. The load-bearing invariant is MONOTONICITY: adding a
    `readers` entry can only ENLARGE (never shrink) the selection — so it can
    never turn a run into an unsound skip (design §2/§4.2)."""

    def setUp(self):
        self.meta = _synthetic_meta()

    def test_readers_union_enlarges_a_crate_owned_selection(self):
        # An `app` change alone selects {app}. A readers entry on the app dir
        # naming `parse` also pulls parse's reverse closure (parse<-engine<-app).
        base = cs.select(["crates/app/src/lib.rs"], self.meta)
        self.assertEqual(base.affected, ["app"])
        m = [{"pattern": "crates/app/**", "readers": ["parse"]}]
        sel = cs.select(["crates/app/src/lib.rs"], self.meta, m)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app", "engine", "parse"])
        self.assertIn("parse", sel.affected)  # the extra reader is present

    def test_readers_added_alongside_prefix_owner(self):
        # The path's normal prefix owner is STILL selected (readers only adds).
        m = [{"pattern": "crates/app/**", "readers": ["parse"]}]
        sel = cs.select(["crates/app/src/lib.rs"], self.meta, m)
        self.assertIn("app", sel.changed_crates)
        self.assertIn("parse", sel.changed_crates)

    def test_unknown_additional_reader_fails_full(self):
        # A `readers` naming a non-member cannot silently skip — fail-safe to full.
        m = [{"pattern": "crates/app/**", "readers": ["ghost-crate"]}]
        sel = cs.select(["crates/app/src/lib.rs"], self.meta, m)
        self.assertEqual(sel.mode, "full")
        self.assertIn("unknown additional-reader", sel.reason)
        self.assertEqual(sel.affected, ALL_MEMBERS)

    def test_readers_never_rescue_an_unowned_path(self):
        # MONOTONICITY GUARD: a `readers` entry must NOT convert an otherwise
        # unowned/unmapped path (which forces full) into a narrow selection —
        # that would SHRINK the run set. It stays full.
        m = [{"pattern": "weird/**", "readers": ["core"]}]
        sel = cs.select(["weird/thing.txt"], self.meta, m)
        self.assertEqual(sel.mode, "full")
        self.assertEqual(sel.affected, ALL_MEMBERS)

    def test_readers_entry_is_not_an_ownership_verdict(self):
        # apply_ownership_map treats a readers-only entry as transparent (None),
        # so it never provides a `crates`/`safe` verdict nor raises.
        m = [{"pattern": "data/**", "readers": ["core"]}]
        self.assertIsNone(cs.apply_ownership_map("data/x", m))

    def test_additional_readers_helper_unions_all_matches(self):
        m = [
            {"pattern": "crates/app/**", "readers": ["parse"]},
            {"pattern": "crates/app/src/**", "readers": ["engine"]},
        ]
        self.assertEqual(cs.additional_readers("crates/app/src/lib.rs", m), ["engine", "parse"])
        self.assertEqual(cs.additional_readers("crates/core/src/x.rs", m), [])

    def test_monotonicity_property_readers_only_enlarges(self):
        # PROPERTY: for ANY changed-path set, adding a `readers` entry yields an
        # affected set that is a SUPERSET of the selection without it. Exercise a
        # spread of cases (leaf / root / mapped / safe / unowned-full).
        readers_entry = {"pattern": "crates/app/**", "readers": ["parse", "core"]}
        base_maps = [
            [],
            [{"pattern": "research/**", "safe": True}],
            [{"pattern": "tests/w3c/**", "crates": ["engine"]}],
        ]
        path_sets = [
            ["crates/app/src/lib.rs"],
            ["crates/core/src/dict.rs"],
            ["crates/devlib/src/lib.rs"],
            ["research/x.md", "crates/app/src/lib.rs"],
            ["tests/w3c/data.ttl", "crates/app/x.rs"],
            ["docs/unowned.md"],  # forces full in both -> superset (equal) holds
        ]
        for base in base_maps:
            augmented = base + [readers_entry]
            for paths in path_sets:
                base_sel = cs.select(paths, self.meta, base)
                aug_sel = cs.select(paths, self.meta, augmented)
                self.assertTrue(
                    set(base_sel.affected).issubset(set(aug_sel.affected)),
                    f"monotonicity violated for paths={paths}, base={base}: "
                    f"{base_sel.affected} !subset {aug_sel.affected}",
                )


class MapValidityTests(unittest.TestCase):
    # #1392 refinement: validity uses a FETCHED/GENERATED allowlist, not a
    # brittle on-disk check, so `tests/w3c/**` (fetched, gitignored) validates
    # clean on a clean clone.
    def test_fetched_root_validates_when_allowlisted(self):
        members = {"sparq-conformance"}
        entries = [{"pattern": "tests/w3c/**", "crates": ["sparq-conformance"]}]
        with tempfile.TemporaryDirectory() as tmp:  # tests/w3c absent on disk
            problems = cs.validate_map(
                entries, members,
                known_generated_roots={"tests/w3c"}, repo_root=tmp,
            )
        self.assertEqual(problems, [])

    def test_unknown_crate_flagged(self):
        problems = cs.validate_map(
            [{"pattern": "x/**", "crates": ["nope"]}],
            members={"sparq-core"}, known_generated_roots={"x"},
        )
        self.assertTrue(any("unknown crate" in p for p in problems))

    def test_missing_nonallowlisted_root_flagged(self):
        with tempfile.TemporaryDirectory() as tmp:
            problems = cs.validate_map(
                [{"pattern": "nonexistent/**", "safe": True}],
                members=set(), known_generated_roots=set(), repo_root=tmp,
            )
        self.assertTrue(any("does not exist" in p for p in problems))


class FailClosedMainTests(unittest.TestCase):
    """(h) ANY internal error => mode=full, exit 0 — the fail-closed boundary."""

    def _run_main(self, argv):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = cs.main(argv)
        return code, json.loads(buf.getvalue())

    def _write(self, text, suffix=".json"):
        fd, path = tempfile.mkstemp(suffix=suffix)
        with os.fdopen(fd, "w") as fh:
            fh.write(text)
        self.addCleanup(lambda: os.path.exists(path) and os.remove(path))
        return path

    def test_malformed_metadata_json_fails_full(self):
        meta_path = self._write("{ this is not json ")
        changed = self._write("crates/app/src/lib.rs\n", suffix=".txt")
        code, obj = self._run_main(["--metadata-file", meta_path, "--changed-file", changed,
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")

    def test_missing_metadata_keys_fails_full(self):
        meta_path = self._write(json.dumps({"packages": []}))  # no workspace_root/members
        changed = self._write("crates/app/src/lib.rs\n", suffix=".txt")
        code, obj = self._run_main(["--metadata-file", meta_path, "--changed-file", changed,
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")

    def test_nonexistent_metadata_file_fails_full(self):
        changed = self._write("crates/app/src/lib.rs\n", suffix=".txt")
        code, obj = self._run_main(["--metadata-file", "/no/such/meta.json",
                                    "--changed-file", changed, "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")

    def test_schedule_event_is_full(self):
        meta_path = self._write(json.dumps(_synthetic_meta()))
        code, obj = self._run_main(["--event", "schedule", "--metadata-file", meta_path,
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")
        self.assertEqual(obj["affected"], ALL_MEMBERS)

    def test_ci_full_override_is_full(self):
        meta_path = self._write(json.dumps(_synthetic_meta()))
        code, obj = self._run_main(["--full", "--metadata-file", meta_path, "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")

    def test_hermetic_end_to_end_selected(self):
        meta_path = self._write(json.dumps(_synthetic_meta()))
        changed = self._write("crates/app/src/lib.rs\n", suffix=".txt")
        code, obj = self._run_main(["--metadata-file", meta_path, "--changed-file", changed,
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "selected")
        self.assertEqual(obj["affected"], ["app"])


class WiringHookTests(unittest.TestCase):
    """[FABLE-5] sq-fmx4u.3: the hooks the CI wiring consumes — the shadow rollout
    mode, the nextest filterset output, and clean full-mode on non-PR events."""

    def _run_main(self, argv):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = cs.main(argv)
        return code, json.loads(buf.getvalue())

    def _write(self, text, suffix=".json"):
        fd, path = tempfile.mkstemp(suffix=suffix)
        with os.fdopen(fd, "w") as fh:
            fh.write(text)
        self.addCleanup(lambda: os.path.exists(path) and os.remove(path))
        return path

    def test_push_event_is_clean_full(self):
        # Any event without a PR diff (push, or a future event name) => full by
        # construction, not via the error trap.
        meta_path = self._write(json.dumps(_synthetic_meta()))
        code, obj = self._run_main(["--event", "push", "--metadata-file", meta_path,
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")
        self.assertNotIn("selector error", obj["reason"])
        self.assertEqual(obj["affected"], ALL_MEMBERS)

    def test_shadow_wraps_selected(self):
        # --shadow: the selection is COMPUTED (affected preserved for the report)
        # but the emitted mode is 'shadow', so no guard's `mode == 'selected'`
        # branch can ever fire => nothing skips.
        meta_path = self._write(json.dumps(_synthetic_meta()))
        changed = self._write("crates/app/src/lib.rs\n", suffix=".txt")
        code, obj = self._run_main(["--shadow", "--metadata-file", meta_path,
                                    "--changed-file", changed, "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "shadow")
        self.assertIn("SHADOW (computed mode=selected", obj["reason"])
        self.assertEqual(obj["affected"], ["app"])

    def test_shadow_wraps_full_and_error_uniformly(self):
        # The wrap is uniform: even a computed full / a selector error emits
        # mode=shadow — one downstream rule (shadow is never 'selected').
        meta_path = self._write(json.dumps(_synthetic_meta()))
        code, obj = self._run_main(["--shadow", "--event", "schedule",
                                    "--metadata-file", meta_path, "--repo-root", ROOT])
        self.assertEqual(obj["mode"], "shadow")
        self.assertIn("computed mode=full", obj["reason"])
        code, obj = self._run_main(["--shadow", "--metadata-file", "/no/such/meta.json",
                                    "--changed-file", self._write("x\n", suffix=".txt"),
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "shadow")
        self.assertIn("selector error", obj["reason"])

    def test_output_file_carries_mode_affected_filterset(self):
        # The $GITHUB_OUTPUT contract the guards + bulk shards consume.
        meta_path = self._write(json.dumps(_synthetic_meta()))
        changed = self._write("crates/app/src/lib.rs\n", suffix=".txt")
        out_path = self._write("", suffix=".out")
        code, obj = self._run_main(["--metadata-file", meta_path, "--changed-file", changed,
                                    "--repo-root", ROOT, "--output-file", out_path])
        self.assertEqual(code, 0)
        with open(out_path, encoding="utf-8") as fh:
            lines = dict(ln.split("=", 1) for ln in fh.read().splitlines() if "=" in ln)
        self.assertEqual(lines["mode"], "selected")
        self.assertEqual(json.loads(lines["affected"]), ["app"])
        self.assertEqual(lines["filterset"], "package(app)")
        # [OPUS-4.8] change-class output present (a crate change => engine).
        self.assertEqual(lines["change_class"], "engine")

    def test_filterset_joins_members_with_plus(self):
        self.assertEqual(
            cs.filterset(cs.Selection(mode="selected", reason="", affected=["a", "b"])),
            "package(a) + package(b)",
        )
        self.assertEqual(cs.filterset(cs.Selection(mode="full", reason="", affected=[])), "")


# [OPUS-4.8] ---- change-class layer (path-aware CI for orchestration PRs) --------
class ChangeClassTests(unittest.TestCase):
    """The classifier fixtures the maintainer brief mandates: engine diff => full;
    an orchestration-only diff (the #3416 file set exactly) => reduced (mode=selected,
    empty closure); docs-only => reduced; mixed => full; a rename crossing classes =>
    full. classify_change is a PURE function of the diff and is fail-closed (an
    unclassified path taints the class to engine/mixed)."""

    def setUp(self):
        self.meta = _synthetic_meta()

    def _select(self, paths, map_entries=None):
        return cs.select(paths, self.meta, map_entries)

    # --- classify_change unit behaviour ---
    def test_classify_engine_on_crate_change(self):
        self.assertEqual(cs.classify_change(["crates/app/src/lib.rs"]), "engine")

    def test_classify_orchestration_only(self):
        # The #3416 file set EXACTLY: an orchestration config + an orchestration script.
        self.assertEqual(
            cs.classify_change(["orchestration/routing.toml", "scripts/triage.py"]),
            "orchestration-only",
        )

    def test_classify_docs_only(self):
        self.assertEqual(cs.classify_change(["docs/guide.md", "research/x.md"]), "docs-only")

    def test_classify_mixed_orchestration_plus_engine(self):
        self.assertEqual(
            cs.classify_change(["scripts/triage.py", "crates/app/src/lib.rs"]), "mixed"
        )

    def test_classify_inert_mixed_docs_plus_orchestration(self):
        # [OPUS-5] sq-g25hr: a diff confined to inert surfaces but SPANNING two of
        # them is `inert-mixed`, NOT `mixed`. It used to collapse into `mixed` — the
        # same token an engine+docs diff produces — so the consumers' skip case-arm
        # could not tell "provably nothing for the Rust matrix" from "some Rust
        # changed too" and conservatively ran the full suite.
        self.assertEqual(
            cs.classify_change(["docs/x.md", "scripts/triage.py"]), "inert-mixed"
        )

    def test_classify_deploy_only(self):
        self.assertEqual(
            cs.classify_change(["deploy/helm/quickstart/values.yaml",
                                "deploy/aws/sparq-server.yaml"]),
            "deploy-only",
        )

    def test_classify_deploy_only_covers_the_deploy_lint_workflows(self):
        # The two deploy lint workflows live under `.github/`, an unconditional
        # full-run TRIGGER — the deploy allowlist is their only rescue.
        self.assertEqual(
            cs.classify_change([".github/workflows/deploy-lint.yml",
                                ".github/workflows/deploy-terraform-lint.yml"]),
            "deploy-only",
        )

    def test_classify_inert_mixed_on_the_observed_cloud_deploy_batch(self):
        # The sq-g25hr motivating class (PRs #2314-2322): manifests + their READMEs
        # + the deploy lint workflow + repo-level prose. Zero Rust => inert-mixed.
        self.assertEqual(
            cs.classify_change([
                "deploy/gcp/sparq-lws.yaml",
                "deploy/gcp/README.md",
                ".github/workflows/deploy-lint.yml",
                "docs/branch-protection.md",
            ]),
            "inert-mixed",
        )

    def test_classify_mixed_deploy_plus_engine(self):
        # FAIL-CLOSED: one crate path taints the whole batch back to `mixed` (=> the
        # consumers' wildcard arm => full suite). This is the "a code change cannot
        # be mislabelled as docs-only" obligation, at the classifier level.
        self.assertEqual(
            cs.classify_change(["deploy/aws/sparq-server.yaml", "crates/app/src/lib.rs"]),
            "mixed",
        )

    def test_classify_engine_on_ci_gate_script(self):
        # A Rust-CI gate script is NOT orchestration-safe => engine (fail-closed).
        self.assertEqual(cs.classify_change(["scripts/coverage-gate.py"]), "engine")

    def test_classify_engine_on_rust_ci_workflow(self):
        self.assertEqual(cs.classify_change([".github/workflows/ci.yml"]), "engine")

    # --- the selection consequences (the whole point) ---
    def test_orchestration_only_diff_selects_empty_closure(self):
        # A pure-orchestration PR: mode=selected with an EMPTY affected set — every
        # Rust lane (incl. the bench/fuzz/wasm seed lanes) skips.
        sel = self._select(["orchestration/routing.toml", "scripts/triage.py"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])
        self.assertEqual(sel.change_class, "orchestration-only")
        self.assertIn("skipped-by-class: orchestration-only", sel.reason)

    def test_docs_only_diff_selects_empty_closure(self):
        m = [{"pattern": "research/**", "safe": True}]
        sel = self._select(["research/design.md"], m)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])
        self.assertEqual(sel.change_class, "docs-only")

    def test_deploy_only_diff_selects_empty_closure(self):
        # [OPUS-5] sq-g25hr: a deploy-only PR — including the `.github/workflows/
        # deploy-*.yml` files, which would otherwise hit the `.github/` full-run
        # trigger — selects mode=selected with an EMPTY affected set, so every Rust
        # lane (incl. the bench/fuzz/wasm seed lanes) skips.
        sel = self._select([
            "deploy/helm/quickstart/values.yaml",
            ".github/workflows/deploy-lint.yml",
        ])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])
        self.assertEqual(sel.change_class, "deploy-only")
        self.assertIn("skipped-by-class: deploy-only", sel.reason)

    def test_deploy_path_with_a_crate_change_still_narrows_not_fulls(self):
        # A deploy path must never FORCE full; the crate change narrows normally and
        # the class taints to `mixed` (=> the workflow wildcard arm => full suite).
        sel = self._select(["deploy/aws/sparq-server.yaml", "crates/app/src/lib.rs"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app"])
        self.assertEqual(sel.change_class, "mixed")

    def test_orchestration_safe_never_triggers_full(self):
        # Even paired with a crate change, the orch-safe path must not FORCE full;
        # the crate change narrows normally (selected), class becomes mixed.
        sel = self._select(["scripts/triage.py", "crates/app/src/lib.rs"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, ["app"])
        self.assertEqual(sel.change_class, "mixed")

    def test_mixed_engine_ci_script_still_forces_full(self):
        # A Rust-CI script (NOT orch-safe) keeps forcing full even alongside orch paths.
        sel = self._select(["scripts/coverage-gate.py", "scripts/triage.py"])
        self.assertEqual(sel.mode, "full")

    def test_workflow_file_change_forces_full_and_runs_gate_selftest(self):
        # A .github/workflows/ Rust-CI workflow edit (ci.yml) is NOT orch-safe: it
        # forces full, so the gate's own self-test leg + actionlint run (a CI change
        # must prove the gate still works). Representative Rust-CI workflow.
        sel = self._select([".github/workflows/ci.yml"])
        self.assertEqual(sel.mode, "full")

    def test_orchestration_workflow_edit_is_safe(self):
        # An orchestration-ONLY workflow file (triage-issue.yml) is proven inert.
        sel = self._select([".github/workflows/triage-issue.yml"])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])
        self.assertEqual(sel.change_class, "orchestration-only")

    def test_rename_crossing_classes_forces_full(self):
        # git diff --no-renames reports a move as delete+add of BOTH paths. Moving an
        # orchestration script INTO a crate dir surfaces both paths: the crate path is
        # owned (engine) => the diff is mixed and the crate is selected (never a skip
        # of the destination crate). Moving a crate file OUT to orchestration surfaces
        # the crate delete (owned) => that crate still runs.
        sel = self._select(["scripts/triage.py", "crates/app/src/triage.rs"])
        self.assertEqual(sel.change_class, "mixed")
        self.assertEqual(sel.mode, "selected")
        self.assertIn("app", sel.affected)

    def test_mutation_break_classifier_reddens_engine_fixture(self):
        # MUTATION SPOT-CHECK (brief): if the classifier were broken to call EVERYTHING
        # orchestration-safe, an engine-diff fixture must go RED. We simulate the break
        # by monkeypatching _orchestration_safe_match to always-True and assert the
        # engine fixture then wrongly skips — proving the real (False) path is what
        # keeps the engine diff running.
        orig = cs._orchestration_safe_match
        try:
            cs._orchestration_safe_match = lambda _p: True
            broken = cs.select(["crates/app/src/lib.rs"], self.meta)
            # Under the break, the crate change is swallowed as "safe" => empty closure.
            self.assertEqual(broken.affected, [], "mutation should make the engine diff skip")
        finally:
            cs._orchestration_safe_match = orig
        # And the REAL classifier keeps the engine crate selected (red-on-wrong-answer).
        good = cs.select(["crates/app/src/lib.rs"], self.meta)
        self.assertEqual(good.affected, ["app"])


# [OPUS-5] ---- #5249: the ownership map's `safe = true` verdicts at the CLASS layer --
class MapSafeChangeClassTests(unittest.TestCase):
    """#5249: `classify_change` consulted only the built-in allowlists, never the
    ownership map, so the two layers DISAGREED about the same diff — `site/**` is
    `safe = true` in ci/path-ownership.toml ("Owns its own CI lane (pages.yml)"), so
    the closure layer selected an EMPTY affected set while the class layer said
    `engine` and the merge_group batch paid the full Rust matrix + CodeQL. The class
    now reads the map (`_CLASS_MAP_SAFE`), and every fail-closed carve-out that keeps
    it from being MORE permissive than the selector is pinned below."""

    def setUp(self):
        self.meta = _synthetic_meta()
        self.real_map = cs.load_ownership_map(str(MAP_FILE))

    # --- the reported bug ---
    def test_site_only_diff_classifies_map_safe_not_engine(self):
        # The issue's exact reproduction (it returned `engine` before #5249).
        self.assertEqual(
            cs.classify_change(["site/index.tsx"], self.real_map), "map-safe"
        )

    def test_class_and_closure_agree_on_a_site_only_diff(self):
        # THE POINT of the fix: one notion of "inert". The closure was already
        # empty; the class is now inert too, so the merge_group gate can skip.
        sel = cs.select(["site/index.tsx"], self.meta, [{"pattern": "site/**", "safe": True}])
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])
        self.assertEqual(sel.change_class, "map-safe")
        self.assertIn(sel.change_class, cs._INERT_CLASSES)
        self.assertIn("skipped-by-class: map-safe", sel.reason)

    # --- fail-closed carve-outs (each can only ever run MORE) ---
    def test_no_map_keeps_the_pre_fix_engine_class(self):
        # Absent map => no safe-listed rescue => `engine` (a full run), exactly the
        # pre-#5249 behaviour. Absence of proof means run (design §2).
        self.assertEqual(cs.classify_change(["site/index.tsx"]), "engine")
        self.assertEqual(cs.classify_change(["site/index.tsx"], []), "engine")

    def test_first_match_wins_keeps_the_zk_anchor_engine(self):
        # sq-1s2.4 attributes two site/ files to sparq-zk-compose with entries that
        # sit BEFORE the `site/**` safe entry. A `crates = [...]` first match is not
        # a safe verdict, so the anchor edit still classifies engine and re-runs the
        # drift guard.
        self.assertEqual(
            cs.classify_change(["site/src/lib/zk-prover.ts"], self.real_map), "engine"
        )
        self.assertEqual(
            cs.classify_change(["site/scripts/capture-zk-manifest.mjs"], self.real_map),
            "engine",
        )

    def test_a_safe_entry_cannot_rescue_a_full_run_trigger(self):
        # The selector resolves §4.1 triggers BEFORE the map (step 1 wins over step
        # 3), so a (bogus) safe entry covering a trigger must not make the class
        # inert — otherwise the class would be MORE permissive than the mode.
        bogus = [{"pattern": ".github/**", "safe": True},
                 {"pattern": "Cargo.lock", "safe": True}]
        self.assertEqual(cs.classify_change([".github/workflows/ci.yml"], bogus), "engine")
        self.assertEqual(cs.classify_change(["Cargo.lock"], bogus), "engine")

    def test_a_safe_entry_cannot_rescue_a_crate_owned_path(self):
        # Crate-prefix ownership (step 2) also wins over the map (step 3). The
        # classifier has no cargo metadata by design, so it uses the `crates/`
        # prefix — every workspace member lives there (root Cargo.toml `members`).
        bogus = [{"pattern": "crates/**", "safe": True}]
        self.assertEqual(cs.classify_change(["crates/app/src/lib.rs"], bogus), "engine")

    def test_malformed_map_entry_fails_closed_to_engine(self):
        # A matched entry that is neither safe, crates, nor readers makes
        # apply_ownership_map raise; the classifier must swallow it as "not proven
        # inert" rather than propagate a class.
        broken = [{"pattern": "site/**", "crates": "not-a-list"}]
        self.assertEqual(cs.classify_change(["site/index.tsx"], broken), "engine")

    # --- composition with the other inert surfaces ---
    def test_map_safe_plus_engine_is_mixed(self):
        self.assertEqual(
            cs.classify_change(["site/index.tsx", "crates/app/src/lib.rs"], self.real_map),
            "mixed",
        )

    def test_map_safe_plus_docs_is_inert_mixed(self):
        self.assertEqual(
            cs.classify_change(["site/index.tsx", "docs/x.md"], self.real_map),
            "inert-mixed",
        )

    def test_docs_and_research_still_classify_docs_only(self):
        # research/** and docs/** are safe-listed in the map AND on
        # _DOCS_ONLY_PREFIXES; the earlier arm must keep winning so the existing
        # token (and the batcher/gate attribution built on it) does not churn.
        self.assertEqual(
            cs.classify_change(["docs/x.md", "research/y.md"], self.real_map), "docs-only"
        )

    def test_every_reachable_safe_pattern_in_the_real_map_classifies_inert(self):
        # DRIFT GUARD: the class layer must stay wired to the map. If
        # classify_change ever stops consulting it, a safe-listed sample path falls
        # back to `engine` and this reddens. Patterns the selector resolves BEFORE
        # the map (triggers / crates/**) are excluded — for those the two layers
        # already agree on `engine`.
        checked = 0
        for entry in self.real_map:
            if entry.get("safe") is not True:
                continue
            pattern = entry["pattern"]
            sample = pattern[:-3] + "/__probe__.txt" if pattern.endswith("/**") else pattern
            if sample.startswith("crates/") or cs._trigger_match(sample) is not None:
                continue
            checked += 1
            self.assertIn(
                cs.classify_change([sample], self.real_map), cs._INERT_CLASSES,
                f"safe-listed {pattern!r} is inert for the closure but not for the class",
            )
        self.assertGreater(checked, 0, "no reachable safe entries found — map moved?")

    def test_mutation_always_map_safe_reddens_the_engine_fixture(self):
        # MUTATION SPOT-CHECK: break _map_safe_match to always-True and the engine
        # fixture must wrongly go inert — proving the real (False) path is what keeps
        # an engine diff running, i.e. these assertions are not vacuous.
        orig = cs._map_safe_match
        try:
            cs._map_safe_match = lambda _p, _m: True
            self.assertEqual(
                cs.classify_change(["crates/app/src/lib.rs"], self.real_map), "map-safe",
                "mutation should swallow the engine path as inert",
            )
        finally:
            cs._map_safe_match = orig
        self.assertEqual(
            cs.classify_change(["crates/app/src/lib.rs"], self.real_map), "engine"
        )


# [FABLE-5] ---- classify-only mode (merge-group change-class gate; #3420/#3421 follow-up)
class ClassifyOnlyMainTests(unittest.TestCase):
    """The `--classify-only` CLI contract the ci.yml / feature-matrix.yml `changes`
    pre-jobs consume on merge_group: stdout is EXACTLY one class token, the
    `change_class=` output line is written, NO cargo metadata is ever needed, and
    EVERY error path fails safe to `engine` (=> the consumer runs the full matrix)
    with exit 0."""

    def _run_main(self, argv):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = cs.main(argv)
        return code, buf.getvalue()

    def _write(self, text, suffix=".txt"):
        fd, path = tempfile.mkstemp(suffix=suffix)
        with os.fdopen(fd, "w") as fh:
            fh.write(text)
        self.addCleanup(lambda: os.path.exists(path) and os.remove(path))
        return path

    def test_docs_only_batch_classifies_docs_only(self):
        # The #2533-shaped merge-group batch: pure docs/research prose. MUTATION
        # VISIBILITY (brief): breaking classify_change back to always-`engine`
        # makes THIS assertion fail — the class-gated merge-group skip cannot
        # silently regress to always-full without a red test.
        changed = self._write("docs/branch-protection.md\nresearch/design-record.md\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "docs-only")

    def test_orchestration_only_batch_classifies_orchestration_only(self):
        changed = self._write("orchestration/routing.toml\nscripts/triage.py\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "orchestration-only")

    def test_engine_batch_classifies_engine(self):
        changed = self._write("crates/app/src/lib.rs\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "engine")

    def test_mixed_batch_classifies_mixed(self):
        # A mixed batch is NOT a skip class in the workflow case-arm => full run.
        changed = self._write("docs/x.md\ncrates/app/src/lib.rs\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "mixed")

    def test_stdout_is_exactly_one_token_line(self):
        # The shell consumer does `cls="$(...)"` and `case`s on the WHOLE value:
        # any extra stdout line would fall into the wildcard arm (safe, but the
        # single-line contract is what makes the gate legible) — pin it.
        changed = self._write("docs/x.md\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed])
        self.assertEqual(code, 0)
        self.assertEqual(out.splitlines(), ["docs-only"])

    def test_output_and_summary_files_carry_the_class(self):
        changed = self._write("docs/x.md\n")
        out_file = self._write("")
        summary = self._write("")
        code, _ = self._run_main(["--classify-only", "--event", "merge_group",
                                  "--changed-file", changed,
                                  "--output-file", out_file, "--summary-file", summary])
        self.assertEqual(code, 0)
        self.assertIn("change_class=docs-only", Path(out_file).read_text(encoding="utf-8"))
        self.assertIn("docs-only", Path(summary).read_text(encoding="utf-8"))

    def test_no_cargo_metadata_needed_even_when_metadata_is_broken(self):
        # classify-only must never touch cargo metadata: a bogus --metadata-file
        # is simply ignored (the whole point — the changes pre-job has no
        # toolchain and pays no metadata cost).
        changed = self._write("docs/x.md\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed,
                                    "--metadata-file", "/no/such/meta.json"])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "docs-only")

    def test_unresolvable_diff_fails_safe_to_engine(self):
        # Garbage SHAs => git diff fails => class engine (full run), exit 0 —
        # the #3421 fail-safe posture (cost, never soundness).
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--base", "deadbeefdeadbeef",
                                    "--head", "cafebabecafebabe"])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "engine")

    def test_missing_base_fails_safe_to_engine(self):
        code, out = self._run_main(["--classify-only", "--event", "merge_group"])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "engine")

    def test_non_diff_event_is_engine(self):
        # schedule/push/workflow_dispatch carry no batch diff => engine (full).
        for event in ("schedule", "push", "workflow_dispatch"):
            code, out = self._run_main(["--classify-only", "--event", event])
            self.assertEqual(code, 0)
            self.assertEqual(out.strip(), "engine", f"event {event}")

    def test_full_override_is_engine(self):
        changed = self._write("docs/x.md\n")
        code, out = self._run_main(["--classify-only", "--full", "--event", "merge_group",
                                    "--changed-file", changed])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "engine")

    # [OPUS-5] #5249: the classify-only entry point now loads the ownership map
    # (still no cargo metadata / toolchain) so a `safe = true` batch is inert at the
    # class layer too — this is the CLI contract the three `changes` pre-jobs consume.
    def test_site_only_batch_classifies_map_safe_via_the_default_map(self):
        # --repo-root pins the map discovery the CI step gets from `git rev-parse`,
        # so the test does not depend on the runner's CWD.
        changed = self._write("site/index.tsx\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed,
                                    "--repo-root", str(REPO_ROOT)])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "map-safe")

    def test_explicit_map_is_honoured(self):
        changed = self._write("site/index.tsx\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed, "--map", str(MAP_FILE)])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "map-safe")

    def test_unreadable_map_degrades_to_engine_not_to_a_crash(self):
        # A missing/malformed map must degrade to the pre-#5249 classifier (no safe
        # rescue => engine => full run), never abort the gate.
        changed = self._write("site/index.tsx\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed, "--map", "/no/such/map.toml"])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "engine")

    def test_a_malformed_map_does_not_taint_an_orchestration_batch(self):
        # The map load is BEST-EFFORT here (unlike the selector's fail-closed raise):
        # a broken map must not reclassify an unrelated orchestration-only batch as
        # engine — degrading to no-map is already the conservative direction.
        bad_map = self._write("this is not = valid toml [[[\n", suffix=".toml")
        changed = self._write("orchestration/routing.toml\nscripts/triage.py\n")
        code, out = self._run_main(["--classify-only", "--event", "merge_group",
                                    "--changed-file", changed, "--map", bad_map])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "orchestration-only")


class OrchestrationSafeInertnessTests(unittest.TestCase):
    """[OPUS-4.8] THE INERTNESS OBLIGATION: every _ORCHESTRATION_SAFE entry must be
    PROVEN not read by any Rust-CI workflow. This greps the real Rust-CI workflow
    files for a reference to each allowlisted SCRIPT/WORKFLOW and FAILS if one is
    referenced — so an entry can never silently become unsound when a script is later
    wired into a gate. Directory prefixes (orchestration/, .claude/, .beads/) are
    audited by convention (never cargo-compiled) and exempt from the grep."""

    # The workflows that run cargo build/test/clippy/coverage/bench/fuzz/CodeQL and
    # therefore MUST NOT reference an orchestration-safe script.
    _RUST_CI_WORKFLOWS = [
        "ci.yml", "feature-matrix.yml", "codeql.yml", "supply-chain.yml",
        "bench.yml", "fuzz.yml", "miri.yml", "asan.yml", "kani.yml",
        "metamorph.yml", "vectorized-feature-off.yml", "ci-select.yml",
        "ci-summary.yml", "formal-verification.yml", "differential.yml",
        "shacl-diff-fuzz.yml", "nightly-full-sweep.yml",
        "datalog-souffle.yml",
    ]

    def test_no_orch_safe_script_is_referenced_by_a_rust_ci_workflow(self):
        wf_dir = REPO_ROOT / ".github" / "workflows"
        corpus = []
        for name in self._RUST_CI_WORKFLOWS:
            p = wf_dir / name
            if p.exists():
                corpus.append(p.read_text(encoding="utf-8"))
        blob = "\n".join(corpus)
        for entry in cs._ORCHESTRATION_SAFE:
            if entry.endswith("/"):
                continue  # directory prefixes: never cargo-compiled (audited by convention)
            if entry.startswith(".github/workflows/"):
                # An orchestration WORKFLOW file: it must not itself be a Rust-CI wf.
                self.assertNotIn(
                    Path(entry).name, self._RUST_CI_WORKFLOWS,
                    f"{entry} is listed as orchestration-safe but is a Rust-CI workflow",
                )
                continue
            # A script: it must not be referenced anywhere in a Rust-CI workflow.
            self.assertNotIn(
                entry, blob,
                f"orchestration-safe {entry} IS referenced by a Rust-CI workflow — it is "
                f"NOT inert; remove it from _ORCHESTRATION_SAFE (fail-closed: a referenced "
                f"script must keep triggering the full matrix).",
            )

    def test_orch_safe_scripts_exist_on_disk(self):
        # A stale allowlist entry (script deleted/renamed) should be caught, else it
        # silently covers nothing. Directory prefixes are checked as dirs.
        for entry in cs._ORCHESTRATION_SAFE:
            path = REPO_ROOT / entry.rstrip("/")
            self.assertTrue(
                path.exists(),
                f"orchestration-safe entry {entry} does not exist on disk (stale allowlist)",
            )


class DeployOnlyInertnessTests(unittest.TestCase):
    """[OPUS-5] sq-g25hr: THE INERTNESS OBLIGATION for `_DEPLOY_ONLY`, the same
    contract OrchestrationSafeInertnessTests enforces for the orchestration
    allowlist. Every entry must be PROVEN not read by any Rust-CI workflow, so an
    entry can never silently become unsound when a deploy path is later wired into
    a Rust gate."""

    _RUST_CI_WORKFLOWS = OrchestrationSafeInertnessTests._RUST_CI_WORKFLOWS

    def test_no_deploy_entry_is_referenced_by_a_rust_ci_workflow(self):
        wf_dir = REPO_ROOT / ".github" / "workflows"
        corpus = []
        for name in self._RUST_CI_WORKFLOWS:
            p = wf_dir / name
            if p.exists():
                corpus.append(p.read_text(encoding="utf-8"))
        blob = "\n".join(corpus)
        for entry in cs._DEPLOY_ONLY:
            if entry.startswith(".github/workflows/"):
                # A deploy WORKFLOW file: it must not itself be a Rust-CI workflow.
                self.assertNotIn(
                    Path(entry).name, self._RUST_CI_WORKFLOWS,
                    f"{entry} is listed as deploy-only but is a Rust-CI workflow",
                )
                continue
            if entry.endswith("/"):
                # Directory prefixes: same convention as the orchestration allowlist
                # (a bare substring grep would match the word in a Rust-CI workflow's
                # own PROSE). `deploy/` is instead enforced LIVE by two stronger,
                # non-textual checks: scripts/ci_audit_inputs.py fails if any crate
                # reads a SAFE-listed path, and test_no_crate_lives_under_a_deploy_entry
                # below fails if a workspace member ever appears there.
                continue
            self.assertNotIn(
                entry, blob,
                f"deploy-only {entry} IS referenced by a Rust-CI workflow — it is NOT "
                f"inert; remove it from _DEPLOY_ONLY (fail-closed: a referenced path "
                f"must keep triggering the full matrix).",
            )

    def test_deploy_entries_exist_on_disk(self):
        for entry in cs._DEPLOY_ONLY:
            path = REPO_ROOT / entry.rstrip("/")
            self.assertTrue(
                path.exists(),
                f"deploy-only entry {entry} does not exist on disk (stale allowlist)",
            )

    def test_no_crate_lives_under_a_deploy_entry(self):
        # A workspace crate under deploy/ would make the whole allowlist unsound.
        # (The complementary "no crate READS deploy/" claim is enforced live by
        # scripts/ci_audit_inputs.py via the `deploy/**  safe = true` map entry.)
        self.assertEqual(
            list((REPO_ROOT / "deploy").rglob("Cargo.toml")), [],
            "a Cargo manifest appeared under deploy/ — it is no longer inert for the "
            "Rust matrix; remove deploy/ from _DEPLOY_ONLY and from the SAFE list",
        )

    def test_deploy_only_is_disjoint_from_orchestration_safe(self):
        # One path, one class: an overlap would make the change-class ambiguous
        # (classify_change consults orchestration FIRST, so the overlap would be
        # silently mislabelled rather than caught).
        self.assertEqual(
            set(cs._DEPLOY_ONLY) & set(cs._ORCHESTRATION_SAFE), set(),
            "an entry is on BOTH inert allowlists — pick one so the audit-trail "
            "class is unambiguous",
        )


class RealMetadataShapeTests(unittest.TestCase):
    """(i) Pinned against the REAL workspace metadata: core is root-like, geo is
    leaf-like, and closure(geo) is a subset of closure(core) (structural: geo
    depends on core, so anything depending on geo depends on core)."""

    @classmethod
    def setUpClass(cls):
        cls.meta = None
        if shutil.which("cargo") is None:
            return
        try:
            out = subprocess.run(
                ["cargo", "metadata", "--no-deps", "--format-version", "1"],
                cwd=str(REPO_ROOT), check=True, capture_output=True, text=True, timeout=180,
            )
            cls.meta = json.loads(out.stdout)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError):
            cls.meta = None

    def setUp(self):
        if self.meta is None:
            self.skipTest("cargo metadata unavailable (hermetic skip)")

    def test_core_root_like_geo_leaf_like(self):
        ws = cs.parse_workspace(self.meta)
        self.assertIn("sparq-core", ws.members)
        self.assertIn("sparq-geo", ws.members)
        core = cs.reverse_closure("sparq-core", ws.reverse_adj)
        geo = cs.reverse_closure("sparq-geo", ws.reverse_adj)
        n = len(ws.members)
        # core is root-like: a large majority of the workspace depends on it.
        self.assertGreaterEqual(len(core), n // 2)
        # geo is leaf-like: only a small handful depend on it.
        self.assertLess(len(geo), n // 4)
        # structural invariants that survive crate additions:
        self.assertIn("sparq-geo", core)               # geo depends on core
        self.assertNotIn("sparq-core", geo)            # core does not depend on geo
        self.assertTrue(geo.issubset(core))            # closure(geo) subset of closure(core)
        self.assertGreater(len(core), len(geo))        # core strictly more root-like

    def test_selected_mode_on_real_leaf_change(self):
        sel = cs.select(["crates/sparq-geo/src/lib.rs"], self.meta)
        self.assertEqual(sel.mode, "selected")
        self.assertIn("sparq-geo", sel.affected)
        self.assertNotIn("sparq-parse", sel.affected)  # parse does not depend on geo

    # ---- sq-m4bxc additional-readers acceptance, REAL metadata + REAL map ------
    # This uses the SHIPPED ci/path-ownership.toml `readers` entry against live
    # cargo metadata: a change to sparq-solid's rule corpus must select
    # sparq-reason (sparq-solid depends on sparq-reason — the reverse cycle).
    #
    # [OPUS-5] sq-3705: the secprop vocabulary needs NO map entry any more. It was
    # the other residual — sparq-zk `include_str!`d sparq-trust's secprop-ext.ttl
    # because a sparq-zk->sparq-trust edge is a cycle — and it is now closed
    # structurally instead, by a real dependency on a zero-dep leaf crate.
    def test_secprop_vocab_change_selects_its_consumers(self):
        real_map = cs.load_ownership_map(str(MAP_FILE))
        sel = cs.select(
            ["crates/sparq-secprop-vocab/ontologies/secprop-ext.ttl"],
            self.meta, real_map,
        )
        self.assertEqual(sel.mode, "selected")
        # Reached by the ORDINARY reverse-dependency closure: each consumer takes
        # an (optional) `sparq-secprop-vocab` edge, and the selector's graph is
        # feature-blind, so all three are attributed with no `readers` entry.
        for consumer in ("sparq-zk", "sparq-trust", "sparq-policy"):
            self.assertIn(
                consumer, sel.affected,
                f"a secprop-ext.ttl change must run {consumer}'s secprop tests",
            )

    def test_secprop_vocab_needs_no_readers_entry(self):
        # The same selection holds with an EMPTY map — proof it is the cargo
        # edges doing the work, not a path-ownership patch (sq-3705).
        sel = cs.select(
            ["crates/sparq-secprop-vocab/ontologies/secprop-ext.ttl"],
            self.meta, [],
        )
        self.assertEqual(sel.mode, "selected")
        for consumer in ("sparq-zk", "sparq-trust", "sparq-policy"):
            self.assertIn(consumer, sel.affected)

    def test_solid_rules_change_selects_reason(self):
        real_map = cs.load_ownership_map(str(MAP_FILE))
        sel = cs.select(["crates/sparq-solid/rules/wac.n3"], self.meta, real_map)
        self.assertEqual(sel.mode, "selected")
        self.assertIn("sparq-reason", sel.affected,
                      "a sparq-solid/rules change must run sparq-reason's N3-equivalence tests")
        self.assertIn("sparq-solid", sel.affected)

    # ---- phase-2 lane mapping against REAL metadata (bead sq-fmx4u.6) ----------
    def test_lane_seed_crates_are_real_workspace_members(self):
        # A seed that is not a current member would make its lane skip exactly when
        # it should run (`lane_runs` fail-closes it to RUN, but the intent is that
        # the list stays real). Pin every fuzz/wasm seed against the live metadata.
        ws = cs.parse_workspace(self.meta)
        for lane, seeds in cs._LANE_SEEDS.items():
            for seed in seeds:
                self.assertIn(seed, ws.members,
                              f"{lane} seed {seed!r} is not a workspace member")

    def test_geo_only_pr_skips_fuzz_and_wasm(self):
        # ACCEPTANCE: a geo-only PR skips the sparq-core fuzz smoke + wasm builds.
        # sparq-geo is a reverse-graph leaf; none of the fuzz/wasm seeds depend on
        # it, so neither lane is affected (skipped-green).
        sel = cs.select(["crates/sparq-geo/src/lib.rs"], self.meta)
        self.assertEqual(sel.mode, "selected")
        self.assertFalse(cs.lane_runs(sel, cs._LANE_SEEDS["fuzz"]),
                         "geo-only PR must SKIP the fuzz lane")
        self.assertFalse(cs.lane_runs(sel, cs._LANE_SEEDS["wasm"]),
                         "geo-only PR must SKIP the wasm lane")

    def test_core_pr_runs_fuzz_and_wasm(self):
        # ACCEPTANCE: a sparq-core PR still runs both — every fuzz seed and every
        # wasm bundle depends (transitively) on sparq-core, so both are affected.
        sel = cs.select(["crates/sparq-core/src/dict.rs"], self.meta)
        self.assertEqual(sel.mode, "selected")
        self.assertTrue(cs.lane_runs(sel, cs._LANE_SEEDS["fuzz"]),
                        "sparq-core PR must RUN the fuzz lane")
        self.assertTrue(cs.lane_runs(sel, cs._LANE_SEEDS["wasm"]),
                        "sparq-core PR must RUN the wasm lane")

    # ---- bench (perf-gate) lane acceptance, real metadata (bead sq-mel85) ------
    def test_engine_pr_runs_bench(self):
        # ACCEPTANCE (sq-mel85): an engine-touching PR runs the perf gate — sparq-engine
        # is a direct bench seed (and the store/dict/parse hard-gated metrics could move).
        sel = cs.select(["crates/sparq-engine/src/lib.rs"], self.meta)
        self.assertEqual(sel.mode, "selected")
        self.assertTrue(cs.lane_runs(sel, cs._LANE_SEEDS["bench"]),
                        "engine-touching PR must RUN the bench lane")

    def test_core_pr_runs_bench(self):
        # A sparq-core change moves the HARD-GATED store/dict/parse byte metrics; core is
        # a dep of every bench seed's release binary, so it lands in the affected closure.
        sel = cs.select(["crates/sparq-core/src/dict.rs"], self.meta)
        self.assertTrue(cs.lane_runs(sel, cs._LANE_SEEDS["bench"]),
                        "sparq-core PR must RUN the bench lane (store/dict/parse floors)")

    def test_wasm_only_pr_runs_bench(self):
        # SOUNDNESS (sq-mel85): the wasm_bundle_bytes floor is enforced ONLY in bench.yml,
        # and a wasm-only diff does NOT flow up into engine/cli/bench — so sparq-wasm is a
        # DIRECT bench seed and a wasm-only PR must still RUN the perf gate.
        sel = cs.select(["crates/sparq-wasm/src/lib.rs"], self.meta)
        self.assertEqual(sel.mode, "selected")
        self.assertTrue(cs.lane_runs(sel, cs._LANE_SEEDS["bench"]),
                        "wasm-only PR must RUN the bench lane (wasm_bundle_bytes floor)")

    def test_isolated_crate_pr_skips_bench(self):
        # ACCEPTANCE (sq-mel85): a PR to a crate that NO bench seed depends on skips the
        # perf gate. sparq-rsp is isolated (its bench is a standalone example, and neither
        # sparq-engine/-cli/-bench/-wasm depends on it), so the bench closure is unaffected
        # => skipped-green. Its trend-only latency comment is informational, not a gate.
        sel = cs.select(["crates/sparq-rsp/src/lib.rs"], self.meta)
        self.assertEqual(sel.mode, "selected")
        self.assertFalse(cs.lane_runs(sel, cs._LANE_SEEDS["bench"]),
                         "isolated-crate PR (sparq-rsp) must SKIP the bench lane")

    def test_cli_linked_crate_runs_bench_conservatively(self):
        # HONEST over-run (sq-mel85): the seed set is "sparq-engine + the release binaries"
        # (bead spec). sparq-cli transitively depends on sparq-geo (sparq-cli -> sparq-server
        # -> sparq-geo), so a geo change rebuilds a benchmarked release binary and the bench
        # lane RUNS. This is a conservative over-run (a geo change cannot move a
        # PR-tier-measured hard-gated metric — store/dict/parse are sparq-core, wasm_bundle
        # is sparq-wasm; geo_compliance_deficit is mode:auto but main-tier-only in ci-bench.sh,
        # where mode=full always), never an unsound skip. Pinned so the behaviour is a
        # documented decision, not a surprise.
        ws = cs.parse_workspace(self.meta)
        self.assertIn("sparq-cli", cs.reverse_closure("sparq-geo", ws.reverse_adj),
                      "test premise: sparq-cli depends transitively on sparq-geo")
        sel = cs.select(["crates/sparq-geo/src/lib.rs"], self.meta)
        self.assertTrue(cs.lane_runs(sel, cs._LANE_SEEDS["bench"]),
                        "geo change rebuilds the sparq-cli release binary => bench RUNS")

    def test_docs_only_pr_skips_bench(self):
        # ACCEPTANCE (sq-mel85): a docs-only PR (SAFE-listed research/**) selects an
        # EMPTY closure, so the perf gate is inert => skipped-green. Real metadata (so the
        # bench seeds are known members) + the research SAFE map entry.
        m = [{"pattern": "research/**", "safe": True}]
        sel = cs.select(["research/change-based-test-selection.md"], self.meta, m)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])
        self.assertFalse(cs.lane_runs(sel, cs._LANE_SEEDS["bench"]),
                         "docs-only PR must SKIP the bench lane")


class LaneMappingTests(unittest.TestCase):
    """[OPUS-4.8] sq-fmx4u.6 (design §5.2, phase 2): hermetic tests of
    `lane_runs` — the executable spec of the fuzz.yml + ci.yml `wasm` job `if:`
    guards — and the SAFE-only coverage-ratchet skip. All synthetic (no cargo)."""

    def _sel(self, mode, affected, members):
        return cs.Selection(mode=mode, reason="", affected=list(affected),
                            all_members=list(members))

    def test_lane_runs_when_a_seed_is_affected(self):
        sel = self._sel("selected", ["sparq-core", "sparq-geo"],
                        ["sparq-core", "sparq-geo", "sparq-wasm"])
        self.assertTrue(cs.lane_runs(sel, ["sparq-core"]))

    def test_lane_skips_when_no_seed_is_affected(self):
        sel = self._sel("selected", ["sparq-geo"],
                        ["sparq-core", "sparq-geo", "sparq-wasm"])
        self.assertFalse(cs.lane_runs(sel, ["sparq-core", "sparq-wasm"]))

    def test_full_and_shadow_modes_always_run_the_lane(self):
        # full (nightly backstop / ci-full / error) + shadow (report-only) => RUN.
        for mode in ("full", "shadow"):
            sel = self._sel(mode, [], ["sparq-core"])
            self.assertTrue(cs.lane_runs(sel, ["sparq-core"]),
                            f"mode={mode} must always run the lane")

    def test_selected_empty_affected_skips_the_lane(self):
        # SAFE-only change: mode=selected, affected=[] => the lane is inert => skip.
        sel = self._sel("selected", [], ["sparq-core"])
        self.assertFalse(cs.lane_runs(sel, ["sparq-core"]))

    def test_unknown_seed_forces_run_fail_closed(self):
        # A typo'd/renamed seed cannot silently skip its lane: lane_runs RUNS it.
        sel = self._sel("selected", ["sparq-geo"], ["sparq-core", "sparq-geo"])
        self.assertTrue(cs.lane_runs(sel, ["sparq-core-TYPO"]),
                        "an unknown seed must fail-closed to RUN")

    def test_lane_seeds_are_the_expected_lanes(self):
        # [SONNET-4.6] sq-mel85 added the `bench` lane to the fuzz + wasm phase-2 set.
        # [FABLE-5] sq-0iqzw added `differential-smoke` (fuzz.yml PR-level blocking
        # sparq-vs-Oxigraph differential regression windows).
        self.assertEqual(set(cs._LANE_SEEDS),
                         {"fuzz", "wasm", "bench", "differential-smoke"})
        self.assertEqual(cs._LANE_SEEDS["fuzz"],
                         ["sparq-core", "sparq-engine", "sparq-shacl"])
        self.assertIn("sparq-wasm", cs._LANE_SEEDS["wasm"])
        self.assertIn("sparq-solid", cs._LANE_SEEDS["wasm"])
        # bench (perf gate): sparq-engine + the release binaries + sparq-wasm (the
        # wasm_bundle_bytes floor is enforced only in bench.yml — a direct seed).
        self.assertEqual(cs._LANE_SEEDS["bench"],
                         ["sparq-engine", "sparq-cli", "sparq-bench", "sparq-wasm"])
        # differential-smoke: the engine under test + the harness/oracle crate.
        self.assertEqual(cs._LANE_SEEDS["differential-smoke"],
                         ["sparq-core", "sparq-engine", "sparq-bench"])

    # ---- SAFE-only coverage-ratchet skip (acceptance) -------------------------
    @staticmethod
    def _coverage_ratchet_skips(sel):
        """Mirror the ci.yml `coverage-measure` job `if:` disjunction:
            mode != 'selected' || affected != '[]'   (RUN when either holds)
        so the ratchet is SKIPPED iff mode == 'selected' AND affected == []."""
        return sel.mode == "selected" and json.dumps(sel.affected) == "[]"

    def test_safe_only_pr_skips_coverage_ratchet(self):
        # ACCEPTANCE: an affected-empty (SAFE-only) PR skips the coverage ratchet.
        m = [{"pattern": "research/**", "safe": True}]
        sel = cs.select(["research/x.md"], _synthetic_meta(), m)
        self.assertEqual(sel.mode, "selected")
        self.assertEqual(sel.affected, [])
        self.assertTrue(self._coverage_ratchet_skips(sel))

    def test_nonempty_affected_keeps_coverage_ratchet_running(self):
        # A non-empty closure keeps coverage always-run (a dep change can shift
        # executed lines in dependents — design §5.2).
        sel = cs.select(["crates/app/src/lib.rs"], _synthetic_meta())
        self.assertEqual(sel.mode, "selected")
        self.assertFalse(self._coverage_ratchet_skips(sel))

    def test_full_mode_keeps_coverage_ratchet_running(self):
        sel = cs.select(["Cargo.lock"], _synthetic_meta())
        self.assertEqual(sel.mode, "full")
        self.assertFalse(self._coverage_ratchet_skips(sel))


class EnforceRolloutTests(unittest.TestCase):
    """[FABLE-5] sq-fmx4u.5: the ENFORCE-mode rollout invariants — the selector
    outputs the wide-lane skip guards consume once the shadow rollout is flipped
    OFF (enforce is the default). Under enforce (no --shadow) the affected closure
    is the RUN set: a crate in it RUNS, a crate absent from it is SKIPPED. Every
    fail-safe path (§4.1 triggers, the ci-full override, non-PR events) still
    resolves to mode=full even without shadow — enforce never weakens fail-safe."""

    def _run_main(self, argv):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = cs.main(argv)
        return code, json.loads(buf.getvalue())

    def _write(self, text, suffix=".json"):
        fd, path = tempfile.mkstemp(suffix=suffix)
        with os.fdopen(fd, "w") as fh:
            fh.write(text)
        self.addCleanup(lambda: os.path.exists(path) and os.remove(path))
        return path

    @staticmethod
    def _guard_runs(affected, crate):
        """Exact mirror of the ci.yml shard skip guard's membership test:
            case "$SELECT_AFFECTED" in *"\\"$SHARD_CRATE\\""*) run ;; *) skip ;;
        i.e. the QUOTED needle '"<crate>"' must be a substring of the affected JSON.
        `affected` is the parsed list; we re-serialise with json.dumps exactly as
        ci_select.py writes the $GITHUB_OUTPUT `affected=` line the guard reads."""
        return f'"{crate}"' in json.dumps(affected)

    def _selected(self, changed_line):
        meta = self._write(json.dumps(_synthetic_meta()))
        changed = self._write(changed_line, suffix=".txt")
        # No --shadow: this is ENFORCE (the sq-fmx4u.5 default).
        code, obj = self._run_main(["--metadata-file", meta, "--changed-file", changed,
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        return obj

    def test_enforce_affected_crate_runs(self):
        # An `engine` change under enforce => mode=selected; engine + its dependent
        # `app` are in the closure, so the shard guard RUNS both.
        obj = self._selected("crates/engine/src/lib.rs\n")
        self.assertEqual(obj["mode"], "selected")
        self.assertTrue(self._guard_runs(obj["affected"], "engine"), "affected -> RUNS")
        self.assertTrue(self._guard_runs(obj["affected"], "app"), "dependent -> RUNS")

    def test_enforce_not_affected_crate_skips(self):
        # An `app` change under enforce => `app` is a reverse-graph leaf, so nothing
        # else is affected; `core`/`parse`/`engine` are NOT in the closure and the
        # shard guard SKIPS them (exit 0 — gate-satisfied via `skipped`, no hang;
        # the merge-queue safety is pinned by TestRequiredCheckAnchor / sq-fmx4u.4).
        obj = self._selected("crates/app/src/lib.rs\n")
        self.assertEqual(obj["mode"], "selected")
        self.assertEqual(obj["affected"], ["app"])
        for skipped in ("core", "parse", "engine"):
            self.assertFalse(self._guard_runs(obj["affected"], skipped),
                             f"{skipped} not affected -> shard guard SKIPS it")

    def test_enforce_ci_full_label_path_is_full(self):
        # The `ci-full` label maps to ci_select.py --full: mode=full even without
        # --shadow, and every member is in `affected` (everything runs).
        meta = self._write(json.dumps(_synthetic_meta()))
        code, obj = self._run_main(["--full", "--metadata-file", meta, "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")
        self.assertEqual(obj["affected"], ALL_MEMBERS)

    def test_enforce_nightly_schedule_is_full(self):
        # The nightly full-matrix backstop: a schedule event carries no PR diff, so
        # the selector returns full even without --shadow.
        meta = self._write(json.dumps(_synthetic_meta()))
        code, obj = self._run_main(["--event", "schedule", "--metadata-file", meta,
                                    "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")
        self.assertEqual(obj["affected"], ALL_MEMBERS)

    def test_enforce_failsafe_trigger_stays_full(self):
        # FAIL-SAFE PRESERVED: a §4.1 trigger (Cargo.lock) under enforce (NO --shadow)
        # still forces full — enforce must never skip a shared/build-file change even
        # when an otherwise-narrow crate change rides alongside it.
        obj = self._selected("Cargo.lock\ncrates/app/src/lib.rs\n")
        self.assertEqual(obj["mode"], "full")
        self.assertEqual(obj["affected"], ALL_MEMBERS)
        # And every crate's guard consequently RUNS (nothing skipped under full).
        for crate in ALL_MEMBERS:
            self.assertTrue(self._guard_runs(obj["affected"], crate))

    def test_enforce_selector_error_stays_full(self):
        # FAIL-CLOSED PRESERVED under enforce: a selector error (bad metadata) with
        # NO --shadow still resolves to full, exit 0 — nothing is skipped on error.
        changed = self._write("crates/app/src/lib.rs\n", suffix=".txt")
        code, obj = self._run_main(["--metadata-file", "/no/such/meta.json",
                                    "--changed-file", changed, "--repo-root", ROOT])
        self.assertEqual(code, 0)
        self.assertEqual(obj["mode"], "full")

    def test_enforce_never_emits_shadow(self):
        # Without --shadow the mode is never 'shadow' (enforce honors selected/full).
        for changed in ("crates/app/src/lib.rs\n", "Cargo.lock\n", "docs/x.md\n"):
            obj = self._selected(changed)
            self.assertIn(obj["mode"], ("selected", "full"))
            self.assertNotEqual(obj["mode"], "shadow")

    def test_guard_needle_quotes_prevent_prefix_collision(self):
        # MUTATION CHECK. A guard that dropped the quote delimiters (a bare-name
        # substring match) would treat `sparq-core` as affected whenever
        # `sparq-core-foo` is — a SILENT UNSOUND run of a crate selection proved
        # unaffected. Prove the REAL guard rule (the QUOTED needle) distinguishes
        # them and the mutant (unquoted substring) does not.
        affected = ["sparq-core-foo"]
        self.assertTrue(self._guard_runs(affected, "sparq-core-foo"))
        self.assertFalse(self._guard_runs(affected, "sparq-core"),
                         "the quoted needle must NOT match the prefix crate")
        # The mutant's unquoted substring test WOULD wrongly match:
        mutant_match = "sparq-core" in json.dumps(affected)
        self.assertTrue(mutant_match, "sanity: the bare substring IS present")
        self.assertNotEqual(
            mutant_match, self._guard_runs(affected, "sparq-core"),
            "the real (quoted) guard and the mutant (unquoted) must disagree here — "
            "that disagreement is exactly the soundness the quoting buys",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
