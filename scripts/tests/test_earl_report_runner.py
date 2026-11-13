#!/usr/bin/env python3
"""Hermetic checks of the multi-suite EARL report script's fetch/run contract."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[2] / "crates/sparq-conformance/earl-report.sh"


class EarlRunnerTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sparq-earl-script-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.upstream = self.root / "upstream"
        self.out = self.root / "reports"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.env = {
            **os.environ,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_ALLOW_PROTOCOL": "file",  # No test can reach a real remote.
            "GIT_CONFIG_COUNT": "3",
            "GIT_CONFIG_KEY_1": f"url.{self.upstream.as_uri()}.insteadOf",
            "GIT_CONFIG_VALUE_1": "https://github.com/w3c/rdf-tests.git",
            "GIT_CONFIG_KEY_2": f"url.{self.upstream.as_uri()}.insteadOf",
            "GIT_CONFIG_VALUE_2": "https://github.com/w3c/N3.git",
            "GIT_CONFIG_KEY_0": f"url.{self.upstream.as_uri()}.insteadOf",
            "GIT_CONFIG_VALUE_0": "https://github.com/w3c/rdf-canon.git",
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "RDF_CANON_TESTS_PIN": "HEAD",
        }
        self.git("init", "-q", str(self.upstream))
        self.git("-C", str(self.upstream), "config", "user.name", "Fixture")
        self.git("-C", str(self.upstream), "config", "user.email", "fixture@example.test")
        (self.upstream / "manifest.ttl").write_text("original fixture\n")
        (self.upstream / ".gitignore").write_text("ignored.txt\n")
        self.git("-C", str(self.upstream), "add", "manifest.ttl", ".gitignore")
        self.git("-C", str(self.upstream), "-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
        self.first_revision = self.git("-C", str(self.upstream), "rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(["git", *args], env=self.env, text=True, stderr=subprocess.PIPE)

    def run_script(self, *, suite="canon", fetch_only=False, **env):
        args = ["bash", str(SCRIPT), suite, str(self.out)]
        if fetch_only:
            args.append("--fetch-only")
        return subprocess.run(args, env={**self.env, **env}, text=True, capture_output=True, check=False)

    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def fake_cargo(self):
        cargo = self.bin / "cargo"
        cargo.write_text("""#!/usr/bin/env bash
set -euo pipefail
for arg in "$@"; do if [[ "$arg" == --no-run ]]; then exit 0; fi; done
printf 'https://w3c.github.io/rdf-canon/tests/manifest#ok\\tpassed\\nhttps://w3c.github.io/rdf-canon/tests/manifest#bad\\tfailed\\n' > "$SPARQ_EARL_OUT"
if [[ "${COMPLETE:-1}" == 1 ]]; then echo 'EARL complete rdf-canon'; fi
exit "${RUN_STATUS:-0}"
""")
        cargo.chmod(0o755)

    def test_new_lanes_preserve_ids_failures_and_require_completion(self):
        cargo = self.bin / "cargo"
        cargo.write_text(r'''#!/usr/bin/env bash
set -euo pipefail
for arg in "$@"; do if [[ "$arg" == --no-run ]]; then exit 0; fi; done
case " $* " in
  *" rdfxml_ratchets "*) marker=rdfxml; dirs="rdf12/rdf-xml/eval rdf11/rdf-xml/manifest.ttl" ;;
  *" semantics_ratchet "*) marker=semantics; dirs="rdf12/rdf-semantics" ;;
  *" rdf11_ratchets "*) marker=rdf11; dirs="rdf11/rdf-turtle/manifest.ttl rdf11/rdf-trig/manifest.ttl rdf11/rdf-n-triples/manifest.ttl rdf11/rdf-n-quads/manifest.ttl rdf11/rdf-xml/manifest.ttl rdf11/rdf-mt/manifest.ttl" ;;
  *" n3_ratchets "*) marker=n3; dirs="N3Tests/manifest-parser.ttl TurtleTests/manifest.ttl" ;;
  *) exit 3 ;;
esac
if [[ "$marker" == n3 ]]; then
  [[ -d "$SPARQ_N3_ROOT" ]]
  prefix=https://w3c.github.io/N3/tests
else
  [[ -d "$SPARQ_RDF_TESTS_ROOT" && " $* " == *" --features rdfxml "* ]]
  prefix=https://w3c.github.io/rdf-tests/rdf
fi
for dir in $dirs; do
  printf '%s/%s#ok\tpassed\n%s/%s#bad\tfailed\n' "$prefix" "$dir" "$prefix" "$dir"
done > "$SPARQ_EARL_OUT"
if [[ "${EXTRA_OUTCOME:-0}" == 1 ]]; then
  printf 'https://example.test/other-manifest#unhandled\tfailed\n' >> "$SPARQ_EARL_OUT"
fi
if [[ "${COMPLETE:-1}" == 1 ]]; then echo "EARL complete $marker"; fi
exit "${RUN_STATUS:-0}"
''')
        cargo.chmod(0o755)
        reports = {
            "rdfxml": ["rdf12-rdf-xml", "rdf11-rdf-xml"],
            "semantics": ["rdf12-semantics"],
            "rdf11": ["rdf11-" + name for name in ["rdf-turtle", "rdf-trig", "rdf-n-triples", "rdf-n-quads", "rdf-xml", "rdf-mt"]],
            "n3": ["n3"],
        }
        for suite, names in reports.items():
            with self.subTest(suite=suite):
                self.assert_success(self.run_script(suite=suite, RUN_STATUS="101"))
                self.assertIn(f"EARL complete {suite}", (self.out / f"{suite}.log").read_text())
                revisions = (self.out / "revisions.tsv").read_text()
                self.assertIn(("N3" if suite == "n3" else "rdf-tests") + "\t" + self.first_revision, revisions)
                for name in names:
                    report = (self.out / f"sparq-{name}-earl.ttl").read_text()
                    self.assertIn("earl:outcome earl:passed", report)
                    self.assertIn("earl:outcome earl:failed", report)
                    self.assertIn(self.first_revision, report)
                    self.assertNotIn("file://", report)
                    expected = 4 if suite == "n3" else 2
                    self.assertEqual(report.count("a earl:Assertion"), expected)
                result = self.run_script(suite=suite, COMPLETE="0", RUN_STATUS="101")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"incomplete {suite} run", result.stderr)
                for name in names:
                    self.assertFalse((self.out / f"sparq-{name}-earl.ttl").exists())
                result = self.run_script(suite=suite, EXTRA_OUTCOME="1")
                self.assertNotEqual(result.returncode, 0)
                if suite != "n3":
                    self.assertIn("uncovered EARL test IRI", result.stderr)
                for name in names:
                    self.assertFalse((self.out / f"sparq-{name}-earl.ttl").exists())

    def test_fetch_restores_fixture_bytes_and_honors_head_and_pin(self):
        self.assert_success(self.run_script(fetch_only=True))
        clone = self.out / "suites/rdf-canon"
        (clone / "manifest.ttl").write_text("local edits\n")
        (clone / "untracked.ttl").write_text("leftover\n")
        (clone / "ignored.txt").write_text("leftover\n")
        self.assert_success(self.run_script(fetch_only=True))
        self.assertEqual((clone / "manifest.ttl").read_text(), "original fixture\n")
        self.assertFalse((clone / "untracked.ttl").exists())
        self.assertFalse((clone / "ignored.txt").exists())

        (self.upstream / "manifest.ttl").write_text("updated fixture\n")
        self.git("-C", str(self.upstream), "add", "manifest.ttl")
        self.git("-C", str(self.upstream), "-c", "commit.gpgsign=false", "commit", "-qm", "update")
        latest = self.git("-C", str(self.upstream), "rev-parse", "HEAD").strip()
        for revision, contents in [("HEAD", "updated fixture\n"), (self.first_revision, "original fixture\n")]:
            self.assert_success(self.run_script(fetch_only=True, RDF_CANON_TESTS_PIN=revision))
            self.assertEqual((clone / "manifest.ttl").read_text(), contents)
            want = latest if revision == "HEAD" else revision
            self.assertEqual((self.out / "revisions.tsv").read_text(), f"rdf-canon\t{want}\n")

    def test_complete_ratchet_failure_still_generates_both_outcomes(self):
        self.fake_cargo()
        self.assert_success(self.run_script(RUN_STATUS="101"))
        report = (self.out / "sparq-rdf-canon-earl.ttl").read_text()
        self.assertEqual(report.count("a earl:Assertion"), 2)
        self.assertIn("earl:outcome earl:passed", report)
        self.assertIn("earl:outcome earl:failed", report)
        self.assertIn(self.first_revision, report)

    def test_incomplete_rerun_cannot_leave_a_stale_report(self):
        self.fake_cargo()
        self.assert_success(self.run_script())
        report = self.out / "sparq-rdf-canon-earl.ttl"
        self.assertTrue(report.exists())
        result = self.run_script(COMPLETE="0", RUN_STATUS="101")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("incomplete rdf-canon run", result.stderr)
        self.assertFalse(report.exists())


if __name__ == "__main__":
    unittest.main()
