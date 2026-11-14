// Research grade and not externally audited. A fresh process bounds each live prover.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { testIssuerKeygen, prove } from '../index.js';
import { issueFixture } from './test-credentials.mjs';
import { proofContract } from './proof-contract.mjs';

const [inputPath, outputPath] = process.argv.slice(2);
const input = JSON.parse(readFileSync(inputPath));
const { sample, index, runtimeHash, entryHash } = input;
const stored = input.request;
const issuer = testIssuerKeygen('00'.repeat(32));
const credentials = sample.sources.map((graphNQuads, slot) => issueFixture(issuer, graphNQuads, (index + slot + 1).toString(16).padStart(2, '0').repeat(32)));
const backend = { type: 'wasm', ...(sample.name === 'authenticated-smoke' ? { presentationSaltHex: '03'.repeat(32) } : {}) };
if (input.knownFailure) {
  await assert.rejects(prove({ request: stored, credentials, backend }), /Circuit execution failed: call to assert_max_bit_size/);
  writeFileSync(outputPath, JSON.stringify({ status: 'known-failure', runtimeHash, entryHash, ...input.knownFailure }));
  process.exit(0);
}
const started = performance.now();
const result = await prove({ request: stored, credentials, backend });
const proveMs = performance.now() - started;
const checked = await proofContract({ stored, result, credentials, backend });
writeFileSync(outputPath, JSON.stringify({ verified: true, runtimeHash, entryHash, request: stored, ...result, report: checked.report, proveMs }));
