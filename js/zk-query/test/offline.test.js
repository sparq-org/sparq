// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, cpSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
test('a fresh checkout can invoke either offline wrapper before generated support exists', () => {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-query-offline-test-'));
  try {
    for (const packageName of ['zk-result', 'zk-query']) {
      const path = join(directory, packageName, 'scripts'); mkdirSync(path, { recursive: true });
      cpSync(fileURLToPath(new URL(`../../${packageName}/scripts/offline.py`, import.meta.url)), join(path, 'offline.py'));
    }
    mkdirSync(join(directory, 'zk-shared')); cpSync(fileURLToPath(new URL('../../zk-shared/offline.py', import.meta.url)), join(directory, 'zk-shared/offline.py'));
    for (const packageName of ['zk-result', 'zk-query']) {
      const result = execFileSync('python3', [join(directory, packageName, 'scripts/offline.py'), process.execPath, '--version'], { encoding: 'utf8' });
      assert.equal(result.trim(), process.version);
    }
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
