#!/usr/bin/env python3
"""[GPT-6] Completion markers must follow hashes, including after partial failure."""
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest


class FinalizationTests(unittest.TestCase):
    def test_success_and_failure_publish_after_hashing_partial_evidence(self):
        source=Path(__file__).with_name('run-instance.sh').read_text()
        function=source[source.index('finalize_artifacts() {'):source.index('date -u +%FT%TZ > "${results}/started-at.txt"')]
        sha=shutil.which('sha256sum')
        if not sha:self.skipTest('sha256sum required by Linux runner')
        for marker,state in [('DONE','complete'),('FAILED','failed')]:
            with self.subTest(marker=marker),tempfile.TemporaryDirectory() as directory:
                base=Path(directory);results=base/'results';results.mkdir();commands=base/'bin';commands.mkdir()
                raw=b'{"record_type":"request","sequence":0}\n{"partial":'
                (results/'cell-requests.jsonl').write_bytes(raw)
                (results/'failure-detail.txt').write_text('injected controlled stop\n')
                # This executes during hash generation, before the marker may be visible.
                probe=commands/'sha256sum'
                probe.write_text('#!/usr/bin/env bash\nset -eu\ntest ! -e "$FINALIZE_TEST_RESULTS/DONE"\ntest ! -e "$FINALIZE_TEST_RESULTS/FAILED"\nexec '+shlex.quote(sha)+' "$@"\n')
                probe.chmod(0o755)
                env=dict(os.environ,PATH=str(commands)+os.pathsep+os.environ['PATH'],FINALIZE_TEST_RESULTS=str(results))
                script='set -euo pipefail\nresults='+shlex.quote(str(results))+'\ncleanup() { :; }\n'+function+'\nfinalize_artifacts '+marker+' '+state+'\n'
                execution=subprocess.run(['bash','-c',script],env=env,capture_output=True,text=True)
                self.assertEqual(execution.returncode,0,execution.stderr)
                self.assertTrue((results/marker).exists())
                self.assertFalse((results/('FAILED' if marker=='DONE' else 'DONE')).exists())
                self.assertEqual((results/'stage.txt').read_text().strip(),state)
                self.assertTrue((results/'finished-at.txt').read_text().strip())
                subprocess.run([sha,'-c','MANIFEST.sha256'],cwd=results,check=True,capture_output=True)
                compressed=results/'cell-requests.jsonl.zst'
                recovered=subprocess.check_output(['zstd','-q','-d','-c',str(compressed)]) if compressed.exists() else (results/'cell-requests.jsonl').read_bytes()
                self.assertEqual(recovered,raw)


if __name__=='__main__':unittest.main()
