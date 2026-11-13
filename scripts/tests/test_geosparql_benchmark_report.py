#!/usr/bin/env python3
"""Exercise benchmark fetch and reporting entry points with local fixtures only."""

import importlib.util
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
        data.write_text("local change")
        run = self.run_report()
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("local changes", run.stderr)
        self.assertEqual(data.read_text(), "local change")

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
