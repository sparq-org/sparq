#!/usr/bin/env python3
"""[GPT-6] Corruption controls for the complete SDK patch inventory."""

from contextlib import contextmanager, redirect_stdout
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("sdk_verify", SOURCE / "verify.py")
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


class ProvenanceControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="sparq-sdk-corruption-")
        cls.repo = Path(cls.temporary.name)
        cls.root = cls.repo / "vendor/zk-sdk"
        shutil.copytree(SOURCE, cls.root, ignore=shutil.ignore_patterns("__pycache__"))
        for relative in ["Cargo.lock", "methods/guest/Cargo.lock", "Cargo.toml", "methods/guest/Cargo.toml"]:
            target = cls.repo / "zk/sparql-evaluator" / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(VERIFY.REPO / "zk/sparql-evaluator" / relative, target)
        VERIFY.ROOT, VERIFY.REPO = cls.root, cls.repo

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    @contextmanager
    def changed(self, path, content):
        original = path.read_bytes()
        path.write_bytes(content)
        try:
            yield
        finally:
            path.write_bytes(original)

    def check(self):
        with redirect_stdout(io.StringIO()):
            VERIFY.check()

    def test_complete_inventory(self):
        self.check()

    def test_omitted_patch_inventory_rejects(self):
        path = self.root / "UPSTREAM.json"
        data = json.loads(path.read_text())
        data["packages"].pop()
        with self.changed(path, json.dumps(data).encode()):
            with self.assertRaisesRegex(AssertionError, "complete patch inventory"):
                self.check()

    def test_every_required_local_selection_rejects_registry_substitution(self):
        packages = json.loads((self.root / "UPSTREAM.json").read_text())["packages"]
        for lane, names in [
            ("Cargo.lock", {p["name"] for p in packages}),
            ("methods/guest/Cargo.lock", {"ark-relations", "ark-crypto-primitives",
                                         "risc0-zkvm", "risc0-zkos-v1compat"}),
        ]:
            path = self.repo / "zk/sparql-evaluator" / lane
            for name in sorted(names):
                with self.subTest(lane=lane, package=name):
                    needle = f'[[package]]\nname = "{name}"\n'
                    self.assertEqual(path.read_text().count(needle), 1)
                    altered = path.read_text().replace(needle, needle +
                        'source = "registry+https://github.com/rust-lang/crates.io-index"\n')
                    with self.changed(path, altered.encode()):
                        with self.assertRaisesRegex(AssertionError, "registry package selected"):
                            self.check()

    def test_each_manifest_patch_path_rejects_a_different_local_copy(self):
        packages = json.loads((self.root / "UPSTREAM.json").read_text())["packages"]
        for lane, names in [
            ("Cargo.toml", {p["name"] for p in packages}),
            ("methods/guest/Cargo.toml", {"ark-relations", "ark-crypto-primitives", "risc0-zkvm", "risc0-zkos-v1compat"}),
        ]:
            path = self.repo / "zk/sparql-evaluator" / lane
            for package in packages:
                name = package["name"]
                if name not in names:
                    continue
                with self.subTest(lane=lane, package=name):
                    needle = f"vendor/zk-sdk/{name}-{package['version']}"
                    altered = path.read_text().replace(needle, "vendor/unrelated/" + name)
                    self.assertNotEqual(altered, path.read_text())
                    with self.changed(path, altered.encode()):
                        with self.assertRaisesRegex(AssertionError, "manifest vendor patch path"):
                            self.check()

    def test_resolved_edge_packages_must_be_the_exact_vendor_paths(self):
        versions = {"risc0-zkvm": "3.0.6", "risc0-zkos-v1compat": "2.2.3"}
        metadata = {"resolve": {"nodes": [{"id": name} for name in versions]},
                    "packages": [{"name": name, "id": name, "version": version, "source": None,
                                  "manifest_path": str(self.root / f"{name}-{version}" / "Cargo.toml")}
                                 for name, version in versions.items()]}
        VERIFY.selected_vendor_manifests(metadata, versions)
        for package in metadata["packages"]:
            path = package["manifest_path"]
            package["manifest_path"] = str(self.repo / "unrelated" / package["name"] / "Cargo.toml")
            with self.assertRaisesRegex(AssertionError, "resolved vendor manifest path"):
                VERIFY.selected_vendor_manifests(metadata, versions)
            package["manifest_path"] = path
            package["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            with self.assertRaisesRegex(AssertionError, "resolved vendor source"):
                VERIFY.selected_vendor_manifests(metadata, versions)
            package["source"] = None

    def test_embedded_kernel_bytes_are_bound(self):
        path = self.root / "risc0-zkos-v1compat-2.2.3/elfs/v1compat.elf"
        data = path.read_bytes()
        with self.changed(path, bytes([data[0] ^ 1]) + data[1:]):
            with self.assertRaisesRegex(AssertionError, "v1compat.elf"):
                self.check()

    def test_false_upstream_hash_rejects(self):
        path = self.root / "UPSTREAM.json"
        data = json.loads(path.read_text())
        package = data["packages"][-1]
        package["upstream_files"]["Cargo.toml"] = "0" * 64
        next(p for p in package["patch_files"] if p["path"] == "Cargo.toml")["upstream_sha256"] = "0" * 64
        with self.changed(path, json.dumps(data).encode()):
            with self.assertRaisesRegex(AssertionError, "reconstructed upstream"):
                self.check()

    def test_wrong_strip_level_rejects_even_with_matching_patch_hash(self):
        inventory = self.root / "UPSTREAM.json"
        data = json.loads(inventory.read_text())
        package = next(p for p in data["packages"] if p["name"] == "risc0-zkvm")
        patch = self.root / "risc0-zkvm-3.0.6.patch"
        content = patch.read_bytes().replace(b"a/Cargo.toml", b"a/nested/Cargo.toml")
        content = content.replace(b"b/Cargo.toml", b"b/nested/Cargo.toml")
        self.assertNotEqual(content, patch.read_bytes())
        package["patch_sha256"] = hashlib.sha256(content).hexdigest()
        with self.changed(patch, content), self.changed(inventory, json.dumps(data).encode()):
            with self.assertRaises(subprocess.CalledProcessError):
                self.check()


if __name__ == "__main__":
    unittest.main()
