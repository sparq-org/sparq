// SPARQL-injection guard, re-proved against sparq's ACTUAL parse path. [OPUS-4.8]
//
// sq-c4ej: these are the PSS `src/storage/sparql.ts` escaper test-vectors,
// lifted out of the PSS unit suite (where they checked the escaper's *output
// string*) and run here as INTEGRATION tests that round-trip each hostile term
// through `termToNT` AND the sparq engine's real SPARQL parser / N-Triples
// loader. That is the only thing that actually proves the guard: that the
// engine sees one IRI / one literal term, never an injected query fragment.
//
// The priority target is ACL-derived IRIs (the PSS `pss:acl` pointer set via
// `setAclPointer`): an attacker who controls a resource's ACL pointer must not
// be able to smuggle SPARQL through it. `iri()` percent-encodes every
// IRIREF-illegal char (incl `>`); `literal()` escapes the closing quote and
// control chars — identically to QLever's lexer rules.
//
// Note on round-tripping: literal-*value* escaping is lossless (a value comes
// back byte-identical). IRI percent-encoding is intentionally NOT a no-op for
// values that contain IRIREF-illegal chars — the encoded form is the canonical
// one the engine stores. That is correct (it is IRI-preserving), so the tests
// assert against the encoded form for hostile IRIs, and exact equality only for
// literal values and already-legal IRIs.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DataFactory as DF, Dataset, SparqStore, quadsToNQuads, termToNT } from '../dist/index.js';

const seed = () => SparqStore.fromString('<http://ex/s> <http://ex/p> <http://ex/o> .', 'nt');

// --- escaper output vectors (string-level, == the original PSS unit vectors) --

test('iri() percent-encodes every IRIREF-illegal char incl ">"', () => {
  // < > " { } | ^ ` \ and space all become %XX; safe chars pass through.
  assert.equal(termToNT(DF.namedNode('http://ex/a')), '<http://ex/a>');
  assert.equal(termToNT(DF.namedNode('http://ex/o>')), '<http://ex/o%3E>');
  assert.equal(termToNT(DF.namedNode('a<b>c')), '<a%3Cb%3Ec>');
  assert.equal(termToNT(DF.namedNode('a b')), '<a%20b>');
  assert.equal(termToNT(DF.namedNode('a{b}c')), '<a%7Bb%7Dc>');
  assert.equal(termToNT(DF.namedNode('a|b^c`d')), '<a%7Cb%5Ec%60d>');
  assert.equal(termToNT(DF.namedNode('a\\b')), '<a%5Cb>');
  assert.equal(termToNT(DF.namedNode('a\u0000b\u001Fc')), '<a%00b%1Fc>');
  // Already-safe percent-escapes are left intact (not double-encoded).
  assert.equal(termToNT(DF.namedNode('http://ex/a%20b')), '<http://ex/a%20b>');
});

test('literal() escapes the closing quote, backslash and control chars', () => {
  assert.equal(termToNT(DF.literal('a"b\\c\nd\re\tf')), '"a\\"b\\\\c\\nd\\re\\tf"');
  assert.equal(termToNT(DF.literal('\b\f')), '"\\b\\f"');
  assert.equal(termToNT(DF.literal('x\u0000y\u001Fz')), '"x\\u0000y\\u001Fz"');
  // The classic break-out attempt: a value that tries to close its own quote
  // and append a triple pattern stays one literal token.
  assert.equal(termToNT(DF.literal('o" . ?x ?y ?z . #')), '"o\\" . ?x ?y ?z . #"');
});

// --- integration: hostile terms vs the engine's real parse path -------------

test('hostile IRI cannot break out of <...> in a generated SELECT (match path)', async () => {
  const store = await seed();
  // Each of these tried to close the <...> and inject a UNION / extra pattern.
  const payloads = [
    'http://ex/o> } UNION { ?s ?p ?inj . VALUES ?inj { <http://pwned/x>',
    'http://ex/o> . ?a ?b ?c . <http://x',
    'http://ex/o>}#',
    'http://ex/o> } UNION { ?s ?p ?o',
  ];
  for (const p of payloads) {
    const hits = store.match(undefined, undefined, DF.namedNode(p));
    // The encoded IRI is a real (non-matching) term: 0 rows, no parse error,
    // and crucially never the 1 row that a successful `} UNION { ?s ?p ?o`
    // injection would surface.
    assert.equal(hits.length, 0, `IRI payload leaked: ${JSON.stringify(p)}`);
  }
  // Control: the genuine object still matches exactly one quad.
  assert.equal(store.match(undefined, undefined, DF.namedNode('http://ex/o')).length, 1);
});

test('hostile datatype IRI cannot break out of ^^<...> (match path)', async () => {
  const store = await seed();
  const dt = DF.namedNode('http://ex/dt> } UNION { ?s ?p ?x . #');
  assert.equal(store.match(undefined, undefined, DF.literal('o', dt)).length, 0);
});

test('hostile literal value cannot break out of "..." (match path)', async () => {
  const store = await seed();
  const payloads = [
    'o" . ?s ?p ?o . #',
    'o"@en . <http://x> <http://y> "z',
    'o\n?s ?p ?o',
  ];
  for (const v of payloads) {
    assert.equal(
      store.match(undefined, undefined, DF.literal(v)).length,
      0,
      `literal payload leaked: ${JSON.stringify(v)}`,
    );
  }
});

test('hostile literal value round-trips losslessly through the N-Triples loader (data path)', async () => {
  const store = await seed();
  // A hostile literal *value* (every escape-relevant char) with a normal
  // datatype: insert via addQuads (-> quadsToNQuads -> engine loader), read
  // back via match. Literal-value escaping is lossless, so the value AND the
  // datatype come back byte-identical, AND no injected triple appears.
  const subj = DF.namedNode('https://pod.example/acl#owner');
  const pred = DF.namedNode('http://pss/acl');
  const evilLit = DF.literal('grant" . <http://pwned> <http://x> "1\n\t#', DF.namedNode('http://ex/t'));

  store.addQuads([DF.quad(subj, pred, evilLit)]);

  const byPred = store.match(undefined, pred, undefined);
  assert.equal(byPred.length, 1, 'expected exactly the inserted quad (no extra injected ones)');
  assert.ok(byPred[0].object.equals(evilLit), 'literal value/datatype did not round-trip');

  // The whole store still holds only the seed quad + the one insert — the
  // `"1` and `<http://pwned>` fragments did NOT become their own triple.
  assert.equal(store.size, 2);
  assert.equal(store.match(DF.namedNode('http://pwned'), undefined, undefined).length, 0);
});

test('hostile ACL-pointer IRI is neutralised to its percent-encoded canonical form (data path)', async () => {
  const store = await seed();
  // ACL-pointer-shaped subject IRI carrying every IRIREF-illegal char. It is
  // stored as exactly one IRI term — its percent-encoded canonical form — so it
  // matches `iri(rawValue)` (which encodes identically) and injects nothing.
  const rawAcl = 'https://pod.example/acl#a>{} |^`"\\ b';
  const aclIri = DF.namedNode(rawAcl);
  const pred = DF.namedNode('http://pss/acl');
  const obj = DF.namedNode('http://ex/grant');

  store.addQuads([DF.quad(aclIri, pred, obj)]);

  // Exactly one quad was stored — the hostile IRI did not fan out into several
  // triples and did not run any injected operation.
  const hits = store.match(undefined, pred, undefined);
  assert.equal(hits.length, 1, 'hostile ACL IRI did not store as a single term');
  // Its subject is the percent-encoded canonical IRI (illegal chars neutralised,
  // identical to what `termToNT(aclIri)` emits inside `<…>`).
  assert.equal(hits[0].subject.termType, 'NamedNode');
  assert.equal(hits[0].subject.value, termToNT(aclIri).slice(1, -1));
  assert.match(hits[0].subject.value, /%3E/); // the hostile ">" survived as %3E
  assert.doesNotMatch(hits[0].subject.value, />/); // ...and no raw ">" remains
  // No injection: store is seed + the single insert; no phantom subjects.
  assert.equal(store.size, 2);
});

test('hostile terms cannot inject through a SPARQL UPDATE delta (applyDelta path)', async () => {
  const store = await seed();
  // applyDelta serialises both sides via quadsToNQuads and feeds DELETE/INSERT
  // DATA to the engine. A break-out here would corrupt unrelated graph state.
  const g = DF.namedNode('http://ex/g> } ; DROP ALL ; INSERT DATA { <http://x> <http://y> <http://z');
  store.applyDelta([DF.quad(DF.namedNode('http://ex/s2'), DF.namedNode('http://ex/p2'), DF.namedNode('http://ex/o2'), g)]);
  // The seed quad survives and DROP ALL did NOT run.
  assert.equal(store.match(DF.namedNode('http://ex/s'), undefined, undefined).length, 1);
  // No phantom <http://x> <http://y> <http://z> triple was injected.
  assert.equal(store.match(DF.namedNode('http://x'), undefined, undefined).length, 0);
});

// --- term parts with no escape form (blank-node labels, language tags, directions) ----------
// These cannot be percent-encoded or escaped, so termToNT validates them (BLANK_NODE_LABEL,
// LANGTAG, ltr/rtl) and throws. Every public writer path goes through it.

const BNODE_PAYLOAD = 'a <urn:p> <urn:o> .\n<urn:injected>';
const LANG_PAYLOAD = 'en . } UNION { ?s ?p ?o';
const evilBnode = () => DF.blankNode(BNODE_PAYLOAD);
const evilLang = () => DF.literal('x', LANG_PAYLOAD);
const evilDir = () => DF.literal('x', { language: 'en', direction: 'ltr <urn:p> <urn:o> .\n<urn:s> <urn:p>' });
const P = DF.namedNode('urn:p');
const O = DF.namedNode('urn:o');

test('termToNT rejects blank-node labels, language tags and directions that could break out', () => {
  assert.throws(() => termToNT(evilBnode()), /BLANK_NODE_LABEL/);
  for (const label of ['', 'a b', 'a.', '.a', '-a', 'a>b', 'a"b', 'a\nb']) {
    assert.throws(() => termToNT(DF.blankNode(label)), /BLANK_NODE_LABEL/, JSON.stringify(label));
  }
  assert.throws(() => termToNT(evilLang()), /LANGTAG/);
  for (const lang of ['en-', '-en', 'e n', '1en', 'en"']) {
    assert.throws(() => termToNT(DF.literal('x', lang)), /LANGTAG/, lang);
  }
  assert.throws(() => termToNT(evilDir()), /base direction/);
  assert.throws(() => termToNT(DF.literal('a\uD800')), /lone surrogate/);
  assert.throws(() => termToNT(DF.namedNode('http://ex/\uDC00')), /lone surrogate/);
  // Valid parts are unchanged.
  assert.equal(termToNT(DF.blankNode('a.b')), '_:a.b');
  assert.equal(termToNT(DF.blankNode('_x-1')), '_:_x-1');
  assert.equal(termToNT(DF.literal('x', 'en-GB')), '"x"@en-gb');
  assert.equal(termToNT(DF.literal('x', { language: 'ar', direction: 'rtl' })), '"x"@ar--rtl');
});

test('quadsToNQuads / Dataset reject an injecting blank node, language tag or direction', async () => {
  assert.throws(() => quadsToNQuads([DF.quad(evilBnode(), P, O)]), /BLANK_NODE_LABEL/);
  assert.throws(() => quadsToNQuads([DF.quad(DF.namedNode('urn:s'), P, evilLang())]), /LANGTAG/);
  await assert.rejects(Dataset.fromQuads([DF.quad(DF.namedNode('urn:s'), P, evilDir())]), /base direction/);
  const ds = await Dataset.fromQuads([DF.quad(DF.namedNode('urn:s'), P, DF.literal('x', 'en'))]);
  assert.equal(ds.toString(), '<urn:s> <urn:p> "x"@en .\n');
  assert.throws(() => ds.has(DF.quad(DF.namedNode('urn:s'), P, evilLang())), /LANGTAG/);
});

test('fromQuads / addQuads / applyDelta reject an injecting blank node, language tag or direction', async () => {
  await assert.rejects(SparqStore.fromQuads([DF.quad(evilBnode(), P, O)]), /BLANK_NODE_LABEL/);
  const store = await seed();
  assert.throws(() => store.addQuads([DF.quad(evilBnode(), P, O)]), /BLANK_NODE_LABEL/);
  assert.throws(() => store.applyDelta([DF.quad(DF.namedNode('urn:s'), P, evilLang())]), /LANGTAG/);
  assert.throws(() => store.applyDelta([], [DF.quad(DF.namedNode('urn:s'), P, evilDir())]), /base direction/);
  // Nothing was written: the store still holds only the seed triple, and no <urn:injected>.
  assert.equal(store.size, 1);
  assert.equal(store.match(DF.namedNode('urn:injected'), undefined, undefined).length, 0);
});

test('match / countQuads reject an injecting language tag or direction instead of querying', async () => {
  const store = await seed();
  assert.throws(() => store.match(undefined, undefined, evilLang()), /LANGTAG/);
  assert.throws(() => store.countQuads(undefined, undefined, evilLang()), /LANGTAG/);
  assert.throws(() => store.countQuads(undefined, undefined, evilDir()), /base direction/);
  assert.equal(store.countQuads(undefined, undefined, DF.literal('x', 'en')), 0);
});
