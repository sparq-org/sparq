// Research grade and not externally audited. Maintainer-only native compilation.
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, existsSync, rmSync, mkdtempSync, lstatSync, statSync, renameSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { copyPinnedBinary } from '../native.mjs';
import { canonicalArtifact } from './artifact.mjs';
import { catalogueBuildDigest, stageSource } from './cache.mjs';
import { snapshotSources } from './sources.mjs';
import { publishCatalogue, recoverCatalogue, withCatalogueLock } from './catalogue-lock.mjs';
import { sha256 } from '../internal/runtime.mjs';
import { examplePolicy, exampleRecord } from './catalogue-example.mjs';
const packageDir = fileURLToPath(new URL('../', import.meta.url));
const assets = join(packageDir, 'assets'), tablePath = join(assets, 'catalogue.json');
recoverCatalogue(assets);
const originalCatalogue = readFileSync(tablePath);
const table = JSON.parse(originalCatalogue);
const originalContents = JSON.stringify(table);
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
const flag = name => { const i = process.argv.indexOf(name); return i < 0 ? undefined : process.argv[i + 1]; };
const update = process.argv.includes('--write-pins');
const sourceBundle = snapshotSources();
const sourceDigest = sha256(sourceBundle);
const toolchainDigest = sha256(readFileSync(join(assets, 'toolchain.pins.json')));
if (!update && table.sourceBundleSha256 !== sourceDigest) throw new Error('source bundle pin differs');
const sources = JSON.parse(sourceBundle).files;
const cache = resolve(flag('--cache') || join(tmpdir(), 'sparq-query-catalogue'));
const concurrency = Number(process.env.CARGO_BUILD_JOBS || '1');
if (!Number.isSafeInteger(concurrency) || concurrency < 1) throw new Error('catalogue build concurrency rejected');
const entries = flag('--cases') ? JSON.parse(readFileSync(flag('--cases'))) : table.entries.map(e => e.example);
const issuer = JSON.parse(host.hostCall('testIssuerKeygen', JSON.stringify('00'.repeat(32))));
function planExample(example) {
  const policy = examplePolicy(example, issuer, 'urn:sparq:catalogue');
  return JSON.parse(host.hostCall('plan', JSON.stringify({ query: example.query, policy })));
}
const buildDigest = plan => catalogueBuildDigest(plan.files, sourceDigest, toolchainDigest);
const plans = new Map();
for (const example of entries) {
  const plan = planExample(example);
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
if (update) {
  const omitted = table.entries.filter(entry => !plans.has(entry.planHash));
  if (table.sourceBundleSha256 !== sourceDigest && omitted.length) {
    throw new Error('source bundle changed; --write-pins must rebuild every catalogue plan');
  }
  for (const entry of omitted) {
    const plan = planExample(entry.example);
    if (plan.planHash !== entry.planHash || buildDigest(plan) !== entry.buildInputSha256) {
      throw new Error(`catalogue build inputs differ for omitted example ${entry.example.name ?? entry.planHash}; include every stale plan in --write-pins`);
    }
  }
}

async function build({ plan, example }, staging, nargo, bb) {
  const entry = table.entries.find(e => e.planHash === plan.planHash);
  if (!entry && !update) throw new Error('missing pinned catalogue plan; use --write-pins');
  const buildInputSha256 = buildDigest(plan);
  if (!update && entry.buildInputSha256 !== buildInputSha256) throw new Error('catalogue build inputs differ from their pin');
  const acir = `${plan.planHash}.json`, vk = `${plan.planHash}.vk.hex`;
  if (entry?.buildInputSha256 === buildInputSha256 && existsSync(join(assets, acir)) && sha256(readFileSync(join(assets, acir))) === entry.acirSha256 && existsSync(join(assets, vk)) && sha256(Buffer.from(readFileSync(join(assets, vk), 'utf8').trim(), 'hex')) === entry.vkSha256) return;
  const work = join(cache, `complete-${buildInputSha256}`);
  const artifactPath = root => join(root, 'target', `${plan.packageName}.json`);
  const readProgram = path => {
    const bytes = canonicalArtifact(readFileSync(path));
    if (!update && sha256(bytes) !== entry.acirSha256) throw new Error('cached catalogue program differs from its pin');
    return bytes;
  };
  const cached = lstatSync(work, { throwIfNoEntry: false });
  if (cached?.isSymbolicLink()) throw new Error('catalogue cache symbolic link rejected');
  if (!cached) {
    const scratch = mkdtempSync(join(cache, '.build-'));
    try {
      for (const [name, text] of Object.entries({ ...sources, ...plan.files })) stageSource(scratch, name, text);
      const nativeArtifact = artifactPath(scratch);
      await run(nargo, ['compile'], { cwd: scratch, env: { ...process.env, RAYON_NUM_THREADS: '1' } });
      readProgram(nativeArtifact);
      await run(bb, ['write_vk', '-b', nativeArtifact, '-o', scratch, '-t', 'noir-recursive', '--crs_path', join(assets, 'crs')], { cwd: scratch, env: { ...process.env, HARDWARE_CONCURRENCY: '1' } });
      if (!statSync(join(scratch, 'vk')).size) throw new Error('native catalogue build produced an empty key');
      try {
        renameSync(scratch, work);
      } catch (error) {
        // A concurrent build may already have published the same cache entry.
        if (error.code !== 'EEXIST' && error.code !== 'ENOTEMPTY') throw error;
      }
    } finally { rmSync(scratch, { recursive: true, force: true }); }
  }
  const bytes = readProgram(artifactPath(work));
  const key = readFileSync(join(work, 'vk'));
  const acirSha256 = sha256(bytes), vkSha256 = sha256(key);
  const candidate = { planHash: plan.planHash, packageName: plan.packageName, acir, vk, acirSha256, vkSha256, buildInputSha256, wasmProving: Boolean(entry?.wasmProving && entry.acirSha256 === acirSha256 && entry.vkSha256 === vkSha256), example: exampleRecord(example) };
  if (!update && (!entry || candidate.acirSha256 !== entry.acirSha256 || candidate.vkSha256 !== entry.vkSha256)) throw new Error('native catalogue rebuild differs from pinned program or key');
  writeFileSync(join(staging, acir), bytes);
  writeFileSync(join(staging, vk), key.toString('hex') + '\n');
  return candidate;
}

mkdirSync(cache, { recursive: true, mode: 0o700 });
const staging = mkdtempSync(join(packageDir, '.catalogue-stage-'));
try {
  const bins = mkdtempSync(join(cache, 'binaries-'));
  const candidates = new Map();
  try {
    const nargo = await copyPinnedBinary('nargo', process.env.SPARQ_NARGO, join(bins, 'nargo'));
    const bb = await copyPinnedBinary('bb', process.env.SPARQ_BB, join(bins, 'bb'));
    const queue = plans.values();
    const results = await Promise.allSettled(Array.from({ length: Math.min(concurrency, plans.size) }, async () => {
      for (let next = queue.next(); !next.done; next = queue.next()) {
        const candidate = await build(next.value, staging, nargo, bb);
        if (candidate) candidates.set(candidate.planHash, candidate);
      }
    }));
    const failed = results.filter(result => result.status === 'rejected');
    if (failed.length) throw new AggregateError(failed.map(result => result.reason), 'native catalogue build refused');
  } finally { rmSync(bins, { recursive: true, force: true }); }

  const names = [];
  for (const planHash of plans.keys()) {
    const candidate = candidates.get(planHash);
    if (!candidate) continue;
    names.push(candidate.acir, candidate.vk);
    if (update) {
      const entry = table.entries.find(e => e.planHash === planHash);
      if (entry) Object.assign(entry, candidate); else table.entries.push(candidate);
    }
  }
  if (update) {
    table.sourceBundleSha256 = sourceDigest;
    if (JSON.stringify(table) !== originalContents) {
      writeFileSync(join(staging, 'catalogue.json'), JSON.stringify(table, null, 2) + '\n');
      names.push('catalogue.json');
    }
  }
  withCatalogueLock(assets, () => {
    if (!originalCatalogue.equals(readFileSync(tablePath))) {
      throw new Error('catalogue changed during build; retry against the current catalogue');
    }
    const bundlePath = join(assets, 'sources.json');
    if (!existsSync(bundlePath) || !sourceBundle.equals(readFileSync(bundlePath))) {
      writeFileSync(join(staging, 'sources.json'), sourceBundle);
      names.unshift('sources.json');
    }
    if (names.length) publishCatalogue(assets, staging, names);
  });
} finally { rmSync(staging, { recursive: true, force: true }); }
