// [SONNET-4.6] sq-y9v8n (#3245) — hardening of the RDF/JS `Source`/`Sink`/`Store` WRITE surface
// (js/src/source.ts): `deleteGraph` graph addressing, and the chunked stream-consume behind
// `import`/`remove`.
//
// What each group pins:
//   (a) deleteGraph — the follow-up asked to VERIFY the string↔term mapping against consumer
//       expectations (N3.js's convention: '' is the default graph, any other string is a
//       named-graph IRI) and to exercise a NAMED-graph delete now that dataset-mode graph
//       addressing is testable (`fromString(..., { dataset: true })`). It also pins the
//       destructive edge the old mapping left open: a `Variable` graph fell through to
//       `SparqStore.match`'s "every graph" wildcard, so `deleteGraph(variable)` deleted the WHOLE
//       dataset. It must now error and delete nothing.
//   (b) `end` ordering — every mutation's returned stream must emit `end` only once the delta is
//       APPLIED. (A plain `new QuadStream()` auto-emits `end` on the next microtask, so the
//       previous out-channel could fire `end` before the work ran; these tests observe the store
//       from INSIDE the `end` handler, which is exactly what a consumer does.)
//   (c) chunked consume — `import`/`remove` apply one delta per `chunkSize` quads instead of
//       buffering the whole stream, so the JS heap holds at most one chunk. The delta calls are
//       counted directly, so the test is about the actual batching, not a proxy for it.
//
// HONESTY — not claimed here: chunking bounds JS-side MEMORY; it is not back-pressure (an RDF/JS
// Stream pushes `data` and sparq's apply is synchronous, so there is no point at which the
// producer can be slowed). Nor is the wasm side streamed — that gap is the same one
// test/source-match-laziness.test.mjs documents for the READ half.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DataFactory as DF, QuadStream, SparqSource, SparqStore } from '../dist/index.js';

const ex = (v) => DF.namedNode(`http://ex/${v}`);
const G1 = ex('g1');
const G2 = ex('g2');

const DATASET = `<http://ex/d> <http://ex/p> "default" .
<http://ex/a> <http://ex/p> "in-g1" <http://ex/g1> .
<http://ex/a> <http://ex/q> "also-g1" <http://ex/g1> .
<http://ex/b> <http://ex/p> "in-g2" <http://ex/g2> .`;

/** A store with 1 default-graph quad, 2 quads in `ex:g1` and 1 in `ex:g2`. */
const loadDataset = () => SparqStore.fromString(DATASET, 'nquads', { dataset: true });

/** Resolves when the stream emits `end`; rejects on `error`. */
const ended = (stream) =>
  new Promise((resolve, reject) => {
    stream.on('end', resolve).on('error', reject);
  });

/** Resolves with the `error` the stream emits; rejects if it ends cleanly instead. */
const errored = (stream) =>
  new Promise((resolve, reject) => {
    stream.on('error', resolve).on('end', () => reject(new Error('expected an error event, got end')));
  });

/** `[defaultGraph, g1, g2]` quad counts — the shape every deleteGraph assertion checks. */
const counts = (store) => [
  store.countQuads(null, null, null, DF.defaultGraph()),
  store.countQuads(null, null, null, G1),
  store.countQuads(null, null, null, G2),
];

/**
 * A hand-rolled RDF/JS Stream that emits on a MACROtask, i.e. strictly later than the microtask on
 * which a `new QuadStream()` would auto-emit `end`. This is what distinguishes "the out-channel
 * ends when the delta is applied" from "the out-channel ends on the next microtask": with an
 * eagerly-ending out-channel the consumer's `end` fires before this stream has emitted anything.
 */
function laterStream(quads) {
  const listeners = new Map();
  const emit = (event, arg) => {
    for (const fn of listeners.get(event) ?? []) fn(arg);
  };
  setTimeout(() => {
    for (const quad of quads) emit('data', quad);
    emit('end');
  }, 0);
  return {
    on(event, fn) {
      const set = listeners.get(event) ?? listeners.set(event, []).get(event);
      set.push(fn);
      return this;
    },
  };
}

/** `n` distinct quads in the default graph. */
const quads = (n) => Array.from({ length: n }, (_, i) => DF.quad(ex(`s${i}`), ex('p'), DF.literal(`o${i}`)));

/**
 * Replaces `store.addQuads` / `store.removeQuads` with counting wrappers (an own property shadows
 * the prototype method the adapter calls), returning the per-call batch sizes. One entry per
 * delta, so `[4, 4, 2]` means "three chunked applies", `[10]` means "one whole-stream apply".
 */
function spyOnDeltas(store, method) {
  const batches = [];
  const original = store[method].bind(store);
  store[method] = (quadsIn) => {
    batches.push(quadsIn.length);
    original(quadsIn);
  };
  return batches;
}

/* ───────────────────────── deleteGraph: graph addressing ───────────────────────── */

test('deleteGraph(NamedNode) removes exactly that named graph', async () => {
  const store = await loadDataset();
  assert.deepEqual(counts(store), [1, 2, 1]);

  await ended(store.asSource().deleteGraph(G1));

  assert.deepEqual(counts(store), [1, 0, 1], 'only ex:g1 should be gone');
  store.free();
});

test('deleteGraph(string) is term-for-term identical to deleteGraph(NamedNode)', async () => {
  const viaString = await loadDataset();
  const viaTerm = await loadDataset();

  await ended(viaString.asSource().deleteGraph('http://ex/g1'));
  await ended(viaTerm.asSource().deleteGraph(G1));

  // The string mapping is the consumer-facing contract (N3.js takes the same argument shape):
  // a non-empty string names the graph whose IRI it is, so both stores must be left identical.
  assert.deepEqual(counts(viaString), counts(viaTerm));
  assert.deepEqual(counts(viaString), [1, 0, 1]);
  viaString.free();
  viaTerm.free();
});

test("deleteGraph('') / deleteGraph(DefaultGraph) remove only the default graph", async () => {
  const viaString = await loadDataset();
  const viaTerm = await loadDataset();

  await ended(viaString.asSource().deleteGraph(''));
  await ended(viaTerm.asSource().deleteGraph(DF.defaultGraph()));

  assert.deepEqual(counts(viaString), [0, 2, 1], "'' must target the default graph, not a graph named ''");
  assert.deepEqual(counts(viaTerm), counts(viaString));
  viaString.free();
  viaTerm.free();
});

test('deleteGraph(Variable) errors instead of deleting every graph', async () => {
  const store = await loadDataset();

  // `SparqStore.match` reads a Variable graph position as a wildcard over EVERY graph, so an
  // unvalidated pass-through here silently wiped the whole dataset.
  const err = await errored(store.asSource().deleteGraph(DF.variable('g')));

  assert.ok(err instanceof Error);
  assert.match(err.message, /Variable/);
  assert.deepEqual(counts(store), [1, 2, 1], 'nothing may be deleted for an un-nameable graph');
  store.free();
});

test('deleteGraph(Literal | undefined) errors and deletes nothing', async () => {
  const store = await loadDataset();

  for (const bad of [DF.literal('g1'), undefined]) {
    const err = await errored(store.asSource().deleteGraph(bad));
    assert.ok(err instanceof Error, `expected an Error for ${String(bad)}`);
  }
  assert.deepEqual(counts(store), [1, 2, 1]);
  store.free();
});

/* ───────────────────────── mutations end only once applied ───────────────────────── */

test('deleteGraph emits end only after the quads are actually gone', async () => {
  const store = await loadDataset();
  const stream = store.asSource().deleteGraph(G1);

  // Observing the store from INSIDE the `end` handler is what a consumer does; it must not see
  // the pre-delete state.
  const observed = await new Promise((resolve, reject) => {
    stream.on('end', () => resolve(counts(store))).on('error', reject);
  });

  assert.deepEqual(observed, [1, 0, 1]);
  store.free();
});

test('import emits end only after the delta is applied, for a stream that emits later', async () => {
  const target = await SparqStore.empty();
  const source = new SparqSource(target);

  const observed = await new Promise((resolve, reject) => {
    source
      .import(laterStream(quads(3)))
      .on('end', () => resolve(target.countQuads()))
      .on('error', reject);
  });

  assert.equal(observed, 3, 'end fired before the imported quads were in the store');
  target.free();
});

/* ───────────────────────── chunked consume ───────────────────────── */

test('import applies one delta per chunkSize quads, not one for the whole stream', async () => {
  const target = await SparqStore.empty();
  const batches = spyOnDeltas(target, 'addQuads');

  await ended(new SparqSource(target, { chunkSize: 4 }).import(new QuadStream(quads(10))));

  assert.deepEqual(batches, [4, 4, 2], 'expected three chunked applies (4 + 4 + remainder)');
  assert.equal(target.countQuads(), 10);
  target.free();
});

test('chunkSize 0 buffers the whole stream and applies exactly one delta', async () => {
  const target = await SparqStore.empty();
  const batches = spyOnDeltas(target, 'addQuads');

  await ended(new SparqSource(target, { chunkSize: 0 }).import(new QuadStream(quads(10))));

  assert.deepEqual(batches, [10]);
  assert.equal(target.countQuads(), 10);
  target.free();
});

test('the default chunk size batches at 1024 quads', async () => {
  const target = await SparqStore.empty();
  const batches = spyOnDeltas(target, 'addQuads');

  await ended(new SparqSource(target).import(new QuadStream(quads(1030))));

  assert.deepEqual(batches, [1024, 6], 'the documented default chunk size is 1024');
  assert.equal(target.countQuads(), 1030);
  target.free();
});

test('a per-call chunkSize overrides the constructor default', async () => {
  const target = await SparqStore.empty();
  const batches = spyOnDeltas(target, 'addQuads');

  await ended(new SparqSource(target, { chunkSize: 0 }).import(new QuadStream(quads(7)), { chunkSize: 3 }));

  assert.deepEqual(batches, [3, 3, 1]);
  target.free();
});

test('remove is chunked the same way as import', async () => {
  const target = await SparqStore.fromQuads(quads(10));
  const batches = spyOnDeltas(target, 'removeQuads');

  await ended(new SparqSource(target, { chunkSize: 4 }).remove(new QuadStream(quads(10))));

  assert.deepEqual(batches, [4, 4, 2]);
  assert.equal(target.countQuads(), 0);
  target.free();
});

test('a mid-stream source error surfaces as error, with no end, keeping applied chunks', async () => {
  const target = await SparqStore.empty();
  const batches = spyOnDeltas(target, 'addQuads');
  function* boom() {
    yield* quads(5);
    throw new Error('source blew up');
  }

  const err = await errored(new SparqSource(target, { chunkSize: 2 }).import(new QuadStream(boom())));

  assert.match(err.message, /source blew up/);
  // Documented incremental contract: chunks applied before the error stay applied.
  assert.deepEqual(batches, [2, 2]);
  assert.equal(target.countQuads(), 4);
  target.free();
});

test('an invalid chunkSize is rejected up front', async () => {
  const store = await SparqStore.empty();
  assert.throws(() => new SparqSource(store, { chunkSize: -1 }), RangeError);
  assert.throws(() => new SparqSource(store, { chunkSize: 1.5 }), RangeError);
  assert.throws(() => new SparqSource(store).import(new QuadStream(), { chunkSize: -2 }), RangeError);
  store.free();
});
