// Research grade and not externally audited. Supplied-cell fixture only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { prove, proverFailure } from '../wasm-prover.mjs';
import { createBackend, verifyProofSync } from '../internal/runtime.mjs';
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
test('WebAssembly backend proves a Rust-generated operator witness and verifies its proof', async () => {
  const generated = JSON.parse(host.hostCall('testOperatorFixture', JSON.stringify({ query: 'ASK {}' })));
  const artifact = { program: JSON.parse(readFileSync(new URL('./fixtures/operator.json', import.meta.url))) };
  const key = Buffer.from(readFileSync(new URL('./fixtures/operator.vk.hex', import.meta.url), 'utf8').trim(), 'hex');
  const crsDirectory = new URL('../assets/crs/', import.meta.url), crsPins = JSON.parse(readFileSync(new URL('../assets/crs.pins.json', import.meta.url)));
  const options = { expectedKey: key, publicInputs: generated.publicInputs, crsDirectory, crsPins };
  const result = await prove(artifact, generated.witnessInputs, options);
  const api = await createBackend(crsDirectory, crsPins, 'verify');
  try { assert.equal(verifyProofSync(api, { ...result, verificationKey: key }), true); }
  finally { api.destroy(); }
  const wrongKey = Buffer.from(key); wrongKey[wrongKey.length - 1] ^= 1;
  await assert.rejects(prove(artifact, generated.witnessInputs, { ...options, expectedKey: wrongKey }), /key mismatch/);
});

test('reported WebAssembly allocation failures become named refusals', async () => {
  for (const error of [new WebAssembly.RuntimeError('unreachable'), new RangeError('allocation failed'), new Error('out of memory')]) assert.match(proverFailure(error).message, /allocation refused/);
  await assert.rejects(prove(null, null, { createBackend: async () => { throw new WebAssembly.RuntimeError('unreachable'); } }), /allocation refused/);
  const invalid = new Error('invalid witness'); assert.equal(proverFailure(invalid), invalid);
});
