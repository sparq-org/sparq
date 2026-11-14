// Research grade and not externally audited. Every manifest claim is visited.
import assert from 'node:assert/strict';
import { compilerControl } from './compiler-control.mjs';
import { readFileSync, writeFileSync, mkdirSync, readdirSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createFileManager, compile_program } from '@noir-lang/noir_wasm';
import { createRequire } from 'node:module';
import { UltraHonkBackend } from '@aztec/bb.js';
import { createBackend, proofOptions, sha256, srsPointsForKey } from '../internal/runtime.mjs';
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
export function equalArtifacts(wasm, native) {
  assert.deepEqual(wasm.abi, native.abi, 'compiler ABI differs');
  assert.deepEqual(Buffer.from(wasm.bytecode, 'base64'), Buffer.from(native.bytecode, 'base64'), 'compiler bytecode differs');
}
export function equalKeys(wasm, native) { assert.deepEqual(Buffer.from(wasm), Buffer.from(native), 'compiler key differs'); }
export function knownXPathFailure(diagnostic, sources) {
  const collision = Object.entries(sources).some(([name, text]) => name.includes('noir_XPath') && name.endsWith('/xpath/Nargo.toml') && /^sha256\s*=\s*\{/m.test(text));
  return collision && diagnostic.includes('No module `constants` at path `sha256/constants.nr` or `sha256/constants/mod.nr`') && !diagnostic.split('\n').filter(line => /^    .*:[0-9]+:/.test(line)).some(line => !/^    sha256\//.test(line));
}
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
    if (existsSync(resultPath)) {
      const cached = JSON.parse(readFileSync(resultPath));
      if (cached.sourceHash === sourceHash && cached.status === 'known-failure' && cached.reason === table.compilerAgreement.xpathKnownFailure && knownXPathFailure(cached.diagnostic, sources)) { programs.set(generated.name, cached); results.push({ case: claim.id, ...cached }); continue; }
    }
    for (const [name, text] of Object.entries({ ...sources, ...generated.files })) { const path = join(work, name); mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); }
    const artifactPath = join(work, 'target', generated.name + '.json');
    let program;
    try { ({ program } = await compile_program(createFileManager(work), join(work, generated.name), () => {}, () => {})); }
    catch (error) {
      const reason = table.compilerAgreement.xpathKnownFailure;
      const diagnostic = String(error) + '\n' + JSON.stringify(error?.diagnostics || []);
      // The dependency graph is inspected as well as the diagnostic: unrelated
      // compiler errors remain fatal, including errors after upstream repairs.
      if (!reason || !knownXPathFailure(diagnostic, sources)) throw error;
      const result = { status: 'known-failure', reason, diagnostic, sourceHash }; writeFileSync(resultPath, JSON.stringify(result)); programs.set(generated.name, result); results.push({ case: claim.id, ...result }); console.log('Compiler known failure:', claim.id, diagnostic); continue;
    }
    execFileSync(nargo, ['compile'], { cwd: work, stdio: 'pipe', env: { ...process.env, RAYON_NUM_THREADS: '1' } });
    const native = JSON.parse(readFileSync(artifactPath));
    equalArtifacts(program, native);
    execFileSync(bb, ['write_vk', '-b', artifactPath, '-o', work, '-t', 'noir-recursive', '--crs_path', new URL('../assets/crs/', import.meta.url).pathname], { stdio: 'pipe', env: { ...process.env, HARDWARE_CONCURRENCY: '1' } });
    const pins = JSON.parse(readFileSync(new URL('../assets/crs.pins.json', import.meta.url)));
    const api = await createBackend(new URL('../assets/crs/', import.meta.url), pins, 'prove', srsPointsForKey(readFileSync(join(work, 'vk')), pins));
    try { equalKeys(await new UltraHonkBackend(program.bytecode, api).getVerificationKey(proofOptions), readFileSync(join(work, 'vk'))); } finally { api.destroy(); }
    const result = { status: 'agreement', sourceHash }; writeFileSync(resultPath, JSON.stringify(result)); programs.set(generated.name, result); results.push({ case: claim.id, ...result });
  }
  assert.equal(results.length, manifest.cases.filter(c => c.status === 'passing').length);
  writeFileSync(join(directory, 'compiler-agreement.json'), JSON.stringify({ authenticated: table.compilerAgreement.authenticated, cases: results }, null, 2));
  return results;
}
