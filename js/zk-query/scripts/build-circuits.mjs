// Research grade and not externally audited. Maintainer-only native compilation.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, existsSync, rmSync, mkdtempSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { copyPinnedBinary } from '../native.mjs';
import { canonicalArtifact } from './artifact.mjs';
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
const bins = mkdtempSync(join(cache, 'binaries-'));
const nargo = await copyPinnedBinary('nargo', process.env.SPARQ_NARGO, join(bins, 'nargo'));
const bb = await copyPinnedBinary('bb', process.env.SPARQ_BB, join(bins, 'bb'));
const entries = flag('--cases') ? JSON.parse(readFileSync(flag('--cases'))) : table.entries.map(e => e.example);
for (const example of entries) {
  const policy = { bounds: example.bounds, ...(example.credentialQuadBounds === undefined ? {} : { credentialQuadBounds: example.credentialQuadBounds }), trustedIssuers: [JSON.parse(host.hostCall('testIssuerKeygen', JSON.stringify('00'.repeat(32)))).publicKey], audience: 'urn:sparq:catalogue' };
  const plan = JSON.parse(host.hostCall('plan', JSON.stringify({ query: example.query, policy })));
  let entry = table.entries.find(e => e.planHash === plan.planHash);
  const acir = `${plan.planHash}.json`, vk = `${plan.planHash}.vk.hex`;
  if (entry && existsSync(join(assets, acir)) && sha256(readFileSync(join(assets, acir))) === entry.acirSha256 && existsSync(join(assets, vk)) && sha256(Buffer.from(readFileSync(join(assets, vk), 'utf8').trim(), 'hex')) === entry.vkSha256) continue;
  const work = join(cache, plan.planHash); mkdirSync(work, { recursive: true, mode: 0o700 });
  for (const [name, text] of Object.entries({ ...sources, ...plan.files })) { const target = join(work, name); mkdirSync(dirname(target), { recursive: true }); writeFileSync(target, text); }
  const nativeArtifact = join(work, 'target', `${plan.packageName}.json`);
  if (!existsSync(nativeArtifact)) execFileSync(nargo, ['compile'], { cwd: work, stdio: 'inherit', env: { ...process.env, RAYON_NUM_THREADS: '1' } });
  if (!existsSync(join(work, 'vk'))) execFileSync(bb, ['write_vk', '-b', nativeArtifact, '-o', work, '-t', 'noir-recursive', '--crs_path', join(assets, 'crs')], { cwd: work, stdio: 'inherit', env: { ...process.env, HARDWARE_CONCURRENCY: '1' } });
  const bytes = canonicalArtifact(readFileSync(nativeArtifact));
  const key = readFileSync(join(work, 'vk'));
  const candidate = { planHash: plan.planHash, packageName: plan.packageName, acir, vk, acirSha256: sha256(bytes), vkSha256: sha256(key), wasmProving: false, example: { query: example.query, bounds: example.bounds, ...(example.credentialQuadBounds === undefined ? {} : { credentialQuadBounds: example.credentialQuadBounds }) } };
  if (!update && (!entry || candidate.acirSha256 !== entry.acirSha256 || candidate.vkSha256 !== entry.vkSha256)) throw new Error('native catalogue rebuild differs from pinned program or key');
  writeFileSync(join(assets, acir), bytes); writeFileSync(join(assets, vk), key.toString('hex') + '\n');
  if (entry) Object.assign(entry, candidate); else table.entries.push(candidate);
  if (update) { table.sourceBundleSha256 = sourceDigest; writeFileSync(tablePath, JSON.stringify(table, null, 2) + '\n'); }
}
rmSync(bins, { recursive: true, force: true });
