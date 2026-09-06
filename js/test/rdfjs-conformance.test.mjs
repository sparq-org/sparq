// [OPUS-4.8] sq-iwhl8 (#1116) — runs the contributable RDF/JS conformance harness
// (`@rdfjs-test/conformance`, packages/rdfjs-conformance) against `@sparq-org/sparq` ITSELF, so the
// SAME spec-derived suites that pass for N3.js (see the harness's own test/n3-parity.test.mjs)
// also pass for sparq. This is the maintainer's "should pass the test suites defined for other
// RDF/JS conformant libraries" requirement, met by sharing one runner across libraries.
//
// The DataFactory suite is fully synchronous; the Dataset / Source suites need the wasm engine
// up, so we `await init()` once and drive sparq's SYNCHRONOUS `datasetFactory` (the RDF/JS
// `DatasetCoreFactory` + `DatasetFactory`) and `store.asSource()` (the RDF/JS `Source`/`Sink`).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  runDataFactoryTests,
  runDatasetTests,
  runStreamTests,
} from '@rdfjs-test/conformance';
import {
  DataFactory,
  datasetFactory,
  SparqStore,
  SparqSource,
  init,
} from '../dist/index.js';

// --- DataFactory + Term hierarchy (synchronous; no engine needed) --------------------------------
await runDataFactoryTests({ factory: DataFactory, label: '@sparq-org/sparq' });

// --- DatasetCore + Dataset algebra (engine up; synchronous factory) ------------------------------
await init();
await runDatasetTests({
  factory: DataFactory,
  datasetFactory: (quads) => datasetFactory.dataset(quads),
  label: '@sparq-org/sparq',
});

// --- Stream / Source / Sink ----------------------------------------------------------------------
const sourceStore = await SparqStore.fromString(
  '<http://ex/s> <http://ex/p> <http://ex/o> .\n<http://ex/s> <http://ex/p2> <http://ex/o2> .',
  'ntriples',
);
const source = sourceStore.asSource();
await runStreamTests({ source, sink: source, factory: DataFactory, label: '@sparq-org/sparq' });

// --- a focused smoke that the Source/Sink adapter round-trips through the engine -----------------
test('@sparq-org/sparq Source/Sink round-trips quads through the engine', async () => {
  const store = await SparqStore.fromString('', 'ntriples');
  const src = new SparqSource(store);
  const q = DataFactory.quad(
    DataFactory.namedNode('http://ex/a'),
    DataFactory.namedNode('http://ex/b'),
    DataFactory.namedNode('http://ex/c'),
  );

  // import via a quad Stream (the adapter applies O(batch) deltas of `chunkSize` quads, so a
  // one-quad stream lands as a single delta on `end` — see test/source-hardening.test.mjs)
  const inStream = new SparqSource(await SparqStore.fromQuads([q])).match();
  await new Promise((resolve, reject) => {
    src.import(inStream).on('end', resolve).on('error', reject);
  });
  assert.equal(store.size, 1);

  // match() yields a spec-shaped Stream: data-per-quad then end
  const seen = [];
  await new Promise((resolve, reject) => {
    src
      .match()
      .on('data', (quad) => seen.push(quad))
      .on('end', resolve)
      .on('error', reject);
  });
  assert.equal(seen.length, 1);
  assert.ok(seen[0].equals(q));

  // removeMatches drains it again
  await new Promise((resolve, reject) => {
    src.removeMatches(null, null, null, null).on('end', resolve).on('error', reject);
  });
  assert.equal(store.size, 0);
  store.free();
});
