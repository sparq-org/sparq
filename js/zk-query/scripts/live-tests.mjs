// Research grade and not externally audited. Invoked only through the live gate.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { prove as wasmProve } from '../wasm-prover.mjs';
import { testIssuerKeygen, testIssue, request, prove, verify } from '../index.js';
import { sha256 } from '../internal/runtime.mjs';
import { agreement } from './compiler-agreement.mjs';
import '../test/binary.live.js';
test('split prover, seeded issuance and compiler agreement live contracts', async () => {
const root = fileURLToPath(new URL('../../../', import.meta.url));
const directory = process.env.SPARQ_QUERY_TEST_DIR; assert(directory, 'evidence directory required');
mkdirSync(directory, { recursive: true, mode: 0o700 });
const nargo = process.env.SPARQ_NARGO, bb = process.env.SPARQ_BB;
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
const table = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
const cases = JSON.parse(readFileSync(new URL('../test/fixtures/n0.json', import.meta.url)));
const issuer = testIssuerKeygen('00'.repeat(32));
const profiles = [{ name: 'authenticated-smoke', query: 'ASK {}', bounds: { credentials: 1, quadsPerCredential: 1, rowsPerTable: 1 }, sources: ['<a:> <b:> <c:> .'] }, ...cases];
function rustSources(path) {
  return readdirSync(path, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap(entry => entry.isDirectory() ? rustSources(join(path, entry.name)) : entry.name.endsWith('.rs') ? [readFileSync(join(path, entry.name))] : []);
}
const runtimeHash = sha256(Buffer.concat([
  ...['../index.js', '../native.mjs', '../wasm-prover.mjs', '../internal/runtime.mjs', '../package-lock.json'].map(name => readFileSync(new URL(name, import.meta.url))),
  ...['sparq-zk-query-wasm', 'sparq-zk-query', 'sparq-merkle-suite'].flatMap(name => rustSources(join(root, 'crates', name, 'src'))),
  readFileSync(join(root, 'Cargo.lock')),
]));
const pending = [];
for (const [index, sample] of profiles.entries()) {
  const path = join(directory, sample.name + '.json');
  const profile = table.entries.find(entry => entry.planHash === (sample.planHash || table.entries[0].planHash));
  const entryHash = profile ? sha256(JSON.stringify([profile.acirSha256, profile.vkSha256, table.sourceBundleSha256])) : '';
  if (existsSync(path)) { const cached = JSON.parse(readFileSync(path)); if (cached.verified && cached.runtimeHash === runtimeHash && cached.entryHash === entryHash) { const checked = await verify({ request: cached.request, presentation: cached.presentation, seenNonces: new Set() }); assert.equal(checked.verified, true, checked.reason); continue; } }
  if (sample.planHash && !table.entries.some(e => e.planHash === sample.planHash)) {
    assert.throws(() => request({ query: sample.query, policy: { bounds: sample.bounds, trustedIssuers: [issuer.publicKey], audience: 'urn:sparq:test:missing' }, nonce: '09'.repeat(32) }), /missing catalogue plan/);
    pending.push({ case: sample.name, status: 'not-yet-catalogued', reason: table.notYetCatalogued.find(c => c.case === sample.name)?.reason });
    continue;
  }
  const credentials = sample.sources.map((graphNQuads, slot) => testIssue({ issuer, graphNQuads, seedHex: (index + slot + 1).toString(16).padStart(2, '0').repeat(32) }));
  let stored = request({ query: sample.query, policy: { bounds: sample.bounds, trustedIssuers: [issuer.publicKey], audience: 'urn:sparq:test:verifier', notBefore: 0, notAfter: 32503680000 }, nonce: (index + 30).toString(16).padStart(2, '0').repeat(32) });
  const backend = { type: 'native' };
  if (sample.name === 'authenticated-smoke') {
    backend.presentationSaltHex = '03'.repeat(32);
    const prepared = JSON.parse(host.hostCall('prepareProof', JSON.stringify({ request: stored, credentials, clock: Math.floor(Date.now() / 1000), presentationSaltHex: backend.presentationSaltHex })));
    stored = request({ query: stored.query, policy: { ...stored.policy, inputCommitment: prepared.presentationSkeleton.inputCommitment }, nonce: stored.nonce });
  }
  const start = performance.now();
  const knownFailure = table.nativeKnownFailures?.find(failure => failure.case === sample.name);
  if (knownFailure) {
    await assert.rejects(prove({ request: stored, credentials, backend }), error => error.message === knownFailure.expectedRefusal);
    writeFileSync(path, JSON.stringify({ runtimeHash, status: 'known-failure', ...knownFailure }));
    console.log('Named native witness failure:', sample.name, knownFailure.reason);
    continue;
  }
  const result = await prove({ request: stored, credentials, backend });
  const proofMs = performance.now() - start;
  const verifyStart = performance.now();
  const checked = await verify({ request: stored, presentation: result.presentation, seenNonces: new Set() });
  assert.equal(checked.verified, true, checked.reason); assert.deepEqual(checked.released, result.result);
  assert.equal(checked.report.complete, true); assert.deepEqual(checked.report.subject, []);
  writeFileSync(path, JSON.stringify({ verified: true, runtimeHash, entryHash, request: stored, ...result, report: checked.report, nativeOperationMs: proofMs, nodeVerifyMs: performance.now() - verifyStart }, null, 2));
  console.log('Authenticated native backend and Node verification passed:', sample.name);
}
writeFileSync(join(directory, 'not-yet-catalogued.json'), JSON.stringify(pending, null, 2));
for (const [index, sample] of profiles.entries()) {
  const profile = table.entries.find(entry => entry.planHash === (sample.planHash || table.entries[0].planHash));
  if (!profile) continue;
  const knownFailure = table.nativeKnownFailures?.find(failure => failure.case === sample.name);
  const nativeResult = JSON.parse(readFileSync(join(directory, sample.name + '.json')));
  if (knownFailure) nativeResult.request = request({ query: sample.query, policy: { bounds: sample.bounds, trustedIssuers: [issuer.publicKey], audience: 'urn:sparq:test:verifier', notBefore: 0, notAfter: 32503680000 }, nonce: (index + 30).toString(16).padStart(2, '0').repeat(32) });
  const credentials = sample.sources.map((graphNQuads, slot) => testIssue({ issuer, graphNQuads, seedHex: (index + slot + 1).toString(16).padStart(2, '0').repeat(32) }));
  if (!profile.wasmProving) {
    await assert.rejects(prove({ request: nativeResult.request, credentials, backend: 'wasm' }), /requires the native prover backend/);
    continue;
  }
  const wasmPath = join(directory, sample.name + '-wasm.json');
  const cached = existsSync(wasmPath) ? JSON.parse(readFileSync(wasmPath)) : undefined;
  const entryHash = sha256(JSON.stringify([profile.acirSha256, profile.vkSha256, table.sourceBundleSha256]));
  if (!cached || cached.runtimeHash !== runtimeHash || cached.entryHash !== entryHash) {
    const input = join(directory, sample.name + '-wasm-input.json');
    writeFileSync(input, JSON.stringify({ sample, index, request: nativeResult.request, runtimeHash, entryHash, knownFailure }), { mode: 0o600 });
    execFileSync(process.execPath, [new URL('./wasm-live-case.mjs', import.meta.url).pathname, input, wasmPath], { stdio: 'inherit' });
  }
  const result = JSON.parse(readFileSync(wasmPath));
  if (knownFailure) { assert.equal(result.status, 'known-failure'); continue; }
  assert.equal(result.verified, true);
  assert.deepEqual(result.report, nativeResult.report);
  assert.equal((await verify({ request: result.request, presentation: result.presentation, seenNonces: new Set() })).verified, true);
  console.log('Opt-in authenticated WebAssembly contracts passed:', sample.name);
}
// Independently call the suite in native Rust, rather than compare two adapters.
const vector = JSON.parse(readFileSync(new URL('../test/fixtures/issuance.json', import.meta.url)));
const reference = execFileSync(process.env.SPARQ_ISSUANCE_REFERENCE, [], { input: JSON.stringify(vector), encoding: 'utf8' });
assert.deepEqual(testIssue({ issuer: testIssuerKeygen(vector.issuerSeedHex), graphNQuads: vector.graphNQuads, status: { proofConfigNQuads: vector.proofConfigNQuads }, seedHex: vector.seedHex }), JSON.parse(reference));
console.log('Native suite and Node seeded issuance agree byte for byte.');
const work = join(directory, 'supplied-cell'); mkdirSync(work, { recursive: true });
const generated = JSON.parse(host.hostCall('testOperatorFixture', JSON.stringify({ query: 'ASK {}' })));
const sources = JSON.parse(readFileSync(new URL('../assets/sources.json', import.meta.url))).files;
for (const [name, text] of Object.entries({ ...sources, ...generated.files })) { const target = join(work, name); mkdirSync(dirname(target), { recursive: true }); writeFileSync(target, text); }
const artifactPath = join(work, 'target', generated.name + '.json');
execFileSync(nargo, ['compile'], { cwd: work, stdio: 'pipe', env: { ...process.env, RAYON_NUM_THREADS: '1' } });
const artifact = { program: JSON.parse(readFileSync(artifactPath)), bytes: readFileSync(artifactPath) };
execFileSync(bb, ['write_vk', '-b', artifactPath, '-o', work, '-t', 'noir-recursive', '--crs_path', new URL('../assets/crs/', import.meta.url).pathname], { stdio: 'pipe', env: { ...process.env, HARDWARE_CONCURRENCY: '1' } });
const pins = JSON.parse(readFileSync(new URL('../assets/crs.pins.json', import.meta.url)));
const result = await wasmProve(artifact, generated.witnessInputs, { expectedKey: readFileSync(join(work, 'vk')), publicInputs: generated.publicInputs, crsDirectory: new URL('../assets/crs/', import.meta.url), crsPins: pins });
writeFileSync(join(work, 'proof'), result.proof);
writeFileSync(join(work, 'public_inputs'), Buffer.concat(result.publicInputs.map(word => Buffer.from(BigInt(word).toString(16).padStart(64, '0'), 'hex'))));
execFileSync(bb, ['verify', '-p', join(work, 'proof'), '-k', join(work, 'vk'), '-i', join(work, 'public_inputs'), '-t', 'noir-recursive', '--crs_path', new URL('../assets/crs/', import.meta.url).pathname], { stdio: 'pipe' });
writeFileSync(join(directory, 'wasm-to-native.json'), JSON.stringify({ verified: true, publicInputs: result.publicInputs }));
console.log('WebAssembly operator proof verifies natively.');
await agreement({ root, directory, nargo, bb });

});
