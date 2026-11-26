// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { generatedReadme } from '../scripts/layout.mjs';
test('documented public-input layout follows the Rust declarations and audited ABI', () => {
  assert.equal(readFileSync(new URL('../README.md', import.meta.url), 'utf8'), generatedReadme());
});

test('the SELECT layout fixture describes the catalogued BGP example', () => {
  const catalogue = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
  const bgp = catalogue.entries.find(entry => entry.example.name === 'bgp');
  const fixture = JSON.parse(readFileSync(new URL('./fixtures/layout-select-abi.json', import.meta.url)));

  assert.equal(fixture.query, bgp.example.query);
  assert.deepEqual(fixture.bounds, bgp.example.bounds);
});
