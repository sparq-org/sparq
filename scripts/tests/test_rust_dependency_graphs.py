#!/usr/bin/env python3
"""[GPT-6] Detached graph coverage and failure propagation, without Cargo/network."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("rust_dependency_graphs", REPO / "scripts/rust-dependency-graphs.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class GraphCoverage(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for manifest in gate.MANIFESTS:
            (self.root / manifest).parent.mkdir(parents=True, exist_ok=True)
            (self.root / manifest).write_text("[workspace]\n")
            (self.root / manifest.with_name("Cargo.lock")).write_text("version = 4\n")

    def test_each_independent_lock_is_required(self):
        self.assertEqual(tuple(map(str, gate.MANIFESTS)), (
            "Cargo.toml", "zk/sparql-evaluator/Cargo.toml",
            "zk/sparql-evaluator/methods/guest/Cargo.toml"))
        (self.root / gate.MANIFESTS[-1].with_name("Cargo.lock")).unlink()
        with patch.object(gate.subprocess, "run", return_value=SimpleNamespace(returncode=0)) as run:
            self.assertEqual(gate.run_graphs("fetch", self.root), 1)
            self.assertEqual(run.call_count, 2)

    def test_child_failure_is_not_hidden_by_successful_siblings(self):
        with patch.object(gate.subprocess, "run", side_effect=[
            SimpleNamespace(returncode=0), SimpleNamespace(returncode=1), SimpleNamespace(returncode=0),
        ]) as run:
            self.assertEqual(gate.run_graphs("deny-advisories", self.root), 1)
            self.assertEqual(run.call_count, 3)

    def test_all_three_successes_are_required(self):
        with patch.object(gate.subprocess, "run", return_value=SimpleNamespace(returncode=0)) as run:
            self.assertEqual(gate.run_graphs("vet", self.root), 0)
            self.assertEqual(run.call_count, 3)
            for call in run.call_args_list:
                args = call.args[0]
                self.assertIn("--cargo-arg=--locked", args)
                self.assertIn("--locked", args)
                self.assertIn("--frozen", args)
                self.assertNotIn("--filter-graph", args)

    def test_deny_uses_one_absolute_policy_for_exactly_three_manifests(self):
        for manifest in gate.MANIFESTS:
            self.assertEqual(gate.command("deny-integrity", manifest, self.root), [
                gate.os.environ.get("CARGO", "cargo"), "deny", "--manifest-path", str(manifest),
                "--config", str(self.root / "deny.toml"), "--locked", "check",
                "bans", "sources", "licenses",
            ])

    def test_cyclonedx_cannot_silently_change_the_locked_graph(self):
        def mutate(args, **kwargs):
            manifest = Path(args[args.index("--manifest-path") + 1])
            (self.root / manifest.with_name("Cargo.lock")).write_text("changed")
            self.assertEqual(kwargs["env"]["CARGO_NET_OFFLINE"], "true")
            self.assertIn("--all-features", args)
            self.assertIn("all", args)
            return SimpleNamespace(returncode=0)
        with patch.object(gate.subprocess, "run", side_effect=mutate):
            self.assertEqual(gate.run_graphs("sbom", self.root), 1)

    def metadata(self, missing=None):
        responses = []
        for i, manifest in enumerate(gate.MANIFESTS):
            name = f"member{i}"
            responses.append(json.dumps({"workspace_members": [name], "packages": [
                {"id": name, "name": name, "manifest_path": str(self.root / manifest)},
            ]}).encode())
            if i != missing:
                (self.root / manifest.parent / f"{name}.cdx.json").write_text(json.dumps({
                    "bomFormat": "CycloneDX", "metadata": {"component": {"name": name}},
                }))
        return responses

    def test_sbom_inventory_includes_detached_guest(self):
        with patch.object(gate.subprocess, "check_output", side_effect=self.metadata()):
            paths = gate.sbom_paths(self.root)
        self.assertEqual(len(paths), 3)
        self.assertIn(Path("zk/sparql-evaluator/methods/guest/member2.cdx.json"), paths)

    def test_root_sboms_cannot_mask_missing_guest_sbom(self):
        with patch.object(gate.subprocess, "check_output", side_effect=self.metadata(missing=2)):
            with self.assertRaises(FileNotFoundError):
                gate.sbom_paths(self.root)

    def test_path_patch_does_not_silently_bypass_upstream_audit(self):
        (self.root / "vendor/zk-sdk").mkdir(parents=True)
        metadata = self.root / "vendor/zk-sdk/UPSTREAM.json"
        packages = [{"name": name, "version": version} for name, version in gate.SDK_PATCHES.items()]
        metadata.write_text(json.dumps({"packages": packages}))
        (self.root / "supply-chain").mkdir()
        config = self.root / "supply-chain/config.toml"
        policy = "".join(f"[policy.{name}]\naudit-as-crates-io = true\n" for name in gate.SDK_PATCHES)
        for name in gate.SDK_PATCHES:
            for replacement in ["", f"[policy.{name}]\naudit-as-crates-io = false\n"]:
                with self.subTest(name=name, replacement=replacement):
                    config.write_text(policy.replace(f"[policy.{name}]\naudit-as-crates-io = true\n", replacement))
                    with patch.object(gate.subprocess, "run") as run:
                        with self.assertRaisesRegex(ValueError, "explicit upstream audit policy"):
                            gate.patch_policy(self.root)
                        run.assert_not_called()
        config.write_text(policy)
        with patch.object(gate.subprocess, "run") as run:
            gate.patch_policy(self.root)
            self.assertEqual(run.call_args.args[0][-1], "vendor/zk-sdk/verify.py")
            self.assertTrue(run.call_args.kwargs["check"])
        for invalid in [[], packages[:-1], packages + [packages[0]],
                        packages[:-1] + [{**packages[-1], "version": "0.0.0"}]]:
            with self.subTest(invalid=invalid):
                metadata.write_text(json.dumps({"packages": invalid}))
                with self.assertRaisesRegex(ValueError, "six pinned packages"):
                    gate.patch_policy(self.root)

    def test_committed_six_host_four_guest_patch_inventory(self):
        metadata = json.loads((REPO / "vendor/zk-sdk/UPSTREAM.json").read_text())
        self.assertEqual({p["name"]: p["version"] for p in metadata["packages"]}, gate.SDK_PATCHES)
        for manifest in gate.MANIFESTS[1:]:
            lock = gate.tomllib.loads((REPO / manifest.with_name("Cargo.lock")).read_text())
            selected = {p["name"]: p for p in lock["package"] if p["name"] in gate.SDK_PATCHES}
            expected = set(gate.SDK_PATCHES)
            if "methods/guest" in str(manifest):
                expected -= {"risc0-build", "rzup"}
            self.assertEqual(set(selected), expected)
            for name, package in selected.items():
                self.assertEqual(package["version"], gate.SDK_PATCHES[name])
                self.assertNotIn("source", package)
                self.assertNotIn("checksum", package)

    def test_workflow_selector_and_watchdog_cover_nested_locks(self):
        workflow = (REPO / ".github/workflows/supply-chain.yml").read_text()
        self.assertIn("'**/Cargo.lock'", workflow)
        self.assertIn(r"(^|/)Cargo\.lock$", workflow)
        self.assertIn("'vendor/zk-sdk/**'", workflow)
        for action in ["deny-integrity", "deny-advisories", "fetch", "vet", "sbom", "sbom-paths", "patch-policy"]:
            self.assertIn(f"scripts/rust-dependency-graphs.py {action}", workflow)
        self.assertIn("verify.py --smoke", workflow)
        self.assertIn("verify.py --feature-matrix", workflow)
        self.assertIn("python3 vendor/zk-sdk/tests/test_provenance.py", workflow)
        self.assertIn("python3 vendor/zk-sdk/edge_matrix.py", workflow)
        # A producer failure must not disappear inside Bash process substitution.
        self.assertIn('sbom-paths > "$RUNNER_TEMP/rust-sbom-paths"', workflow)
        self.assertNotIn("<(python3 scripts/rust-dependency-graphs.py", workflow)
        watchdog = (REPO / ".github/workflows/dependency-monitoring.yml").read_text()
        self.assertIn("scripts/rust-dependency-graphs.py deny-advisories", watchdog)


if __name__ == "__main__":
    unittest.main()
