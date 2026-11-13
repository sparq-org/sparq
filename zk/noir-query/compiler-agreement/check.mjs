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
function controls() {
  const artifact = { abi: { parameters: [] }, bytecode: Buffer.from([1, 2]).toString('base64') };
  artifactAgreement(artifact, structuredClone(artifact));
  assert.throws(() => artifactAgreement(artifact, { ...artifact, abi: { parameters: [1] } }), /ABI/);
  assert.throws(() => artifactAgreement(artifact, { ...artifact, bytecode: 'AQM=' }), /bytecode/);
  keyAgreement(Buffer.from([1, 2]), Buffer.from([1, 2]));
  assert.throws(() => keyAgreement(Buffer.from([1, 2]), Buffer.from([1, 3])), /verification key/);
}
controls();
if (process.argv[2] === '--self-test') process.exit(0);

const [packageDir, nativeArtifact, nativeKey, cache, crsPath] = process.argv.slice(2);
assert(packageDir && nativeArtifact && nativeKey && cache, 'compiler-agreement arguments');
assert(Number(process.versions.node.split('.')[0]) >= 24, 'Node runtime');
fs.mkdirSync(cache, { recursive: true });
const { program } = await compile_program(createFileManager(cache), path.resolve(packageDir), () => {}, () => {});
const native = JSON.parse(fs.readFileSync(nativeArtifact, 'utf8'));
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
console.log('Compiler bytecode, ABI and verification key agree.');
