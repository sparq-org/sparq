// Research grade and not externally audited. Synthetic public test keys only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createRequire } from 'node:module';
import { issuerKeygen, testIssuerKeygen, issue, testIssue, request, prove, verify } from '../index.js';
const fixture = JSON.parse(readFileSync(new URL('./fixtures/authenticated.json', import.meta.url)));
const vector = JSON.parse(readFileSync(new URL('./fixtures/issuance.json', import.meta.url)));
const clone = x => structuredClone(x);
const refused = answer => { assert.equal(answer.verified, false); assert.ok(answer.reason); assert.equal('report' in answer, false); };

test('suite deterministic issuance matches the native reference byte for byte', () => {
  const issuer = testIssuerKeygen(vector.issuerSeedHex);
  assert.equal(issuer.publicKey, vector.expected.publicKey);
  const input = { issuer, graphNQuads: vector.graphNQuads, status: { proofConfigNQuads: vector.proofConfigNQuads }, seedHex: vector.seedHex };
  const { publicKey, ...expected } = vector.expected;
  assert.deepEqual(testIssue(input), { ...expected, issuer: publicKey });
  assert.deepEqual(testIssue(input), testIssue(input));
});
test('production issuance draws fresh randomness and test issuance refuses production keys', () => {
  const issuer = issuerKeygen(), other = issuerKeygen();
  assert.notEqual(issuer.secretKeyHex, other.secretKeyHex);
  assert.throws(() => issuerKeygen(vector.issuerSeedHex), /seed/);
  const input = { issuer, graphNQuads: vector.graphNQuads };
  assert.notEqual(issue(input).saltHex, issue(input).saltHex);
  assert.throws(() => testIssue({ ...input, seedHex: vector.seedHex }), /test issuer/);
  assert.throws(() => issue({ ...input, seedHex: vector.seedHex }), /seed/);
});
test('stored request derives its plan and refuses uncatalogued plans and unsupported policies', () => {
  const r = fixture.request;
  assert.deepEqual(request({ query: r.query, policy: r.policy, nonce: r.nonce }), r);
  assert.throws(() => request({ query: 'SELECT ?s WHERE { ?s <urn:uncatalogued> ?o }', policy: r.policy, nonce: r.nonce }), /missing catalogue plan/);
  assert.throws(() => request({ query: r.query, policy: { ...r.policy, holderBinding: {} }, nonce: r.nonce }), /unavailable/);
});
test('holder refuses a key mismatch before decoding credentials', async () => {
  const r = clone(fixture.request); r.verificationKey = '00';
  await assert.rejects(prove({ request: r, credentials: null }), /request reconstruction|key mismatch/);
});
test('holder refuses missing native binaries with the expected version and digest', async () => {
  await assert.rejects(prove({ request: fixture.request, credentials: null, backend: { type: 'native', nargoPath: '/missing-sparq-binary' } }), /missing.*nargo expected.*SHA-256/);
});
test('binary digest is checked before any child executes', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-query-pin-test-'));
  const binary = join(directory, 'counterfeit'), marker = join(directory, 'executed');
  try {
    writeFileSync(binary, `#!/bin/sh\ntouch '${marker}'\necho counterfeit\n`, { mode: 0o700 });
    await assert.rejects(prove({ request: fixture.request, credentials: null, backend: { type: 'native', nargoPath: binary } }), /binary pin mismatch.*nargo expected.*SHA-256/);
    assert.equal(existsSync(marker), false);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
test('native authenticated proof verifies in Node and reports only the bound guarantees', async () => {
  const seen = new Set();
  const answer = await verify({ ...fixture, seenNonces: seen });
  assert.equal(answer.verified, true, answer.reason);
  assert.deepEqual(answer.released, fixture.result);
  assert.deepEqual(answer.report.rule, { query: fixture.request.query, planHash: fixture.request.planHash });
  assert.deepEqual(answer.report.issuers, fixture.request.policy.trustedIssuers);
  assert.deepEqual(answer.report.subject, []);
  assert.equal(answer.report.complete, true);
  assert.equal(answer.report.fresh, fixture.request.nonce);
  assert.deepEqual([...seen], [fixture.request.nonce]);
});
test('replayed challenge is refused without a report', async () => {
  refused(await verify({ ...fixture, seenNonces: new Set([fixture.request.nonce]) }));
});
test('wrong verifier request is refused before consuming its challenge', async () => {
  const r = fixture.request;
  const wrong = request({ query: r.query, policy: r.policy, nonce: '08'.repeat(32) });
  const seen = new Set(); refused(await verify({ request: wrong, presentation: fixture.presentation, seenNonces: seen }));
  assert.equal(seen.size, 0);
});
test('altered released result reaches proof rejection without consuming the challenge', async () => {
  const presentation = clone(fixture.presentation); presentation.result.boolean = !presentation.result.boolean;
  const seen = new Set(); const answer = await verify({ request: fixture.request, presentation, seenNonces: seen });
  refused(answer); assert.match(answer.reason, /proof rejected/); assert.equal(seen.size, 0);
});
test('missing nonce store fails closed', async () => { const answer = await verify(fixture); refused(answer); assert.match(answer.reason, /nonce store is required/); });
test('nonce-store infrastructure errors fail closed', async () => {
  refused(await verify({ ...fixture, seenNonces: { consume() { throw new Error('store offline'); } } }));
});
test('invalid proof cannot consume a nonce even when the store would fail', async () => {
  let calls = 0; const presentation = clone(fixture.presentation); presentation.proof = '00';
  refused(await verify({ request: fixture.request, presentation, seenNonces: { consume() { calls++; throw new Error('must not run'); } } }));
  assert.equal(calls, 0);
});

test('agreed commitments use the supplied presentation salt and reject a different salt', () => {
  const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
  const issuer = testIssuerKeygen('00'.repeat(32));
  const credentials = [testIssue({ issuer, graphNQuads: '<a:> <b:> <c:> .', seedHex: '01'.repeat(32) })];
  const salt = '03'.repeat(32);
  const prepare = (stored, presentationSaltHex) => JSON.parse(host.hostCall('prepareProof', JSON.stringify({ request: stored, credentials, clock: 1, presentationSaltHex })));
  const prepared = prepare(fixture.request, salt);
  const r = fixture.request;
  const anchored = request({ query: r.query, policy: { ...r.policy, inputCommitment: prepared.presentationSkeleton.inputCommitment }, nonce: r.nonce });
  assert.equal(prepare(anchored, salt).presentationSkeleton.inputCommitment, anchored.policy.inputCommitment);
  assert.throws(() => prepare(anchored, '04'.repeat(32)), /dataset commitment agreement/);
});
