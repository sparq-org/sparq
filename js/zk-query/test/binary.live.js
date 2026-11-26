// Research grade and not externally audited. Requires the pinned native executable.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { copyPinnedBinary } from '../native.mjs';
import pins from '../assets/toolchain.pins.json' with { type: 'json' };
test('the pinned executable also has to report the expected version', async t => {
  const binaries = pins.platforms[`${process.platform}-${process.arch}`];
  if (!binaries) {
    t.skip('native platform has no binary pin');
    return;
  }
  const version = binaries.nargo.version;
  const directory = await mkdtemp(join(tmpdir(), 'sparq-query-version-test-'));
  try {
    await copyPinnedBinary('nargo', process.env.SPARQ_NARGO, join(directory, 'nargo'));
    binaries.nargo.version = 'deliberately mismatched expected version';
    await assert.rejects(copyPinnedBinary('nargo', process.env.SPARQ_NARGO, join(directory, 'other')), /binary version mismatch/);
  } finally { binaries.nargo.version = version; await rm(directory, { recursive: true, force: true }); }
});
