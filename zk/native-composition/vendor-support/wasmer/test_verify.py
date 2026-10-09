"""[GPT-6] Static adversarial checks; no compiler, resolver or network calls."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest
import verify


class ProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sparq-wasmer-static-")
        self.addCleanup(self.temp.cleanup)
        self.native = Path(self.temp.name) / "native"
        for name in ("Cargo.toml", "Cargo.lock", "vendor/wasmer-derive-6.1.0",
                     "vendor-support/wasmer"):
            source, target = verify.NATIVE / name, self.native / name
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, target, ignore=shutil.ignore_patterns("__pycache__"))
            else:
                shutil.copyfile(source, target)
        self.policy = Path(self.temp.name) / "config.toml"
        shutil.copyfile(verify.NATIVE.parents[1] / "supply-chain/config.toml", self.policy)

    def check(self):
        return verify.verify(self.native, self.policy)

    def test_reverse_and_forward_exact_archive_inventory(self):
        self.assertEqual(self.check()["static_provenance"], "pass")

    def test_source_and_unlisted_file_reject(self):
        for name in ("src/lib.rs", "UNLISTED"):
            with self.subTest(name=name):
                p = self.native / "vendor/wasmer-derive-6.1.0" / name
                before = p.read_bytes() if p.exists() else None
                p.write_bytes((before or b"") + b"\n// changed\n")
                with self.assertRaisesRegex(ValueError, "inventory differs"):
                    self.check()
                if before is None:
                    p.unlink()
                else:
                    p.write_bytes(before)

    def test_symlink_rejects(self):
        p = self.native / "vendor/wasmer-derive-6.1.0/linked"
        p.symlink_to(self.policy)
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.check()

    def test_corrupt_patch_or_license_rejects(self):
        for name in ("diagnostics.patch", "LICENSE.upstream"):
            with self.subTest(name=name):
                p = self.native / "vendor-support/wasmer" / name
                original = p.read_bytes()
                p.write_bytes(original + b"changed")
                with self.assertRaises(ValueError):
                    self.check()
                p.write_bytes(original)

    def test_upstream_hash_cannot_be_replaced_by_patched_hash(self):
        p = self.native / "vendor-support/wasmer/UPSTREAM.json"
        data = json.loads(p.read_text())
        data["files"]["src/lib.rs"]["upstream_sha256"] = data["files"]["src/lib.rs"]["patched_sha256"]
        p.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, "original reconstruction"):
            self.check()

    def test_false_or_absent_upstream_audit_policy_rejects(self):
        original = self.policy.read_text()
        for replacement in ("[policy.wasmer-derive]\naudit-as-crates-io = false", "[policy.other]\naudit-as-crates-io = true"):
            self.policy.write_text(original.replace("[policy.wasmer-derive]\naudit-as-crates-io = true", replacement))
            with self.assertRaises((ValueError, KeyError)):
                self.check()

    def test_local_patch_path_and_remaining_lock_edge_reject(self):
        p = self.native / "Cargo.toml"
        original = p.read_text()
        p.write_text(original.replace('path = "vendor/wasmer-derive-6.1.0"', 'path = "vendor/other"'))
        with self.assertRaisesRegex(ValueError, "patch path"):
            self.check()
        p.write_text(original)
        lock = self.native / "Cargo.lock"
        lock.write_text(lock.read_text() + '\n[[package]]\nname = "proc-macro-error2"\nversion = "2.0.1"\n')
        with self.assertRaisesRegex(ValueError, "diagnostic lock edge"):
            self.check()

    def test_metadata_wrong_path_registry_inactive_and_diagnostic_edges(self):
        metadata = {"packages":[{"name":"wasmer-derive", "version":"6.1.0", "source":None,
                    "manifest_path":str(self.native / "vendor/wasmer-derive-6.1.0/Cargo.toml"), "id":"derive"}],
                    "resolve":{"nodes":[{"id":"derive"}]}}
        verify.check_metadata(metadata, self.native)
        for mutation in (lambda m:m["packages"][0].update(manifest_path="/other/Cargo.toml"),
                         lambda m:m["packages"][0].update(source="registry"),
                         lambda m:m["resolve"].update(nodes=[]),
                         lambda m:m["packages"].append({"name":"proc-macro-error2"})):
            bad = copy.deepcopy(metadata)
            mutation(bad)
            with self.assertRaises(ValueError):
                verify.check_metadata(bad, self.native)


if __name__ == "__main__":
    unittest.main()
