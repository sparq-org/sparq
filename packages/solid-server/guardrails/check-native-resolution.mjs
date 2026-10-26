import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import net from 'node:net';
import http from 'node:http';
import https from 'node:https';
import dns from 'node:dns';
import childProcess from 'node:child_process';

const offline = () => { throw new Error('native resolution attempted network access or a subprocess'); };
net.Socket.prototype.connect = offline;
http.request = http.get = https.request = https.get = offline;
dns.lookup = dns.resolve = offline;
childProcess.spawn = childProcess.spawnSync = childProcess.exec = childProcess.execSync = childProcess.execFile = childProcess.execFileSync = offline;
globalThis.fetch = offline;
syncBuiltinESMExports();

const { nativePackageName } = await import('../src/native-platforms.js');
for (const [os, cpu] of [['linux', 'x64'], ['linux', 'arm64'], ['darwin', 'x64'], ['darwin', 'arm64'], ['win32', 'x64']]) {
  assert.equal(nativePackageName(os, cpu), `@sparq-org/solid-server-native-${os}-${cpu}`);
}
if (['linux-x64', 'linux-arm64', 'darwin-x64', 'darwin-arm64', 'win32-x64'].includes(`${process.platform}-${process.arch}`)) {
  const name = `@sparq-org/solid-server-native-${process.platform}-${process.arch}`;
  const dir = mkdtempSync(join(tmpdir(), 'solid-offline-resolution-'));
  try {
    cpSync(new URL('../src', import.meta.url), join(dir, 'src'), { recursive: true });
    const host = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
    writeFileSync(join(dir, 'package.json'), JSON.stringify(host));
    const { resolveNativeBinary } = await import(pathToFileURL(join(dir, 'src/native.js')));
    assert.throws(() => resolveNativeBinary(), { message: `missing native server package ${name}; install ${name}@${host.version} and enable optional dependencies` });
    const pkg = join(dir, 'node_modules', name);
    mkdirSync(join(pkg, 'bin'), { recursive: true });
    const binary = join(pkg, 'bin', `sparq-lws-core${process.platform === 'win32' ? '.exe' : ''}`);
    writeFileSync(binary, 'offline executable fixture');
    writeFileSync(join(pkg, 'package.json'), JSON.stringify({ name, version: host.version }));
    assert.equal(resolveNativeBinary(), binary);
    writeFileSync(join(pkg, 'package.json'), JSON.stringify({ name, version: '0.0.0' }));
    assert.throws(() => resolveNativeBinary(), /must match @sparq-org\/solid-server version/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
} else {
  const { resolveNativeBinary } = await import('../src/native.js');
  assert.throws(() => resolveNativeBinary(), /native server is unavailable/);
}
