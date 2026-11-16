#!/usr/bin/env python3
"""Research grade and not externally audited. Package execution contracts."""
from copy import deepcopy
import importlib.util
from pathlib import Path
import sys
import unittest
import re

from _yaml_seam import yaml_or_local_skip

yaml = yaml_or_local_skip()
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("package_gate", ROOT / "scripts/check-zk-toolchain-gate.py")
gate = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = gate
spec.loader.exec_module(gate)


class PackageExecution(unittest.TestCase):
    def setUp(self):
        self.doc = yaml.safe_load((ROOT / gate.WORKFLOW_REL).read_text())

    def test_package_bridge_is_required_and_fail_closed(self):
        self.assertEqual(gate.check_query_workflow(ROOT, self.doc), [])
        for change in ("job", "ledger", "bridge", "offline", "verifier", "condition", "admission", "swallow", "tools", "shell"):
            with self.subTest(change=change):
                doc = deepcopy(self.doc)
                job = doc["jobs"]["query-package"]
                if change == "job":
                    del doc["jobs"]["query-package"]
                elif change == "ledger":
                    del job["env"][gate.LEDGER_ENV]
                elif change == "bridge":
                    job["steps"] = [s for s in job["steps"] if s.get("name") != "Build and exercise the split query prover package offline"]
                elif change == "offline":
                    step = next(s for s in job["steps"] if s.get("name") == "Build and exercise the split query prover package offline")
                    step["run"] = step["run"].replace("python3 js/zk-query/scripts/offline.py ", "")
                elif change == "verifier":
                    job["steps"].pop()
                elif change == "condition":
                    job["steps"][-1]["if"] = "false"
                elif change == "admission":
                    job["if"] = "false"
                elif change == "swallow":
                    job["continue-on-error"] = True
                elif change == "tools":
                    job["steps"] = [s for s in job["steps"] if s.get("uses") != "./.github/actions/noir-toolchain"]
                elif change == "shell":
                    job["steps"][-1]["shell"] = "python"
                self.assertTrue(any(f.where == "query-package" for f in gate.check_query_workflow(ROOT, doc)))

    def test_package_ledger_partitions_the_complete_inventory(self):
        trees = gate.collect(ROOT)
        self.assertEqual(trees.findings, [])
        sites, findings = gate.inventory(trees)
        self.assertEqual(findings, [])
        all_keys = gate.profile_keys(ROOT, sites, "all")
        package = gate.profile_keys(ROOT, sites, "query-package")
        estate = gate.profile_keys(ROOT, sites, "estate")
        query = gate.profile_keys(ROOT, sites, "query-full")
        self.assertTrue(package)
        self.assertTrue(all(key.startswith("crates/sparq-zk-query-wasm/") for key in package))
        self.assertFalse(package & estate)
        self.assertFalse(package & query)
        self.assertEqual(all_keys, package | estate | query)

    def test_package_paths_and_catalogue_cache_survive_workflow_merges(self):
        events = self.doc.get("on", self.doc.get(True))
        required = {"crates/sparq-zk-query-wasm/**", "js/zk-query/**", "js/zk-shared/**"}
        for event in ("pull_request", "push"):
            self.assertLessEqual(required, set(events[event]["paths"]))
        detection = next(s for s in self.doc["jobs"]["forge-suite"]["steps"] if s.get("id") == "changes")["run"]
        expression = re.search(r"'(\^\([^\n]+)'", detection).group(1)
        for path in ("crates/sparq-zk-query-wasm/src/lib.rs", "js/zk-query/index.js", "js/zk-shared/runtime.mjs"):
            self.assertRegex(path, expression)
        steps = self.doc["jobs"]["query-package"]["steps"]
        cache = next(s for s in steps if s.get("name") == "Cache native query catalogue compilations")
        self.assertEqual(cache["with"]["path"], "/tmp/sparq-query-catalogue")
        self.assertIn("js/zk-query/assets/catalogue.json", cache["with"]["key"])


if __name__ == "__main__":
    unittest.main()
