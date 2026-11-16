// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { sharedFiles, assertPinnedPackages } from '../../zk-shared/sync.mjs';
import { fileURLToPath } from 'node:url';
test('both packed runtimes match their single shared source', () => {
  for (const name of sharedFiles) for (const packageName of ['zk-query', 'zk-result']) {
    assert.deepEqual(readFileSync(new URL(`../../${packageName}/internal/${name}`, import.meta.url)), readFileSync(new URL(`../../zk-shared/${name}`, import.meta.url)));
  }
  for (const name of ['zk-query', 'zk-result']) assertPinnedPackages(fileURLToPath(new URL(`../../${name}/`, import.meta.url)));
});
