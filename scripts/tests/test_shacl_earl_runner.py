#!/usr/bin/env python3
"""Exercise SHACL report selection with isolated local command fixtures."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
PIN = "04ecbb4a5b930f4fa9df6c399ac5c98ccec672c9"
ABSOLUTE = "https://w3c.github.io/rdf-tests/shacl/shacl12/absolute-rule"


class ShaclEarlRunnerTests(unittest.TestCase):
    def run_fixture(self, truncate=False, omit_feature=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "crates/sparq-shacl/earl-report.sh"
            script.parent.mkdir(parents=True)
            text = (ROOT / "crates/sparq-shacl/earl-report.sh").read_text()
            if omit_feature:
                text = text.replace("--features shacl-af,sparql-rl", "--features shacl-af")
            script.write_text(text)
            scripts = root / "scripts"
            scripts.mkdir()
            shutil.copyfile(ROOT / "scripts/earl-report.py", scripts / "earl-report.py")
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.0.0"\n')
            suite = script.parent / "tests/shacl/data-shapes"
            suite.mkdir(parents=True)
            binary = root / "bin"
            binary.mkdir()
            # Every git invocation is intercepted. Unexpected operations (including
            # any network access) fail rather than falling back to the real program.
            git = binary / "git"
            git.write_text("#!/usr/bin/env python3\nimport sys\n"
                           "assert sys.argv[-2:] == ['rev-parse','HEAD'], sys.argv\n"
                           f"print({PIN!r})\n")
            git.chmod(0o755)
            cargo = binary / "cargo"
            cargo.write_text('''#!/usr/bin/env python3
import os,sys
from pathlib import Path
args=sys.argv[1:]
features=args[args.index('--features')+1].split(',')
if set(features) != {'shacl-af','sparql-rl'}:
    raise SystemExit('Missing rule features')
if '--no-run' in args: raise SystemExit(0)
runner=args[args.index('--test')+1]
suite=Path(os.environ['FIXTURE_SUITE'])
legacy=runner in {'w3c_core','w3c_sparql','w3c_sparql_component'}
section='data-shapes-test-suite' if legacy else 'shacl12-test-suite'
count=342 if runner=='w3c_sparql_rl_shacl12' else 47 if runner=='w3c_inference_rules_shacl12' else 1
written=count-1 if os.environ.get('FIXTURE_TRUNCATE')=='1' and runner=='w3c_sparql_rl_shacl12' else count
with Path(os.environ['SPARQ_EARL_OUT']).open('w') as out:
    for i in range(written):
        iri='https://w3c.github.io/rdf-tests/shacl/shacl12/absolute-rule' if runner=='w3c_sparql_rl_shacl12' and i==0 else f'file://{suite}/{section}/tests/{runner}/entry-{i}'
        outcome='failed' if runner=='w3c_sparql_rl_shacl12' and i>=339 else 'passed'
        out.write(f'{iri}\\t{outcome}\\n')
print(f'CENSUS_COMPLETE {count}')
''')
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=f"{binary}:{os.environ['PATH']}",
                       SHACL_TESTS_PIN=PIN, FIXTURE_SUITE=str(suite),
                       FIXTURE_TRUNCATE="1" if truncate else "0")
            output = root / "out"
            process = subprocess.run(["bash", str(script), str(output)], env=env,
                                     capture_output=True, text=True, check=False)
            files = {p.name: p.read_text() for p in output.glob("*") if p.is_file()}
            return process, files

    def test_mixed_absolute_and_local_test_iris_reach_the_final_report(self):
        process, files = self.run_fixture()
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        report = files["sparq-shacl12-earl.ttl"]
        self.assertTrue(f"earl:test <{ABSOLUTE}>" in report, "Published test IRI was dropped")
        self.assertEqual(report.count("a earl:Assertion"), 393)
        self.assertEqual(report.count("earl:outcome earl:failed"), 3)
        self.assertNotIn(ABSOLUTE, files["sparq-shacl-earl.ttl"])
        self.assertEqual(len(files["w3c_sparql_rl_shacl12.tsv"].splitlines()), 342)
        self.assertEqual(len(files["shacl12-test-suite.tsv"].splitlines()), 393)

    def test_truncated_processor_census_is_rejected(self):
        process, _ = self.run_fixture(truncate=True)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn("Incomplete EARL census from w3c_sparql_rl_shacl12", process.stderr)

    def test_missing_processor_feature_is_rejected(self):
        process, _ = self.run_fixture(omit_feature=True)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn("Missing rule features", process.stderr)


if __name__ == "__main__":
    unittest.main()
