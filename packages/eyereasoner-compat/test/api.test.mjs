// [FABLE-5] sq-ohnj1 — overload, output-mode, and migration-stub coverage for the eye-js
// compat surface. One direct assertion per public branch.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  n3reasoner, dataFactory, parseNTriples, writeQuads,
  SwiplEye, loadEyeImage, loadImage, runQuery, buildQuery, qaQuery, query, queryOnce,
  executeBasicEyeQuery, linguareasoner, EYE_PVM,
} from '../dist/index.js';

const S = 'http://example.org/socrates#';
const RDF_TYPE = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type';

// N3 with a rule so `derivations` and `deductive_closure` differ observably.
const RULE_DATA = `@prefix : <${S}>.
:Socrates a :Human.
{ ?x a :Human } => { ?x a :Mortal }.`;

test('string data -> string result (default overload)', async () => {
  const r = await n3reasoner(RULE_DATA);
  assert.equal(typeof r, 'string');
});

test('outputType:quads coerces a string-data result to Quad[]', async () => {
  const r = await n3reasoner(RULE_DATA, undefined, { outputType: 'quads' });
  assert.ok(Array.isArray(r));
  assert.ok(r.every((q) => q.termType === 'Quad'));
  assert.ok(r.some((q) => q.object.value === `${S}Mortal`), 'derived Mortal quad present');
});

test('Quad[] data -> Quad[] result (round-trips ground facts through deductive_closure)', async () => {
  const { namedNode, quad } = dataFactory;
  const quads = [quad(namedNode(`${S}Socrates`), namedNode(RDF_TYPE), namedNode(`${S}Human`))];
  const r = await n3reasoner(quads, undefined, { output: 'deductive_closure' });
  assert.ok(Array.isArray(r));
  assert.ok(r.some((q) => q.subject.value === `${S}Socrates` && q.object.value === `${S}Human`));
});

test('output:derivations returns only newly-derived; deductive_closure includes the base', async () => {
  const derivations = await n3reasoner(RULE_DATA, undefined, { output: 'derivations' });
  const closure = await n3reasoner(RULE_DATA, undefined, { output: 'deductive_closure' });
  assert.ok(derivations.includes(`${S}Mortal`), 'derivations has the derived Mortal typing');
  assert.ok(!derivations.includes(`${S}Human`), 'derivations excludes the asserted Human typing');
  assert.ok(closure.includes(`${S}Mortal`) && closure.includes(`${S}Human`), 'closure has both');
});

test("output:none returns an empty result", async () => {
  assert.equal(await n3reasoner(RULE_DATA, undefined, { output: 'none' }), '');
  assert.deepEqual(await n3reasoner(RULE_DATA, undefined, { output: 'none', outputType: 'quads' }), []);
});

test('combining an explicit output with a query throws (eye-js parity)', async () => {
  const q = `@prefix : <${S}>. {:Socrates a ?w} => {:Socrates a ?w}.`;
  await assert.rejects(() => n3reasoner(RULE_DATA, q, { output: 'deductive_closure' }),
    /Cannot use explicit output with explicit query/);
});

test('the _plus_rules output modes fail loudly (deferred, not silently wrong)', async () => {
  await assert.rejects(() => n3reasoner(RULE_DATA, undefined, { output: 'deductive_closure_plus_rules' }),
    /not yet supported/);
  await assert.rejects(() => n3reasoner(RULE_DATA, undefined, { output: 'grounded_deductive_closure_plus_rules' }),
    /not yet supported/);
});

test('a query premise EVALUATES a builtin (sq-xqchl.1) rather than failing closed', async () => {
  const q = `@prefix math: <http://www.w3.org/2000/10/swap/math#>.
@prefix : <${S}>.
{ ?x :age ?n. ?n math:greaterThan 18 } => { ?x a :Adult }.`;
  const nt = await n3reasoner(`@prefix : <${S}>. :a :age 21 . :b :age 12 .`, q);
  assert.match(nt, new RegExp(`<${S}a> <http://www\\.w3\\.org/1999/02/22-rdf-syntax-ns#type> <${S}Adult> \\.`));
  // The builtin must FILTER: matching it as data would answer for :b as well.
  assert.doesNotMatch(nt, new RegExp(`<${S}b>`));
});

test('a query document with no forward rule is an error, not an empty answer', async () => {
  await assert.rejects(() => n3reasoner(`@prefix : <${S}>. :a :p :b .`, `@prefix : <${S}>. :a :p :b .`),
    /forward rule/);
});

test('SWIPL/EYE-image-bound exports throw a clear migration error', () => {
  for (const [name, fn] of Object.entries({
    SwiplEye, loadEyeImage, loadImage, runQuery, buildQuery, qaQuery, query, queryOnce,
    executeBasicEyeQuery, linguareasoner,
  })) {
    assert.throws(() => fn(), new RegExp(name === 'linguareasoner' ? 'RDF Lingua' : 'sparq'), `${name} should throw`);
  }
  assert.throws(() => EYE_PVM.length, /EYE image/);
});

test('rdf helpers round-trip N-Triples <-> quads', () => {
  const { namedNode, literal, quad } = dataFactory;
  const q = [
    quad(namedNode('http://a/s'), namedNode('http://a/p'), literal('hi', 'en')),
    quad(namedNode('http://a/s'), namedNode('http://a/n'), literal('42', namedNode('http://www.w3.org/2001/XMLSchema#integer'))),
  ];
  const back = parseNTriples(writeQuads(q));
  assert.equal(back.length, 2);
  assert.equal(back[0].object.language, 'en');
  assert.equal(back[1].object.datatype.value, 'http://www.w3.org/2001/XMLSchema#integer');
});
