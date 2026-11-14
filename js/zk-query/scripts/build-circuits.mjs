// Research grade and not externally audited. Maintainer-only native compilation.
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, existsSync, rmSync, mkdtempSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { copyPinnedBinary } from '../native.mjs';
import { canonicalArtifact } from './artifact.mjs';
import { stageSource } from './cache.mjs';
import { snapshotSources } from './sources.mjs';
import { sha256 } from '../internal/runtime.mjs';
const packageDir = fileURLToPath(new URL('../', import.meta.url));
const assets = join(packageDir, 'assets'), tablePath = join(assets, 'catalogue.json');
const table = JSON.parse(readFileSync(tablePath));
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
const flag = name => { const i = process.argv.indexOf(name); return i < 0 ? undefined : process.argv[i + 1]; };
const update = process.argv.includes('--write-pins');
const sourceDigest = snapshotSources();
if (!update && table.sourceBundleSha256 !== sourceDigest) throw new Error('source bundle pin differs');
const sources = JSON.parse(readFileSync(join(assets, 'sources.json'))).files;
const cache = resolve(flag('--cache') || join(tmpdir(), 'sparq-query-catalogue'));
mkdirSync(cache, { recursive: true, mode: 0o700 });
const concurrency = Number(process.env.CARGO_BUILD_JOBS || '1');
if (!Number.isSafeInteger(concurrency) || concurrency < 1) throw new Error('catalogue build concurrency rejected');
const bins = mkdtempSync(join(cache, 'binaries-'));
try {
const nargo = await copyPinnedBinary('nargo', process.env.SPARQ_NARGO, join(bins, 'nargo'));
const bb = await copyPinnedBinary('bb', process.env.SPARQ_BB, join(bins, 'bb'));
const entries = flag('--cases') ? JSON.parse(readFileSync(flag('--cases'))) : table.entries.map(e => e.example);
const plans = new Map();
for (const example of entries) {
  const policy = { bounds: example.bounds, ...(example.credentialQuadBounds === undefined ? {} : { credentialQuadBounds: example.credentialQuadBounds }), trustedIssuers: [JSON.parse(host.hostCall('testIssuerKeygen', JSON.stringify('00'.repeat(32)))).publicKey], audience: 'urn:sparq:catalogue' };
  const plan = JSON.parse(host.hostCall('plan', JSON.stringify({ query: example.query, policy })));
  if (plans.has(plan.planHash) && JSON.stringify(plans.get(plan.planHash).plan.files) !== JSON.stringify(plan.files)) throw new Error('catalogue plan generated different sources');
  plans.set(plan.planHash, { plan, example });
}
function run(binary, args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, { ...options, stdio: 'inherit', shell: false });
    child.once('error', reject);
    child.once('close', code => code === 0 ? resolve() : reject(new Error('native catalogue command failed')));
  });
}
async function build({ plan, example }) {
  let entry = table.entries.find(e => e.planHash === plan.planHash);
  if (!entry && !update) throw new Error('missing pinned catalogue plan; use --write-pins');
  const acir = `${plan.planHash}.json`, vk = `${plan.planHash}.vk.hex`;
  if (entry && existsSync(join(assets, acir)) && sha256(readFileSync(join(assets, acir))) === entry.acirSha256 && existsSync(join(assets, vk)) && sha256(Buffer.from(readFileSync(join(assets, vk), 'utf8').trim(), 'hex')) === entry.vkSha256) return;
  const work = join(cache, plan.planHash); mkdirSync(work, { recursive: true, mode: 0o700 });
  for (const [name, text] of Object.entries({ ...sources, ...plan.files })) stageSource(work, name, text);
  const nativeArtifact = join(work, 'target', `${plan.packageName}.json`);
  if (!existsSync(nativeArtifact)) await run(nargo, ['compile'], { cwd: work, env: { ...process.env, RAYON_NUM_THREADS: '1' } });
  const bytes = canonicalArtifact(readFileSync(nativeArtifact));
  if (!update && sha256(bytes) !== entry.acirSha256) throw new Error('cached catalogue program differs from its pin');
  if (!existsSync(join(work, 'vk'))) await run(bb, ['write_vk', '-b', nativeArtifact, '-o', work, '-t', 'noir-recursive', '--crs_path', join(assets, 'crs')], { cwd: work, env: { ...process.env, HARDWARE_CONCURRENCY: '1' } });
  const key = readFileSync(join(work, 'vk'));
  const acirSha256 = sha256(bytes), vkSha256 = sha256(key);
  const candidate = { planHash: plan.planHash, packageName: plan.packageName, acir, vk, acirSha256, vkSha256, wasmProving: Boolean(entry?.wasmProving && entry.acirSha256 === acirSha256 && entry.vkSha256 === vkSha256), example: { query: example.query, bounds: example.bounds, ...(example.credentialQuadBounds === undefined ? {} : { credentialQuadBounds: example.credentialQuadBounds }) } };
  if (!update && (!entry || candidate.acirSha256 !== entry.acirSha256 || candidate.vkSha256 !== entry.vkSha256)) throw new Error('native catalogue rebuild differs from pinned program or key');
  writeFileSync(join(assets, acir), bytes); writeFileSync(join(assets, vk), key.toString('hex') + '\n');
  if (entry) Object.assign(entry, candidate); else table.entries.push(candidate);
  if (update) { table.sourceBundleSha256 = sourceDigest; writeFileSync(tablePath, JSON.stringify(table, null, 2) + '\n'); }
}
const queue = plans.values();
  const results = await Promise.allSettled(Array.from({ length: Math.min(concurrency, plans.size) }, async () => {
    for (let next = queue.next(); !next.done; next = queue.next()) await build(next.value);
  }));
  const failed = results.filter(result => result.status === 'rejected');
  if (failed.length) throw new AggregateError(failed.map(result => result.reason), 'native catalogue build refused');
} finally { rmSync(bins, { recursive: true, force: true }); }
