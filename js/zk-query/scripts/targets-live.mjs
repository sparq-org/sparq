// Research grade and not externally audited. Invoked only through the live gate.
// The six Solid targets prove natively from Node with their full credential
// envelopes, and the envelope refusals fire.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { request, prove, verify, testHolderKeygen } from '../index.js';
import { sha256 } from '../internal/runtime.mjs';
import { examplePolicy } from './catalogue-example.mjs';
import { targetFixture, targetEntry, targetNames, instant } from './solid-targets.mjs';
import { liveRuntimeHash } from './live-runtime.mjs';
test('Solid targets prove with full envelopes; revoked, stale, expired and missing-holder evidence refuses', async () => {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const directory = join(process.env.SPARQ_QUERY_TEST_DIR, 'targets'); mkdirSync(directory, { recursive: true, mode: 0o700 });
  const table = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
  const runtimeHash = liveRuntimeHash(root);
  const backend = { type: 'native' };
  const stored = (target, fixture, overrides = {}, nonce = '5a') => {
    const entry = targetEntry(table, target); assert(entry, `${targetNames[target]} is catalogued`);
    const policy = { ...examplePolicy(entry.example, { publicKey: fixture.policy.trustedIssuers[0] }, 'urn:sparq:test:verifier'), ...fixture.policy, notBefore: 0, notAfter: 32503680000, ...overrides };
    return request({ query: entry.example.query, policy, nonce: nonce.repeat(32) });
  };
  for (const target of Object.keys(targetNames)) {
    const entry = targetEntry(table, target);
    const fixture = targetFixture(target);
    const path = join(directory, targetNames[target] + '.json');
    const entryHash = sha256(JSON.stringify([entry.acirSha256, entry.vkSha256, table.sourceBundleSha256]));
    let evidence = existsSync(path) ? JSON.parse(readFileSync(path)) : undefined;
    if (!evidence || evidence.runtimeHash !== runtimeHash || evidence.entryHash !== entryHash) {
      const req = stored(target, fixture);
      const result = await prove({ request: req, credentials: fixture.credentials, holder: fixture.holder, statusLists: fixture.statusLists, backend });
      evidence = { runtimeHash, entryHash, request: req, ...result };
      writeFileSync(path, JSON.stringify(evidence, null, 2));
    }
    const checked = await verify({ request: evidence.request, presentation: evidence.presentation, seenNonces: new Set() });
    assert.equal(checked.verified, true, checked.reason);
    assert.deepEqual(checked.released, evidence.result);
    assert.equal(evidence.result.rows.length, 1, `${target} honest answer`);
    assert.match(String(evidence.result.rows[0][0]), /^"true"/);
    const issuers = fixture.policy.trustMembers.map(m => m.issuer).sort();
    assert.deepEqual([...checked.report.subject].sort(), fixture.holder ? issuers : []);
    assert.equal(checked.report.validityTime, instant); assert.equal(checked.report.earliestCreated, instant);
    assert.equal(checked.report.status.allowMissing, false);
    console.log('Solid target proved and verified from Node:', targetNames[target]);
  }
  // Refusals on insurance 1a: named host refusals before proving, then witness refusals.
  const honest = targetFixture('1a');
  const req = stored('1a', honest);
  const proveWith = (fixture, overrides = {}) => prove({ request: req, credentials: fixture.credentials, holder: fixture.holder, statusLists: fixture.statusLists, backend, ...overrides });
  await assert.rejects(proveWith(targetFixture('1a', 'revoked')), /credential status is set \(revocation\)/);
  await assert.rejects(proveWith(honest, { statusLists: honest.statusLists.map(l => ({ ...l, bits: '00' })) }), /status snapshot is not accepted by the request/);
  await assert.rejects(proveWith(honest, { holder: undefined }), /holder key required by the request/);
  const witness = /native witness execution failed/;
  await assert.rejects(proveWith(honest, { holder: testHolderKeygen('29'.repeat(32)) }), witness);
  await assert.rejects(prove({ request: stored('1a', honest, { validityTime: Date.parse('2031-01-01T00:00:00Z') }, '5b'), credentials: honest.credentials, holder: honest.holder, statusLists: honest.statusLists, backend }), witness);
  await assert.rejects(prove({ request: stored('1a', honest, { earliestCreated: Date.parse('2026-11-14T00:00:00Z') }, '5c'), credentials: honest.credentials, holder: honest.holder, statusLists: honest.statusLists, backend }), witness);
  console.log('Solid target envelope refusals passed: revoked, stale snapshot, missing holder, foreign holder, expired, stale credential.');
});
