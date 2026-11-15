import { before, test } from 'node:test';
import assert from 'node:assert/strict';
import { prove, verify } from '../index.js';
import { fixture } from './helpers/fixtures.js';

const issuerA = 'urn:issuer:a';
const issuerB = 'urn:issuer:b';
const scope = query => query.replace('WHERE {', `WHERE { ?g <https://w3id.org/sparq/vcq#verifiedIssuer> <${issuerA}> . GRAPH ?g {`) + '\n}';
let flat, scoped, mixed, roster;
before(async () => {
  flat = fixture();
  const [a, b] = flat.policy.trustedIssuers;
  roster = [
    { issuer: issuerA, verificationMethod: 'urn:method:a', publicKey: a },
    { issuer: issuerB, verificationMethod: 'urn:method:b', publicKey: b },
  ];
  // One policy object, with the query package's exact member field names.
  flat.policy.trustMembers = [roster[1], roster[0], roster[1]];
  scoped = { ...flat, query: scope(flat.query) };
  Object.assign(flat, await prove(flat));
  Object.assign(scoped, await prove(scoped));
  mixed = fixture(true);
  mixed.policy = flat.policy;
  Object.assign(mixed, await prove(mixed));
});

const check = (source, policy = source.policy, extra = {}) => verify({
  expectedQuery: source.query, presentation: source.presentation,
  policy, nonce: source.nonce, seenNonces: new Set(), ...extra,
});
async function refused(source, policy, reason, exactReason) {
  await assert.rejects(prove({ ...source, policy }), error => {
    assert.match(String(error), reason);
    if (exactReason !== undefined) assert.equal(String(error), exactReason);
    return true;
  });
  let calls = 0;
  const checked = await check(source, policy, {
    seenNonces: { has() { calls++; return false; }, add() { calls++; } },
  });
  assert.equal(checked.verified, false);
  assert.deepEqual(checked.released, []);
  assert.equal(Object.hasOwn(checked, 'report'), false);
  assert.match(checked.reason, reason);
  if (exactReason !== undefined) assert.equal(checked.reason, `statement rejected: ${exactReason}`);
  assert.equal(calls, 0, 'policy rejection must precede nonce access');
}

test('one query-shaped policy proves and verifies scoped and unscoped results unchanged', async () => {
  assert.equal(scoped.policy, flat.policy);
  assert.deepEqual(scoped.result, flat.result);
  for (const source of [flat, scoped]) {
    const original = structuredClone(source.policy);
    const checked = await check(source);
    assert.equal(checked.verified, true, checked.reason);
    assert.deepEqual(checked.released, source.result);
    assert.deepEqual(checked.report.issuers, [issuerA]);
    assert.deepEqual(checked.report.trustMembers, roster);
    assert.deepEqual(source.policy, original, 'caller policy must remain unchanged');
    const withoutList = { ...source.policy };
    delete withoutList.trustedIssuers;
    const derived = await check(source, withoutList);
    assert.equal(derived.verified, true, derived.reason);
    assert.deepEqual(derived.report, checked.report);
    const reordered = {
      ...source.policy,
      trustedIssuers: [roster[1].publicKey, roster[0].publicKey, roster[1].publicKey],
      trustMembers: [roster[0], roster[1], roster[0]],
    };
    const reorderedCheck = await check(source, reordered);
    assert.equal(reorderedCheck.verified, true, reorderedCheck.reason);
    assert.deepEqual(reorderedCheck.report, checked.report);
  }
});

test('mismatched key sets reject both paths, including explicit empty lists', async () => {
  for (const changes of [
    { trustedIssuers: [] },
    { trustedIssuers: [roster[0].publicKey] },
    { trustMembers: [] },
  ]) {
    await refused(flat, { ...flat.policy, ...changes }, /trust member keys differ from trusted issuers/, 'trust member keys differ from trusted issuers');
  }
  const repairedMap = { ...flat.policy, trustedIssuers: [], issuers: { [issuerA]: roster[0].publicKey, [issuerB]: roster[1].publicKey } };
  await refused(flat, repairedMap, /trust member keys differ from trusted issuers/, 'trust member keys differ from trusted issuers');
});

test('issuer map conflicts reject, and agreeing entries keep the same report', async () => {
  await refused(flat, { ...flat.policy, issuers: { [issuerA]: roster[1].publicKey } }, /trust member key differs from issuer map/);
  const policy = { ...flat.policy, issuers: { [issuerA]: roster[0].publicKey } };
  const checked = await check(scoped, policy);
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.report.trustMembers, roster);
  assert.deepEqual(checked.report.issuers, [issuerA]);
});

test('member IRIs and key spellings are validated in both paths', async () => {
  for (const field of ['issuer', 'verificationMethod']) {
    const trustMembers = [{ ...roster[0], [field]: 'relative' }, roster[1]];
    await refused(flat, { ...flat.policy, trustMembers }, /IRI/);
  }
  for (const publicKey of ['invalid', `0x${roster[0].publicKey}`, roster[0].publicKey.toUpperCase()]) {
    const trustMembers = [{ ...roster[0], publicKey }, roster[1]];
    const policy = { ...flat.policy, trustMembers };
    delete policy.trustedIssuers;
    await refused(flat, policy, /(?:invalid|noncanonical) trust member key/);
  }
});

test('key rotation accepts unscoped proofs but rejects scoped queries even with one used key', async () => {
  const trustMembers = [roster[0], { ...roster[1], issuer: issuerA }];
  const policy = { ...flat.policy, trustMembers };
  for (const source of [flat, mixed]) {
    const checked = await check(source, policy);
    assert.equal(checked.verified, true, checked.reason);
    assert.deepEqual(checked.released, source.result);
    assert.deepEqual(checked.report.trustMembers, trustMembers);
    assert.deepEqual(checked.report.issuers, source.presentation.issuer_slots.map(() => issuerA));
  }
  await refused(scoped, policy, /issuer-scope-ambiguous-key/);
  const query = scoped.query.replaceAll(issuerA, issuerB);
  const checked = await check(scoped, { ...policy, issuers: { [issuerB]: roster[0].publicKey } }, {
    expectedQuery: query, presentation: { ...scoped.presentation, query },
  });
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.report.issuers, [issuerB]);
  assert.deepEqual(checked.report.trustMembers, trustMembers);
});

test('legacy policies retain their report shape and trusted-key spellings', async () => {
  const policy = { ...flat.policy, trustedIssuers: flat.policy.trustedIssuers.map(key => `0x${key}`) };
  delete policy.trustMembers;
  const checked = await check(flat, policy);
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.report.issuers, flat.presentation.issuer_slots);
  assert.equal(Object.hasOwn(checked.report, 'trustMembers'), false);
});
