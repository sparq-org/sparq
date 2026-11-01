import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import { SparqStore } from '../dist/index.js';
import { Store } from '../wasm/sparq_wasm.js';

test('default web and Node bundles load inline and registry JSON-LD contexts', async () => {
  const parsed = await SparqStore.fromString('<https://example.test/s> <https://example.test/p> "value" .', 'ntriples');
  try {
    assert.equal(parsed.size, 1);
    assert.equal(parsed.queryBoolean('ASK { <https://example.test/s> <https://example.test/p> "value" }'), true);
  } finally {
    parsed.free();
  }

  const require = createRequire(import.meta.url);
  const { Store: NodeStore } = require('../wasm-node/sparq_wasm.js');
  const url = 'https://contexts.example/name';
  const context = JSON.stringify({ '@context': { name: 'https://example.test/name' } });
  const document = JSON.stringify({
    '@context': [url, { title: 'https://example.test/title' }],
    '@id': 'https://example.test/alice',
    name: 'Alice',
    title: 'Example',
  });
  for (const EngineStore of [Store, NodeStore]) {
    assert.equal(typeof EngineStore.loadJsonLdWithContexts, 'function');
    const store = EngineStore.loadJsonLdWithContexts(document, [[url, context]]);
    try {
      assert.equal(store.size, 2);
      assert.equal(store.ask('ASK { <https://example.test/alice> <https://example.test/name> "Alice"; <https://example.test/title> "Example" }'), true);
    } finally {
      store.free();
    }
  }
});
