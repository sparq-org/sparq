// [OPUS-5] sq-2hk (#3396): the `--target nodejs` (CommonJS) wasm build shipped
// alongside the `--target web` one.
//
// The point of the second target is that a CommonJS consumer can `require()` the
// engine and use it SYNCHRONOUSLY — no `await import(...)`, no `init()`. The web
// glue in `wasm/` cannot do that: it is ESM, and its `Store` statics throw until
// the default async `init` (or `initSync`) has run. So the assertions below are
// deliberately about the CJS-specific contract, not about query results in general
// (those are covered by store.test.mjs against the ESM wrapper).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const readJson = (p) => JSON.parse(readFileSync(resolve(pkgDir, p), 'utf8'));

test('package.json publishes the CommonJS engine under the ./wasm-node subpath', () => {
  const pkg = readJson('package.json');
  assert.ok(
    pkg.files.includes('wasm-node'),
    '`files` must allowlist wasm-node/, or the CJS engine is dropped from the tarball',
  );
  // With an `exports` field present, every subpath Node can reach must be declared —
  // and `require()` picks the `require` condition.
  assert.equal(pkg.exports['./wasm-node'].require, './wasm-node/sparq_wasm.js');
  assert.equal(pkg.exports['./wasm-node'].types, './wasm-node/sparq_wasm.d.ts');
  // The ESM entry must keep resolving to the `--target web` build, not the CJS one.
  assert.equal(pkg.exports['.'].default, './dist/index.js');
});

test('wasm-node/ is scoped back to CommonJS despite the package-root "type": "module"', () => {
  assert.equal(readJson('package.json').type, 'module', 'precondition: the package root is ESM');
  assert.equal(
    readJson('wasm-node/package.json').type,
    'commonjs',
    'without this marker Node parses the CJS glue as ESM and `require()` throws ' +
      '"module is not defined"',
  );
});

test('require() of the nodejs-target glue yields a ready Store — no init() step', () => {
  // `createRequire` gives this ESM test the exact resolution a CommonJS consumer gets.
  const require = createRequire(import.meta.url);
  const engine = require('../wasm-node/sparq_wasm.js');

  assert.equal(typeof engine.Store, 'function');
  // The web glue's async bootstrap must NOT be part of this surface: the nodejs target
  // instantiates the module eagerly inside require(), so there is nothing to await.
  assert.equal(typeof engine.default, 'undefined');

  // Used immediately, with no init: this is the whole contract of the second target.
  const store = engine.Store.load('<http://e/a> <http://e/b> <http://e/c> .', 'ntriples');
  try {
    // `size` is a `#[wasm_bindgen(getter)]` on the Rust `Store`, so it is a PROPERTY on
    // the generated class, not a method (`heapBytes()` next to it is the method).
    assert.equal(store.size, 1);
    assert.equal(store.ask('ASK { ?s ?p ?o }'), true);
    const rows = JSON.parse(store.query('SELECT ?o WHERE { ?s ?p ?o }')).results.bindings;
    assert.deepEqual(
      rows.map((r) => r.o.value),
      ['http://e/c'],
    );
  } finally {
    store.free();
  }
});
