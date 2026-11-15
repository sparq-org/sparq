"""Check local suite registration and deployment routes without external services."""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]


class DeploymentTests(unittest.TestCase):
    def test_runner_generates_presentation_verifier_for_both_suites(self):
        script = (ROOT / 'run-w3c-suites.sh').read_text()
        marker = 'python3 - "$vc_report_dir/did.json" "$vc_suite_root" <<\'PY\'\n'
        self.assertEqual(script.count(marker), 1)
        generator = script.split(marker)[1].split('\nPY\n', 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            did = directory / 'did.json'
            did.write_text(json.dumps({'id': 'did:key:test-issuer'}))
            suites = ['vc-data-model-2.0-test-suite', 'vc-di-eddsa-test-suite']
            for suite in suites:
                (directory / suite).mkdir()
            subprocess.run([sys.executable, '-', str(did), str(directory)],
                           input=generator, text=True, check=True)
            for suite in suites:
                text = (directory / suite / 'localConfig.cjs').read_text()
                config = json.loads(text.removeprefix('module.exports = ').rstrip(';\n'))
                implementation = config['implementations'][0]
                for key, route in [('issuers', '/credentials/issue'),
                                   ('verifiers', '/credentials/verify'),
                                   ('vpVerifiers', '/presentations/verify')]:
                    entry, = implementation[key]
                    self.assertEqual(entry['endpoint'], 'http://127.0.0.1:3000' + route)
                    self.assertEqual(entry['tags'], ['vc2.0', 'eddsa-rdfc-2022'])

    def test_vercel_presentation_rewrite_bin_and_forwarded_route_agree(self):
        config = json.loads((ROOT / 'vercel.json').read_text())
        route, = [r for r in config['rewrites'] if r['source'] == '/presentations/verify']
        self.assertEqual(route['destination'], '/api/verify-presentation')
        manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text())
        binary, = [b for b in manifest['bin'] if b['name'] == 'verify-presentation']
        source = (ROOT / binary['path']).read_text()
        self.assertEqual(re.findall(r'const ROUTE: &str = "([^"]+)";', source),
                         ['/presentations/verify'])
        self.assertIn('include!("../adapter.rs");', source)

    def test_each_vercel_bin_uses_the_runtime_handler_entry_point(self):
        manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text())
        for binary in manifest['bin']:
            with self.subTest(binary=binary['name']):
                source = (ROOT / binary['path']).read_text()
                body, = re.findall(
                    r'async fn main\(\) -> Result<\(\), vercel_runtime::Error> \{\s*(.*?)\s*\}',
                    source, re.DOTALL)
                self.assertEqual(body, 'vercel_runtime::run(handler).await')
