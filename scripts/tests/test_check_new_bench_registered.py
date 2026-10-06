#!/usr/bin/env python3
# Hermetic tests for the proactive merge-gate G3 — new-bench->registry+
# dashboard (bead sq-ncvq.6, epic sq-ncvq).
#
# Hermetic w.r.t. git/network: imports scripts/check-new-bench-registered.py and
# drives its pure evaluate() entry point against FIXTURE diff/registry/dashboard
# strings — NO live git and NO subprocess. main() is exercised only via --dry-run
# + --changed-files + --base-toml so it never shells out (it does read the
# committed in-repo bench/benchmarks.toml + dashboard.js, which are deterministic
# files, not git/network state).
#
# Run:  python3 scripts/tests/test_check_new_bench_registered.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


g3 = _load("check_new_bench_registered", "check-new-bench-registered.py")


def _added(paths: list[str]) -> list[str]:
    """A `git diff --name-status` added-files fixture → the parsed added list."""
    _, added = g3.parse_status_lines([f"A\t{p}" for p in paths])
    return added


def _registry(entries: list[dict]) -> str:
    """Build a minimal benchmarks.toml from a list of entry dicts.

    Each entry: {id, source?, invoke?, featured_false?}. Mirrors the real
    registry's aligned-`=` prose-heavy shape closely enough for the textual
    parser."""
    out = []
    for e in entries:
        out.append("[[benchmark]]")
        out.append(f'id          = "{e["id"]}"')
        if "source" in e:
            out.append(f'source      = "{e["source"]}"')
        if "invoke" in e:
            out.append(f'invoke      = "{e["invoke"]}"')
        if e.get("featured_false"):
            out.append("featured    = false")
        out.append("")
    return "\n".join(out)


_DASHBOARD = (
    "var FEATURED_SUITES = [\n"
    "  { key: 'WatDiv', title: 'WatDiv', aliases: ['watdiv'] },\n"
    "  { key: 'GeoSPARQL', title: 'GeoSPARQL', aliases: ['geosparql', 'geo'] },\n"
    "];\n"
)
_FEATURED = g3.dashboard_featured_text(_DASHBOARD)


# --------------------------------------------------------------------------- #
# T2 — a new bench/<suite>/ directory.
# --------------------------------------------------------------------------- #
class NewSuiteDirTest(unittest.TestCase):
    def test_unregistered_unfeatured_capability_suite_fails_both(self):
        # A brand-new capability suite dir with NO registry entry + NO dashboard
        # row → both (a) and (b) are missing. Two files under the same suite dir
        # must still de-dup to ONE detected suite.
        added = _added(["bench/sparkle/run.sh", "bench/sparkle/queries/q1.rq"])
        v = g3.evaluate(
            added, head_toml=_registry([]), base_toml="", featured_text=_FEATURED
        )
        self.assertEqual(len(v), 1)
        label, missing = v[0]
        self.assertEqual(label, "sparkle")
        self.assertEqual(len(missing), 2)
        joined = " ".join(missing)
        self.assertIn("registered", joined)
        self.assertIn("FEATURED_SUITES", joined)

    def test_registered_and_featured_passes(self):
        # Registered (source references bench/sparkle/) AND its family token is
        # in FEATURED_SUITES → PASS. (We feature it by alias 'geo' here via a
        # geo-family suite to reuse the fixture dashboard.)
        added = _added(["bench/geo-extra/run.sh"])
        reg = _registry([{"id": "geo-extra-bench", "source": "bench/geo-extra/run.sh"}])
        v = g3.evaluate(added, head_toml=reg, base_toml="", featured_text=_FEATURED)
        self.assertEqual(v, [])  # 'geo' family token is featured

    def test_registered_but_unfeatured_capability_fails_dashboard_only(self):
        added = _added(["bench/sparkle/run.sh"])
        reg = _registry([{"id": "sparkle-bench", "source": "bench/sparkle/run.sh"}])
        v = g3.evaluate(added, head_toml=reg, base_toml="", featured_text=_FEATURED)
        self.assertEqual(len(v), 1)
        _, missing = v[0]
        self.assertEqual(len(missing), 1)
        self.assertIn("FEATURED_SUITES", missing[0])

    def test_featured_false_flag_waives_dashboard(self):
        added = _added(["bench/sparkle/run.sh"])
        reg = _registry(
            [
                {
                    "id": "sparkle-bench",
                    "source": "bench/sparkle/run.sh",
                    "featured_false": True,
                }
            ]
        )
        v = g3.evaluate(added, head_toml=reg, base_toml="", featured_text=_FEATURED)
        self.assertEqual(v, [])  # featured=false is the escape hatch

    def test_noncapability_family_exempt_from_dashboard(self):
        # A new bench/cli-foo/ dir is a non-capability runner family: registered
        # (to avoid the (a) miss) and NOT featured → still PASS (exempt from b).
        added = _added(["bench/cli-foo/run.sh"])
        reg = _registry([{"id": "cli-foo-bench", "source": "bench/cli-foo/run.sh"}])
        v = g3.evaluate(added, head_toml=reg, base_toml="", featured_text=_FEATURED)
        self.assertEqual(v, [])

    def test_noncapability_family_still_needs_registry(self):
        # Even a non-capability family must be REGISTERED.
        added = _added(["bench/cli-foo/run.sh"])
        v = g3.evaluate(
            added, head_toml=_registry([]), base_toml="", featured_text=_FEATURED
        )
        self.assertEqual(len(v), 1)
        _, missing = v[0]
        self.assertEqual(len(missing), 1)
        self.assertIn("registered", missing[0])

    def test_preexisting_suite_dir_not_flagged(self):
        # A file added under an ALREADY-EXISTING suite dir is not a new suite.
        added = _added(["bench/watdiv/queries/extra.rq"])
        v = g3.evaluate(
            added,
            head_toml=_registry([]),
            base_toml="",
            featured_text=_FEATURED,
            base_suites={"watdiv"},
        )
        self.assertEqual(v, [])

    def test_non_bench_paths_ignored(self):
        added = _added(["crates/sparq-core/src/store.rs", "research/notes.md"])
        v = g3.evaluate(
            added, head_toml=_registry([]), base_toml="", featured_text=_FEATURED
        )
        self.assertEqual(v, [])


# --------------------------------------------------------------------------- #
# T1 — a new benchmarks.toml id (no new dir).
# --------------------------------------------------------------------------- #
class NewRegistryIdTest(unittest.TestCase):
    def test_new_id_capability_unfeatured_fails(self):
        # A new id (present at HEAD, absent at base) for a capability family with
        # no dashboard row → flagged for the dashboard miss (registry is satisfied
        # by construction).
        base = _registry([{"id": "old-bench", "source": "bench/old/run.sh"}])
        head = _registry(
            [
                {"id": "old-bench", "source": "bench/old/run.sh"},
                {"id": "sparkle-bench", "source": "bench/sparkle/run.sh"},
            ]
        )
        # No new DIR added (the dir already existed at base) — only the id is new.
        v = g3.evaluate(
            [],
            head_toml=head,
            base_toml=base,
            featured_text=_FEATURED,
            base_suites={"sparkle", "old"},
        )
        self.assertEqual(len(v), 1)
        label, missing = v[0]
        self.assertEqual(label, "sparkle")  # keyed on the referenced suite dir
        self.assertEqual(len(missing), 1)
        self.assertIn("FEATURED_SUITES", missing[0])

    def test_new_id_featured_passes(self):
        base = _registry([{"id": "old-bench", "source": "bench/old/run.sh"}])
        head = _registry(
            [
                {"id": "old-bench", "source": "bench/old/run.sh"},
                {"id": "watdiv-extra", "source": "bench/watdiv/run.sh"},
            ]
        )
        v = g3.evaluate(
            [],
            head_toml=head,
            base_toml=base,
            featured_text=_FEATURED,
            base_suites={"watdiv", "old"},
        )
        self.assertEqual(v, [])  # watdiv is featured

    def test_new_id_featured_false_passes(self):
        base = _registry([])
        head = _registry(
            [
                {
                    "id": "sparkle-bench",
                    "source": "bench/sparkle/run.sh",
                    "featured_false": True,
                }
            ]
        )
        v = g3.evaluate(
            [],
            head_toml=head,
            base_toml=base,
            featured_text=_FEATURED,
            base_suites={"sparkle"},
        )
        self.assertEqual(v, [])

    def test_dir_and_id_dedup_to_one_violation(self):
        # A PR that adds bench/sparkle/ AND its new id reports ONE violation, not two.
        added = _added(["bench/sparkle/run.sh"])
        head = _registry([{"id": "sparkle-bench", "source": "bench/sparkle/run.sh"}])
        v = g3.evaluate(added, head_toml=head, base_toml="", featured_text=_FEATURED)
        self.assertEqual(len(v), 1)
        self.assertEqual(v[0][0], "sparkle")

    def test_no_new_id_no_new_dir_passes(self):
        reg = _registry([{"id": "old-bench", "source": "bench/old/run.sh"}])
        v = g3.evaluate(
            [], head_toml=reg, base_toml=reg, featured_text=_FEATURED
        )
        self.assertEqual(v, [])


# --------------------------------------------------------------------------- #
# Unit helpers.
# --------------------------------------------------------------------------- #
class HelperTest(unittest.TestCase):
    def test_registry_blocks_parses_featured_false(self):
        text = _registry(
            [
                {"id": "a", "source": "bench/a/x", "featured_false": True},
                {"id": "b", "source": "bench/b/x"},
            ]
        )
        blocks = g3.registry_blocks(text)
        self.assertEqual(len(blocks), 2)
        self.assertTrue(blocks[0]["featured_false"])
        self.assertFalse(blocks[1]["featured_false"])

    def test_block_for_suite_matches_dir_token(self):
        text = _registry([{"id": "a", "invoke": "run bench/foo-bar/run.sh now"}])
        blocks = g3.registry_blocks(text)
        self.assertIsNotNone(g3.block_for_suite(blocks, "foo-bar"))
        self.assertIsNone(g3.block_for_suite(blocks, "foo"))  # prefix must not match

    def test_suite_for_block_does_not_leak_shell_text(self):
        # A multi-word `invoke` that embeds `bench/<suite>/… && cargo bench` must
        # yield the clean suite token, never the trailing shell text. (Real
        # registry source/invoke values carry a trailing slash, e.g.
        # `bench/zk-trace/{Cargo.toml,README.md}`.)
        text = _registry(
            [
                {
                    "id": "x",
                    "invoke": "cd bench/zk-trace/ && cargo bench  # then bench/other/y",
                }
            ]
        )
        block = g3.registry_blocks(text)[0]
        self.assertEqual(g3.suite_for_block(block), "zk-trace")

    def test_suite_family_strips_variant(self):
        self.assertEqual(g3.suite_family("zk-compose"), "zk")
        self.assertEqual(g3.suite_family("qlever-100m"), "qlever")
        self.assertEqual(g3.suite_family("watdiv"), "watdiv")

    def test_new_registry_ids_diff(self):
        base = _registry([{"id": "a", "source": "x"}])
        head = _registry([{"id": "a", "source": "x"}, {"id": "b", "source": "y"}])
        self.assertEqual(g3.new_registry_ids(head, base), ["b"])

    def test_added_bench_suites_excludes_preexisting_dirs(self):
        # A file ADDED inside a pre-existing suite dir is not a new suite (this is
        # what git_base_bench_suites() supplies in the live CI path).
        self.assertEqual(
            g3.added_bench_suites(["bench/zk/benches/new.rs"], {"zk", "watdiv"}),
            [],
        )
        # A brand-new suite dir is still detected.
        self.assertEqual(
            g3.added_bench_suites(["bench/brandnew/run.sh"], {"zk"}),
            ["brandnew"],
        )

    def test_added_bench_suites_token_charset_rejects_garbage(self):
        # Only valid dir-name components are treated as suite dirs.
        self.assertEqual(g3.added_bench_suites(["bench/ok-suite/x"], set()), ["ok-suite"])
        self.assertEqual(g3.added_bench_suites(["bench/benchmarks.toml"], set()), [])

    def test_normalize_base_handles_refs_heads(self):
        self.assertEqual(g3._normalize_base("refs/heads/main"), "origin/main")
        self.assertEqual(g3._normalize_base("main"), "origin/main")
        self.assertEqual(g3._normalize_base("origin/dev"), "origin/dev")


# --------------------------------------------------------------------------- #
# main() smoke — hermetic via --dry-run + --changed-files + --base-toml.
# --------------------------------------------------------------------------- #
class MainSmokeTest(unittest.TestCase):
    def _write(self, tmp: Path, name: str, text: str) -> str:
        p = tmp / name
        p.write_text(text, encoding="utf-8")
        return str(p)

    def test_dry_run_clean_diff_passes(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            cf = self._write(tmp, "diff.txt", "A\tresearch/foo.md\n")
            bt = self._write(tmp, "base.toml", g3._read(g3.BENCH_REGISTRY))
            rc = g3.main(["--dry-run", "--changed-files", cf, "--base-toml", bt])
            self.assertEqual(rc, 0)

    def test_dry_run_reports_violation_but_exits_zero(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            cf = self._write(tmp, "diff.txt", "A\tbench/zzznewsuite/run.sh\n")
            bt = self._write(tmp, "base.toml", "")  # empty base registry
            rc = g3.main(["--dry-run", "--changed-files", cf, "--base-toml", bt])
            self.assertEqual(rc, 0)  # dry-run never fails


# --------------------------------------------------------------------------- #
# Back-catalogue guard: the gate must be GREEN against the current repo state.
# (A new suite landing later is caught by the diff; the committed registry +
# dashboard must agree with each other so the gate can ship HARD eventually.)
# --------------------------------------------------------------------------- #
class BackCatalogueTest(unittest.TestCase):
    def test_no_added_files_means_pass_on_current_repo(self):
        # With an empty added-list and identical base==head registry, the gate is
        # vacuously green — i.e. merely existing committed state never trips it.
        reg = g3._read(g3.BENCH_REGISTRY)
        featured = g3.dashboard_featured_text(g3._read(g3.DASHBOARD_JS))
        v = g3.evaluate([], head_toml=reg, base_toml=reg, featured_text=featured)
        self.assertEqual(v, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
