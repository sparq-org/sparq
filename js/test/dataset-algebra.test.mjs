// [OPUS-4.8] #1047 — the FULL RDF/JS `Dataset` algebra on `@sparq-org/sparq`'s `Dataset`, exercised
// against the built dist/. Two halves:
//   1. the set algebra / iteration / materialisation members beyond `DatasetCore`; and
//   2. the INTEROP requirement (the maintainer's hard ask): the binary set ops must work whether
//      the operand is OUR-OWN `Dataset` or a FOREIGN RDF/JS dataset (here: an `N3.Store`).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Store as N3Store, DataFactory as N3DF, Parser as N3Parser } from 'n3';
import { Dataset, DataFactory as DF } from '../dist/index.js';

const EX = (l) => DF.namedNode(`http://ex/${l}`);
const Q = (s, p, o, g) => DF.quad(EX(s), EX(p), EX(o), g);

const ABC = `@prefix ex: <http://ex/> .
ex:a ex:p ex:b .
ex:a ex:p ex:c .
ex:b ex:p ex:c .`;

/** Build an N3.Store (a FOREIGN RDF/JS dataset) from a Turtle doc. */
function n3Store(ttl) {
  const store = new N3Store();
  store.addQuads(new N3Parser().parse(ttl));
  return store;
}

// --- set algebra: our-own operand ----------------------------------------------------------------

test('union (our-own): set-deduplicates the combined quads', async () => {
  const a = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b .', 'turtle');
  const b = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:b ex:p ex:c .', 'turtle');
  const u = a.union(b);
  assert.equal(u.size, 2, 'union dedupes the shared quad');
  assert.ok(u.has(Q('a', 'p', 'b')) && u.has(Q('b', 'p', 'c')));
  // operands untouched
  assert.equal(a.size, 1);
  assert.equal(b.size, 2);
  a.free(); b.free(); u.free();
});

test('intersection (our-own): keeps only shared quads', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const b = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:x ex:p ex:y .', 'turtle');
  const i = a.intersection(b);
  assert.equal(i.size, 1);
  assert.ok(i.has(Q('a', 'p', 'b')));
  a.free(); b.free(); i.free();
});

test('difference (our-own): drops quads present in the other', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const b = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b .', 'turtle');
  const d = a.difference(b);
  assert.equal(d.size, 2);
  assert.ok(!d.has(Q('a', 'p', 'b')));
  a.free(); b.free(); d.free();
});

test('contains / equals (our-own)', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const sub = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b .', 'turtle');
  const same = await Dataset.fromString(ABC, 'turtle');
  assert.ok(a.contains(sub), 'a superset contains its subset');
  assert.ok(!sub.contains(a), 'a subset does not contain its superset');
  assert.ok(a.equals(same));
  assert.ok(!a.equals(sub));
  a.free(); sub.free(); same.free();
});

test('addAll / deleteMatches mutate the receiver', async () => {
  const a = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b .', 'turtle');
  const ret = a.addAll([Q('b', 'p', 'c'), Q('c', 'p', 'd')]);
  assert.equal(ret, a, 'addAll returns this');
  assert.equal(a.size, 3);
  // delete every quad whose object is ex:c
  a.deleteMatches(null, null, EX('c'));
  assert.equal(a.size, 2);
  assert.ok(!a.has(Q('b', 'p', 'c')));
  a.free();
});

// --- iteration / functional members --------------------------------------------------------------

test('filter / map / forEach / some / every / reduce / toArray', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const onlyB = a.filter((q) => q.subject.equals(EX('b')));
  assert.equal(onlyB.size, 1);

  const mapped = a.map((q) => DF.quad(q.subject, EX('q'), q.object, q.graph));
  assert.equal(mapped.size, 3);
  assert.ok([...mapped].every((q) => q.predicate.equals(EX('q'))));

  let count = 0;
  a.forEach(() => count++);
  assert.equal(count, 3);

  assert.ok(a.some((q) => q.object.equals(EX('c'))));
  assert.ok(a.every((q) => q.predicate.equals(EX('p'))));
  assert.ok(!a.every((q) => q.object.equals(EX('c'))));

  const subjects = a.reduce((acc, q) => acc.add(q.subject.value), new Set());
  assert.deepEqual([...subjects].sort(), ['http://ex/a', 'http://ex/b']);

  assert.equal(a.toArray().length, 3);
  a.free(); onlyB.free(); mapped.free();
});

test('reduce on empty dataset with no initial value throws', async () => {
  const e = await Dataset.create();
  assert.throws(() => e.reduce((acc) => acc), TypeError);
  assert.equal(e.reduce((acc) => acc, 'seed'), 'seed');
  e.free();
});

// --- materialisation: toStream / import / toString / toCanonical ----------------------------------

test('toStream emits every quad then end', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const seen = await new Promise((resolve, reject) => {
    const out = [];
    const s = a.toStream();
    s.on('data', (q) => out.push(q));
    s.on('error', reject);
    s.on('end', () => resolve(out));
  });
  assert.equal(seen.length, 3);
  a.free();
});

test('import consumes an RDF/JS quad stream (from an N3.Store.match)', async () => {
  const target = await Dataset.create();
  const src = n3Store(ABC);
  await target.import(src.match(null, null, null, null));
  assert.equal(target.size, 3);
  assert.ok(target.has(Q('a', 'p', 'b')));
  target.free();
});

test('toString is N-Quads; toCanonical is sorted + deterministic', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const b = await Dataset.fromString('@prefix ex: <http://ex/> . ex:b ex:p ex:c . ex:a ex:p ex:c . ex:a ex:p ex:b .', 'turtle');
  // Same triples, different declaration order → identical canonical form (no blank nodes here).
  assert.equal(a.toCanonical(), b.toCanonical());
  assert.ok(a.toString().includes('<http://ex/a> <http://ex/p> <http://ex/b>'));
  a.free(); b.free();
});

// --- RDFC-1.0 blank-node canonicalisation (sq-1dd5t / #1047) --------------------------------------
// toCanonical / equals / contains must be RDF-DATASET-ISOMORPHISM-aware: two datasets that
// differ ONLY in blank-node labels (and/or quad order) are the same dataset and must compare
// equal / canonicalise identically. Before sq-1dd5t these were label-sensitive.

// Two isomorphic graphs whose blank-node labels differ (and triple order is permuted).
const BNODE_A = `@prefix ex: <http://ex/> .
[] ex:p _:shared . _:shared ex:q "v" .`;
const BNODE_B = `@prefix ex: <http://ex/> .
_:other ex:q "v" . [] ex:p _:other .`;

test('toCanonical: isomorphic blank-node graphs canonicalise identically (relabelled to c14nN)', async () => {
  const a = await Dataset.fromString(BNODE_A, 'turtle');
  const b = await Dataset.fromString(BNODE_B, 'turtle');
  const ca = a.toCanonical();
  assert.equal(ca, b.toCanonical(), 'isomorphic graphs must produce byte-identical canonical N-Quads');
  assert.ok(ca.includes('_:c14n'), 'blank nodes relabelled to the RDFC-1.0 c14nN form');
  // A genuinely different graph (extra edge) must NOT canonicalise the same.
  const c = await Dataset.fromString(`${BNODE_A}\n_:extra ex:r "w" .`, 'turtle');
  assert.notEqual(ca, c.toCanonical());
  a.free(); b.free(); c.free();
});

test('equals: isomorphic blank-node datasets compare equal (label-insensitive)', async () => {
  const a = await Dataset.fromString(BNODE_A, 'turtle');
  const b = await Dataset.fromString(BNODE_B, 'turtle');
  assert.ok(a.equals(b), 'datasets differing only in bnode labels must be equal');
  assert.ok(b.equals(a), 'equals is symmetric');
  // Not equal when the structure differs.
  const more = await Dataset.fromString(`${BNODE_A}\n[] ex:r "w" .`, 'turtle');
  assert.ok(!a.equals(more));
  a.free(); b.free(); more.free();
});

test('equals: a foreign (N3.Store) isomorphic blank-node dataset compares equal', async () => {
  const a = await Dataset.fromString(BNODE_A, 'turtle');
  const foreign = n3Store(BNODE_B); // foreign lib, different bnode labels
  assert.ok(a.equals(foreign), 'isomorphism-aware equals works across libraries');
  a.free();
});

test('contains: an isomorphic blank-node subgraph is contained (relabel-insensitive)', async () => {
  const a = await Dataset.fromString(BNODE_A, 'turtle');
  // The same first edge but with a DIFFERENT bnode label — still a contained subgraph.
  const sub = await Dataset.fromString('@prefix ex: <http://ex/> . [] ex:p _:zzz . _:zzz ex:q "v" .', 'turtle');
  assert.ok(a.contains(sub), 'a relabelled isomorphic subgraph must be contained');
  // A blank-node graph that is NOT a subgraph (extra edge that a does not have) is not contained.
  const notSub = await Dataset.fromString('@prefix ex: <http://ex/> . _:n ex:p _:m . _:m ex:nope "x" .', 'turtle');
  assert.ok(!a.contains(notSub));
  a.free(); sub.free(); notSub.free();
});

test('contains: ground + blank-node mix — superset relationship honoured', async () => {
  const a = await Dataset.fromString(`${ABC}\n@prefix ex: <http://ex/> .\n[] ex:p _:b . _:b ex:q "v" .`, 'turtle');
  const groundSub = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b .', 'turtle');
  assert.ok(a.contains(groundSub), 'a ground subgraph is still contained');
  const bnodeSub = await Dataset.fromString('@prefix ex: <http://ex/> . [] ex:p _:x . _:x ex:q "v" .', 'turtle');
  assert.ok(a.contains(bnodeSub), 'a relabelled blank-node subgraph is contained');
  a.free(); groundSub.free(); bnodeSub.free();
});

// --- INTEROP: FOREIGN operand (N3.js) — the maintainer's hard requirement -------------------------

test('union (foreign N3.Store): combines + dedupes across libraries', async () => {
  const a = await Dataset.fromString('@prefix ex: <http://ex/> . ex:a ex:p ex:b .', 'turtle');
  const foreign = n3Store('@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:b ex:p ex:c .');
  const u = a.union(foreign);
  assert.equal(u.size, 2, 'union with an N3.Store dedupes the shared quad');
  assert.ok(u.has(Q('a', 'p', 'b')) && u.has(Q('b', 'p', 'c')));
  a.free(); u.free();
});

test('intersection / difference (foreign N3.Store)', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const foreign = n3Store('@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:x ex:p ex:y .');
  const i = a.intersection(foreign);
  assert.equal(i.size, 1);
  assert.ok(i.has(Q('a', 'p', 'b')));
  const d = a.difference(foreign);
  assert.equal(d.size, 2);
  assert.ok(!d.has(Q('a', 'p', 'b')));
  a.free(); i.free(); d.free();
});

test('contains / equals / addAll (foreign N3.Store)', async () => {
  const a = await Dataset.fromString(ABC, 'turtle');
  const foreignSub = n3Store('@prefix ex: <http://ex/> . ex:a ex:p ex:b .');
  assert.ok(a.contains(foreignSub), 'contains() works against an N3.Store');
  const foreignSame = n3Store(ABC);
  assert.ok(a.equals(foreignSame), 'equals() works against an N3.Store');

  const target = await Dataset.create();
  target.addAll(n3Store('@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:b ex:p ex:c .'));
  assert.equal(target.size, 2, 'addAll() ingests an N3.Store');
  a.free(); target.free();
});

test('a sparq Dataset is itself a valid operand to N3.js (round-trip via iteration)', async () => {
  // The reverse direction: our Dataset is iterable, so a foreign library can consume it.
  const a = await Dataset.fromString(ABC, 'turtle');
  const n3 = new N3Store();
  n3.addQuads([...a]); // N3 ingests our quads by iterating us
  assert.equal(n3.size, 3);
  assert.ok(n3.has(N3DF.quad(N3DF.namedNode('http://ex/a'), N3DF.namedNode('http://ex/p'), N3DF.namedNode('http://ex/b'))));
  a.free();
});

// --- the Dataset still satisfies RDF.Dataset structurally -----------------------------------------

test('Dataset exposes the full RDF/JS Dataset member set', async () => {
  const a = await Dataset.create();
  for (const m of [
    'add', 'delete', 'has', 'match', 'size', 'addAll', 'contains', 'deleteMatches', 'difference',
    'equals', 'every', 'filter', 'forEach', 'import', 'intersection', 'map', 'reduce', 'some',
    'toArray', 'toCanonical', 'toStream', 'toString', 'union',
  ]) {
    assert.ok(m in a || typeof a[m] === 'function' || m === 'size', `Dataset has ${m}`);
  }
  a.free();
});
