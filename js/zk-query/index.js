// Research grade and not externally audited.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { createBackend, sha256, verifyProofSync } from './internal/runtime.mjs';

const host = createRequire(import.meta.url)('./wasm/sparq_zk_query_wasm.cjs');
const assets = new URL('./assets/', import.meta.url);
const call = (operation, input) => {
  try { return JSON.parse(host.hostCall(operation, JSON.stringify(input))); }
  catch (error) { throw error instanceof Error ? error : new Error(String(error)); }
};
const clock = () => Math.floor(Date.now() / 1000);
const snapshot = value => JSON.parse(JSON.stringify(value));
const refused = reason => ({ verified: false, released: [], reason: String(reason) });

function catalogue() {
  const bytes = readFileSync(new URL('catalogue.json', assets));
  if (sha256(bytes) !== host.catalogueHash()) throw new Error('catalogue digest rejected');
  const pins = readFileSync(new URL('toolchain.pins.json', assets));
  if (sha256(pins) !== host.toolchainPinsHash()) throw new Error('toolchain pin data rejected');
  const table = JSON.parse(bytes);
  const entries = new Map();
  for (const entry of table.entries) {
    if (!/^[a-f0-9]{64}$/.test(entry.planHash) || entry.packageName !== `query_${entry.planHash}` || entries.has(entry.planHash) ||
        entry.acir !== `${entry.planHash}.json` || entry.vk !== `${entry.planHash}.vk.hex`) throw new Error('catalogue entry rejected');
    const text = readFileSync(new URL(entry.vk, assets), 'utf8').trim();
    if (!/^(?:[a-f0-9]{2})+$/.test(text)) throw new Error('catalogue key encoding rejected');
    const key = Buffer.from(text, 'hex');
    if (sha256(key) !== entry.vkSha256) throw new Error('catalogue key digest rejected');
    entries.set(entry.planHash, { ...entry, key });
  }
  return { table, entries };
}
function member(hash, known = catalogue()) {
  const entry = known.entries.get(hash);
  if (!entry) throw new Error('missing catalogue plan');
  return entry;
}
function crsPins(table) {
  const bytes = readFileSync(new URL('crs.pins.json', assets));
  if (sha256(bytes) !== table.crsPinsSha256) throw new Error('CRS pin data rejected');
  return JSON.parse(bytes);
}
function artifact(entry, request) {
  const bytes = readFileSync(new URL(entry.acir, assets));
  if (sha256(bytes) !== entry.acirSha256) throw new Error('catalogue program digest rejected');
  const program = JSON.parse(bytes);
  call('auditArtifact', { request, artifact: program });
  return { program, bytes };
}
function sources(table, queryPlan) {
  const bytes = readFileSync(new URL('sources.json', assets));
  if (sha256(bytes) !== table.sourceBundleSha256) throw new Error('catalogue source digest rejected');
  const files = { ...JSON.parse(bytes).files, ...queryPlan.files };
  return files;
}

export function issuerKeygen() {
  if (arguments.length) throw new Error('issuerKeygen does not accept a seed');
  return call('issuerKeygen', {});
}
export function testIssuerKeygen(seedHex) { return call('testIssuerKeygen', seedHex); }
export function issue(input) { return call('issue', input); }
export function testIssue(input) { return call('testIssue', input); }
export function request({ query, policy, nonce }) {
  const plan = call('plan', { query, policy });
  const entry = member(plan.planHash);
  return call('request', { query, policy, nonce, verificationKey: entry.key.toString('hex'), clock: clock() });
}
export async function prove({ request, credentials, backend = 'native' }) {
  request = snapshot(request);
  const known = catalogue();
  const checked = call('holder', { request, clock: clock() });
  const entry = member(checked.planHash, known);
  const options = typeof backend === 'string' ? { type: backend } : snapshot(backend);
  if (!options || !['native', 'wasm'].includes(options.type) || Object.keys(options).some(k => !['type', 'nargoPath', 'bbPath', 'presentationSaltHex'].includes(k))) throw new Error('prover backend rejected');
  if (options.type === 'wasm' && !entry.wasmProving) throw new Error('catalogue plan requires the native prover backend');
  if (options.type === 'native') { const native = await import('./native.mjs'); await native.preflight(options); }
  const program = artifact(entry, request);
  const plan = call('plan', { query: request.query, policy: request.policy });
  const prepared = call('prepareProof', { request, credentials, clock: clock(), ...(options.presentationSaltHex === undefined ? {} : { presentationSaltHex: options.presentationSaltHex }) });
  const pins = crsPins(known.table), crsDirectory = new URL('crs/', assets);
  const common = { expectedKey: entry.key, publicInputs: prepared.publicInputs, crsDirectory, crsPins: pins };
  let result;
  if (options.type === 'native') {
    const { checkPinnedFiles } = await import('./internal/runtime.mjs');
    await checkPinnedFiles(pins, crsDirectory);
    const native = await import('./native.mjs');
    result = await native.prove(program, prepared.witnessInputs, { ...common, ...options,
      packageName: entry.packageName, sources: sources(known.table, plan), crsPath: fileURLToPath(crsDirectory), maxPresentationBytes: request.policy.maxPresentationBytes ?? (1 << 24) });
  } else {
    const wasm = await import('./wasm-prover.mjs');
    result = await wasm.prove(program, prepared.witnessInputs, common);
  }
  return call('bindProof', { request, presentation: { ...prepared.presentationSkeleton, proof: Buffer.from(result.proof).toString('hex') }, publicInputs: result.publicInputs, clock: clock() });
}
export async function verify({ request, presentation, seenNonces }) {
  if (!seenNonces || (typeof seenNonces.consume !== 'function' && !(seenNonces instanceof Set))) {
    return refused('nonce store is required');
  }
  let api;
  try {
    const input = JSON.stringify({ request: snapshot(request), presentation: snapshot(presentation) });
    const known = catalogue();
    const pins = crsPins(known.table);
    api = await createBackend(new URL('crs/', assets), pins, 'verify');
    const proof = text => {
      const statement = JSON.parse(text);
      return verifyProofSync(api, { proof: Buffer.from(statement.proof, 'hex'), publicInputs: statement.publicInputs, verificationKey: Buffer.from(statement.verificationKey, 'hex') });
    };
    const consume = nonce => {
      if (typeof seenNonces.consume === 'function') return seenNonces.consume(nonce);
      if (seenNonces.has(nonce)) return 'alreadyConsumed';
      seenNonces.add(nonce);
      return 'fresh';
    };
    return JSON.parse(host.verifyWith(input, proof, consume, clock));
  } catch (error) { return refused(error); }
  finally { api?.destroy(); }
}
