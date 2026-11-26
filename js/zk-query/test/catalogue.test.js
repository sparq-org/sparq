// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cpSync, symlinkSync, mkdtempSync, readFileSync, writeFileSync, rmSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { prove, request, testIssuerKeygen } from '../index.js';
import { sha256 } from '../internal/runtime.mjs';
import { catalogueBuildDigest } from '../scripts/cache.mjs';
import { canonicalGraph, issueFixture } from '../scripts/test-credentials.mjs';
import { examplePolicy } from '../scripts/catalogue-example.mjs';
import { wasmProverFits, keyDyadicSize } from '../wasm-prover.mjs';
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
test('every catalogue example derives its pinned plan and leaves no stale assets', () => {
  const directory = new URL('../assets/', import.meta.url);
  const table = JSON.parse(readFileSync(new URL('catalogue.json', directory)));
  const toolchainDigest = sha256(readFileSync(new URL('toolchain.pins.json', directory)));
  const issuer = testIssuerKeygen('00'.repeat(32));
  for (const entry of table.entries) {
    const query = entry.example.query;
    const policy = examplePolicy(entry.example, issuer, 'urn:sparq:test:catalogue');
    const stored = request({ query, policy, nonce: '09'.repeat(32) });
    assert.equal(stored.planHash, entry.planHash);
    assert.equal(stored.verificationKey, readFileSync(new URL(entry.vk, directory), 'utf8').trim());
    const plan = JSON.parse(host.hostCall('plan', JSON.stringify({ query, policy })));
    assert.equal(catalogueBuildDigest(plan.files, table.sourceBundleSha256, toolchainDigest), entry.buildInputSha256, `stale build inputs for ${entry.example.name}`);
    if (entry.wasmProving) assert(wasmProverFits(keyDyadicSize(Buffer.from(stored.verificationKey, 'hex'))), `${entry.planHash} admits WebAssembly beyond its bound`);
  }
  const keys = readdirSync(directory).filter(name => /^[a-f0-9]{64}\.vk\.hex$/.test(name));
  assert.deepEqual(keys.sort(), table.entries.map(entry => entry.vk).sort());
  for (const name of readdirSync(directory).filter(name => /^[a-f0-9]{64}\.json$/.test(name))) assert(table.entries.some(entry => entry.acir === name), 'stale catalogue program');
});
test('the refreshed holder requires a fresh request for an indexed SCAN plan', async t => {
  const old = JSON.parse(readFileSync(new URL('./fixtures/bgp-before-indexed-scan.json', import.meta.url)));
  const clock = old.policy.notBefore;
  t.mock.method(Date, 'now', () => clock * 1000);

  await assert.rejects(prove({ request: old, credentials: [] }), /verification key mismatch/);
  const fresh = request({ query: old.query, policy: old.policy, nonce: '0a'.repeat(32) });
  assert.equal(fresh.planHash, old.planHash);
  assert.notEqual(fresh.verificationKey, old.verificationKey);
  assert.equal(JSON.parse(host.hostCall('holder', JSON.stringify({ request: fresh, clock }))).planHash, old.planHash);
});
test('default proof preparation supplies the canonical envelope private inputs', () => {
  const fixture = JSON.parse(readFileSync(new URL('./fixtures/authenticated.json', import.meta.url)));
  const issuer = testIssuerKeygen('00'.repeat(32));
  const credential = issueFixture(issuer, canonicalGraph(''), '01'.repeat(32));
  const prepared = JSON.parse(host.hostCall('prepareProof', JSON.stringify({ request: fixture.request, credentials: [credential], clock: fixture.request.policy.notBefore })));
  const artifact = JSON.parse(readFileSync(new URL('./fixtures/layout-abi.json', import.meta.url)));
  const inputs = prepared.witnessInputs.json;
  assert(artifact.abi.parameters.some(p => p.name === 'holder_evidence' && p.visibility === 'private'));
  assert(artifact.abi.parameters.some(p => p.name === 'envelope_0' && p.visibility === 'private'));
  for (const parameter of artifact.abi.parameters) assert(Object.hasOwn(inputs, parameter.name), `missing canonical input: ${parameter.name}`);
  assert(Array.isArray(inputs.holder_evidence));
  assert.equal(typeof inputs.envelope_0, 'object');
});
test('catalogue, binary pins and keys fail closed when their stored digests differ', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-query-catalogue-test-'));
  const packageDir = fileURLToPath(new URL('../', import.meta.url));
  try {
    for (const name of ['index.js', 'package.json', 'internal']) cpSync(join(packageDir, name), join(directory, name), { recursive: true });
    for (const name of ['node_modules', 'wasm']) symlinkSync(join(packageDir, name), join(directory, name));
    cpSync(join(packageDir, 'assets'), join(directory, 'assets'), { recursive: true, filter: path => !path.endsWith('.dat') && !path.endsWith('sources.json') && (!path.endsWith('.json') || /catalogue|pins/.test(path)) });
    const api = await import(pathToFileURL(join(directory, 'index.js')));
    const fixture = JSON.parse(readFileSync(new URL('./fixtures/authenticated.json', import.meta.url)));
    const input = { query: fixture.request.query, policy: fixture.request.policy, nonce: fixture.request.nonce };
    assert.equal(api.request(input).planHash, fixture.request.planHash);
    for (const name of ['catalogue.json', 'toolchain.pins.json', `${fixture.request.planHash}.vk.hex`]) {
      const path = join(directory, 'assets', name), original = readFileSync(path);
      writeFileSync(path, name.endsWith('.hex') ? Buffer.from('00\n') : Buffer.concat([original, Buffer.from(' ')]));
      assert.throws(() => api.request(input), /digest rejected|pin data rejected/);
      writeFileSync(path, original);
    }
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
