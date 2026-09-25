#!/usr/bin/env python3
"""[GPT-6] Discriminating checks for exact native dependency selection."""

import copy
import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location("native_vendor_verify", Path(__file__).with_name("verify.py"))
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


class MetadataSelectionTests(unittest.TestCase):
    def setUp(self):
        self.metadata = {
            "packages": [
                {"name": "ark-relations", "version": "0.4.0", "source": None,
                 "manifest_path": str(VERIFY.PACKAGE / "Cargo.toml"), "id": "local-ark"},
                {"name": "tracing-subscriber", "version": "0.3.23", "source": "registry",
                 "manifest_path": "/registry/tracing-subscriber/Cargo.toml", "id": "subscriber"},
            ],
            "resolve": {"nodes": [{"id": "local-ark"}, {"id": "subscriber"}]},
        }

    def test_exact_vendor_is_accepted(self):
        VERIFY.check_metadata(self.metadata)

    def test_wrong_local_vendor_path_rejects(self):
        self.metadata["packages"][0]["manifest_path"] = str(VERIFY.NATIVE / "other/Cargo.toml")
        with self.assertRaisesRegex(ValueError, "exact vendor manifest"):
            VERIFY.check_metadata(self.metadata)

    def test_registry_substitution_rejects(self):
        self.metadata["packages"][0]["source"] = "registry+https://github.com/rust-lang/crates.io-index"
        with self.assertRaisesRegex(ValueError, "must be local"):
            VERIFY.check_metadata(self.metadata)

    def test_inactive_patch_rejects(self):
        self.metadata["resolve"]["nodes"] = [{"id": "subscriber"}]
        with self.assertRaisesRegex(ValueError, "active graph"):
            VERIFY.check_metadata(self.metadata)

    def test_old_subscriber_rejects(self):
        self.metadata["packages"][1]["version"] = "0.2.25"
        with self.assertRaisesRegex(ValueError, "maintained subscriber"):
            VERIFY.check_metadata(self.metadata)

    def test_duplicate_subscriber_rejects(self):
        old = copy.deepcopy(self.metadata["packages"][1])
        old.update(version="0.2.25", id="old-subscriber")
        self.metadata["packages"].append(old)
        self.metadata["resolve"]["nodes"].append({"id": "old-subscriber"})
        with self.assertRaisesRegex(ValueError, "maintained subscriber"):
            VERIFY.check_metadata(self.metadata)


if __name__ == "__main__":
    unittest.main()
