import { before, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { prove, verify, issue } from '../index.js';
import { fixture } from './fixtures.js';

let one, two;
before(async () => {
  one = fixture();
  Object.assign(one, await prove(one));
  two = fixture(true);
  Object.assign(two, await prove(two));
});
const check = (source = one, changes = {}) => verify({ expectedQuery: source.query, presentation: source.presentation, policy: source.policy, nonce: source.nonce, seenNonces: new Set(), ...changes });
async function refused(changes, reason) {
  let reads = 0, adds = 0;
  const seenNonces = { has() { reads++; return false; }, add() { adds++; } };
  const result = await check(one, { ...changes, seenNonces });
  assert.equal(result.verified, false);
  assert.deepEqual(result.released, []);
  assert.match(result.reason, reason);
  assert.equal(reads, 0, 'rejected input must not reach the nonce store');
  assert.equal(adds, 0, 'rejected input must not consume a nonce');
}

test('one credential proves public filters, degree constant and identity pattern', async () => {
  const result = await check();
  assert.equal(result.verified, true, result.reason);
  assert.deepEqual(result.released, [{ age: { kind: 'literal', value: '30', datatype: 'http://www.w3.org/2001/XMLSchema#integer' }, p: { kind: 'iri', value: 'https://example.org/alice' } }]);
  assert.equal(one.presentation.issuer_slots.length, 1);
});
test('two credentials prove the joined public patterns', async () => {
  const result = await check(two);
  assert.equal(result.verified, true, result.reason);
  assert.deepEqual(result.released, one.result);
  assert.equal(two.presentation.issuer_slots.length, 2);
});
test('wrong expected query rejects before proof verification', async () => {
  await refused({ expectedQuery: one.query.replace('21', '22') }, /statement rejected:.*query differs/);
  const presentation = structuredClone(one.presentation);
  presentation.proof[0] ^= 1;
  await refused({ expectedQuery: one.query.replace('21', '22'), presentation }, /statement rejected:.*query differs/);
});
test('canonical key check precedes statement reconstruction', async () => {
  const table = JSON.parse(readFileSync(new URL('../assets/members.json', import.meta.url)));
  const path = new URL(`../assets/${table[0].vk}`, import.meta.url);
  const original = readFileSync(path);
  try {
    const changed = Buffer.from(original); changed[0] ^= 1; writeFileSync(path, changed);
    await refused({ expectedQuery: 'wrong request' }, /^canonical member or VK rejected$/);
  } finally { writeFileSync(path, original); }
});
test('noncanonical member rejects before statement reconstruction', async () => {
  const path = new URL('../assets/members.json', import.meta.url);
  const original = readFileSync(path);
  try {
    const table = JSON.parse(original); table[0].member = 'not_a_result_member';
    writeFileSync(path, JSON.stringify(table));
    await refused({ expectedQuery: 'wrong request' }, /^canonical member or VK rejected$/);
  } finally { writeFileSync(path, original); }
});
test('untrusted key rejects with the issuer policy reason', async () => {
  const policy = structuredClone(one.policy); policy.trustedIssuers = [one.policy.trustedIssuers[1]];
  await refused({ policy }, /statement rejected:.*issuer/i);
});
test('status outside freshness policy rejects with the status reason', async () => {
  const policy = structuredClone(one.policy); policy.minVersion++; policy.maxVersion++;
  await refused({ policy }, /statement rejected:.*(status|snapshot)/i);
});
test('revoked status cannot verify against the original proof', async () => {
  const policy = structuredClone(one.policy); policy.snapshots[0].bits[0] = 1;
  await refused({ policy }, /^cryptographic proof rejected$/);
});
test('altered released IRI fails cryptographic binding', async () => {
  const presentation = structuredClone(one.presentation); presentation.rows[0].p.value += '/altered';
  await refused({ presentation }, /^cryptographic proof rejected$/);
});
test('altered public filter operand rejects during reconstruction', async () => {
  const presentation = structuredClone(one.presentation); presentation.rows[0].age.value = '19';
  await refused({ presentation }, /statement rejected:.*(predicate|filter)/i);
});
test('altered proof byte rejects without consuming the nonce', async () => {
  const presentation = structuredClone(one.presentation); presentation.proof[0] ^= 1;
  await refused({ presentation }, /^cryptographic proof rejected$/);
});
test('wrong verifier nonce rejects during reconstruction', async () => {
  await refused({ nonce: '0x123' }, /statement rejected:.*challenge/i);
});
test('nonce is recorded only after a valid proof and replay rejects', async () => {
  const keys = new Set(); let calls = 0;
  const seenNonces = { has: key => keys.has(key), add(key) { calls++; keys.add(key); } };
  const invalid = structuredClone(one.presentation); invalid.proof[0] ^= 1;
  assert.equal((await check(one, { presentation: invalid, seenNonces })).verified, false);
  assert.equal(calls, 0);
  assert.equal((await check(one, { seenNonces })).verified, true);
  assert.equal(calls, 1);
  const replay = await check(one, { seenNonces, nonce: one.nonce.toUpperCase().replace('0X', '0x') });
  assert.equal(replay.verified, false);
  assert.equal(replay.reason, 'challenge already consumed');
  assert.equal(calls, 1);
});
test('test issuance uses independent CSPRNG salts', () => {
  const f = fixture();
  const issuer = { seedHex: 'a1', publicKey: f.credentials[0].issuer };
  const again = issue(issuer, f.credentials[0].graphNQuads, f.credentials[0].statusRef);
  assert.notEqual(again.salt, f.credentials[0].salt);
});

test('proof bytes cannot be coerced to a valid transcript', async () => {
  const presentation = structuredClone(one.presentation); presentation.proof[0] += 256;
  await refused({ presentation }, /statement rejected:.*expected u8/);
});
test('a nonce store is mandatory even for a valid proof', async () => {
  const result = await check(one, { seenNonces: undefined });
  assert.equal(result.verified, false);
  assert.deepEqual(result.released, []);
  assert.equal(result.reason, 'nonce store rejected');
});

test('missing proof rejects with the native missing-proof reason', async () => {
  const presentation = structuredClone(one.presentation); presentation.proof = [];
  await refused({ presentation }, /statement rejected: missing proof/);
});

test('partial proof fields reject before the cryptographic backend', async () => {
  const presentation = structuredClone(one.presentation); presentation.proof.pop();
  await refused({ presentation }, /^malformed proof encoding$/);
});
