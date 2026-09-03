from __future__ import annotations

import importlib.util
import json
import pathlib
import tempfile
import unittest


MODULE_PATH = pathlib.Path(__file__).parents[1] / "verify-paper-evidence.py"
SPEC = importlib.util.spec_from_file_location("verify_paper_evidence", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


def write_json(path: pathlib.Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def provenance() -> dict[str, object]:
    return {
        "study_id": "fixture-study",
        "run_id": "fixture-run",
        "collected_at_utc": "2026-09-03T12:00:00Z",
        "source_git_commit": "a" * 40,
        "analysis_git_commit": "d" * 40,
        "host_class": "controlled-single-process-ec2",
        "host_label": "fixture host",
        "tenancy": "shared",
        "noise_limitation": "Shared-cloud tenancy can experience noisy neighbours.",
        "workload": "fixture workload",
        "dataset": "fixture dataset",
        "query_scope": "fixture query",
        "protocol": "bench/protocol.md v1",
        "bootstrap_draws": 10000,
        "bootstrap_seed": 7,
        "publisher_path": "bench/ac/scaling/publish_paper_summary.py",
        "publisher_sha256": "e" * 64,
        "input_file_count": 1,
        "raw_archive_kind": "committed",
        "raw_archive_location": "bench/canonical-competitor-results/fixture/raw.tar.zst",
        "raw_archive_public_url": "",
        "raw_archive_sha256": "c" * 64,
        "raw_archive_bytes": 1234,
        "raw_archive_build_verification": "local-rehash",
        "raw_archive_member_verification_authority": "publisher-recorded",
        "raw_archive_manifest_member": "manifest.json",
        "raw_archive_manifest_sha256": "f" * 64,
        "raw_archive_manifest_bytes": 512,
        "raw_archive_manifest_entry_count": 1,
        "raw_archive_regular_members": 2,
        "raw_archive_all_members_rehashed": True,
        "raw_archive_exact_member_set": True,
        "raw_archive_sanitization_scan_passed": True,
        "raw_archive_deterministic_tar_headers": True,
        "raw_archive_zstd_version": "1.5.7",
        "raw_archive_zstd_executable_sha256": "9" * 64,
    }


class CanonicalTimingVerificationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.base = self.root / "base.json"
        self.timing = self.root / "timing.json"
        self.allowlist = self.root / "allowlist.json"
        self.envelope_rel = "bench/ac/canonical/run/paper-summary.json"
        self.envelope = self.root / self.envelope_rel
        write_json(self.base, {"schemaVersion": 2, "records": {}})
        write_json(self.allowlist, {"seed_count": 0, "keys": []})
        write_json(self.envelope, {"results": {"ratio": 1.0125}})
        self.record = {
            "value": 1.0125,
            "unit": "ratio",
            "environment": "canonical-timing",
            "kind": "canonical-timing",
            "source": self.envelope_rel + "#/results/ratio",
            "binding": {
                "kind": "json-pointer",
                "file": self.envelope_rel,
                "pointer": "/results/ratio",
            },
            "result_bindings": [{
                "artifact": "summary.csv",
                "sha256": "8" * 64,
                "locator": "row=fixture; field=ratio",
            }],
            "timing_provenance": provenance(),
            "papers": ["fixture-paper"],
            "note": "Fixture timing.",
            "_generated_by": "site/scripts/sync-canonical-timing.mjs",
        }

    def tearDown(self) -> None:
        self.temp.cleanup()

    def problems(self, record: dict[str, object]) -> list[str]:
        write_json(self.timing, {"schemaVersion": 1, "records": {"fixture.ratio": record}})
        return VERIFY.verify_evidence(
            str(self.base), str(self.allowlist), str(self.root), supplements=[str(self.timing)]
        )

    def test_valid_sync_generated_json_bound_timing_passes(self) -> None:
        self.assertEqual(self.problems(self.record), [])

    def test_timing_cannot_omit_provenance(self) -> None:
        self.record.pop("timing_provenance")
        self.assertTrue(any("timing_provenance is missing" in p for p in self.problems(self.record)))

    def test_source_and_analysis_commits_are_independently_required_40_hex_fields(self) -> None:
        timing_provenance = self.record["timing_provenance"]
        assert isinstance(timing_provenance, dict)
        timing_provenance["analysis_git_commit"] = "not-a-commit"
        problems = self.problems(self.record)
        self.assertTrue(any("analysis_git_commit must be lowercase 40-hex" in p for p in problems))

    def test_archive_verification_and_shared_tenancy_cannot_be_overstated(self) -> None:
        timing_provenance = self.record["timing_provenance"]
        assert isinstance(timing_provenance, dict)
        timing_provenance["raw_archive_build_verification"] = "descriptor-only"
        timing_provenance["tenancy"] = "dedicated"
        problems = self.problems(self.record)
        self.assertTrue(any("committed raw archive must be locally re-hashed" in p for p in problems))
        self.assertTrue(any("must declare shared tenancy" in p for p in problems))

    def test_publisher_recorded_member_verification_is_structurally_required(self) -> None:
        timing_provenance = self.record["timing_provenance"]
        assert isinstance(timing_provenance, dict)
        timing_provenance["publisher_sha256"] = "not-a-digest"
        timing_provenance["raw_archive_member_verification_authority"] = "build-verified"
        timing_provenance["raw_archive_all_members_rehashed"] = False
        timing_provenance["raw_archive_regular_members"] = 9
        problems = self.problems(self.record)
        self.assertTrue(any("publisher_sha256 must be lowercase 64-hex" in p for p in problems))
        self.assertTrue(any("must be labelled publisher-recorded" in p for p in problems))
        self.assertTrue(any("raw_archive_all_members_rehashed must be true" in p for p in problems))
        self.assertTrue(any("regular member count must equal" in p for p in problems))

    def test_timing_value_drift_fails(self) -> None:
        self.record["value"] = 9.0
        self.assertTrue(any("DRIFT" in p for p in self.problems(self.record)))

    def test_scalar_timing_requires_complete_source_artifact_bindings(self) -> None:
        self.record.pop("result_bindings")
        self.assertTrue(any(
            "non-empty result_bindings provenance is required" in p
            for p in self.problems(self.record)
        ))

    def test_derived_boolean_verdict_passes_but_free_form_text_does_not(self) -> None:
        write_json(self.envelope, {"results": {"minimal": True}})
        self.record.update({
            "value": True,
            "unit": "boolean",
            "kind": "canonical-timing-verdict",
            "hypothesis": "H2",
            "binding": {
                "kind": "json-pointer",
                "file": self.envelope_rel,
                "pointer": "/results/minimal",
            },
        })
        self.assertEqual(self.problems(self.record), [])
        self.record["value"] = "meets"
        self.assertTrue(any("value/unit must be boolean" in p for p in self.problems(self.record)))

    def test_h3_through_h5_cannot_be_mechanical_verdicts(self) -> None:
        write_json(self.envelope, {"results": {"exploratory": True}})
        self.record.update({
            "value": True,
            "unit": "boolean",
            "kind": "canonical-timing-verdict",
            "hypothesis": "H4",
            "binding": {
                "kind": "json-pointer",
                "file": self.envelope_rel,
                "pointer": "/results/exploratory",
            },
        })
        self.assertTrue(any("hypothesis H1/H2" in p for p in self.problems(self.record)))

    def test_timing_cannot_use_rust_anchor_or_allowlist(self) -> None:
        self.record["binding"] = {"kind": "rust-anchor", "file": "x.rs", "anchor": "x"}
        write_json(self.allowlist, {"seed_count": 1, "keys": ["fixture.ratio"]})
        problems = self.problems(self.record)
        self.assertTrue(any("only a json-pointer binding" in p for p in problems))
        self.assertTrue(any("may never use the transitional allowlist" in p for p in problems))

    def test_digest_bound_figure_shape_passes(self) -> None:
        digest = "b" * 64
        write_json(self.envelope, {"figures": {"scaling": {"sha256": digest}}})
        figure = {
            **self.record,
            "value": digest,
            "unit": "sha256",
            "kind": "canonical-timing-figure",
            "source": self.envelope_rel + "#/figures/scaling/sha256",
            "binding": {
                "kind": "json-pointer",
                "file": self.envelope_rel,
                "pointer": "/figures/scaling/sha256",
            },
            "figure": {
                "typst_path": "/papers/figures/canonical-timing/ac/scaling.svg",
                "media_type": "image/svg+xml",
                "alt": "Scaling figure.",
                "output_name": "scaling.svg",
                "bytes": 1234,
            },
        }
        figure.pop("result_bindings")
        self.assertEqual(self.problems(figure), [])

    def test_duplicate_key_across_ledgers_fails_closed(self) -> None:
        write_json(self.base, {"records": {"fixture.ratio": {"value": 1}}})
        write_json(self.timing, {"records": {"fixture.ratio": self.record}})
        with self.assertRaisesRegex(VERIFY.VerifyError, "duplicate evidence key"):
            VERIFY.verify_evidence(
                str(self.base), str(self.allowlist), str(self.root), supplements=[str(self.timing)]
            )


if __name__ == "__main__":
    unittest.main()
