// Research grade and not externally audited. A fresh process bounds each live prover.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { testIssuerKeygen, testIssue, request, prove, verify } from '../index.js';

const [inputPath, outputPath] = process.argv.slice(2);
const input = JSON.parse(readFileSync(inputPath));
const { sample, index, runtimeHash, entryHash } = input;
const stored = input.request;
const issuer = testIssuerKeygen('00'.repeat(32));
const credentials = sample.sources.map((graphNQuads, slot) => testIssue({ issuer, graphNQuads, seedHex: (index + slot + 1).toString(16).padStart(2, '0').repeat(32) }));
const backend = { type: 'wasm', ...(sample.name === 'authenticated-smoke' ? { presentationSaltHex: '03'.repeat(32) } : {}) };
if (input.knownFailure) {
  await assert.rejects(prove({ request: stored, credentials, backend }), /Circuit execution failed: call to assert_max_bit_size/);
  writeFileSync(outputPath, JSON.stringify({ status: 'known-failure', runtimeHash, entryHash, ...input.knownFailure }));
  process.exit(0);
}
const started = performance.now();
const result = await prove({ request: stored, credentials, backend });
const proveMs = performance.now() - started;
const seen = new Set();
const checked = await verify({ request: stored, presentation: result.presentation, seenNonces: seen });
assert.equal(checked.verified, true, checked.reason);
assert.deepEqual(checked.released, result.result);
assert.deepEqual(checked.report.rule, { query: stored.query, planHash: stored.planHash });
assert.deepEqual(checked.report.issuers, stored.policy.trustedIssuers);
assert.deepEqual(checked.report.subject, []);
assert.equal(checked.report.complete, true);
assert.equal(checked.report.fresh, stored.nonce);
assert.deepEqual([...seen], [stored.nonce]);
const refused = answer => { assert.equal(answer.verified, false); assert.equal('report' in answer, false); };
refused(await verify({ request: stored, presentation: result.presentation }));
refused(await verify({ request: stored, presentation: result.presentation, seenNonces: seen }));
const wrong = request({ query: stored.query, policy: stored.policy, nonce: '08'.repeat(32) });
const untouched = new Set();
refused(await verify({ request: wrong, presentation: result.presentation, seenNonces: untouched }));
const altered = structuredClone(result.presentation);
if ('boolean' in altered.result) altered.result.boolean = !altered.result.boolean;
else altered.result.rows = altered.result.rows.length ? [] : [[null]];
refused(await verify({ request: stored, presentation: altered, seenNonces: untouched }));
assert.equal(untouched.size, 0);
const badKey = { ...stored, verificationKey: '00' };
await assert.rejects(prove({ request: badKey, credentials, backend }), /request reconstruction|key mismatch/);
const missing = { ...stored, query: 'SELECT ?s WHERE {?s <urn:uncatalogued> ?o}' };
await assert.rejects(prove({ request: missing, credentials, backend }), /request reconstruction|missing catalogue plan/);
assert.throws(() => request({ query: missing.query, policy: stored.policy, nonce: stored.nonce }), /missing catalogue plan/);
writeFileSync(outputPath, JSON.stringify({ verified: true, runtimeHash, entryHash, request: stored, ...result, report: checked.report, proveMs }));
