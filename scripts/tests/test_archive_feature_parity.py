#!/usr/bin/env python3
# [GPT-5] #6273 — keep every consumer of the nextest archive opt-in feature set
# synchronized. Hermetic and stdlib-only; no workflow execution or network access.

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

CI = REPO_ROOT / ".github" / "workflows" / "ci.yml"
HEAVY_ALARM = REPO_ROOT / ".github" / "workflows" / "heavy-set-alarm.yml"
BUILDFARM = REPO_ROOT / "scripts" / "ec2-buildfarm.sh"
DOCS_QUALITY = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"

# [SONNET-4.6] Declaration depth and YAML scalar quoting are presentation details.
_YAML_FEATURES = re.compile(
    r"^\s+ARCHIVE_FEATURES:\s*([^#\n]+?)\s*$", re.MULTILINE
)
_BUILDFARM_FEATURES = re.compile(
    r'^ARCHIVE_FEATURES="\$\{BUILDFARM_FEATURES:-([^}]+)\}"$', re.MULTILINE
)


def extract(pattern: re.Pattern[str], text: str, source: str) -> str:
    matches = pattern.findall(text)
    if len(matches) != 1:
        raise ValueError(f"{source}: expected exactly one archive feature declaration, found {len(matches)}")
    return matches[0].strip().strip("'\"")


def feature_set(features: str) -> frozenset[str]:
    return frozenset(feature.strip() for feature in features.split(","))


def parity_errors(ci_text: str, alarm_text: str, buildfarm_text: str) -> list[str]:
    declarations = {
        "ci.yml build-archive": extract(_YAML_FEATURES, ci_text, "ci.yml"),
        "heavy-set-alarm.yml measurement": extract(
            _YAML_FEATURES, alarm_text, "heavy-set-alarm.yml"
        ),
        "ec2-buildfarm.sh default": extract(
            _BUILDFARM_FEATURES, buildfarm_text, "ec2-buildfarm.sh"
        ),
    }
    canonical = declarations["ci.yml build-archive"]
    canonical_set = feature_set(canonical)
    return [
        f"{source} uses {features!r}; ci.yml uses {canonical!r}"
        for source, features in declarations.items()
        if feature_set(features) != canonical_set
    ]


class ArchiveFeatureParityTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.ci = CI.read_text(encoding="utf-8")
        cls.alarm = HEAVY_ALARM.read_text(encoding="utf-8")
        cls.buildfarm = BUILDFARM.read_text(encoding="utf-8")

    def test_live_declarations_match(self):
        self.assertEqual(parity_errors(self.ci, self.alarm, self.buildfarm), [])

    def test_each_surface_executes_with_its_declared_value(self):
        self.assertRegex(
            self.ci,
            r"cargo nextest archive[^\n]+--features \"\$\{ARCHIVE_FEATURES\}\"",
        )
        self.assertRegex(
            self.ci,
            r"cargo test --workspace --features \"\$\{ARCHIVE_FEATURES\}\" --doc",
        )
        self.assertRegex(
            self.alarm,
            r"--features \"\$\{ARCHIVE_FEATURES\}\"",
        )
        for command in ("build", "clippy", "test"):
            self.assertRegex(
                self.buildfarm,
                rf"run cargo {command}\s+[^\n]*--features \$\{{ARCHIVE_FEATURES\}}",
            )

    def test_alarm_drift_fails(self):
        changed = self.alarm.replace(
            "ARCHIVE_FEATURES: approx-ann,filtered-ann,vec-predicate",
            "ARCHIVE_FEATURES: approx-ann,filtered-ann",
            1,
        )
        self.assertTrue(parity_errors(self.ci, changed, self.buildfarm))

    def test_buildfarm_drift_fails(self):
        changed = self.buildfarm.replace(
            "BUILDFARM_FEATURES:-approx-ann,filtered-ann,vec-predicate",
            "BUILDFARM_FEATURES:-approx-ann,filtered-ann",
            1,
        )
        self.assertTrue(parity_errors(self.ci, self.alarm, changed))

    def test_ci_declaration_must_be_unique(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            parity_errors(self.ci + "\n      ARCHIVE_FEATURES: extra\n", self.alarm, self.buildfarm)

    def test_yaml_indentation_quoting_and_order_are_not_semantic(self):
        changed = self.alarm.replace(
            "          ARCHIVE_FEATURES: approx-ann,filtered-ann,vec-predicate",
            '      ARCHIVE_FEATURES: "filtered-ann,vec-predicate,approx-ann"',
            1,
        )
        self.assertEqual(parity_errors(self.ci, changed, self.buildfarm), [])

    def test_docs_quality_runs_this_suite_unconditionally(self):
        docs_quality = DOCS_QUALITY.read_text(encoding="utf-8")
        invocation = "run: python3 scripts/tests/test_archive_feature_parity.py"
        invocation_line = f"        {invocation}"
        self.assertEqual(
            docs_quality.splitlines().count(invocation_line),
            1,
            "docs-quality.yml no longer invokes this suite — archive feature parity would stop being enforced",
        )
        steps = re.findall(
            r"^      - name:[^\n]*\n(?:(?!^      - name:)[\s\S])*(?=^      - name:|\Z)",
            docs_quality,
            re.MULTILINE,
        )
        call_sites = [step for step in steps if invocation_line in step.splitlines()]
        self.assertEqual(len(call_sites), 1, "expected exactly one archive parity call site")
        self.assertNotRegex(call_sites[0], r"^        (?:if|continue-on-error):")


if __name__ == "__main__":
    unittest.main()
