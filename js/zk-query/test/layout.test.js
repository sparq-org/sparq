// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { generatedReadme } from '../scripts/layout.mjs';
import { sha256 } from '../internal/runtime.mjs';
test('documented public-input layout follows the Rust declarations and audited ABI', () => {
  assert.equal(readFileSync(new URL('../README.md', import.meta.url), 'utf8'), generatedReadme());
});

test('the SELECT layout fixture matches the pinned BGP program ABI', () => {
  const assets = new URL('../assets/', import.meta.url);
  const catalogue = JSON.parse(readFileSync(new URL('catalogue.json', assets)));
  const bgp = catalogue.entries.find(entry => entry.example.name === 'bgp');
  const program = readFileSync(new URL(bgp.acir, assets));
  const fixture = JSON.parse(readFileSync(new URL('./fixtures/layout-select-abi.json', import.meta.url)));

  assert.equal(sha256(program), bgp.acirSha256);
  assert.equal(fixture.query, bgp.example.query);
  assert.deepEqual(fixture.bounds, bgp.example.bounds);
  assert.deepEqual(fixture.abi, JSON.parse(program).abi);
});
