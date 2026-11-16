// Research grade and not externally audited. Every manifest claim is visited.
import assert from 'node:assert/strict';
import { compilerControl } from './compiler-control.mjs';
import { readFileSync, writeFileSync, mkdirSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { sha256 } from '../internal/runtime.mjs';
import { wasmProverFits } from '../wasm-prover.mjs';
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
export function equalArtifacts(wasm, native) {
  assert.deepEqual(wasm.abi, native.abi, 'compiler ABI differs');
  assert.deepEqual(Buffer.from(wasm.bytecode, 'base64'), Buffer.from(native.bytecode, 'base64'), 'compiler bytecode differs');
}
export function equalKeys(wasm, native) { assert.deepEqual(Buffer.from(wasm), Buffer.from(native), 'compiler key differs'); }
export async function agreement({ root, directory, nargo, bb }) {
  await compilerControl({ directory, nargo, bb, equalArtifacts, equalKeys });
  const table = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
  const manifest = JSON.parse(readFileSync(join(root, 'zk/noir-query/coverage.json')));
  const replaced = new Set();
  for (const name of readdirSync(join(root, 'zk/noir-query/coverage.d')).sort()) if (name.endsWith('.json')) {
    const fragment = JSON.parse(readFileSync(join(root, 'zk/noir-query/coverage.d', name)));
    for (const row of fragment.cases || []) { assert(!replaced.has(row.id), 'conflicting coverage fragment'); replaced.add(row.id); const index = manifest.cases.findIndex(c => c.id === row.id); assert(index >= 0); manifest.cases[index] = row; }
  }
  const corpus = JSON.parse(readFileSync(join(root, 'zk/sparql-evaluator/fixtures/conformance/cases.json')));
  const sources = JSON.parse(readFileSync(new URL('../assets/sources.json', import.meta.url))).files;
  const results = [], programs = new Map();
  for (const claim of manifest.cases.filter(c => c.status === 'passing')) {
    const fixture = corpus.cases.find(c => c.id === claim.id); assert(fixture, 'manifest fixture absent');
    const generated = JSON.parse(host.hostCall('testOperatorFixture', JSON.stringify({ query: fixture.query, bounds: { credentials: 1, quadsPerCredential: 16, rowsPerTable: 16 }, generateOnly: true })));
    const sourceHash = sha256(JSON.stringify({ ...sources, ...generated.files }));
    if (programs.has(generated.name)) { assert.equal(programs.get(generated.name).sourceHash, sourceHash, 'one plan hash generated different programs'); results.push({ case: claim.id, ...programs.get(generated.name) }); continue; }
    const work = join(directory, generated.name); mkdirSync(work, { recursive: true });
    const resultPath = join(work, 'agreement-result.json');
    for (const [name, text] of Object.entries({ ...sources, ...generated.files })) { const path = join(work, name); mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); }
    const artifactPath = join(work, 'target', generated.name + '.json');
    const crs = new URL('../assets/crs/', import.meta.url).pathname;
    const step = (name, fn) => { try { return fn(); } catch (error) { throw new Error(`compiler agreement ${claim.id} (${generated.name}): ${name}: ${String(error.stderr || '').trim().split('\n').slice(-3).join(' | ') || error.message}`, { cause: error }); } };
    step('native compile', () => execFileSync(nargo, ['compile'], { cwd: work, stdio: 'pipe', env: { ...process.env, RAYON_NUM_THREADS: '1' } }));
    step('native verification key', () => execFileSync(bb, ['write_vk', '-b', artifactPath, '-o', work, '-t', 'noir-recursive', '--crs_path', crs], { stdio: 'pipe', env: { ...process.env, HARDWARE_CONCURRENCY: '1' } }));
    const circuitSize = step('native gate count', () => { const out = execFileSync(bb, ['gates', '-b', artifactPath, '-s', 'ultra_honk', '-t', 'noir-recursive'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }); return JSON.parse(out.slice(out.indexOf('{'))).functions[0].circuit_size; });
    // Key agreement is claimed up to the WebAssembly prover bound; larger programs
    // (which that backend refuses) need ABI and bytecode agreement and a native key.
    const keyed = wasmProverFits(2 ** Math.ceil(Math.log2(circuitSize)));
    step(keyed ? 'WebAssembly compile and key' : 'WebAssembly compile', () => execFileSync(process.execPath, [new URL('./compiler-agreement-case.mjs', import.meta.url).pathname, work, generated.name, join(work, 'vk'), work, ...(keyed ? [] : ['compile-only'])], { stdio: 'pipe' }));
    step('bytecode and ABI agreement', () => equalArtifacts(JSON.parse(readFileSync(join(work, 'wasm-program.json'))), JSON.parse(readFileSync(artifactPath))));
    if (keyed) step('key agreement', () => equalKeys(readFileSync(join(work, 'wasm.vk')), readFileSync(join(work, 'vk'))));
    const result = { status: keyed ? 'agreement' : 'bytecode-agreement-native-key', sourceHash, circuitSize }; writeFileSync(resultPath, JSON.stringify(result)); programs.set(generated.name, result); results.push({ case: claim.id, ...result });
  }
  assert.equal(results.length, manifest.cases.filter(c => c.status === 'passing').length);
  const count = status => new Set(results.filter(r => r.status === status).map(r => r.sourceHash)).size;
  const summary = { programs: { agreement: count('agreement'), bytecodeAgreementNativeKey: count('bytecode-agreement-native-key') }, keyAgreementBound: 'dyadic circuit size at most the WebAssembly prover bound (2^21)' };
  writeFileSync(join(directory, 'compiler-agreement.json'), JSON.stringify({ authenticated: table.compilerAgreement.authenticated, summary, cases: results }, null, 2));
  return results;
}
