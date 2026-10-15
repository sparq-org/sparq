// #7031: end-to-end exercise of `checkOwlDl` — the OWL 2 DL reasoner bundle
// (crates/sparq-dl-wasm, shipped as wasm-dl/) — through the BUILT dist/ in a real wasm
// runtime, plus the guarantee that the main entry never loads that bundle statically.
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { checkOwlDl, SparqStore } from '../dist/index.js';

const distDir = resolve(dirname(fileURLToPath(import.meta.url)), '..', 'dist');

const PRE = `
@prefix : <http://ex/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
`;

// Qualified cardinality, inverse properties, a functional object property, a union domain
// and AllDisjointClasses.
const TBOX = `${PRE}
:Person a owl:Class . :Parent a owl:Class . :Mother a owl:Class . :Woman a owl:Class .
:Man a owl:Class . :Company a owl:Class .
:hasChild a owl:ObjectProperty ; owl:inverseOf :hasParent .
:hasParent a owl:ObjectProperty .
:hasMother a owl:ObjectProperty , owl:FunctionalProperty ; rdfs:subPropertyOf :hasParent .
:worksFor a owl:ObjectProperty ; rdfs:domain [ a owl:Class ; owl:unionOf ( :Man :Woman ) ] .
:Woman rdfs:subClassOf :Person . :Man rdfs:subClassOf :Person .
[] a owl:AllDisjointClasses ; owl:members ( :Man :Woman :Company ) .
:Parent owl:equivalentClass [ a owl:Restriction ; owl:onProperty :hasChild ;
    owl:minQualifiedCardinality "1"^^xsd:nonNegativeInteger ; owl:onClass :Person ] .
:Mother owl:equivalentClass [ a owl:Class ; owl:intersectionOf ( :Woman :Parent ) ] .
`;

test('the main entry does not import the DL bundle statically', () => {
  const staticImport = /(?:^|[;\s])(?:import|export)\b[^;()]*?\bfrom\s*['"][^'"]*wasm-dl[^'"]*['"]|(?:^|[;\s])import\s*['"][^'"]*wasm-dl/m;
  const dynamicImport = /\bimport\(\s*['"]\.\.\/wasm-dl\/sparq_dl_wasm\.js['"]\s*\)/;
  const loaders = [];
  for (const f of readdirSync(distDir).filter((f) => f.endsWith('.js'))) {
    const src = readFileSync(resolve(distDir, f), 'utf8');
    assert.doesNotMatch(src, staticImport, `dist/${f} imports wasm-dl statically`);
    if (dynamicImport.test(src)) loaders.push(f);
  }
  assert.deepEqual(loaders, ['owl-dl.js']);
});

test('a consistent ontology is classified, with the inferred subsumptions', async () => {
  const r = await checkOwlDl(TBOX);
  assert.deepEqual(Object.keys(r), [
    'consistent',
    'reason',
    'profile',
    'unsatisfiable',
    'inferredSubClassOf',
    'classification',
  ]);
  assert.equal(r.consistent, true);
  assert.equal(r.reason, null);
  assert.deepEqual(r.profile, { dl: true, violations: [], unchecked: [] });
  assert.equal(r.classification, 'complete');
  assert.deepEqual(r.unsatisfiable, []);
  assert.ok(
    r.inferredSubClassOf.some(([a, b]) => a === 'http://ex/Mother' && b === 'http://ex/Person'),
    JSON.stringify(r.inferredSubClassOf),
  );
});

test('the ABox is merged into the TBox and can make it inconsistent', async () => {
  const bad = await checkOwlDl(TBOX, `${PRE} :alice a :Woman , :Man .`);
  assert.equal(bad.consistent, false);
  assert.equal(bad.classification, null);
  const ok = await checkOwlDl(TBOX, `${PRE} :alice a :Mother ; :worksFor :acme .`);
  assert.equal(ok.consistent, true);
});

test('unsatisfiable classes are reported', async () => {
  const r = await checkOwlDl(
    `${PRE} :A a owl:Class . :B a owl:Class . :C a owl:Class .
     :A rdfs:subClassOf :B , :C . :B owl:disjointWith :C .`,
  );
  assert.equal(r.consistent, true);
  assert.deepEqual(r.unsatisfiable, ['http://ex/A']);
});

test('data properties are decided; an ill-typed literal is inconsistent', async () => {
  const tbox = `${TBOX} :age a owl:DatatypeProperty ; rdfs:domain :Person ; rdfs:range xsd:nonNegativeInteger .`;
  assert.equal((await checkOwlDl(tbox, `${PRE} :ann :age 16 .`)).consistent, true);
  assert.equal((await checkOwlDl(tbox, `${PRE} :ann :age -1 .`)).consistent, false);
  assert.equal((await checkOwlDl(tbox, `${PRE} :ann :age "sixteen"^^xsd:nonNegativeInteger .`)).consistent, false);
});

test('nominals are out of fragment: unknown, with the offending triples', async () => {
  const r = await checkOwlDl(
    `${TBOX} :ann a owl:NamedIndividual . :Ann owl:equivalentClass [ a owl:Class ; owl:oneOf ( :ann ) ] .`,
  );
  assert.equal(r.consistent, 'unknown');
  assert.equal(r.reason.kind, 'OutOfFragment');
  assert.equal(typeof r.reason.construct, 'string');
  assert.ok(r.reason.triples.length > 0);
  for (const t of r.reason.triples) assert.match(t, / \.$/);
  assert.equal(r.classification, null);
});

test('an undecided datatype question answers DatatypeUnknown with its triples', async () => {
  const r = await checkOwlDl(
    `${TBOX} :d a owl:DatatypeProperty ; rdfs:range [ a rdfs:Datatype ; owl:onDatatype xsd:string ; owl:withRestrictions ( [ xsd:pattern "\\\\p{IsBasicLatin}+" ] ) ] . :a :d "x" .`,
  );
  assert.equal(r.consistent, 'unknown');
  assert.equal(r.reason.kind, 'DatatypeUnknown');
  assert.ok(r.reason.triples.length > 0);
});

test('an extraction refusal resolves with the refusal as reason', async () => {
  const r = await checkOwlDl(
    `${PRE} :A a owl:Class . :B a owl:Class . :r a owl:ObjectProperty .
     :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ;
         owl:someValuesFrom :B ; owl:allValuesFrom :B ] .`,
  );
  assert.equal(r.consistent, 'unknown');
  assert.equal(r.reason.kind, 'Malformed');
  assert.equal(typeof r.reason.detail, 'string');
  assert.ok(r.reason.triples.length > 0);
  assert.equal(r.profile.dl, 'unknown');
});

test('a blocking profile violation answers NotInDl', async () => {
  const r = await checkOwlDl(
    `${PRE} :A a owl:Class . :r a owl:ObjectProperty , owl:TransitiveProperty .
     :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ;
         owl:maxCardinality "1"^^xsd:nonNegativeInteger ] .`,
  );
  assert.equal(r.consistent, 'unknown');
  assert.equal(r.reason.kind, 'NotInDl');
  assert.equal(r.profile.dl, false);
  assert.ok(r.profile.violations.some((v) => v.kind === 'NonSimpleProperty' && v.triples.length > 0));
});

test('N-Triples input and budget options', async () => {
  const ty = '<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>';
  const cls = '<http://www.w3.org/2002/07/owl#Class>';
  const sub = '<http://www.w3.org/2000/01/rdf-schema#subClassOf>';
  const nt = [
    `<http://ex/A> ${ty} ${cls} .`,
    `<http://ex/B> ${ty} ${cls} .`,
    `<http://ex/C> ${ty} ${cls} .`,
    `<http://ex/A> ${sub} <http://ex/B> .`,
    `<http://ex/B> ${sub} <http://ex/C> .`,
  ].join('\n');
  const r = await checkOwlDl(nt, undefined, { format: 'ntriples' });
  assert.deepEqual(r.inferredSubClassOf, [['http://ex/A', 'http://ex/C']]);

  const starved = await checkOwlDl(TBOX, `${PRE} :alice a :Mother .`, { budget: { maxSteps: 1 } });
  assert.equal(starved.consistent, 'unknown');
  assert.deepEqual(starved.reason, { kind: 'ResourceBudget' });

  await assert.rejects(checkOwlDl(TBOX, undefined, { budget: { maxSteps: -1 } }), TypeError);
  await assert.rejects(checkOwlDl(TBOX, undefined, { budget: { maxNodes: 1.5 } }), TypeError);
  await assert.rejects(checkOwlDl(TBOX, undefined, { budget: { maxNodes: 2 ** 32 } }), TypeError);
  assert.equal((await checkOwlDl(TBOX, undefined, { budget: { maxNodes: 2 ** 32 - 1 } })).consistent, true);
});

test('a parse error rejects', async () => {
  await assert.rejects(checkOwlDl('<a> <b> .', undefined, { format: 'ntriples' }), /TBox parse error/);
  await assert.rejects(checkOwlDl(TBOX, 'not turtle at all'), /ABox parse error/);
});

test('SparqStore.prototype.checkOwlDl uses the default graph as the TBox', async () => {
  const store = await SparqStore.fromString(TBOX, 'turtle');
  try {
    assert.equal((await store.checkOwlDl()).consistent, true);
    const r = await store.checkOwlDl('<http://ex/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Man> , <http://ex/Company> .');
    assert.equal(r.consistent, false);
  } finally {
    store.free();
  }
});

test('malformed options are a TypeError, never a silent default', async () => {
  const bad = [
    null,
    [],
    'turtle',
    { budegt: { maxSteps: 1 } },
    { format: 'jsonld' },
    { format: 1 },
    { budget: 5 },
    { budget: null },
    { budget: [] },
    { budget: { maxStep: 1 } },
    { budget: { maxSteps: '1' } },
  ];
  for (const options of bad) {
    await assert.rejects(checkOwlDl(TBOX, undefined, options), TypeError, JSON.stringify(options));
  }
  const store = await SparqStore.fromString(TBOX, 'turtle');
  for (const options of [...bad, { format: 'turtle' }]) {
    await assert.rejects(store.checkOwlDl(undefined, options), TypeError, JSON.stringify(options));
  }
  // Well-formed options still work through both entry points.
  assert.equal((await checkOwlDl(TBOX, undefined, { format: 'turtle', budget: {} })).consistent, true);
  assert.equal((await store.checkOwlDl(undefined, { budget: { maxNodes: 100000 } })).consistent, true);
});

test('options are read from own plain data properties only, and forwarded canonically', async () => {
  const withProto = Object.create({ budget: { maxSteps: 1 } });
  const hidden = {};
  Object.defineProperty(hidden, 'budget', { value: { maxSteps: 1 }, enumerable: false });
  const getter = { get budget() { return { maxSteps: 1 }; } };
  class Opts { constructor() { this.budget = {}; } }
  const bad = [
    withProto,
    hidden,
    getter,
    new Opts(),
    { [Symbol('x')]: 1 },
    { format: null },
    { budget: { maxSteps: Number.NaN } },
    { budget: Object.create({ maxSteps: 1 }) },
  ];
  const store = await SparqStore.fromString(TBOX, 'turtle');
  for (const options of bad) {
    await assert.rejects(checkOwlDl(TBOX, undefined, options), TypeError);
    await assert.rejects(store.checkOwlDl(undefined, options), TypeError);
  }
  for (const options of [{ budget: { maxSteps: -1 } }, { budget: { maxNodes: 1.5 } }]) {
    await assert.rejects(store.checkOwlDl(undefined, options), TypeError);
  }
  // A null-prototype object is plain; a starved budget reaches the bundle through the store.
  const starved = Object.assign(Object.create(null), { budget: { maxSteps: 1 } });
  const viaStore = await store.checkOwlDl(`${PRE} :alice a :Mother .`, starved);
  const direct = await checkOwlDl(store.serialize('turtle', { pretty: false }), `${PRE} :alice a :Mother .`, starved);
  assert.deepEqual(viaStore, direct);
});
