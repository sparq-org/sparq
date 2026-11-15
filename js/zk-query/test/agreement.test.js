// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { equalArtifacts, equalKeys } from '../scripts/compiler-agreement.mjs';
test('compiler agreement rejects changed bytecode, ABI and independently derived keys', () => {
  const program = { abi: { parameters: [] }, bytecode: Buffer.from([1, 2]).toString('base64') };
  equalArtifacts(program, structuredClone(program)); equalKeys(Buffer.from([1]), Buffer.from([1]));
  assert.throws(() => equalArtifacts(program, { ...program, bytecode: Buffer.from([1, 3]).toString('base64') }), /bytecode differs/);
  assert.throws(() => equalArtifacts(program, { ...program, abi: {} }), /ABI differs/);
  assert.throws(() => equalKeys(Buffer.from([1]), Buffer.from([2])), /key differs/);
});

