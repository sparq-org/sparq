"""Exercise cache preparation without permitting real network access."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    'vc_context_preparation', PACKAGE_ROOT / 'prepare-contexts.py')
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


class Response(io.BytesIO):
    url = 'https://cache.example/resolved'


class PreparationTests(unittest.TestCase):
    def setUp(self):
        poison = patch.object(PREPARE.urllib.request, 'urlopen',
                              side_effect=AssertionError('network is forbidden in these tests'))
        self.urlopen = poison.start()
        self.addCleanup(poison.stop)

    def run_main(self, directory):
        with patch.object(PREPARE.sys, 'argv', ['prepare-contexts.py', str(directory)]):
            with patch('sys.stdout', new_callable=io.StringIO):
                PREPARE.main()

    def test_original_bytes_and_provenance_are_preserved(self):
        data = b'{ "@context": {"name":"https://schema.org/name"} }\n'
        self.urlopen.side_effect = lambda *_args, **_kwargs: Response(data)
        with tempfile.TemporaryDirectory() as directory:
            self.run_main(directory)
            root = Path(directory)
            manifest = json.loads((root / 'manifest.json').read_text())
            self.assertEqual(set(manifest), set(PREPARE.CONTEXTS))
            self.assertEqual(self.urlopen.call_count, 7)
            for filename, url in PREPARE.CONTEXTS.items():
                self.assertEqual((root / filename).read_bytes(), data)
                self.assertEqual(manifest[filename], {
                    'url': url, 'resolvedUrl': Response.url,
                    'sha256': hashlib.sha256(data).hexdigest()})

    def test_failed_preparation_does_not_replace_existing_cache(self):
        responses = iter([b'{"@context":{}}', b'<html>not a context</html>'])
        self.urlopen.side_effect = lambda *_args, **_kwargs: Response(next(responses))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            existing = root / 'credentials-v2.jsonld'
            existing.write_bytes(b'original cache bytes')
            with self.assertRaises(json.JSONDecodeError):
                self.run_main(directory)
            self.assertEqual(existing.read_bytes(), b'original cache bytes')
            self.assertFalse((root / 'manifest.json').exists())

    def test_non_context_documents_are_rejected(self):
        self.urlopen.side_effect = lambda *_args, **_kwargs: Response(b'{}')
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, 'not a JSON-LD context'):
                self.run_main(directory)

    def test_default_cache_follows_the_script_location(self):
        self.urlopen.side_effect = lambda *_args, **_kwargs: Response(b'{"@context":{}}')
        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / 'relocated-package' / 'prepare-contexts.py'
            with patch.object(PREPARE, '__file__', str(script)):
                with patch.object(PREPARE.sys, 'argv', [str(script)]):
                    with patch('sys.stdout', new_callable=io.StringIO):
                        PREPARE.main()
            for filename in PREPARE.CONTEXTS:
                self.assertTrue((script.parent / 'contexts' / filename).is_file())


if __name__ == '__main__':
    unittest.main()
