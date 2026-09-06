import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DataFactory as DF, SparqStore } from '../dist/index.js';

const XSD = 'http://www.w3.org/2001/XMLSchema#';
const DATA = `@prefix ex: <http://ex/> .
ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
ex:bob ex:name "Bob"@en ; ex:age 25 .
_:org ex:name "ACME" .`;

const load = () => SparqStore.fromString(DATA, 'turtle');

test('fromString parses and deduplicates', async () => {
  const store = await load();
  assert.equal(store.size, 6);
  assert.ok(store.heapBytes() > 0);
});

test('fetch-style bytes ingest is result-equivalent to string ingest', async () => {
  const response = new Response(new TextEncoder().encode(DATA));
  const bytes = new Uint8Array(await response.arrayBuffer());
  const fromBytes = await SparqStore.fromBytes(bytes, 'turtle');
  const fromString = await SparqStore.fromString(DATA, 'turtle');
  const query = 'SELECT ?s ?p ?o WHERE { ?s ?p ?o }';

  assert.equal(fromBytes.size, fromString.size);
  assert.equal(fromBytes.queryJson(query), fromString.queryJson(query));
});

test('fromString compressed returns identical results', async () => {
  const raw = await load();
  const cmp = await SparqStore.fromString(DATA, 'turtle', { compressed: true });
  assert.equal(cmp.size, raw.size);
  assert.equal(
    cmp.queryJson('SELECT ?s ?p ?o WHERE { ?s ?p ?o }'),
    raw.queryJson('SELECT ?s ?p ?o WHERE { ?s ?p ?o }'),
  );
});

// [OPUS-4.8] sq-dvyi: JSON-LD is parsed engine-side (oxjsonld) when the wasm bundle is
// built with the OPT-IN `jsonld` feature (the js `build:wasm` script enables it, so this
// published bundle and the site REPL both have it). With the feature on, `fromString(_,
// 'jsonld')` works like the other syntaxes — the site REPL's JSON-LD upload/URL path. The
// DEFAULT (lean) wasm bundle does NOT link oxjsonld (it stays under the perf-gate floor).
// (`dataset: true` preserves a JSON-LD `@graph` as a named graph.)
test('fromString parses JSON-LD (folded), queryable like Turtle', async () => {
  const jsonld = JSON.stringify({
    '@context': { ex: 'http://ex/' },
    '@id': 'ex:alice',
    'ex:name': 'Alice',
    'ex:knows': { '@id': 'ex:bob' },
  });
  const store = await SparqStore.fromString(jsonld, 'jsonld');
  assert.equal(store.size, 2);
  const rows = store.query('PREFIX ex: <http://ex/> SELECT ?n WHERE { ex:alice ex:name ?n }');
  assert.equal(rows.length, 1);
  assert.equal(rows[0].get('n').value, 'Alice');
});

test('fromString JSON-LD with dataset:true preserves @graph as a named graph', async () => {
  const jsonld = JSON.stringify({
    '@id': 'http://ex/g1',
    '@graph': [{ '@id': 'http://ex/s', 'http://ex/p': { '@id': 'http://ex/o' } }],
  });
  const store = await SparqStore.fromString(jsonld, 'jsonld', { dataset: true });
  const rows = store.query('SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }');
  assert.equal(rows.length, 1);
  assert.equal(rows[0].get('g').value, 'http://ex/g1');
});

test('SELECT returns RDF/JS bindings with spec-compliant terms', async () => {
  const store = await load();
  const rows = store.query(
    'PREFIX ex: <http://ex/> SELECT ?s ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a } ORDER BY ?a',
  );
  assert.equal(rows.length, 2);

  const [bob, alice] = rows;
  // named node
  const s = alice.get('s');
  assert.equal(s.termType, 'NamedNode');
  assert.equal(s.value, 'http://ex/alice');
  // plain literal: xsd:string datatype, empty language
  const n = alice.get('n');
  assert.equal(n.termType, 'Literal');
  assert.equal(n.value, 'Alice');
  assert.equal(n.language, '');
  assert.equal(n.datatype.value, `${XSD}string`);
  // typed literal
  const a = alice.get('a');
  assert.equal(a.value, '30');
  assert.equal(a.datatype.value, `${XSD}integer`);
  // language-tagged literal
  const bobName = bob.get('n');
  assert.equal(bobName.value, 'Bob');
  assert.equal(bobName.language, 'en');
  assert.equal(bobName.datatype.value, 'http://www.w3.org/1999/02/22-rdf-syntax-ns#langString');
});

test('bindings are Map-like per the RDF/JS query spec', async () => {
  const store = await load();
  const [row] = store.query(
    'PREFIX ex: <http://ex/> SELECT ?s ?n WHERE { ?s ex:name ?n . ?s ex:knows ?x }',
  );
  assert.equal(row.type, 'bindings');
  assert.equal(row.size, 2);
  // get accepts a Variable term or a bare string
  assert.equal(row.get(DF.variable('n')).value, 'Alice');
  assert.equal(row.get('n').value, 'Alice');
  assert.equal(row.get('missing'), undefined);
  assert.ok(row.has('s') && row.has(DF.variable('s')));
  // iteration yields [variable, term] pairs
  const names = [...row].map(([variable]) => variable.value).sort();
  assert.deepEqual(names, ['n', 's']);
  assert.deepEqual([...row.keys()].map(v => v.termType), ['Variable', 'Variable']);
  assert.equal([...row.values()].length, 2);
  // immutable set/delete/filter/map/merge
  const extended = row.set('extra', DF.literal('x'));
  assert.equal(extended.size, 3);
  assert.equal(row.size, 2);
  assert.equal(row.delete('n').size, 1);
  assert.equal(row.filter(t => t.termType === 'Literal').size, 1);
  assert.ok(row.map(() => DF.literal('y')).get('n').equals(DF.literal('y')));
  assert.ok(row.equals(row.set('n', DF.literal('Alice'))));
  assert.ok(!row.equals(row.delete('n')));
  assert.ok(row.merge(extended).equals(extended));
  assert.equal(row.merge(row.set('n', DF.literal('clash'))), undefined);
});

test('ASK queries (both ASK {…} and ASK WHERE {…})', async () => {
  const store = await load();
  assert.equal(store.queryBoolean('PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }'), true);
  assert.equal(store.queryBoolean('PREFIX ex: <http://ex/> ASK WHERE { ex:bob ex:knows ex:alice }'), false);
  // the form detector must not trip on ASK inside an IRI or literal
  assert.equal(
    store.queryBoolean('PREFIX ask: <http://ex/ASK#> # ASK comment\nASK { ?s ?p "ASK" }'),
    false,
  );
});

test('ASK is evaluated natively (boolean JSON form, FILTER honoured)', async () => {
  const store = await load();
  // queryJson returns the SPARQL 1.1 boolean results form for ASK
  assert.deepEqual(JSON.parse(store.queryJson('PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }')), {
    head: {},
    boolean: true,
  });
  // ASK over a pattern with a FILTER (exercises real evaluation, not just a count)
  assert.equal(store.queryBoolean('PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 28) }'), true);
  assert.equal(store.queryBoolean('PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 99) }'), false);
  // queryBoolean rejects non-ASK forms
  assert.throws(() => store.queryBoolean('SELECT * WHERE { ?s ?p ?o }'), /ASK/);
});

test('full-text-style matching via plain SPARQL string functions', async () => {
  // sparq's full-text crate (sparq-text) rewrites text: magic predicates into
  // plain SPARQL — which the wasm engine evaluates directly. This pins the
  // plain-SPARQL substring path that browser callers get.
  const store = await load();
  const rows = store.query(
    'SELECT ?s WHERE { ?s ?p ?o FILTER(isLiteral(?o) && CONTAINS(LCASE(STR(?o)), "ali")) }',
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].get('s').value, 'http://ex/alice');
  assert.equal(store.queryBoolean('ASK { ?s ?p ?o FILTER(STRSTARTS(STR(?o), "AC")) }'), true); // "ACME"
  assert.equal(store.queryBoolean('ASK { ?s ?p ?o FILTER(CONTAINS(STR(?o), "zzz")) }'), false);
  // REGEX/REPLACE are deliberately compiled out of the wasm build (the engine's
  // non-default `regex` cargo feature) to keep the bundle small — pin the error.
  assert.throws(() => store.queryBoolean('ASK { ?s ?p ?o FILTER(REGEX(STR(?o), "z")) }'), /Regex/);
});

test('query() dispatches on the query form', async () => {
  const store = await load();
  assert.equal(store.query('ASK { ?s ?p ?o }'), true);
  const rows = store.query('SELECT ?s WHERE { ?s ?p ?o }');
  assert.ok(Array.isArray(rows) && rows.length === 6);
});

test('count() avoids materialisation but agrees with SELECT', async () => {
  const store = await load();
  assert.equal(store.count('SELECT ?s WHERE { ?s ?p ?o }'), 6);
});

test('match() with wildcard, constant and variable positions', async () => {
  const store = await load();
  const name = DF.namedNode('http://ex/name');

  assert.equal(store.match().length, 6);
  assert.equal(store.match(null, name, null).length, 3);
  // Variable positions are wildcards, per RDF/JS
  assert.equal(store.match(DF.variable('x'), name).length, 3);

  const quads = store.match(DF.namedNode('http://ex/alice'), name);
  assert.equal(quads.length, 1);
  assert.equal(quads[0].termType, 'Quad');
  assert.ok(quads[0].subject.equals(DF.namedNode('http://ex/alice')));
  assert.ok(quads[0].predicate.equals(name));
  assert.ok(quads[0].object.equals(DF.literal('Alice')));
  assert.equal(quads[0].graph.termType, 'DefaultGraph');

  // constant literal object
  assert.equal(store.match(null, null, DF.literal('Bob', 'en')).length, 1);
  assert.equal(store.match(null, null, DF.literal('Bob')).length, 0);

  // all-constant probe
  const age = DF.literal('30', DF.namedNode(`${XSD}integer`));
  assert.equal(store.match(DF.namedNode('http://ex/alice'), DF.namedNode('http://ex/age'), age).length, 1);
  assert.equal(store.match(DF.namedNode('http://ex/bob'), DF.namedNode('http://ex/age'), age).length, 0);

  // named graph argument matches nothing (triples live in the default graph)
  assert.equal(store.match(null, null, null, DF.namedNode('http://ex/g')).length, 0);
  assert.equal(store.match(null, null, null, DF.defaultGraph()).length, 6);
});

test('match() on a blank-node subject filters by label', async () => {
  const store = await load();
  const [orgQuad] = store.match(null, null, DF.literal('ACME'));
  assert.equal(orgQuad.subject.termType, 'BlankNode');
  const byBnode = store.match(orgQuad.subject, null, null);
  assert.equal(byBnode.length, 1);
  assert.ok(byBnode[0].object.equals(DF.literal('ACME')));
  assert.equal(store.match(DF.blankNode('no-such-label')).length, 0);
});

test('countQuads agrees with match', async () => {
  const store = await load();
  assert.equal(store.countQuads(), 6);
  assert.equal(store.countQuads(null, DF.namedNode('http://ex/name')), 3);
  assert.equal(store.countQuads(null, null, null, DF.namedNode('http://ex/g')), 0);
});

test('fromQuads round-trips RDF/JS terms', async () => {
  const ex = v => DF.namedNode(`http://ex/${v}`);
  const quads = [
    DF.quad(ex('s'), ex('p'), DF.literal('plain "quoted" \\ value\nline2')),
    DF.quad(ex('s'), ex('p'), DF.literal('hallo', 'de')),
    DF.quad(ex('s'), ex('p'), DF.literal('5', DF.namedNode(`${XSD}integer`))),
    DF.quad(DF.blankNode('b0'), ex('p'), ex('o')),
    DF.quad(ex('g-s'), ex('p'), ex('o'), ex('namedGraph')), // folded into the default graph
  ];
  const store = await SparqStore.fromQuads(quads);
  assert.equal(store.size, 5);
  for (const quad of quads) {
    const found = store.match(quad.subject.termType === 'BlankNode' ? null : quad.subject, quad.predicate, quad.object);
    assert.ok(found.length >= 1, `round-trip lost ${quad.object.value}`);
    const match = found.find(f => f.object.equals(quad.object));
    assert.ok(match, `object term changed for ${quad.object.value}`);
    if (quad.subject.termType !== 'BlankNode') assert.ok(match.subject.equals(quad.subject));
  }
});

test('SPARQL update: INSERT DATA / DELETE DATA / DELETE-INSERT WHERE / CLEAR', async () => {
  const store = await load();
  store.update('PREFIX ex: <http://ex/> INSERT DATA { ex:carol ex:name "Carol" }');
  assert.equal(store.size, 7);
  assert.equal(store.queryBoolean('PREFIX ex: <http://ex/> ASK { ex:carol ex:name "Carol" }'), true);

  store.update('PREFIX ex: <http://ex/> DELETE DATA { ex:carol ex:name "Carol" }');
  assert.equal(store.size, 6);

  store.update('PREFIX ex: <http://ex/> DELETE { ?s ex:age ?a } INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }');
  assert.equal(store.match(null, DF.namedNode('http://ex/age')).length, 0);
  assert.equal(store.match(null, DF.namedNode('http://ex/years')).length, 2);

  store.update('CLEAR ALL');
  assert.equal(store.size, 0);
});

const DATASET = `<http://ex/d> <http://ex/p> "default" .
<http://ex/a> <http://ex/p> "in-g1" <http://ex/g1> .
<http://ex/a> <http://ex/q> "also-g1" <http://ex/g1> .
<http://ex/b> <http://ex/p> "in-g2" <http://ex/g2> .`;

const loadDataset = () => SparqStore.fromString(DATASET, 'nquads', { dataset: true });

test('dataset stores preserve named graphs (GRAPH / FROM / FROM NAMED)', async () => {
  const store = await loadDataset();
  assert.equal(store.size, 1); // size reports the default graph

  const g1 = store.query('SELECT ?o WHERE { GRAPH <http://ex/g1> { ?s <http://ex/p> ?o } }');
  assert.equal(g1.length, 1);
  assert.equal(g1[0].get('o').value, 'in-g1');

  const all = store.query('SELECT ?g ?o WHERE { GRAPH ?g { ?s ?p ?o } }');
  assert.equal(all.length, 3);
  assert.deepEqual([...new Set(all.map(r => r.get('g').value))].sort(), ['http://ex/g1', 'http://ex/g2']);

  assert.equal(store.queryBoolean('ASK { GRAPH <http://ex/g2> { ?s ?p "in-g2" } }'), true);
  assert.equal(store.queryBoolean('ASK { GRAPH <http://ex/g2> { ?s ?p "in-g1" } }'), false);

  // FROM merges the named graph into the active default graph
  const from = store.query('SELECT ?o FROM <http://ex/g1> WHERE { ?s <http://ex/p> ?o }');
  assert.deepEqual(from.map(r => r.get('o').value), ['in-g1']);
  // FROM NAMED scopes which graphs GRAPH ?g ranges over
  const fromNamed = store.query(
    'SELECT ?o FROM NAMED <http://ex/g2> WHERE { GRAPH ?g { ?s ?p ?o } }',
  );
  assert.deepEqual(fromNamed.map(r => r.get('o').value), ['in-g2']);

  // folding (the default) is unchanged
  const folded = await SparqStore.fromString(DATASET, 'nquads');
  assert.equal(folded.size, 4);
  // dataset + compressed is rejected with a clear error
  await assert.rejects(SparqStore.fromString(DATASET, 'nquads', { dataset: true, compressed: true }), /compressed/);
});

test('SPARQL update addresses named graphs (GRAPH blocks, CLEAR GRAPH)', async () => {
  const store = await loadDataset();
  store.update('INSERT DATA { GRAPH <http://ex/g3> { <http://ex/c> <http://ex/p> "in-g3" } }');
  assert.equal(store.queryBoolean('ASK { GRAPH <http://ex/g3> { ?s ?p "in-g3" } }'), true);
  assert.equal(store.size, 1); // default graph untouched

  store.update('DELETE DATA { GRAPH <http://ex/g1> { <http://ex/a> <http://ex/p> "in-g1" } }');
  assert.equal(store.count('SELECT ?o WHERE { GRAPH <http://ex/g1> { ?s ?p ?o } }'), 1);

  // DELETE/INSERT with a GRAPH template moves data between graphs
  store.update(
    'DELETE { GRAPH <http://ex/g2> { ?s ?p ?o } } INSERT { GRAPH <http://ex/g3> { ?s ?p ?o } } WHERE { GRAPH <http://ex/g2> { ?s ?p ?o } }',
  );
  assert.equal(store.queryBoolean('ASK { GRAPH <http://ex/g2> { ?s ?p ?o } }'), false);
  assert.equal(store.count('SELECT ?o WHERE { GRAPH <http://ex/g3> { ?s ?p ?o } }'), 2);

  store.update('CLEAR GRAPH <http://ex/g3>');
  assert.equal(store.queryBoolean('ASK { GRAPH ?g { ?s ?p ?o } }'), store.count('SELECT ?o WHERE { GRAPH ?g { ?s ?p ?o } }') > 0);
  assert.equal(store.count('SELECT ?o WHERE { GRAPH <http://ex/g3> { ?s ?p ?o } }'), 0);
});

test('match()/countQuads are graph-aware on dataset stores', async () => {
  const store = await loadDataset();
  const g1 = DF.namedNode('http://ex/g1');

  // graph wildcard spans default + named graphs
  const all = store.match();
  assert.equal(all.length, 4);
  assert.equal(all.filter(q => q.graph.termType === 'DefaultGraph').length, 1);
  assert.equal(all.filter(q => q.graph.equals(g1)).length, 2);

  // constant graph
  const inG1 = store.match(null, null, null, g1);
  assert.equal(inG1.length, 2);
  assert.ok(inG1.every(q => q.graph.equals(g1)));
  assert.ok(inG1.some(q => q.object.equals(DF.literal('also-g1'))));

  // constant graph + constant triple positions
  assert.equal(store.match(DF.namedNode('http://ex/a'), DF.namedNode('http://ex/p'), DF.literal('in-g1'), g1).length, 1);
  assert.equal(store.match(DF.namedNode('http://ex/a'), DF.namedNode('http://ex/p'), DF.literal('in-g1'), DF.defaultGraph()).length, 0);

  // default graph scoping
  assert.equal(store.match(null, null, null, DF.defaultGraph()).length, 1);

  // counts agree (count path is non-materialising where possible)
  assert.equal(store.countQuads(), 4);
  assert.equal(store.countQuads(null, null, null, g1), 2);
  assert.equal(store.countQuads(null, null, null, DF.defaultGraph()), 1);
  assert.equal(store.countQuads(null, null, null, DF.namedNode('http://ex/absent')), 0);
});

test('fromQuads preserves named graphs under options.dataset', async () => {
  const ex = v => DF.namedNode(`http://ex/${v}`);
  const quads = [
    DF.quad(ex('s'), ex('p'), DF.literal('default')),
    DF.quad(ex('s'), ex('p'), DF.literal('named'), ex('g')),
  ];
  const store = await SparqStore.fromQuads(quads, { dataset: true });
  assert.equal(store.size, 1);
  assert.equal(store.countQuads(), 2);
  const [named] = store.match(null, null, null, ex('g'));
  assert.ok(named.object.equals(DF.literal('named')));
  assert.ok(named.graph.equals(ex('g')));
});

test('applyDelta: incremental quad-level inserts and removals (no rebuild)', async () => {
  const ex = v => DF.namedNode(`http://ex/${v}`);
  const store = await load();

  // insert: new terms (typed + language literals) grow the dictionary append-only
  store.addQuads([
    DF.quad(ex('carol'), ex('name'), DF.literal('Carol')),
    DF.quad(ex('carol'), ex('age'), DF.literal('28', DF.namedNode(`${XSD}integer`))),
    DF.quad(ex('carol'), ex('greets'), DF.literal('hallo', 'de')),
  ]);
  assert.equal(store.size, 9);
  assert.equal(store.queryBoolean('ASK { ?s ?p "28"^^<http://www.w3.org/2001/XMLSchema#integer> }'), true);
  // numeric filter cache covers the appended literal
  assert.equal(store.queryBoolean('PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a = 28) }'), true);

  // remove: delete one of them again + a no-op delete of an absent triple
  store.removeQuads([
    DF.quad(ex('carol'), ex('greets'), DF.literal('hallo', 'de')),
    DF.quad(ex('nobody'), ex('name'), DF.literal('Nobody')),
  ]);
  assert.equal(store.size, 8);

  // deletes are applied before inserts within one batch
  store.applyDelta(
    [DF.quad(ex('carol'), ex('name'), DF.literal('Caroline'))],
    [DF.quad(ex('carol'), ex('name'), DF.literal('Carol'))],
  );
  assert.equal(store.match(ex('carol'), ex('name')).length, 1);
  assert.equal(store.match(ex('carol'), ex('name'))[0].object.value, 'Caroline');

  // bnode triples are retractable BY LABEL (impossible via SPARQL DELETE DATA)
  const [orgQuad] = store.match(null, null, DF.literal('ACME'));
  assert.equal(orgQuad.subject.termType, 'BlankNode');
  store.removeQuads([orgQuad]);
  assert.equal(store.match(null, null, DF.literal('ACME')).length, 0);
});

test('applyDelta routes named-graph quads (auto-creating graphs)', async () => {
  const ex = v => DF.namedNode(`http://ex/${v}`);
  const store = await loadDataset();
  store.applyDelta(
    [
      DF.quad(ex('c'), ex('p'), DF.literal('new-default')),
      DF.quad(ex('c'), ex('p'), DF.literal('new-in-g1'), ex('g1')),
      DF.quad(ex('c'), ex('p'), DF.literal('new-in-g9'), ex('g9')), // absent graph: auto-created
    ],
    [DF.quad(ex('a'), ex('p'), DF.literal('in-g1'), ex('g1'))],
  );
  assert.equal(store.size, 2);
  assert.equal(store.countQuads(null, null, null, ex('g1')), 2); // -1 +1
  assert.equal(store.countQuads(null, null, null, ex('g9')), 1);
  assert.equal(store.queryBoolean('ASK { GRAPH <http://ex/g9> { ?s ?p "new-in-g9" } }'), true);
  // delete-only against an absent graph is a no-op, and never creates the graph
  store.removeQuads([DF.quad(ex('z'), ex('p'), DF.literal('x'), ex('never'))]);
  assert.equal(store.count('SELECT DISTINCT ?g WHERE { GRAPH ?g { ?s ?p ?o } }'), 3); // g1 g2 g9
});

test('update() applies in place: handle stays valid, named graphs preserved', async () => {
  const store = await loadDataset();
  store.update('INSERT DATA { <http://ex/n> <http://ex/p> "via-update" }');
  store.update('DELETE DATA { <http://ex/d> <http://ex/p> "default" }');
  assert.equal(store.size, 1);
  // named graphs survive default-graph data operations
  assert.equal(store.countQuads(null, null, null, DF.namedNode('http://ex/g1')), 2);
  // interleave with quad-level deltas on the same handle
  store.addQuads([DF.quad(DF.namedNode('http://ex/n2'), DF.namedNode('http://ex/p'), DF.literal('mixed'))]);
  assert.equal(store.size, 2);
});

test('engine errors surface as JS exceptions', async () => {
  const store = await load();
  assert.throws(() => store.query('SELECT ?s WHERE { broken'), /error|expected/i);
  assert.throws(() => store.query('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }'), /SELECT/);
});

// [OPUS-4.8] sq-1gkw: CONSTRUCT/DESCRIBE through queryQuads() / queryQuadsString() /
// queryQuadsStream(), parsed back to RDF/JS quads.
test('queryQuads() returns CONSTRUCT result as RDF/JS quads', async () => {
  const store = await load();
  const quads = store.queryQuads(
    'PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }',
  );
  // Three name triples (alice, bob, _:org) -> three constructed triples.
  assert.equal(quads.length, 3);
  const label = quads.find((q) => q.subject.value === 'http://ex/alice');
  assert.ok(label, 'alice was constructed');
  assert.equal(label.predicate.value, 'http://ex/label');
  assert.equal(label.object.termType, 'Literal');
  assert.equal(label.object.value, 'Alice');
  assert.equal(label.graph.termType, 'DefaultGraph');
  // The language tag survives the N-Triples round-trip.
  const bob = quads.find((q) => q.subject.value === 'http://ex/bob');
  assert.equal(bob.object.language, 'en');
  // A blank-node subject is preserved (label is engine-assigned but present).
  const org = quads.find((q) => q.object.value === 'ACME');
  assert.equal(org.subject.termType, 'BlankNode');
});

test('queryQuads() returns DESCRIBE concise bounded description', async () => {
  const store = await load();
  const quads = store.queryQuads('DESCRIBE <http://ex/bob>');
  // CBD of ex:bob = its outgoing triples (name + age); nothing inbound.
  assert.ok(quads.every((q) => q.subject.value === 'http://ex/bob'));
  assert.ok(quads.some((q) => q.predicate.value === 'http://ex/name' && q.object.value === 'Bob'));
  const age = quads.find((q) => q.predicate.value === 'http://ex/age');
  assert.equal(age.object.termType, 'Literal');
  assert.equal(age.object.datatype.value, `${XSD}integer`);
  assert.equal(age.object.value, '25');
});

test('queryQuadsString() returns the raw N-Triples document', async () => {
  const store = await load();
  const nt = store.queryQuadsString(
    'PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }',
  );
  assert.match(nt, /<http:\/\/ex\/alice> <http:\/\/ex\/label> "Alice" \./);
  // Parsing the string and the quads path agree.
  const fromString = store.queryQuads(
    'PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }',
  );
  assert.equal(nt.trim().split('\n').filter((l) => l.trim()).length, fromString.length);
});

test('queryQuadsStream() yields the same quads as queryQuads()', async () => {
  const store = await load();
  const q = 'PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }';
  const whole = store.queryQuads(q);
  const streamed = [...store.queryQuadsStream(q, 1)];
  assert.equal(streamed.length, whole.length);
  // Multiset-equal regardless of batch boundaries.
  for (const w of whole) {
    assert.ok(streamed.some((s) => s.equals(w)), `streamed result contains ${w.subject.value}`);
  }
});

test('queryQuadsStream() frees the cursor when abandoned early', async () => {
  const store = await load();
  const q = 'PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }';
  const it = store.queryQuadsStream(q, 1);
  const first = it.next();
  assert.equal(first.done, false);
  it.return(); // triggers the finally{} cursor.free()
  // Store is still usable afterwards.
  assert.equal(store.queryQuads(q).length, 3);
});

test('queryQuads() rejects a SELECT / ASK query', async () => {
  const store = await load();
  assert.throws(() => store.queryQuads('SELECT ?s WHERE { ?s ?p ?o }'), /CONSTRUCT.*DESCRIBE/i);
});

test('queryQuads() round-trips datatyped, escaped and IRI-object terms', async () => {
  const store = await SparqStore.fromString(
    `@prefix ex: <http://ex/> .
     ex:s ex:quote "a \\"q\\" \\n b" ; ex:link ex:target ; ex:n "3.5"^^<http://www.w3.org/2001/XMLSchema#decimal> .`,
    'turtle',
  );
  const quads = store.queryQuads('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }');
  const quote = quads.find((q) => q.predicate.value === 'http://ex/quote');
  assert.equal(quote.object.value, 'a "q" \n b'); // escapes decoded
  const link = quads.find((q) => q.predicate.value === 'http://ex/link');
  assert.equal(link.object.termType, 'NamedNode');
  assert.equal(link.object.value, 'http://ex/target');
  const num = quads.find((q) => q.predicate.value === 'http://ex/n');
  assert.equal(num.object.datatype.value, `${XSD}decimal`);
});

// [OPUS-4.8] sq-ty78o (#1114): the empty `SparqStore.empty()` constructor — build a store
// from nothing and grow it with update(), including a named-graph block (the round-trip the
// issue reported, gated only by the previously-missing wasm constructor).
test('empty() builds a mutable store; named-graph updates round-trip', async () => {
  const store = await SparqStore.empty();
  assert.equal(store.size, 0);
  // Default-graph insert grows the store.
  store.update('INSERT DATA { <http://ex/a> <http://ex/p> "v" }');
  assert.equal(store.size, 1);
  // Named-graph INSERT then GRAPH ?g query returns the inserted row (no dataset flag needed).
  store.update('INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }');
  const rows = store.query('SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }');
  assert.equal(rows.length, 1);
  assert.equal(rows[0].get('g').value, 'http://ex/g');
});

test('emptySync() builds an empty store (engine already initialised)', async () => {
  // Ensure the wasm engine is up (emptySync() does not await init()).
  await SparqStore.empty();
  const store = SparqStore.emptySync();
  assert.equal(store.size, 0);
  store.addQuads([
    DF.quad(DF.namedNode('http://ex/a'), DF.namedNode('http://ex/p'), DF.literal('v')),
  ]);
  assert.equal(store.size, 1);
});

// [OPUS-4.8] sq-f66jz (#1115): the baseIri option resolves a document's relative IRIs.
test('fromString with baseIri resolves relative IRIs', async () => {
  const store = await SparqStore.fromString('<a> <p> <../up/o> .', 'turtle', {
    baseIri: 'http://ex/dir/',
  });
  assert.equal(store.size, 1);
  const rows = store.query('SELECT ?s ?o WHERE { ?s ?p ?o }');
  assert.equal(rows[0].get('s').value, 'http://ex/dir/a');
  assert.equal(rows[0].get('o').value, 'http://ex/up/o');
});

test('fromStringSync with baseIri resolves relative IRIs', async () => {
  await SparqStore.empty(); // ensure wasm init
  const store = SparqStore.fromStringSync('<a> <p> <o> .', 'turtle', { baseIri: 'http://ex/' });
  const rows = store.query('SELECT ?s WHERE { ?s ?p ?o }');
  assert.equal(rows[0].get('s').value, 'http://ex/a');
});

test('baseIri rejects an invalid base, and is not combinable with dataset/compressed', async () => {
  await assert.rejects(
    SparqStore.fromString('<a> <p> <o> .', 'turtle', { baseIri: 'not a iri' }),
    /base|iri/i,
  );
  await assert.rejects(
    SparqStore.fromString('<a> <p> <o> .', 'turtle', { baseIri: 'http://ex/', dataset: true }),
    /baseIri/,
  );
  await assert.rejects(
    SparqStore.fromString('<a> <p> <o> .', 'turtle', { baseIri: 'http://ex/', compressed: true }),
    /baseIri/,
  );
});

// [OPUS-4.8] sq-u78ol (#1117 / #1129): serialize() / dump() expose the store's contents as a
// Turtle / TriG / JSON-LD document (the wasm serialize-rdf writer, surfaced on the wrapper).
test('serialize() writes Turtle; round-trips to the same triple count', async () => {
  const store = await load();
  const ttl = store.serialize('turtle');
  assert.ok(ttl.length > 0);
  assert.ok(ttl.includes('@prefix'), 'abbreviated by default'); // well-known prefixes present
  // dump() is an alias for serialize().
  assert.equal(store.dump('turtle'), ttl);
  // Round-trip: re-parse the serialised Turtle -> same number of triples.
  const reparsed = await SparqStore.fromString(ttl, 'turtle');
  assert.equal(reparsed.size, store.size);
});

test('serialize() honours abbreviate=false and a custom prefix map (#1129)', async () => {
  const store = await SparqStore.fromString('<http://ex/s> <http://ex/p> <http://ex/o> .', 'turtle');
  // No abbreviation: full <...> IRIs, no @prefix header.
  const full = store.serialize('turtle', { abbreviate: false });
  assert.ok(!full.includes('@prefix'), 'no prefix header when abbreviate=false');
  assert.ok(full.includes('<http://ex/s>'), 'full IRIs preserved');
  // Caller-supplied prefix map drives @prefix compaction (#1129).
  const compact = store.serialize('turtle', { prefixes: [['ex', 'http://ex/']] });
  assert.ok(compact.includes('@prefix ex:'), 'caller prefix declared');
  assert.ok(compact.includes('ex:s'), 'IRI compacted to the caller prefix');
});

test('serialize("trig") emits named graphs as GRAPH blocks', async () => {
  const store = await SparqStore.fromString(
    '<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .\n',
    'nquads',
    { dataset: true },
  );
  const trig = store.serialize('trig');
  assert.ok(trig.includes('GRAPH'), 'TriG carries the named graph as a GRAPH block');
});

test('serialize() rejects an unknown format', async () => {
  const store = await load();
  assert.throws(() => store.serialize('rdfxml'), /serialize|format/i);
});
