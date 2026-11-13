// Research grade; not externally audited.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { compile_program, createFileManager } from '@noir-lang/noir_wasm';
import { Barretenberg, BackendType, UltraHonkBackend } from '@aztec/bb.js';

function artifactAgreement(program, native) {
  assert.deepEqual(program.abi, native.abi, 'native and WebAssembly ABI');
  assert.deepEqual(Buffer.from(program.bytecode, 'base64'), Buffer.from(native.bytecode, 'base64'),
    'native and WebAssembly bytecode');
}
function keyAgreement(key, native) {
  assert.deepEqual(key, native, 'native and WebAssembly verification key');
}
const knownFailure = JSON.parse(fs.readFileSync(new URL('./known-failures.json', import.meta.url), 'utf8'));
function classifyFailure(error, manifest) {
  const expected = knownFailure['xpath-vendored-sha-alias'];
  const dependency = manifest.split('\n').find(line => /^\s*sparq_xpath\s*=/.test(line));
  if (!dependency || !String(error).includes(expected.diagnostic)) return null;
  const diagnostics = error?.cause?.diagnostics;
  if (!Array.isArray(diagnostics) || diagnostics.length === 0
    || diagnostics.some(item => typeof item.file !== 'string' || !item.file.startsWith('sha256/'))) return null;
  for (const [key, value] of Object.entries({ git: expected.git, tag: expected.tag, directory: expected.directory })) {
    if (!dependency.includes(`${key} = "${value}"`)) return null;
  }
  return { status: 'known_failure', id: 'xpath-vendored-sha-alias', reason: expected.reason };
}
function controls() {
  const artifact = { abi: { parameters: [] }, bytecode: Buffer.from([1, 2]).toString('base64') };
  artifactAgreement(artifact, structuredClone(artifact));
  assert.throws(() => artifactAgreement(artifact, { ...artifact, abi: { parameters: [1] } }), /ABI/);
  assert.throws(() => artifactAgreement(artifact, { ...artifact, bytecode: 'AQM=' }), /bytecode/);
  const manifest = 'sparq_xpath = { git = "https://github.com/sparq-org/noir_XPath", tag = "v0.3.0", directory = "xpath" }';
  const diagnostic = new Error(knownFailure['xpath-vendored-sha-alias'].diagnostic,
    { cause: { diagnostics: [{ file: 'sha256/sha256.nr' }] } });
  assert.equal(classifyFailure(diagnostic, manifest).status, 'known_failure');
  assert.equal(classifyFailure('bytecode mismatch', manifest), null);
  assert.equal(classifyFailure(new Error('unrelated compiler failure', { cause: diagnostic.cause }), manifest), null);
  assert.equal(classifyFailure(diagnostic, ''), null);
  assert.equal(classifyFailure(new Error(diagnostic.message), manifest), null);
  const unrelated = new Error(diagnostic.message,
    { cause: { diagnostics: [{ file: 'sha256/sha256.nr' }, { file: 'main.nr' }] } });
  assert.equal(classifyFailure(unrelated, manifest), null);
  assert.equal(classifyFailure(diagnostic, manifest.replace('v0.3.0', 'v0.3.1')), null);
  assert.equal(classifyFailure(diagnostic, manifest.replace('noir_XPath', 'different')), null);
  keyAgreement(Buffer.from([1, 2]), Buffer.from([1, 2]));
  assert.throws(() => keyAgreement(Buffer.from([1, 2]), Buffer.from([1, 3])), /verification key/);
}
controls();
if (process.argv[2] === '--self-test') process.exit(0);

const allowKnown = process.argv.includes('--record-known-failures');
const [packageDir, nativeArtifact, nativeKey, cache, crsPath] = process.argv.slice(2).filter(arg => arg !== '--record-known-failures');
assert(packageDir && nativeArtifact && nativeKey && cache, 'compiler-agreement arguments');
assert(Number(process.versions.node.split('.')[0]) >= 24, 'Node runtime');
const native = JSON.parse(fs.readFileSync(nativeArtifact, 'utf8'));
fs.mkdirSync(cache, { recursive: true });
let program;
try {
  ({ program } = await compile_program(createFileManager(cache), path.resolve(packageDir), () => {}, () => {}));
} catch (error) {
  if (allowKnown) {
    const manifest = fs.readFileSync(path.resolve(packageDir, '../zk/noir-query/core/Nargo.toml'), 'utf8');
    const failure = classifyFailure(error, manifest);
    if (failure) {
      const record = { ...failure, program: path.basename(packageDir), bytecode: 'not_compared', key: 'not_compared' };
      fs.writeFileSync(path.join(cache, 'compiler-agreement.json'), JSON.stringify(record));
      console.error(JSON.stringify(record));
      process.exit(2);
    }
  }
  throw error;
}
artifactAgreement(program, native);
fs.writeFileSync(path.join(cache, 'compiled.json'), JSON.stringify(program));
if (nativeKey === '--compile-only') {
  console.log('Compiler bytecode and ABI agree. Key comparison is pending.');
  process.exit(0);
}
// Derive the second key through the WebAssembly backend rather than reusing the
// native key or treating ABI agreement as a key agreement verdict.
const options = { backend: BackendType.Wasm, threads: 4 };
if (crsPath) options.crsPath = crsPath;
const api = await Barretenberg.new(options);
try {
  const backend = new UltraHonkBackend(program.bytecode, api);
  const key = Buffer.from(await backend.getVerificationKey({ verifierTarget: 'noir-recursive' }));
  keyAgreement(key, fs.readFileSync(nativeKey));
} finally {
  await api.destroy();
}
fs.writeFileSync(path.join(cache, 'compiler-agreement.json'), JSON.stringify({ status: 'passing', program: path.basename(packageDir) }));
console.log('Compiler bytecode, ABI and verification key agree.');
