import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

test('circuit regeneration uses the caller cache and records its committed inputs', t => {
  const root = mkdtempSync(join(tmpdir(), 'sparq-result-build-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (path, data, options) => {
    const file = join(root, path);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, data, options);
  };
  for (const path of ['zk-result/scripts/build-circuits.mjs', 'zk-shared/crs.mjs']) {
    const dest = join(root, 'js', path);
    mkdirSync(dirname(dest), { recursive: true });
    copyFileSync(new URL('../../' + path, import.meta.url), dest);
  }
  const member = { member: 'example', acir: 'example.json', vk: 'example.vk', proofBytes: 1 };
  const digest = bytes => createHash('sha256').update(bytes).digest('hex');
  const assets = 'js/zk-result/assets';
  write('.github/workflows/zk-toolchain.yml', 'NARGO_VERSION: "fixture"\nBB_VERSION: "fixture"\n');
  write(assets + '/members.json', JSON.stringify([member]));
  write(assets + '/crs.pins.json', JSON.stringify(Object.fromEntries([1, 2].map(n => [
    'bn254_g' + n + '.dat',
    { url: 'https://crs.aztec-cdn.foundation/g' + n + '.dat', bytes: 1, sha256: digest('x') },
  ]))));
  write('crates/sparq-zk-compose/tests/result_method_keys.json', JSON.stringify({
    target: 'noir-recursive', members: { example: { vk_sha256: digest('key') } },
  }));
  write('caller-cache/nargo/pinned-dependency', 'cached');
  mkdirSync(join(root, 'zk/compose'), { recursive: true });
  const command = (name, body) => write('bin/' + name, '#!' + process.execPath + '\n' + body, { mode: 0o700 });
  command('nargo', `
    const fs = require('node:fs');
    if (process.argv[2] === '--version') { console.log('nargo version = fixture'); process.exit(0); }
    require('node:assert/strict').equal(fs.readFileSync(process.env.HOME + '/nargo/pinned-dependency', 'utf8'), 'cached');
    fs.mkdirSync('target', { recursive: true });
    fs.writeFileSync('target/example.json', JSON.stringify({ abi: {}, bytecode: 'compiled' }));
  `);
  command('bb', `
    const args = process.argv.slice(2);
    if (args[0] === '--version') { console.log('fixture'); process.exit(0); }
    require('node:fs').writeFileSync(args[args.indexOf('-o') + 1] + '/vk', 'key');
  `);
  command('curl', `
    const args = process.argv.slice(2);
    require('node:fs').writeFileSync(args[args.indexOf('--output') + 1], 'x');
  `);
  command('git', `
    const assert = require('node:assert/strict');
    const args = process.argv.slice(2);
    if (args[0] === 'rev-parse') { console.log('f'.repeat(40)); process.exit(0); }
    assert.ok(args.includes('--'));
    assert.ok(args.includes('zk/compose'));
    assert.ok(args.includes('zk/dual-leaf-v2'));
    assert.equal(require('node:fs').existsSync('js/zk-result/assets/example.json'), false);
  `);
  execFileSync(process.execPath, ['js/zk-result/scripts/build-circuits.mjs'], {
    cwd: root, env: { ...process.env, PATH: join(root, 'bin'), SPARQ_RESULT_COMPILER_HOME: 'caller-cache' },
  });
  const provenance = JSON.parse(readFileSync(join(root, assets, 'provenance.json')));
  assert.equal(provenance.sourceCommit, 'f'.repeat(40));
  assert.equal(provenance.uncommittedSourceChanges, false);
  assert.equal(provenance.members[0].acirSha256, digest('{"abi":{},"bytecode":"compiled"}\n'));
  assert.equal(provenance.members[0].vkSha256, digest('key'));
  assert.equal(readFileSync(join(root, 'caller-cache/nargo/pinned-dependency'), 'utf8'), 'cached');
});
