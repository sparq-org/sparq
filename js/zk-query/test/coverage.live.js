// Research grade and not externally audited. Synthetic coverage authority only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { coverageDocument, request, prove, testIssue, testIssuerKeygen, verify } from '../index.js';
import { canonicalGraph, issueFixture, trustMembers } from '../scripts/test-credentials.mjs';

test('a native proof binds the detached coverage digest as well as the agreed dataset', async () => {
  const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
  const issuer = testIssuerKeygen('00'.repeat(32));
  const signer = testIssuerKeygen('10'.repeat(32));
  const credentials = [issueFixture(issuer, canonicalGraph(''), '01'.repeat(32))];
  const presentationSaltHex = '03'.repeat(32);
  const clock = Math.floor(Date.now() / 1000);
  const query = 'ASK {}';
  const nonce = '47'.repeat(32);
  const policy = {
    bounds: { credentials: 1, quadsPerCredential: 2, rowsPerTable: 1 },
    trustedIssuers: [issuer.publicKey], trustMembers: trustMembers(issuer),
    audience: 'urn:coverage:test:verifier', notBefore: clock, notAfter: clock + 3600,
  };
  const open = request({ query, policy, nonce });
  const prepared = JSON.parse(host.hostCall('prepareProof', JSON.stringify({
    request: open, credentials, clock, presentationSaltHex,
  })));
  const inputCommitment = prepared.presentationSkeleton.inputCommitment;
  const plain = request({ query, policy: { ...policy, inputCommitment }, nonce });
  const scope = {
    registry: 'urn:registry', subject: 'urn:subject', claimTypes: ['urn:claim'],
    periodStart: 0, periodEnd: 1, method: 'urn:inventory', profile: 'urn:inventory:v1',
  };
  const authority = { issuer: 'urn:authority', verificationMethod: 'urn:key', publicKey: signer.publicKey };
  const { issuer: authorityIri, verificationMethod } = authority;
  const credential = testIssue({
    issuer: signer, seedHex: '13'.repeat(32),
    ...coverageDocument({ issuer: authorityIri, verificationMethod, scope, asOf: clock,
      expiresAt: policy.notAfter, inputCommitment }),
  });
  const stored = request({ query, policy: { ...policy, inputCommitment,
    coverage: { authority, scope, credential } }, nonce });
  const { presentation, result } = await prove({ request: stored, credentials,
    backend: { type: 'native', presentationSaltHex } });
  assert.deepEqual(result, { boolean: true });
  const checked = await verify({ request: stored, presentation, seenNonces: new Set() });
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.released, result);
  assert.equal(checked.report.coverage.issuer, authority.issuer);
  assert.deepEqual(checked.report.coverage.scope, scope);
  assert.equal(checked.report.coverage.inputCommitment, inputCommitment);

  const seenNonces = new Set();
  const movingProof = { ...presentation, requestDigest: plain.digest };
  const refused = await verify({ request: plain, presentation: movingProof, seenNonces });
  assert.equal(refused.verified, false);
  assert.match(refused.reason, /cryptographic proof rejected/);
  assert.equal(seenNonces.size, 0);
});
