// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { sha256 } from '../internal/runtime.mjs';

const packageDir = fileURLToPath(new URL('../', import.meta.url));
const cases = [
  { name: 'subject', query: 'SELECT ?s WHERE { ?s <urn:p> <urn:o> }', bounds: {} },
  { name: 'object', query: 'SELECT ?o WHERE { <urn:s> <urn:p> ?o }', bounds: {} },
];
const previousAbi = { parameters: [{ name: 'dataset' }] };
const indexedAbi = { parameters: [{ name: 'dataset' }, { name: 'target_0' }, { name: 'source_0' }] };
const planHash = example => sha256(example.query);

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'sparq-query-builder-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const directory = join(root, 'js', 'zk-query');
  const assets = join(directory, 'assets');
  const host = join(directory, 'wasm');
  const core = join(root, 'zk', 'noir-query', 'core');
  for (const path of [assets, host, join(core, 'src')]) mkdirSync(path, { recursive: true });
  for (const name of ['index.js', 'native.mjs', 'package.json', 'internal', 'scripts']) {
    cpSync(join(packageDir, name), join(directory, name), { recursive: true });
  }
  symlinkSync(join(packageDir, 'node_modules'), join(directory, 'node_modules'));
  cpSync(join(packageDir, 'test/fixtures/catalogue-build-host.cjs'), join(host, 'sparq_zk_query_wasm.cjs'));
  const tool = join(packageDir, 'test/fixtures/catalogue-build-tool.mjs');
  const sha = sha256(readFileSync(tool));
  writeFileSync(join(assets, 'toolchain.pins.json'), JSON.stringify({
    platform: process.platform, arch: process.arch,
    binaries: { nargo: { version: 'nargo', sha256: sha }, bb: { version: 'bb', sha256: sha } },
  }));
  writeFileSync(join(assets, 'catalogue.json'), JSON.stringify({ entries: [] }));
  writeFileSync(join(core, 'Nargo.toml'), '[package]\nname = "query_core"\n');
  const source = text => writeFileSync(join(core, 'src', 'lib.nr'), text);
  const abi = (value, example) => writeFileSync(
    join(host, example ? `scan-abi-${planHash(example)}` : 'scan-abi'),
    JSON.stringify(value),
  );
  source('previous scan library');
  abi(previousAbi);

  let callsPath;
  const calls = () => readFileSync(callsPath, 'utf8').trim().split('\n').filter(Boolean);
  const command = (script, args, failPlan, preload) => {
    callsPath = join(mkdtempSync(join(directory, 'run-')), 'calls');
    writeFileSync(callsPath, '');
    const imports = preload ? ['--import', preload] : [];
    return [process.execPath, [...imports, join(directory, 'scripts', script), ...args], {
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: [dirname(process.execPath), process.env.PATH].join(delimiter),
        SPARQ_NARGO: tool,
        SPARQ_BB: tool,
        CARGO_BUILD_JOBS: '1',
        CATALOGUE_TEST_CALLS: callsPath,
        CATALOGUE_TEST_FAIL_PLAN: failPlan ? `query_${planHash(failPlan)}` : '',
      },
    }];
  };
  const run = (...args) => {
    const result = spawnSync(...command(...args));
    assert.ifError(result.error);
    return result;
  };
  const buildCommand = ({ update = false, selected = cases, failPlan, rejectCatalogueRename = false, preload, cache = 'cache' } = {}) => {
    const selection = join(mkdtempSync(join(directory, 'selection-')), 'cases.json');
    writeFileSync(selection, JSON.stringify(selected));
    if (rejectCatalogueRename) {
      preload = join(directory, 'reject-rename.mjs');
      writeFileSync(preload, `
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { basename } from 'node:path';
const rename = fs.renameSync;
fs.renameSync = (from, to) => {
  if (basename(to) === 'catalogue.json') throw new Error('fixture publication failure');
  rename(from, to);
};
syncBuiltinESMExports();
`);
    }
    return ['build-circuits.mjs', [
      '--cache', join(directory, cache), '--cases', selection,
      ...(update ? ['--write-pins'] : []),
    ], failPlan, preload];
  };
  const build = options => run(...buildCommand(options));
  const pauseBuild = ({ pauseAt, ...options }) => {
    const pause = mkdtempSync(join(directory, 'pause-'));
    const ready = join(pause, 'ready'), release = join(pause, 'release');
    const preload = join(pause, 'preload.mjs');
    writeFileSync(preload, `
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { basename } from 'node:path';
const read = fs.readFileSync, rename = fs.renameSync;
function pause() {
  if (fs.existsSync(${JSON.stringify(ready)})) return;
  fs.writeFileSync(${JSON.stringify(ready)}, '');
  const deadline = Date.now() + 15000;
  while (!fs.existsSync(${JSON.stringify(release)})) {
    if (Date.now() >= deadline) throw new Error('fixture pause timed out');
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10);
  }
}
if (${JSON.stringify(pauseAt)} === 'read') {
  fs.readFileSync = (path, ...args) => {
    const bytes = read(path, ...args);
    if (path === ${JSON.stringify(join(assets, 'catalogue.json'))}) pause();
    return bytes;
  };
} else {
  fs.renameSync = (from, to) => {
    if (basename(to) === 'catalogue.json') pause();
    rename(from, to);
  };
}
syncBuiltinESMExports();
`);
    const child = spawn(...command(...buildCommand({ ...options, preload })));
    let stderr = '';
    child.stdout.resume();
    child.stderr.on('data', data => { stderr += data; });
    const result = new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('close', status => resolve({ status, stderr }));
    });
    return {
      async wait() {
        while (!existsSync(ready)) {
          const ended = await Promise.race([result, delay(10)]);
          assert.equal(ended, undefined, stderr || 'builder exited before pausing');
        }
      },
      resume() {
        writeFileSync(release, '');
        return result;
      },
    };
  };
  const readAsset = name => readFileSync(join(assets, name));
  assertPass(build({ update: true }));
  writeFileSync(join(host, 'approved-catalogue'), sha256(readAsset('catalogue.json')));
  return {
    abi, build, calls, source, readAsset, pauseBuild,
    table: () => JSON.parse(readAsset('catalogue.json')),
    snapshot: () => Object.fromEntries(readdirSync(assets).map(name => [name, readAsset(name)])),
    hasAsset: name => existsSync(join(assets, name)),
    removeAsset: name => rmSync(join(assets, name)),
    api: () => import(pathToFileURL(join(directory, 'index.js'))),
    buildSources: (...args) => run('sources.mjs', args),
  };
}

function assertPass(result) {
  assert.equal(result.status, 0, result.stderr);
}

test('changing the source pin requires every existing catalogue plan', t => {
  const f = fixture(t);
  const previous = f.snapshot();
  f.source('indexed scan library');
  f.abi(indexedAbi);

  const refused = f.build({ update: true, selected: [cases[0]] });
  assert.notEqual(refused.status, 0, 'partial refresh unexpectedly succeeded');
  assert.match(refused.stderr, /must rebuild every catalogue plan/);
  assert.deepEqual(f.calls(), []);
  assert.deepEqual(f.snapshot(), previous);

  assertPass(f.build({ update: true }));
  assert.notEqual(f.table().sourceBundleSha256, JSON.parse(previous['catalogue.json']).sourceBundleSha256);
  assertPass(f.build({ selected: [cases[1]] }));
});

test('a generator-only ABI change refuses a partial refresh with stale omitted plans', t => {
  const f = fixture(t);
  const last = { name: 'empty', query: 'ASK {}', bounds: {} };
  assertPass(f.build({ update: true, selected: [last] }));
  const previous = f.snapshot();
  f.abi(indexedAbi, cases[0]);
  f.abi(indexedAbi, last);

  const refused = f.build({ update: true, selected: [cases[0]] });
  assert.notEqual(refused.status, 0, 'partial refresh unexpectedly succeeded');
  assert.match(refused.stderr, /catalogue build inputs differ.*empty/);
  assert.deepEqual(f.calls(), []);
  assert.deepEqual(f.snapshot(), previous);

  assertPass(f.build({ update: true, selected: [cases[0], last] }));
  assert.equal(f.table().sourceBundleSha256, JSON.parse(previous['catalogue.json']).sourceBundleSha256);
  assertPass(f.build({ selected: [...cases, last] }));
  assert.deepEqual(f.calls(), []);
});

test('an unchanged pin refresh does not replace the catalogue', t => {
  const f = fixture(t);
  const previous = f.snapshot();
  assertPass(f.build({ update: true, rejectCatalogueRename: true }));
  assert.deepEqual(f.calls(), []);
  assert.deepEqual(f.snapshot(), previous);
});

test('refreshes based on an older catalogue cannot overwrite a concurrent addition', async t => {
  const f = fixture(t);
  const added = { query: 'ASK {}', bounds: {} };
  const stale = { query: 'SELECT * WHERE {}', bounds: {} };
  const cached = f.pauseBuild({ update: true, pauseAt: 'read' });
  const pending = f.pauseBuild({ update: true, selected: [stale], cache: 'pending-cache', pauseAt: 'read' });
  let published;
  try {
    await Promise.all([cached.wait(), pending.wait()]);
    assertPass(f.build({ update: true, selected: [added], cache: 'other-cache' }));
    assert.equal(f.table().entries.length, 3);
    published = f.snapshot();
  } finally {
    const results = [await cached.resume(), await pending.resume()];
    for (const result of results) {
      assert.notEqual(result.status, 0, 'outdated refresh unexpectedly succeeded');
      assert.match(result.stderr, /catalogue changed during build/);
    }
  }
  assert.deepEqual(f.snapshot(), published);
  assertPass(f.build({ selected: [added] }));
});

test('another writer cannot publish while a catalogue replacement is in progress', async t => {
  const f = fixture(t);
  const first = { query: 'ASK {}', bounds: {} };
  const second = { query: 'SELECT * WHERE {}', bounds: {} };
  const missing = f.table().entries[0].acir;
  f.removeAsset(missing);
  const writer = f.pauseBuild({ update: true, selected: [first], pauseAt: 'rename' });
  try {
    await writer.wait();
    const refused = f.build({ update: true, selected: [second], cache: 'other-cache' });
    assert.notEqual(refused.status, 0);
    assert.match(refused.stderr, /catalogue publication already in progress/);
    assert.equal(f.hasAsset(`${planHash(second)}.json`), false);
    const repair = f.build({ selected: [cases[0]] });
    assert.notEqual(repair.status, 0);
    assert.match(repair.stderr, /catalogue publication already in progress/);
    assert.equal(f.hasAsset(missing), false);
    const sources = f.buildSources();
    assert.notEqual(sources.status, 0);
    assert.match(sources.stderr, /catalogue publication already in progress/);
  } finally {
    assertPass(await writer.resume());
  }
  assertPass(f.build({ update: true, selected: [second], cache: 'other-cache' }));
  assert.equal(f.table().entries.length, 4);
  assertPass(f.build({ selected: [cases[0]] }));
  assert.equal(f.hasAsset(missing), true);
});

test('a failed second build leaves the previous catalogue loadable', async t => {
  const f = fixture(t);
  const previous = f.snapshot();
  const api = await f.api();
  const keyFor = ({ query }) => api.request({ query, policy: {}, nonce: '' }).verificationKey;
  const oldKeys = cases.map(keyFor);
  f.source('indexed scan library');
  f.abi(indexedAbi);

  const refused = f.build({ update: true, failPlan: cases[1] });
  assert.notEqual(refused.status, 0);
  assert.match(refused.stderr, /native catalogue build refused/);
  assert.deepEqual(f.calls(), [
    `compile query_${planHash(cases[0])}`,
    `write_vk query_${planHash(cases[0])}`,
    `compile query_${planHash(cases[1])}`,
  ]);
  assert.deepEqual(cases.map(keyFor), oldKeys);
  assert.deepEqual(f.snapshot(), previous);

  const publishFailure = f.build({
    update: true,
    selected: [...cases, { query: 'ASK {}', bounds: {} }],
    rejectCatalogueRename: true,
  });
  assert.notEqual(publishFailure.status, 0);
  assert.match(publishFailure.stderr, /fixture publication failure/);
  assert.deepEqual(cases.map(keyFor), oldKeys);
  assert.deepEqual(f.snapshot(), previous);
  assert.deepEqual(f.calls(), [
    `compile query_${planHash(cases[1])}`,
    `write_vk query_${planHash(cases[1])}`,
    `compile query_${sha256('ASK {}')}`,
    `write_vk query_${sha256('ASK {}')}`,
  ]);

  assertPass(f.build({ update: true }));
  assert.deepEqual(f.calls(), []);
});

test('the builder bypasses a warm program and key when the SCAN ABI changes', t => {
  const f = fixture(t);
  const [old] = f.table().entries;
  assert.deepEqual(JSON.parse(f.readAsset(old.acir)).abi, previousAbi);
  f.abi(indexedAbi);
  assertPass(f.build({ update: true }));

  const [entry] = f.table().entries;
  const program = f.readAsset(entry.acir);
  assert.equal(entry.planHash, old.planHash);
  assert.notEqual(entry.acirSha256, old.acirSha256);
  assert.notEqual(entry.vkSha256, old.vkSha256);
  assert.deepEqual(JSON.parse(program).abi, indexedAbi);
  assert.equal(entry.acirSha256, sha256(program));
  const key = Buffer.from(f.readAsset(entry.vk).toString().trim(), 'hex');
  assert.equal(key.toString('hex'), sha256(program));
  assert.equal(entry.vkSha256, sha256(key));

  f.removeAsset(entry.acir);
  f.removeAsset(entry.vk);
  assertPass(f.build({ selected: [cases[0]], failPlan: cases[0] }));
  assert.deepEqual(f.calls(), []);
  assert.deepEqual(f.readAsset(entry.acir), program);
  assert.equal(f.readAsset(entry.vk).toString().trim(), key.toString('hex'));
});

test('standalone source regeneration preserves the published bundle on a pin mismatch', t => {
  const f = fixture(t);
  const previous = f.snapshot();
  f.source('indexed scan library');

  const build = f.build();
  assert.notEqual(build.status, 0);
  assert.match(build.stderr, /source bundle pin differs/);
  const refused = f.buildSources();
  assert.notEqual(refused.status, 0);
  assert.match(refused.stderr, /source bundle differs from catalogue pin/);
  const update = f.buildSources('--write-pins');
  assert.notEqual(update.status, 0);
  assert.match(update.stderr, /Refresh both with build-circuits\.mjs --write-pins/);
  assert.deepEqual(f.snapshot(), previous);
  f.source('previous scan library');
  f.removeAsset('sources.json');
  assertPass(f.buildSources('--write-pins'));
  assert.deepEqual(f.snapshot(), previous);
});
