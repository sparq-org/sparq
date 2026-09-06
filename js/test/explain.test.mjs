// [OPUS-4.8] sq-ncvq.14: query-plan introspection (EXPLAIN / EXPLAIN ANALYZE)
// exposed on the raw wasm `Store`, closing the Rust+HTTP vs WASM/JS asymmetry.
// The raw `WasmStore` is the layer that carries the plan-introspection methods
// (the high-level `SparqStore` wrapper covers SELECT/ASK only, mirroring how
// CONSTRUCT/DESCRIBE live on the raw store too), so this exercises it directly.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { init, WasmStore } from '../dist/wasm.js';

const DATA = `@prefix ex: <http://ex/> .
ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
ex:bob ex:name "Bob"@en ; ex:age 25 .`;

const load = async () => {
  await init();
  return WasmStore.load(DATA, 'turtle');
};

test('explain() returns the planning-only plan text (no execution)', async () => {
  const store = await load();
  const plan = store.explain(
    'PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a }',
  );
  assert.equal(typeof plan, 'string');
  assert.match(plan, /EXPLAIN \(SELECT\)/);
  assert.match(plan, /Plan:/);
  // EXPLAIN accepts every query form, including the graph-valued ones.
  assert.match(
    store.explain('PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:q ?o } WHERE { ?s ex:knows ?o }'),
    /EXPLAIN \(CONSTRUCT\)/,
  );
  // A malformed query surfaces as a thrown JS error.
  assert.throws(() => store.explain('SELECT WHERE {'));
});

test('explainAnalyze() runs SELECT/ASK and appends the operator trace', async () => {
  const store = await load();
  const trace = store.explainAnalyze('PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }');
  assert.match(trace, /EXPLAIN ANALYZE \(SELECT\)/);
  assert.match(trace, /Plan:/);
  // CONSTRUCT/DESCRIBE are explain-only — explainAnalyze rejects them.
  assert.throws(
    () => store.explainAnalyze('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }'),
    /EXPLAIN ANALYZE/,
  );
});

// [FABLE-5] sq-ixc3.19 — the STRUCTURED plan bindings (explain-json, in the published
// bundle): the typed camelCase tree (the sq-jbqh4 schema contract) the GUI plan explorer
// renders. Same engine functions as the text forms; JSON.parse-able verbatim.
test('explainPlanJson() returns the camelCase typed tree (planning-only)', async () => {
  const store = await load();
  const tree = JSON.parse(
    store.explainPlanJson('PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }'),
  );
  assert.equal(typeof tree.operator, 'string');
  assert.ok(Array.isArray(tree.children));
  // A dry run executes nothing.
  assert.equal(tree.actual, null);
  assert.equal(tree.qError, null);
});

test('explainPlanAnalyzeJson() executes and fills exact actual rows + real wall nanos on wasm', async () => {
  const store = await load();
  const tree = JSON.parse(
    store.explainPlanAnalyzeJson('PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }'),
  );
  // Two ex:name triples in the fixture — exact row counts cross the boundary.
  assert.equal(tree.actual, 2);
  // sq-vx7ez (#2428): ANALYZE installs the performance.now() host clock, so wall nanos
  // are REAL per-operator times — a finite NUMBER (not null). A tiny operator may still
  // legitimately read 0 under host-timer coarsening, so assert a valid measurement (a
  // non-negative finite integer) rather than a specific value.
  assert.equal(typeof tree.nanos, 'number');
  assert.ok(Number.isFinite(tree.nanos) && tree.nanos >= 0, `nanos=${tree.nanos}`);
  // Graph-valued forms are rejected, matching the Rust API.
  assert.throws(
    () => store.explainPlanAnalyzeJson('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }'),
    /EXPLAIN ANALYZE/,
  );
});
