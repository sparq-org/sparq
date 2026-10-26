import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import net from 'node:net';
import http from 'node:http';
import https from 'node:https';
import dns from 'node:dns';
import childProcess from 'node:child_process';

const offline = () => { throw new Error('native resolution attempted network access or a subprocess'); };
net.Socket.prototype.connect = offline;
http.request = http.get = https.request = https.get = offline;
dns.lookup = dns.resolve = offline;
childProcess.spawn = childProcess.exec = childProcess.execFile = offline;
globalThis.fetch = offline;
syncBuiltinESMExports();

const { resolveNativeBinary } = await import('../src/native.js');
const { nativePackageName } = await import('../src/native-platforms.js');
for (const [os, cpu] of [['linux', 'x64'], ['linux', 'arm64'], ['darwin', 'x64'], ['darwin', 'arm64'], ['win32', 'x64']]) {
  assert.equal(nativePackageName(os, cpu), `@sparq-org/solid-server-native-${os}-${cpu}`);
}
const expected = `@sparq-org/solid-server-native-${process.platform}-${process.arch}`;
try {
  const path = resolveNativeBinary();
  assert.ok(existsSync(path), 'resolved executable must exist');
  assert.ok(path.includes(`solid-server-native-${process.platform}-${process.arch}`));
} catch (error) {
  assert.ok(error.message.includes(`missing native server package ${expected}`)
    || error.message.startsWith(`native server is unavailable for ${process.platform}-${process.arch};`), error.message);
}
