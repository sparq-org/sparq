// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { generatedReadme } from '../scripts/layout.mjs';
test('documented public-input layout follows the Rust declarations and audited ABI', () => {
  assert.equal(readFileSync(new URL('../README.md', import.meta.url), 'utf8'), generatedReadme());
});
