import { before, test } from 'node:test';
import { createRequire } from 'node:module';
import assert from 'node:assert/strict';
import { prove, verify, testIssue, testIssuerKeygen } from '../index.js';
import { fixture } from './helpers/fixtures.js';
import { filterFixture } from './helpers/filters.js';

const host = createRequire(import.meta.url)('../wasm/sparq_zk_result_wasm.cjs');

const issuerA = 'urn:issuer:a';
const issuerB = 'urn:issuer:b';
const predicate = 'https://w3id.org/sparq/vcq#verifiedIssuer';
const scope = (query, iri = issuerA) => query.replace('WHERE {', `WHERE { ?g <${predicate}> <${iri}> . GRAPH ?g {`) + '\n}';
const twoGraphs = `PREFIX ex: <https://example.org/>
SELECT DISTINCT ?p ?age WHERE {
  ?g <${predicate}> <${issuerA}> .
  ?h <${predicate}> <${issuerA}> .
  GRAPH ?g { ?p ex:age ?age . FILTER (?age >= 21) }
  GRAPH ?h { ?p ex:degree ex:BSc . ?p ex:identity.verified true }
}`;
let one, two, wrong, mixed;
before(async () => {
  one = fixture();
  one.policy.issuers = { [issuerA]: one.policy.trustedIssuers[0], [issuerB]: one.policy.trustedIssuers[1] };
  one.query = scope(one.query);
  // Exercise the map without requiring the older trust-list property.
  delete one.policy.trustedIssuers;
  Object.assign(one, await prove(one));

  two = fixture(true);
  two.policy = one.policy;
  const first = testIssuerKeygen('a1');
  two.credentials[1] = testIssue({ issuer: first, graphNQuads: two.credentials[1].graphNQuads, status: two.credentials[1].statusRef, seedHex: 'c3' });
  two.query = twoGraphs;
  Object.assign(two, await prove(two));

  wrong = fixture();
  wrong.policy = one.policy;
  wrong.credentials = [testIssue({ issuer: testIssuerKeygen('b2'), graphNQuads: wrong.credentials[0].graphNQuads, status: wrong.credentials[0].statusRef, seedHex: 'd4' })];
  Object.assign(wrong, await prove(wrong));
  mixed = fixture(true);
  mixed.policy = one.policy;
  Object.assign(mixed, await prove(mixed));
});

const check = (source, changes = {}) => verify({ expectedQuery: source.query, presentation: source.presentation, policy: source.policy, nonce: source.nonce, seenNonces: new Set(), ...changes });
async function refused(source, changes, reason) {
  let calls = 0;
  const checked = await check(source, { ...changes, seenNonces: { has() { calls++; return false; }, add() { calls++; } } });
  assert.equal(checked.verified, false);
  assert.deepEqual(checked.released, []);
  assert.equal(Object.hasOwn(checked, 'report'), false);
  assert.match(checked.reason, reason);
  assert.equal(calls, 0, 'scope refusals must leave the nonce store untouched');
}

test('scoped single issuer proves and reports the receiver IRI', async () => {
  const checked = await check(one);
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.released, one.result);
  assert.equal(checked.released[0].p.value, 'https://example.org/alice');
  assert.equal(checked.report.rule.query, one.query);
  assert.deepEqual(checked.report.issuers, [issuerA]);
  assert.deepEqual(checked.report.subject, []);
  assert.equal(checked.report.complete, false);
});

test('two graph variables with the same issuer prove using separate credentials', async () => {
  assert.equal(two.presentation.issuer_slots.length, 2);
  const checked = await check(two);
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.released, one.result);
  assert.deepEqual(checked.report.issuers, [issuerA]);
});

test('proving excludes another trusted issuer even when its facts match', async () => {
  await assert.rejects(prove({ ...wrong, query: scope(wrong.query) }));
  // An unrelated matching solution must not enter the released result either.
  const unrelated = testIssue({ issuer: testIssuerKeygen('b2'), graphNQuads: wrong.credentials[0].graphNQuads.replaceAll('alice', 'bob'), status: wrong.credentials[0].statusRef, seedHex: 'e5' });
  const prepared = await prove({ ...one, credentials: [unrelated, ...one.credentials] });
  assert.deepEqual(prepared.result, one.result);
  assert.deepEqual(prepared.presentation.issuer_slots, one.presentation.issuer_slots);
});

test('a genuine flat proof from the other trusted key fails scope before nonce access', async () => {
  const query = scope(wrong.query);
  const presentation = { ...wrong.presentation, query };
  await refused(wrong, { expectedQuery: query, presentation }, /issuer-scope-mismatch/);
  const unscoped = await check(wrong);
  assert.equal(unscoped.verified, true, unscoped.reason);
  assert.deepEqual(unscoped.report.issuers, [issuerB]);
});

test('every issuer slot must match, including a mixed genuine proof', async () => {
  const presentation = { ...mixed.presentation, query: twoGraphs };
  assert.equal(presentation.issuer_slots.length, 2);
  for (const iri of [issuerA, issuerB]) {
    const query = twoGraphs.replaceAll(issuerA, iri);
    await refused(mixed, { expectedQuery: query, presentation: { ...presentation, query } }, /issuer-scope-mismatch/);
  }
  const unscoped = await check(mixed);
  assert.equal(unscoped.verified, true, unscoped.reason);
  assert.deepEqual(new Set(unscoped.report.issuers), new Set([issuerA, issuerB]));
});

test('different issuer IRIs reject even when the policy aliases the same key', async () => {
  const query = twoGraphs.replace(`?h <${predicate}> <${issuerA}>`, `?h <${predicate}> <${issuerB}>`);
  const policy = { ...one.policy, issuers: { [issuerA]: one.credentials[0].issuer, [issuerB]: one.credentials[0].issuer } };
  await assert.rejects(prove({ ...two, query, policy }), /per-fact-issuer-binding-needs-query-method/);
  await refused(two, { expectedQuery: query, policy, presentation: { ...two.presentation, query } }, /per-fact-issuer-binding-needs-query-method/);
});

test('unknown IRI rejects even if the key is in the older trusted list', async () => {
  const query = one.query.replace(issuerA, 'urn:issuer:unknown');
  const policy = { ...one.policy, trustedIssuers: [one.credentials[0].issuer] };
  await assert.rejects(prove({ ...one, query, policy }), /issuer-not-in-policy/);
  await refused(one, { expectedQuery: query, policy, presentation: { ...one.presentation, query } }, /issuer-not-in-policy/);
});

test('scoped hidden date and integer lowering preserves the existing statement', () => {
  for (const type of ['date', 'integer']) {
    const f = filterFixture(type);
    f.policy.issuers = { [issuerA]: f.credentials[0].issuer };
    const prepare = query => JSON.parse(host.prepare(query, f.credentials.map(c => JSON.stringify(c)), JSON.stringify(f.policy), f.nonce, '{}'));
    const flat = prepare(f.query);
    const scopedQuery = scope(f.query);
    const scoped = prepare(scopedQuery);
    assert.equal(scoped.member, flat.member);
    assert.deepEqual(scoped.witnessInputs, flat.witnessInputs);
    const reconstruct = (query, skeleton) => JSON.parse(host.reconstructPublicInputs(query, JSON.stringify(skeleton), JSON.stringify(f.policy), f.nonce));
    const scopedStatement = reconstruct(scopedQuery, scoped.presentationSkeleton);
    assert.deepEqual(scopedStatement.publicInputs, reconstruct(f.query, flat.presentationSkeleton).publicInputs);
    assert.deepEqual(scopedStatement.report.issuers, [issuerA]);
  }
});
