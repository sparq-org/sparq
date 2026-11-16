// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { equalArtifacts, equalKeys } from '../scripts/compiler-agreement.mjs';
import { wasmProverFits, keyDyadicSize, WASM_PROVER_DYADIC_LIMIT, prove } from '../wasm-prover.mjs';
test('compiler agreement rejects changed bytecode, ABI and independently derived keys', () => {
  const program = { abi: { parameters: [] }, bytecode: Buffer.from([1, 2]).toString('base64') };
  equalArtifacts(program, structuredClone(program)); equalKeys(Buffer.from([1]), Buffer.from([1]));
  assert.throws(() => equalArtifacts(program, { ...program, bytecode: Buffer.from([1, 3]).toString('base64') }), /bytecode differs/);
  assert.throws(() => equalArtifacts(program, { ...program, abi: {} }), /ABI differs/);
  assert.throws(() => equalKeys(Buffer.from([1]), Buffer.from([2])), /key differs/);
});

test('the WebAssembly prover bound is a dyadic size, and larger programs refuse up front', async () => {
  assert.equal(WASM_PROVER_DYADIC_LIMIT, 2 ** 21);
  assert.equal(wasmProverFits(WASM_PROVER_DYADIC_LIMIT), true);
  assert.equal(wasmProverFits(WASM_PROVER_DYADIC_LIMIT * 2), false);
  assert.equal(wasmProverFits(0), false);
  const header = log => Buffer.concat([Buffer.alloc(31), Buffer.from([log])]);
  assert.equal(keyDyadicSize(header(21)), WASM_PROVER_DYADIC_LIMIT);
  await assert.rejects(prove({}, {}, { expectedKey: header(22) }), /exceeds the WebAssembly prover memory bound/);
});
