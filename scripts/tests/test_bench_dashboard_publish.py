#!/usr/bin/env python3
"""[GPT-6-ASTRA] Execute the real dashboard shell step against local Git races.

Only file:// transport is enabled. A Git wrapper inserts deterministic competing
commits before the publisher's push; the publisher itself is extracted unchanged
from bench.yml. No GitHub token, network request or remote publication is used.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
BENCH_YML = REPO_ROOT / ".github/workflows/bench.yml"
REAL_GIT = shutil.which("git")
ASSETS = ("index.html", "dashboard.js", "dashboard.css", "metric-labels.json",
          "competitors.json", "vendor/Chart.min.js")

# This wrapper alters only the fixture's on-disk repository, then runs real Git.
# Failed-transport cases deliberately provide no porcelain non-fast-forward record.
WRAPPER = r"""#!/usr/bin/env python3
import json, os, pathlib, shutil, subprocess, sys
args = sys.argv[1:]
events = pathlib.Path(os.environ['PUBLISH_TEST_EVENTS'])
mode = os.environ.get('PUBLISH_TEST_MODE', '')
real = os.environ['PUBLISH_TEST_GIT']

def git(*cmd):
    p = subprocess.run([real, *cmd], text=True, capture_output=True)
    if p.returncode:
        raise RuntimeError(p.stderr)
    return p.stdout.strip()

if 'fetch' in args or 'push' in args:
    kind = 'push' if 'push' in args else 'fetch'
    prior = [json.loads(l) for l in events.read_text().splitlines()] if events.exists() else []
    count = sum(x['kind'] == kind for x in prior) + 1
    with events.open('a') as f:
        f.write(json.dumps({'kind': kind, 'count': count}) + '\n')
    if kind == 'push':
        if count <= int(os.environ.get('PUBLISH_TEST_COLLISIONS', '0')):
            actor = pathlib.Path(os.environ['PUBLISH_TEST_ACTOR'])
            git('-C', str(actor), 'fetch', '-q', 'origin', 'benchmark-data')
            git('-C', str(actor), 'checkout', '-q', '-B', 'benchmark-data', 'FETCH_HEAD')
            for name in ['data.js', 'unrelated.txt']:
                f = actor / name
                f.write_text(f.read_text() + f'concurrent-{count}\n')
            if mode == 'same_assets':
                src = pathlib.Path(os.environ['GITHUB_WORKSPACE']) / 'bench/dashboard'
                for f in src.rglob('*'):
                    if f.is_file():
                        dest = actor / ('index.html' if f.name == 'root-redirect.html'
                                        else 'dev/bench/' + f.relative_to(src).as_posix())
                        dest.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copyfile(f, dest)
            git('-C', str(actor), 'add', '.')  # synthetic fixture only
            if mode == 'rewrite':
                # Test-only local ref replacement; never a force push.
                tree = git('-C', str(actor), 'write-tree')
                commit = git('-C', str(actor), 'commit-tree', tree, '-m', 'replacement root')
                git('-C', str(actor), 'push', '-q', 'origin', commit + ':refs/heads/replacement')
                git('--git-dir', os.environ['PUBLISH_TEST_BARE'], 'update-ref',
                    'refs/heads/benchmark-data', commit)
            else:
                git('-C', str(actor), 'commit', '-q', '-m', f'concurrent history {count}')
                git('-C', str(actor), 'push', '-q', 'origin', 'benchmark-data:benchmark-data')
        if mode in ['authentication', 'rate_limit', 'unknown']:
            print({'authentication': 'fatal: Authentication failed',
                   'rate_limit': 'fatal: remote rate limit exceeded',
                   'unknown': 'fatal: unexpected transport failure'}[mode], file=sys.stderr)
            sys.exit(128)
        if mode == 'mixed_rejections':
            print('!\trefs/heads/benchmark-data:refs/heads/benchmark-data\t[rejected] (fetch first)')
            print('!\trefs/tags/other:refs/tags/other\t[remote rejected] (policy refusal)')
            sys.exit(1)
        if mode == 'stationary_nff':
            print('!\trefs/heads/benchmark-data:refs/heads/benchmark-data\t[rejected] (non-fast-forward)')
            sys.exit(1)
sys.exit(subprocess.run([real, *args]).returncode)
"""


class TestBenchDashboardPublish(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bare = self.root / "history.git"
        self.actor = self.root / "actor"
        self.workspace = self.root / "workspace"
        self.src = self.workspace / "bench/dashboard"
        self.events = self.root / "events.jsonl"
        config = self.root / "gitconfig"
        config.write_text(
            '[user]\n name = Offline Test\n email = offline@example.invalid\n'
            '[commit]\n gpgsign = false\n'
            f'[url "{self.bare.as_uri()}"]\n'
            ' insteadOf = https://x-access-token:offline@github.com/offline/bench.git\n')
        self.env = dict(os.environ, GIT_CONFIG_GLOBAL=str(config), GIT_CONFIG_NOSYSTEM="1",
                        GIT_ALLOW_PROTOCOL="file", GIT_TERMINAL_PROMPT="0",
                        GH_TOKEN="offline", GITHUB_REPOSITORY="offline/bench",
                        GITHUB_WORKSPACE=str(self.workspace),
                        PUBLISH_TEST_GIT=REAL_GIT, PUBLISH_TEST_ACTOR=str(self.actor),
                        PUBLISH_TEST_BARE=str(self.bare), PUBLISH_TEST_EVENTS=str(self.events))
        self.git('init', '-q', '--bare', str(self.bare))
        self.git('init', '-q', str(self.actor))
        self.git('-C', str(self.actor), 'checkout', '-q', '-b', 'benchmark-data')
        self.git('-C', str(self.actor), 'remote', 'add', 'origin', self.bare.as_uri())
        for name in ['data.js', 'unrelated.txt']:
            (self.actor / name).write_text('initial\n')
        self.git('-C', str(self.actor), 'add', 'data.js', 'unrelated.txt')
        self.git('-C', str(self.actor), 'commit', '-q', '-m', 'initial history')
        self.git('-C', str(self.actor), 'push', '-q', 'origin', 'benchmark-data')
        for name in (*ASSETS, 'root-redirect.html'):
            f = self.src / name
            f.parent.mkdir(parents=True, exist_ok=True)
            f.write_text('source: ' + name + '\n')
        bindir = self.root / 'bin'
        bindir.mkdir()
        wrapper = bindir / 'git'
        wrapper.write_text(WRAPPER)
        wrapper.chmod(0o755)
        self.env['PATH'] = str(bindir) + os.pathsep + os.environ['PATH']

    def git(self, *args):
        return subprocess.run([REAL_GIT, *args], env=self.env, text=True,
                              capture_output=True, check=True).stdout.strip()

    def remote(self, path):
        return self.git('--git-dir', str(self.bare), 'show', 'benchmark-data:' + path)

    def run_step(self, *, mode='', collisions=0):
        wf = yaml.safe_load(BENCH_YML.read_text())
        step = next(s for s in wf['jobs']['bench']['steps']
                    if s.get('name') == 'Seed Pages dashboard onto benchmark-data (if absent)')
        self.assertEqual(step['if'], "github.event_name != 'schedule' && github.ref == 'refs/heads/main'")
        script = self.root / 'publish.sh'
        script.write_text(step['run'])
        env = dict(self.env, PUBLISH_TEST_MODE=mode, PUBLISH_TEST_COLLISIONS=str(collisions))
        return subprocess.run(['/bin/bash', str(script)], env=env, cwd=self.workspace,
                              text=True, capture_output=True, timeout=30)

    def counts(self):
        rows = [json.loads(l) for l in self.events.read_text().splitlines()]
        return {kind: sum(r['kind'] == kind for r in rows) for kind in ['fetch', 'push']}

    def assert_assets(self):
        for name in ASSETS:
            self.assertEqual(self.remote('dev/bench/' + name), 'source: ' + name)
        self.assertEqual(self.remote('index.html'), 'source: root-redirect.html')

    def test_first_push_publishes_only_dashboard_assets(self):
        r = self.run_step()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.counts(), {'fetch': 1, 'push': 1})
        self.assert_assets()
        self.assertEqual(self.remote('data.js'), 'initial')
        self.assertEqual(self.remote('unrelated.txt'), 'initial')

    def test_concurrent_history_advance_recovers_without_lost_data(self):
        r = self.run_step(collisions=1)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.counts(), {'fetch': 2, 'push': 2})
        self.assertIn('advance verified', r.stdout)
        self.assert_assets()
        self.assertEqual(self.remote('data.js'), 'initial\nconcurrent-1')
        self.assertEqual(self.remote('unrelated.txt'), 'initial\nconcurrent-1')

    def test_repeated_collisions_stop_at_the_bound(self):
        r = self.run_step(collisions=10)
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.counts(), {'fetch': 3, 'push': 3})
        self.assertIn('exhausted', r.stdout)
        self.assertEqual(self.remote('data.js'), 'initial\nconcurrent-1\nconcurrent-2\nconcurrent-3')
        self.assertEqual(self.remote('unrelated.txt'), self.remote('data.js'))

    def test_identical_concurrent_publication_becomes_a_no_op(self):
        r = self.run_step(mode='same_assets', collisions=1)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.counts(), {'fetch': 2, 'push': 1})
        self.assertIn('already up to date', r.stdout)
        self.assert_assets()
        self.assertEqual(self.remote('data.js'), 'initial\nconcurrent-1')
        before = self.git('--git-dir', str(self.bare), 'rev-parse', 'benchmark-data')
        self.events.unlink()
        r = self.run_step()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.counts(), {'fetch': 1, 'push': 0})
        self.assertEqual(self.git('--git-dir', str(self.bare), 'rev-parse', 'benchmark-data'), before)

    def test_actual_permanent_receive_hook_rejection_is_not_retried(self):
        hook = self.bare / 'hooks/pre-receive'
        hook.write_text('#!/bin/sh\necho "permanent policy refusal" >&2\nexit 1\n')
        hook.chmod(0o755)
        r = self.run_step()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('pre-receive hook declined', r.stdout)
        self.assertEqual(self.counts(), {'fetch': 1, 'push': 1})
        self.assertEqual(self.remote('data.js'), 'initial')

    def test_transport_failures_do_not_retry_even_after_a_real_advance(self):
        for mode in ['authentication', 'rate_limit', 'unknown', 'mixed_rejections']:
            with self.subTest(mode=mode):
                if self.events.exists():
                    self.events.unlink()
                r = self.run_step(mode=mode, collisions=1)
                self.assertNotEqual(r.returncode, 0)
                self.assertEqual(self.counts(), {'fetch': 1, 'push': 1})
                self.assertNotIn('advance verified', r.stdout)

    def test_non_fast_forward_without_tip_change_is_not_retried(self):
        r = self.run_step(mode='stationary_nff')
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(self.counts(), {'fetch': 2, 'push': 1})
        self.assertIn('not followed by a verified branch advance', r.stdout)

    def test_non_ancestor_tip_replacement_is_not_retried(self):
        r = self.run_step(mode='rewrite', collisions=1)
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(self.counts(), {'fetch': 2, 'push': 1})
        self.assertIn('not followed by a verified branch advance', r.stdout)


if __name__ == '__main__':
    unittest.main()
