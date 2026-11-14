// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { equalArtifacts, equalKeys, knownXPathFailure } from '../scripts/compiler-agreement.mjs';
test('compiler agreement rejects changed bytecode, ABI and independently derived keys', () => {
  const program = { abi: { parameters: [] }, bytecode: Buffer.from([1, 2]).toString('base64') };
  equalArtifacts(program, structuredClone(program)); equalKeys(Buffer.from([1]), Buffer.from([1]));
  assert.throws(() => equalArtifacts(program, { ...program, bytecode: Buffer.from([1, 3]).toString('base64') }), /bytecode differs/);
  assert.throws(() => equalArtifacts(program, { ...program, abi: {} }), /ABI differs/);
  assert.throws(() => equalKeys(Buffer.from([1]), Buffer.from([2])), /key differs/);
});

test('XPath known failures cannot hide a different compiler error', () => {
  const diagnostic = 'No module `constants` at path `sha256/constants.nr` or `sha256/constants/mod.nr`\n    sha256/sha256.nr:9: constants';
  const sources = { 'noir_XPath/pinned/xpath/Nargo.toml': 'sha256 = { path = "../vendor/sha256" }' };
  assert.equal(knownXPathFailure(diagnostic, sources), true);
  assert.equal(knownXPathFailure(diagnostic + '\n    main.nr:8: unrelated', sources), false);
  assert.equal(knownXPathFailure('different compiler error', sources), false);
  assert.equal(knownXPathFailure(diagnostic, {}), false);
});
