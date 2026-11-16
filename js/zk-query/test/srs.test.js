// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { srsPointsForKey } from '../internal/runtime.mjs';
test('prover SRS capacity follows the pinned key header and refuses insufficient points', () => {
  const table = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
  const pins = JSON.parse(readFileSync(new URL('../assets/crs.pins.json', import.meta.url)));
  const key = Buffer.from(readFileSync(new URL('../assets/' + table.entries[0].vk, import.meta.url), 'utf8').trim(), 'hex');
  const count = srsPointsForKey(key, pins);
  const exact = { ...pins, 'bn254_g1.dat': { ...pins['bn254_g1.dat'], bytes: count * 64 } };
  assert.equal(srsPointsForKey(key, exact), count);
  assert.throws(() => srsPointsForKey(key, { ...exact, 'bn254_g1.dat': { ...exact['bn254_g1.dat'], bytes: (count - 1) * 64 } }), /exceeds bundled CRS/);
  assert.throws(() => srsPointsForKey(Buffer.alloc(0), pins), /key header rejected/);
});
