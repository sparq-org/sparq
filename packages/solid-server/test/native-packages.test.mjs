import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { nativePackageName } from '../src/native-platforms.js';

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');

test('unsupported native platforms are explicit', () => {
  assert.throws(() => nativePackageName('linux', 'riscv64'), /native server is unavailable for linux-riscv64/);
});

test('release archives assemble all platform packages with matching versions, binaries, and no scripts', async (t) => {
  const dir = await mkdtemp(join(tmpdir(), 'solid-native-packages-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const manifest = JSON.parse(await readFile(join(packageDir, 'package.json')));
  const artifacts = join(dir, 'artifacts');
  await mkdir(artifacts);
  const platforms = [
    ['linux-x64', 'x64-baseline', 'linux', 'x64'],
    ['linux-arm64', 'arm64-linux', 'linux', 'arm64'],
    ['darwin-x64', 'x64-darwin', 'darwin', 'x64'],
    ['darwin-arm64', 'arm64-darwin', 'darwin', 'arm64'],
    ['win32-x64', 'win-x64-baseline', 'win32', 'x64'],
  ];
  for (const [platform, tier, os, cpu] of platforms) {
    const root = `sparq-cli-v${manifest.version}-${tier}`;
    const executable = `sparq-lws-core${os === 'win32' ? '.exe' : ''}`;
    await mkdir(join(dir, root));
    await writeFile(join(dir, root, executable), `native executable fixture for ${platform}`);
    await writeFile(join(dir, root, 'LICENSE'), 'license fixture');
    if (os === 'win32') {
      execFileSync('zip', ['-qr', join(artifacts, `${root}.zip`), root], { cwd: dir });
    } else {
      execFileSync('tar', ['czf', join(artifacts, `${root}.tar.gz`), root], { cwd: dir });
    }
    const generated = join(dir, 'packages', platform);
    await mkdir(join(generated, 'bin'), { recursive: true });
    await writeFile(join(generated, 'bin', 'stale-executable'), 'old build');
    execFileSync(process.execPath, [join(packageDir, 'native/assemble.mjs'), artifacts, join(dir, 'packages'), platform]);
    const pkg = JSON.parse(await readFile(join(generated, 'package.json')));
    const name = `@sparq-org/solid-server-native-${platform}`;
    assert.equal(pkg.name, name);
    assert.equal(pkg.version, manifest.version);
    assert.equal(manifest.optionalDependencies[name], pkg.version);
    assert.deepEqual(pkg.os, [os]); assert.deepEqual(pkg.cpu, [cpu]);
    assert.equal(pkg.scripts, undefined);
    assert.equal(await readFile(join(generated, 'bin', executable), 'utf8'), `native executable fixture for ${platform}`);
    assert.equal(await readFile(join(generated, 'LICENSE'), 'utf8'), 'license fixture');
    assert.ok((await readFile(join(generated, 'README.md'), 'utf8')).includes(name));
    const packed = JSON.parse(execFileSync('npm', ['pack', generated, '--dry-run', '--ignore-scripts', '--json'], { encoding: 'utf8' }));
    assert.deepEqual(packed[0].files.map((entry) => entry.path).sort(), ['LICENSE', 'README.md', `bin/${executable}`, 'package.json'].sort());
    for (const file of ['package.json', 'README.md', 'LICENSE', `bin/${executable}`]) {
      assert.ok(packed[0].files.some((entry) => entry.path === file), `${name} must pack ${file}`);
    }
  }
  await rm(join(artifacts, `sparq-cli-v${manifest.version}-x64-baseline.tar.gz`));
  assert.throws(() => execFileSync(process.execPath, [join(packageDir, 'native/assemble.mjs'), artifacts, join(dir, 'missing'), 'linux-x64'], { stdio: 'pipe' }), /Command failed/);
});
