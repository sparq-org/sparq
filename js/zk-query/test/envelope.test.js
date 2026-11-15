// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { holderKeygen, testHolderKeygen, statusSnapshot, request } from '../index.js';
import { examplePolicy } from '../scripts/catalogue-example.mjs';
import { targetFixture, targetEntry, targetNames } from '../scripts/solid-targets.mjs';
const table = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
test('holder keys: random by default, deterministic only through the test entry point', () => {
  assert.throws(() => holderKeygen('11'.repeat(32)), /does not accept a seed/);
  const a = holderKeygen(), b = holderKeygen();
  assert.notEqual(a.secretKeyHex, b.secretKeyHex);
  assert.deepEqual(testHolderKeygen('11'.repeat(32)), testHolderKeygen('11'.repeat(32)));
  assert.match(a.multikey, /^[0-9a-f]+$/); assert.notEqual(a.multikey, b.multikey);
});
test('status snapshots commit to list, purpose and every bit', () => {
  const list = { list: 'urn:list', purpose: 'revocation', bits: '0010' };
  const snapshot = statusSnapshot(list);
  assert.deepEqual(snapshot, statusSnapshot(list));
  assert.equal(snapshot.depth, 2); assert.equal(snapshot.bitLength, 4);
  for (const changed of [{ bits: '0011' }, { bits: '00100' }, { purpose: 'suspension' }, { list: 'urn:other' }]) assert.notEqual(statusSnapshot({ ...list, ...changed }).digest, snapshot.digest);
  assert.throws(() => statusSnapshot({ ...list, bits: '' }), /empty/);
  assert.throws(() => statusSnapshot({ ...list, bits: '012' }), /0 or 1/);
});
test('every Solid target request derives its catalogued plan and stores a canonical envelope policy', () => {
  for (const target of Object.keys(targetNames)) {
    const entry = targetEntry(table, target); assert(entry, `${targetNames[target]} is catalogued`);
    const fixture = targetFixture(target);
    const digests = fixture.policy.acceptedSnapshots.digests.map(d => d.toUpperCase()).reverse();
    const policy = { ...examplePolicy(entry.example, { publicKey: fixture.policy.trustedIssuers[0] }, 'urn:sparq:test:verifier'), ...fixture.policy, acceptedSnapshots: { digests: [...digests, digests[0]] } };
    const stored = request({ query: entry.example.query, policy, nonce: '07'.repeat(32) });
    assert.equal(stored.planHash, entry.planHash);
    assert.deepEqual(stored.policy.acceptedSnapshots.digests, [...new Set(fixture.policy.acceptedSnapshots.digests)].sort());
    assert.equal(Boolean(stored.policy.holderBinding), Boolean(entry.example.envelope.holder));
    const { trustMembers, ...withoutMembers } = policy;
    assert.throws(() => request({ query: entry.example.query, policy: withoutMembers, nonce: '07'.repeat(32) }));
  }
});
