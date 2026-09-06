// Benchmark sketch: sparq (this package) vs oxigraph's npm package on a small
// synthetic workload — load N-Triples, full scan, predicate scan, a two-pattern
// join, and a COUNT aggregate. oxigraph is optional: `npm i --no-save oxigraph`
// to enable the comparison; the script skips it gracefully when absent.
//
// This is a *sketch*, not a rigorous benchmark (single process, no warm-up
// isolation, one dataset shape). Treat the numbers as directional only.

import { SparqStore } from '../dist/index.js';

const N = Number(process.env.BENCH_TRIPLES ?? 100_000);

function makeNTriples(n) {
  const people = Math.floor(n / 4);
  let out = '';
  for (let i = 0; i < people; i++) {
    out += `<http://ex/p${i}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Person> .\n`;
    out += `<http://ex/p${i}> <http://ex/name> "Person ${i}" .\n`;
    out += `<http://ex/p${i}> <http://ex/age> "${20 + (i % 60)}"^^<http://www.w3.org/2001/XMLSchema#integer> .\n`;
    out += `<http://ex/p${i}> <http://ex/knows> <http://ex/p${(i + 1) % people}> .\n`;
  }
  return out;
}

const QUERIES = {
  'full scan (count rows)': 'SELECT ?s ?p ?o WHERE { ?s ?p ?o }',
  'predicate scan': 'SELECT ?s ?n WHERE { ?s <http://ex/name> ?n }',
  'join knows→name': 'SELECT ?a ?n WHERE { ?a <http://ex/knows> ?b . ?b <http://ex/name> ?n }',
  'filtered + limit': 'SELECT ?s ?a WHERE { ?s <http://ex/age> ?a FILTER(?a > 70) } LIMIT 100',
};

function time(fn) {
  const t0 = performance.now();
  const out = fn();
  return [performance.now() - t0, out];
}

function bench(label, fn, repeat = 3) {
  fn(); // warm-up
  let best = Infinity;
  let result;
  for (let i = 0; i < repeat; i++) {
    const [ms, out] = time(fn);
    if (ms < best) [best, result] = [ms, out];
  }
  console.log(`  ${label.padEnd(28)} ${best.toFixed(1).padStart(8)} ms  (${result} rows)`);
}

function measure(fn, repeat = 3) {
  fn();
  let best = Infinity;
  let result;
  for (let i = 0; i < repeat; i++) {
    const [ms, out] = time(fn);
    if (ms < best) [best, result] = [ms, out];
  }
  return { ms: best, result };
}

async function measureAsync(fn, repeat = 3) {
  await fn();
  let best = Infinity;
  let result;
  for (let i = 0; i < repeat; i++) {
    const t0 = performance.now();
    const out = await fn();
    const ms = performance.now() - t0;
    if (ms < best) [best, result] = [ms, out];
  }
  return { ms: best, result };
}

function share(rawMs, wrappedMs) {
  return wrappedMs === 0 ? 0 : Math.max(0, (wrappedMs - rawMs) / wrappedMs);
}

function consumeResultStream(streamPromise) {
  return streamPromise.then((stream) => new Promise((resolve, reject) => {
    let rows = 0;
    stream
      .on('data', () => {
        rows++;
      })
      .on('end', () => resolve(rows))
      .on('error', reject);
  }));
}

const nt = makeNTriples(N);
console.log(`dataset: ${N.toLocaleString()} triples (${(nt.length / 1e6).toFixed(1)} MB N-Triples)\n`);

// --- sparq ---------------------------------------------------------------------------------------
{
  console.log('sparq (wasm):');
  const t0 = performance.now();
  const store = await SparqStore.fromString(nt, 'ntriples');
  console.log(`  ${'load N-Triples'.padEnd(28)} ${(performance.now() - t0).toFixed(1).padStart(8)} ms  (${store.size} triples)`);
  for (const [label, q] of Object.entries(QUERIES)) {
    bench(label, () => store.query(q).length);
  }
  bench('count (no materialise)', () => store.count(QUERIES['full scan (count rows)']));
  console.log(`  heapBytes ≈ ${(store.heapBytes() / 1e6).toFixed(1)} MB\n`);

  const wrapperShare = {};
  for (const [label, query] of Object.entries(QUERIES)) {
    const raw = measure(() => store.queryJson(query).length);
    const bindingsStream = measure(() => [...store.queryBindingsStream(query)].length);
    const bindings = measure(() => store.query(query).length);
    const resultStream = await measureAsync(() => consumeResultStream(store.queryBindings(query)));
    wrapperShare[label] = {
      raw_query_json_ms: raw.ms,
      query_materialised_ms: bindings.ms,
      query_bindings_stream_ms: bindingsStream.ms,
      query_bindings_result_stream_ms: resultStream.ms,
      wrapper_share_materialised: share(raw.ms, bindings.ms),
      wrapper_share_stream: share(raw.ms, bindingsStream.ms),
      wrapper_share_result_stream: share(raw.ms, resultStream.ms),
    };
  }
  const ask = 'ASK { ?s ?p ?o }';
  const boolean = measure(() => store.queryBoolean(ask));
  const count = measure(() => store.count(QUERIES['full scan (count rows)']));
  console.log(JSON.stringify({
    advisory: true,
    canonical: false,
    note: 'Single-process wrapper-layer measurements; directional only. queryBoolean/count avoid SPARQL-JSON and RDF/JS term materialisation.',
    triples: N,
    select: wrapperShare,
    cheap_paths: { query_boolean_ms: boolean.ms, count_ms: count.ms },
  }, null, 2));
  store.free();
}

// --- oxigraph (optional) -------------------------------------------------------------------------
let oxigraph;
try {
  oxigraph = await import('oxigraph');
} catch {
  console.log('oxigraph: not installed — `npm i --no-save oxigraph` to compare. Skipping.');
}
if (oxigraph) {
  console.log('oxigraph (wasm):');
  const t0 = performance.now();
  const store = new oxigraph.Store();
  store.load(nt, { format: 'application/n-triples' });
  console.log(`  ${'load N-Triples'.padEnd(28)} ${(performance.now() - t0).toFixed(1).padStart(8)} ms  (${store.size} triples)`);
  for (const [label, q] of Object.entries(QUERIES)) {
    bench(label, () => store.query(q).length);
  }
}
