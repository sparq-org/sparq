// Research grade and not externally audited. Synthetic credentials only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Noir } from '@noir-lang/noir_js';
import { request, testIssuerKeygen } from '../index.js';
import { sha256 } from '../internal/runtime.mjs';
import { examplePolicy } from '../scripts/catalogue-example.mjs';
import { issueFixture } from '../scripts/test-credentials.mjs';

test('the pinned SCAN program executes the package-generated witness offline', async () => {
  const assets = new URL('../assets/', import.meta.url);
  const read = name => readFileSync(new URL(name, assets));
  const table = JSON.parse(read('catalogue.json'));
  assert.equal(sha256(read('sources.json')), table.sourceBundleSha256);
  const example = JSON.parse(readFileSync(new URL('./fixtures/n0.json', import.meta.url)))
    .find(sample => sample.name === 'bgp');
  const issuer = testIssuerKeygen('00'.repeat(32));
  const stored = request({
    query: example.query,
    policy: examplePolicy(example, issuer, 'urn:sparq:test:scan'),
    nonce: '09'.repeat(32),
  });
  const entry = table.entries.find(entry => entry.planHash === stored.planHash);
  const programBytes = read(entry.acir);
  assert.equal(sha256(programBytes), entry.acirSha256);
  const program = JSON.parse(programBytes);
  for (const name of ['target_0', 'source_0']) {
    assert.equal(program.abi.parameters.find(p => p.name === name)?.visibility, 'private');
  }
  const credentials = example.sources.map(graph => issueFixture(issuer, graph, '01'.repeat(32)));
  const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
  const prepared = JSON.parse(host.hostCall('prepareProof', JSON.stringify({
    request: stored, credentials, clock: stored.policy.notBefore,
  })));
  assert.deepEqual(prepared.result.variables, ['s', 'x']);
  assert.deepEqual(prepared.result.rows.toSorted(), [
    ['<urn:a>', '"1"^^<http://www.w3.org/2001/XMLSchema#integer>'],
    ['<urn:b>', '"2"^^<http://www.w3.org/2001/XMLSchema#integer>'],
  ]);
  const { witness } = await new Noir(program).execute(prepared.witnessInputs.json);
  assert(witness.length > 0);
});
