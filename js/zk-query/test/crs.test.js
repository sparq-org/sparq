// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { checkPinnedFiles, sha256 } from '../internal/runtime.mjs';
test('CRS bytes used by the backend are the bytes whose pins were checked', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-query-crs-test-'));
  const first = Buffer.alloc(64, 1), second = Buffer.alloc(128, 2);
  const pins = { 'bn254_g1.dat': { bytes: first.length, sha256: sha256(first) }, 'bn254_g2.dat': { bytes: second.length, sha256: sha256(second) } };
  try {
    writeFileSync(join(directory, 'bn254_g1.dat'), first); writeFileSync(join(directory, 'bn254_g2.dat'), second);
    const captured = await checkPinnedFiles(pins, pathToFileURL(directory + '/'), { 'bn254_g1.dat': first.length });
    assert.deepEqual(Buffer.from(captured['bn254_g1.dat']), first);
    writeFileSync(join(directory, 'bn254_g1.dat'), Buffer.alloc(first.length));
    await assert.rejects(checkPinnedFiles(pins, pathToFileURL(directory + '/')), /digest rejected/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
