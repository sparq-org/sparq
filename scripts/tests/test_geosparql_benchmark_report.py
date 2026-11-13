#!/usr/bin/env python3
"""Exercise benchmark fetch and reporting entry points with local fixtures only."""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BenchmarkReportTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sparq-gsb-report-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "project"
        self.geo = self.root / "crates/sparq-geo"
        self.geo.mkdir(parents=True)
        (self.root / "scripts").mkdir()
        for name in ("fetch-geosparql-benchmark.sh", "earl-report.sh", "benchmark-report.py"):
            shutil.copyfile(ROOT / "crates/sparq-geo" / name, self.geo / name)
        shutil.copyfile(ROOT / "scripts/earl-report.py", self.root / "scripts/earl-report.py")
        (self.root / "Cargo.toml").write_text('version = "0.1.8"\n')
        self.upstream = self.base / "upstream"
        self.out = self.base / "report"
        self.bin = self.base / "bin"
        self.bin.mkdir()
        self.env = {
            **os.environ,
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_OPTIONAL_LOCKS": "1",
            "GIT_ALLOW_PROTOCOL": "file",  # A missed fake cannot reach the network.
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": f"url.{self.upstream.as_uri()}.insteadOf",
            "GIT_CONFIG_VALUE_0": "https://github.com/OpenLinkSoftware/GeoSPARQLBenchmark.git",
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
        }
        self.env.pop("GEOSPARQL_BENCHMARK_DIR", None)
        self.env.pop("GEOSPARQL_BENCHMARK_PIN", None)
        for repo in (self.root, self.upstream):
            self.git("init", "-q", str(repo))
            self.git("-C", str(repo), "config", "user.name", "Fixture")
            self.git("-C", str(repo), "config", "user.email", "fixture@example.test")
        self.resources = self.upstream / "src/main/resources"
        for name in ("gsb_queries", "gsb_answers", "gsb_dataset"):
            (self.resources / name).mkdir(parents=True)
        for i in range(206):
            (self.resources / "gsb_queries" / f"query-r01-{i}.rq").write_text("SELECT * {}")
        (self.resources / "gsb_answers/placeholder").write_text("fixture")
        (self.resources / "gsb_dataset/dataset.rdf").write_text("fixture")
        self.commit(self.root)
        self.first = self.commit(self.upstream)
        self.clone = self.geo / "tests/fixtures/geosparql-benchmark"
        cargo = self.bin / "cargo"
        cargo.write_text('''#!/usr/bin/env python3
import os, pathlib, sys
if '--no-run' in sys.argv:
    sys.exit(0)
queries = sorted((pathlib.Path(os.environ['GEOSPARQL_BENCHMARK_DIR']) / 'src/main/resources/gsb_queries').glob('query-r*.rq'))
lines = [q.stem + ('\\tpassed' if i == 0 else '\\tfailed') for i, q in enumerate(queries)]
mode = os.environ.get('MODE', '')
if mode == 'partial': lines = lines[:1]
if mode == 'duplicate': lines.append(lines[0])
if mode == 'missing': lines = lines[:-1]
pathlib.Path(os.environ['SPARQ_EARL_OUT']).write_text('\\n'.join(lines) + '\\n')
if mode not in ('partial', 'unsealed'): print('EARL complete geosparql-benchmark')
sys.exit(101)
''')
        cargo.chmod(0o755)

    def git(self, *args):
        return subprocess.check_output(["git", *args], env=self.env, text=True, stderr=subprocess.PIPE).strip()

    def commit(self, repo):
        self.git("-C", str(repo), "add", ".")
        self.git("-C", str(repo), "-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
        return self.git("-C", str(repo), "rev-parse", "HEAD")

    def run_report(self, **env):
        return subprocess.run(["bash", str(self.geo / "earl-report.sh"), str(self.out)],
                              env={**self.env, **env}, text=True, capture_output=True, check=False)

    def run_fetch(self, **env):
        return subprocess.run(["bash", str(self.geo / "fetch-geosparql-benchmark.sh")],
                              env={**self.env, **env}, text=True, capture_output=True, check=False)

    def assert_success(self, run):
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)

    def test_complete_ratchet_failure_reports_exact_query_iris_and_counts(self):
        self.assert_success(self.run_report())
        report = (self.out / "sparq-geosparql-earl.ttl").read_text()
        self.assertEqual(report.count("a earl:Assertion"), 206)
        self.assertEqual(report.count("earl:outcome earl:passed"), 1)
        self.assertIn(f"/blob/{self.first}/src/main/resources/gsb_queries/query-r01-0.rq>", report)
        table = (self.out / "results.md").read_text()
        self.assertIn("| Total | 206 queries | 1 | 205 |", table)
        self.assertIn("| R17 | No query (unscored) | 0 | 0 |", table)
        self.assertIn(self.first, report)

    def test_partial_or_duplicate_reruns_remove_old_reports(self):
        for mode in ("partial", "unsealed", "missing", "duplicate"):
            with self.subTest(mode=mode):
                self.assert_success(self.run_report())
                run = self.run_report(MODE=mode)
                self.assertNotEqual(run.returncode, 0)
                self.assertFalse((self.out / "sparq-geosparql-earl.ttl").exists())
                self.assertFalse((self.out / "results.md").exists())

    def test_head_pin_and_read_only_directory_override(self):
        self.assert_success(self.run_report())
        (self.resources / "gsb_dataset/dataset.rdf").write_text("updated fixture")
        latest = self.commit(self.upstream)
        self.assert_success(self.run_report())
        self.assertEqual(self.git("-C", str(self.clone), "rev-parse", "HEAD"), latest)
        self.assert_success(self.run_report(GEOSPARQL_BENCHMARK_PIN=self.first))
        self.assertEqual(self.git("-C", str(self.clone), "rev-parse", "HEAD"), self.first)
        data = self.resources / "gsb_dataset/dataset.rdf"
        stat = data.stat()
        os.utime(data, ns=(stat.st_atime_ns, stat.st_mtime_ns + 1_000_000_000))
        index = self.upstream / ".git/index"
        index_before = index.read_bytes()
        self.assert_success(self.run_report(GEOSPARQL_BENCHMARK_DIR=str(self.upstream),
                                            GEOSPARQL_BENCHMARK_PIN="must-not-fetch"))
        self.assertEqual(index.read_bytes(), index_before, "read-only override refreshed the index")
        self.assertEqual(self.git("-C", str(self.upstream), "rev-parse", "HEAD"), latest)

    def test_fetch_refuses_edited_cache(self):
        self.assert_success(self.run_report())
        data = self.clone / "src/main/resources/gsb_dataset/dataset.rdf"
        pristine = data.read_text()
        data.write_text("local change")
        run = self.run_report()
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("local changes", run.stderr)
        self.assertEqual(data.read_text(), "local change")
        data.write_text(pristine)
        self.assert_success(self.run_report())
        self.assertEqual(data.read_text(), pristine)

    def test_fetch_refuses_fixtures_without_checkout_provenance(self):
        unverified = self.root / "unverified"
        shutil.copytree(self.resources, unverified / "src/main/resources")
        # The parent is a clean repository too: status/rev-parse alone would
        # otherwise attribute this plain directory to the parent's commit.
        self.commit(self.root)
        run = self.run_report(GEOSPARQL_BENCHMARK_DIR=str(unverified))
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("not a Git checkout root", run.stderr)
        self.assertFalse((self.out / "sparq-geosparql-earl.ttl").exists())
        self.assert_success(self.run_report())
        # A matching ref string is not proof when its commit object is missing.
        missing = "e" * 40
        (self.clone / ".git/HEAD").write_text(missing + "\n")
        run = self.run_fetch(GEOSPARQL_BENCHMARK_PIN=missing)
        self.assertNotEqual(run.returncode, 0)

    def test_fetch_restores_clean_but_different_cache_revision_to_pin(self):
        self.assert_success(self.run_report())
        data = self.clone / "src/main/resources/gsb_dataset/dataset.rdf"
        data.write_text("locally committed replacement")
        self.git("-C", str(self.clone), "config", "user.name", "Fixture")
        self.git("-C", str(self.clone), "config", "user.email", "fixture@example.test")
        local_revision = self.commit(self.clone)
        self.assertNotEqual(local_revision, self.first)
        self.assertEqual(self.git("-C", str(self.clone), "status", "--porcelain"), "")
        for _ in range(2):
            self.assert_success(self.run_fetch(GEOSPARQL_BENCHMARK_PIN=self.first))
            self.assertEqual(self.git("-C", str(self.clone), "rev-parse", "HEAD"), self.first)
            self.assertEqual(data.read_text(), "fixture")

    def test_verified_pin_cache_needs_no_fetch_but_other_refs_do(self):
        self.assert_success(self.run_report())
        self.git("-C", str(self.clone), "remote", "set-url", "origin",
                 str(self.base / "unavailable.git"))
        self.assert_success(self.run_fetch(GEOSPARQL_BENCHMARK_PIN=self.first))
        self.assertEqual(self.git("-C", str(self.clone), "rev-parse", "HEAD"), self.first)
        for revision in ("HEAD", "e" * 40):
            with self.subTest(revision=revision):
                run = self.run_fetch(GEOSPARQL_BENCHMARK_PIN=revision)
                self.assertNotEqual(run.returncode, 0)
                self.assertEqual(self.git("-C", str(self.clone), "rev-parse", "HEAD"), self.first)

    def test_documented_examples_use_the_shared_checkout_resources(self):
        cargo = self.bin / "cargo"
        cargo.write_text("#!/usr/bin/env python3\nimport json, os, pathlib, sys\n"
                         "pathlib.Path(os.environ['ARGS_OUT']).write_text(json.dumps(sys.argv[1:]))\n")
        rustdoc = (ROOT / "crates/sparq-geo/examples/gsb_compliance.rs").read_text()
        rustdoc = "\n".join(line[4:] for line in rustdoc.splitlines() if line.startswith("//! "))
        readme = (ROOT / "bench/geo/README.md").read_text().split(
            "## GeoSPARQL Compliance Benchmark family", 1)[1]
        for name, text in (("example", rustdoc), ("readme", readme)):
            with self.subTest(name=name):
                snippet = text.split("```sh\n", 1)[1].split("```", 1)[0]
                args = self.base / "args.json"
                run = subprocess.run(["bash", "-e", "-c", snippet], cwd=self.root,
                                     env={**self.env, "GEOSPARQL_BENCHMARK_PIN": self.first,
                                          "ARGS_OUT": str(args)},
                                     text=True, capture_output=True, check=False)
                self.assert_success(run)
                self.assertEqual(json.loads(args.read_text())[-1],
                                 str(self.clone / "src/main/resources"))

    def test_fetch_and_report_tests_are_unconditionally_wired_in_quick_gates(self):
        import yaml

        workflow = yaml.safe_load((ROOT / ".github/workflows/docs-quality.yml").read_text())
        job = workflow["jobs"]["quick-gates"]
        command = "python3 scripts/tests/test_geosparql_benchmark_report.py"
        steps = [step for step in job["steps"] if step.get("run") == command]
        self.assertEqual(len(steps), 1)
        self.assertNotIn("continue-on-error", job)
        self.assertNotIn("if", steps[0])
        self.assertNotIn("continue-on-error", steps[0])

    def test_table_rejects_missing_wrong_and_malformed_outcomes(self):
        spec = importlib.util.spec_from_file_location("gsb_report", ROOT / "crates/sparq-geo/benchmark-report.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        good = "\n".join(f"query-r01-{i}\tpassed" for i in range(206))
        self.assertIn("| Total | 206 queries | 206 | 0 |", module.render(good, self.resources, self.first))
        with self.assertRaises(ValueError):
            module.render(good, self.base, self.first)
        for bad in (good.rsplit("\n", 1)[0], good.replace("query-r01-0\t", "query-r02-0\t"),
                    good.replace("passed", "unknown", 1), good + "\nquery-r01-0\tpassed",
                    good.replace("query-r01-0\t", "query-r17\t")):
            with self.subTest(bad=bad[:30]), self.assertRaises(ValueError):
                module.render(bad, self.resources, self.first)


if __name__ == "__main__":
    unittest.main()
