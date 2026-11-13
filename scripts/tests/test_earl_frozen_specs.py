#!/usr/bin/env python3
"""Hermetic subprocess checks of the Solid, LDP and ODRL report drivers.

Every external runner is replaced on PATH, so these tests need no network,
Docker, Maven or TCP listener. LDP exports are checked against distinct native reports.
"""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
NATIVE = "@prefix earl: <http://www.w3.org/ns/earl#>.\n[] earl:outcome earl:passed.\n[] earl:outcome earl:failed.\n"
LDP_NATIVE = {
    "basic": "# native Basic report\r\n@prefix earl: <http://www.w3.org/ns/earl#>.\r\n"
             "[] a earl:Assertion; earl:test <http://w3c.github.io/ldp-testsuite/manifest#MemberResource-GetResource>;\r\n"
             "   earl:result [ earl:outcome earl:passed ].\r\n",
    "direct": "# native Direct probe\n@prefix earl: <http://www.w3.org/ns/earl#>.\n"
              "[] a earl:Assertion; earl:test <http://w3c.github.io/ldp-testsuite/manifest#MemberResource-GetResource>;\n"
              "   earl:result [ earl:outcome earl:failed ].\n",
}

TOOLS = r'''#!/usr/bin/env python3
import os
from pathlib import Path
import sys

name = Path(sys.argv[0]).name
args = sys.argv[1:]
complete = os.environ.get("MOCK_COMPLETE", "1") == "1"
status = int(os.environ.get("MOCK_STATUS", "0"))
native = os.environ["MOCK_NATIVE"]
if name == "git":
    if "rev-parse" in args: print("a" * 40)
elif name == "curl":
    if "%{http_code}" in args: print("200", end="")
elif name == "docker":
    if args[:2] == ["image", "inspect"]:
        if "--format" in args:
            print("sha256:fixture" if "{{.Id}}" in args else "b" * 40)
    elif "--output" in args:
        target = next(a[:-len(":/reports")] for a in args if a.endswith(":/reports"))
        if complete: (Path(target) / "report.ttl").write_text(native)
        sys.exit(status)
elif name == "mvn":
    suite = Path(args[args.index("-f") + 1]).parent
    jar = suite / "target/ldp-testsuite-fixture-shaded.jar"
    jar.parent.mkdir(parents=True, exist_ok=True)
    jar.touch()
elif name == "java":
    assert "--earl" in args and "--non-rdf" in args
    groups = args[args.index("--includedGroups") + 1:args.index("--includedGroups") + 4]
    assert groups == ["MUST", "SHOULD", "MAY"]
    level = "basic" if "--basic" in args else "direct"
    assert args[args.index("--server") + 1] == f"http://127.0.0.1:3001/{level}/"
    out = Path(args[args.index("--output") + 1]) / "report"
    out.mkdir()
    if complete:
        (out / "ldp-testsuite-execution-report-earl.ttl").write_bytes(
            os.environ[f"MOCK_{level.upper()}_NATIVE"].encode())
    sys.exit(status)
elif name == "cargo":
    if "--no-run" not in args:
        raw = os.environ.get("SPARQ_EARL_OUT")
        if raw:
            Path(raw).write_text("urn:uuid:ok\tpassed\nurn:uuid:bad\tfailed\n")
            Path(os.environ["SPARQ_ODRL_TABLE_OUT"]).write_text("fixture table\n")
            if complete: print("EARL complete odrl 2")
            sys.exit(status)
else:
    raise AssertionError(f"unexpected external command: {name} {args}")
'''

SERVER = '''#!/usr/bin/env python3
import signal
import sys
assert len(sys.argv) == 1, "fixture server accepts no arguments"
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
signal.pause()
'''


class FrozenReportDrivers(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="sparq-frozen-earl-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        for relative in [
            "crates/sparq-lws-core/conformance/run.sh",
            "crates/sparq-lws-core/conformance/ldp.sh",
            "crates/sparq-lws-core/conformance/server-lifecycle.sh",
            "crates/sparq-policy/earl-report.sh",
            "scripts/earl-report.py",
        ]:
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / relative, target)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ["git", "curl", "docker", "mvn", "java", "cargo"]:
            target = self.bin / name
            target.write_text(TOOLS)
            target.chmod(0o755)
        server = self.root / "build/debug/examples/ldp_conformance"
        server.parent.mkdir(parents=True)
        server.write_text(SERVER)
        server.chmod(0o755)
        self.out = self.root / "out"
        self.out.mkdir()
        here = self.root / "crates/sparq-lws-core/conformance"
        for directory in ["config", "tls", ".cache/specification-tests"]:
            (here / directory).mkdir(parents=True)
        for relative in ["config/sparq-lws-core.env", "tls/server-cert.pem", "tls/server-key.pem"]:
            (here / relative).touch()
        (here / "baseline.json").write_text('{"min_passed":2,"max_failed":0,"expected_total":2}')
        (self.root / "tests/odrl-test-suite/data/test_cases").mkdir(parents=True)
        (self.root / "suite").mkdir()
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "SERVER_BIN": str(server), "CARGO_TARGET_DIR": str(self.root / "build"),
            "LDP_TESTSUITE": str(self.root / "suite"),
            "MOCK_NATIVE": NATIVE, "CTH_REVISION": "b" * 40,
            "MOCK_BASIC_NATIVE": LDP_NATIVE["basic"],
            "MOCK_DIRECT_NATIVE": LDP_NATIVE["direct"],
            "PSS_SPARQ_BACKEND": "memory", "CTH_ENFORCE_BASELINE": "1",
            "GIT_ALLOW_PROTOCOL": "",  # A forgotten real Git runner cannot fetch.
        }

    def run_driver(self, suite, **env):
        if suite == "cth":
            script = "crates/sparq-lws-core/conformance/run.sh"
            args = ["--keep-earl", str(self.out)]
        else:
            script = {"ldp": "crates/sparq-lws-core/conformance/ldp.sh",
                      "odrl": "crates/sparq-policy/earl-report.sh"}[suite]
            args = [str(self.out)]
        return subprocess.run(["bash", str(self.root / script), *args],
                              env={**self.env, **env}, capture_output=True, text=True, timeout=30)

    def test_cth_keeps_native_bytes_and_revisions_before_unchanged_ratchet(self):
        result = self.run_driver("cth", MOCK_STATUS="1", CTH_REVISION="")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("BASELINE RATCHET FAILED", result.stderr)
        report = self.out / "sparq-solid-cth-earl.ttl"
        self.assertTrue(report.is_file(), "completed harness must keep its native report")
        self.assertEqual(report.read_text(), NATIVE)
        meta = (self.out / "sparq-solid-cth-revisions.tsv").read_text()
        self.assertIn("specification-tests\t" + "a" * 40, meta)
        self.assertIn("conformance-test-harness\t" + "b" * 40, meta)
        self.assertIn("cth-image\tsha256:fixture", meta)

    def test_incomplete_cth_rerun_removes_export(self):
        self.run_driver("cth")
        result = self.run_driver("cth", MOCK_COMPLETE="0", MOCK_STATUS="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no FRESH EARL report", result.stderr)
        self.assertFalse((self.out / "sparq-solid-cth-earl.ttl").exists())

    def test_cth_requires_source_revision_and_clears_stale_export(self):
        report = self.out / "sparq-solid-cth-earl.ttl"
        report.write_text("stale")
        result = self.run_driver("cth", CTH_REVISION="not-a-commit")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires CTH_REVISION", result.stderr)
        self.assertFalse(report.exists())

    def test_ldp_exports_basic_and_direct_native_reports_separately(self):
        result = self.run_driver("ldp", MOCK_STATUS="2")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        for level, filename in [("basic", "sparq-ldp-earl.ttl"),
                                ("direct", "sparq-ldp-direct-probe-earl.ttl")]:
            report = self.out / filename
            self.assertTrue(report.is_file(), f"missing {level} report")
            self.assertEqual(report.read_bytes(), LDP_NATIVE[level].encode())
            native = self.out / f"ldp-{level}/report/ldp-testsuite-execution-report-earl.ttl"
            self.assertEqual(report.read_bytes(), native.read_bytes())
        meta = (self.out / "ldp-revisions.tsv").read_text()
        self.assertIn("basic-exit\t2\ndirect-exit\t2", meta)
        self.assertIn("ldp-testsuite\t" + "a" * 40, meta)

    def test_missing_ldp_report_cannot_reuse_previous_export(self):
        self.assertEqual(self.run_driver("ldp").returncode, 0)
        result = self.run_driver("ldp", MOCK_COMPLETE="0")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.out / "sparq-ldp-earl.ttl").exists())
        self.assertFalse((self.out / "sparq-ldp-direct-probe-earl.ttl").exists())

    def test_complete_odrl_failure_exports_both_outcomes_and_keeps_status(self):
        result = self.run_driver("odrl", MOCK_STATUS="101")
        self.assertEqual(result.returncode, 101, result.stdout + result.stderr)
        report = (self.out / "sparq-odrl-earl.ttl").read_text()
        self.assertEqual(report.count("a earl:Assertion"), 2)
        self.assertIn("earl:test <urn:uuid:bad>", report)
        self.assertIn("earl:outcome earl:failed", report)

    def test_incomplete_odrl_run_clears_previous_report_and_table(self):
        self.assertEqual(self.run_driver("odrl").returncode, 0)
        result = self.run_driver("odrl", MOCK_COMPLETE="0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Incomplete ODRL run", result.stderr)
        self.assertFalse((self.out / "sparq-odrl-earl.ttl").exists())
        self.assertFalse((self.out / "sparq-odrl-results.md").exists())


if __name__ == "__main__":
    unittest.main()
