import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { createHash, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { get as getHttps } from 'node:https';
import { calculateJwkThumbprint, exportJWK, generateKeyPair, SignJWT } from 'jose';
import { chmod, cp, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const binary = process.env.SPARQ_NATIVE_TEST_BINARY;
const platform = `${process.platform}-${process.arch}`;
const packageName = `@sparq-org/solid-server-native-${platform}`;
const require = createRequire(import.meta.url);
const hostManifest = JSON.parse(await readFile(join(packageDir, 'package.json'), 'utf8'));
const cleanEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(SOLID_SERVER_|SPARQ_SOLID_|PSS_SPARQ_|PORT$)/.test(key)));

async function fixture(t, withBinary) {
  const dir = await mkdtemp(join(tmpdir(), 'solid-native-test-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  for (const part of ['src', 'bin', 'package.json']) await cp(join(packageDir, part), join(dir, part), { recursive: true });
  await symlink(join(packageDir, 'wasm'), join(dir, 'wasm'), 'dir');
  await mkdir(join(dir, 'node_modules/@solid'), { recursive: true });
  const verifier = require.resolve('@solid/access-token-verifier');
  // The verifier entry is dist/index.js; keep its own installed dependency tree.
  await symlink(resolve(dirname(verifier), '..'), join(dir, 'node_modules/@solid/access-token-verifier'), 'dir');
  if (withBinary) {
    const pkg = join(dir, 'node_modules', packageName);
    await mkdir(join(pkg, 'bin'), { recursive: true });
    await writeFile(join(pkg, 'package.json'), JSON.stringify({ name: packageName, version: hostManifest.version }));
    const executable = join(pkg, 'bin', `sparq-lws-core${process.platform === 'win32' ? '.exe' : ''}`);
    if (typeof withBinary === 'string') {
      await writeFile(executable, `#!${process.execPath}\n${withBinary}`);
      await chmod(executable, 0o755);
    } else {
      await symlink(resolve(binary), executable);
    }
  }
  return dir;
}

async function boot(t, dir, args, env = {}) {
  const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), ...args], {
    env: { ...cleanEnv, ...env }, stdio: ['ignore', 'pipe', 'pipe'],
    detached: process.platform !== 'win32',
  });
  let stderr = '';
  let stdout = '';
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const exited = once(child, 'exit');
  t.after(async () => {
    const target = process.platform === 'win32' ? child.pid : -child.pid;
    const signal = (name) => {
      try { process.kill(target, name); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    };
    signal('SIGTERM');
    const timer = setTimeout(() => signal('SIGKILL'), 2000);
    await exited;
    clearTimeout(timer);
    if (process.platform !== 'win32') signal('SIGKILL');
  });
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  const deadline = Date.now() + 20_000;
  while (!stdout.includes('\n')) {
    if (child.exitCode !== null || child.signalCode !== null) throw new Error(stderr);
    if (Date.now() > deadline) throw new Error(`startup timeout: ${stderr}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  return { child, exited, startup: JSON.parse(stdout.split('\n')[0]) };
}

test('missing native package names the exact dependency and does not start a wasm server', { timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, false);
  for (const args of [['--protocol', 'lws'], ['--native']]) {
    const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), ...args], { env: cleanEnv, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = ''; let stderr = '';
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    const timer = setTimeout(() => child.kill('SIGKILL'), 2000);
    const [code] = await once(child, 'close');
    clearTimeout(timer);
    assert.equal(code, 1, stderr);
    assert.ok(stderr.includes(`missing native server package ${packageName}`), stderr);
    assert.equal(stdout, '', 'missing native package must never fall back to wasm');
  }
});

test('CLI preserves the native child exit code during startup', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, 'process.exit(7);');
  const executable = join(dir, 'node_modules', packageName, 'bin/sparq-lws-core');
  await rm(executable);
  await writeFile(executable, '#!/bin/sh\nexit 7\n');
  await chmod(executable, 0o755);
  const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), '--protocol', 'lws'], { env: cleanEnv, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = ''; let stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  assert.deepEqual(await once(child, 'close'), [7, null]);
  assert.equal(stdout, '');
  assert.match(stderr, /native server exited during startup/);
});

test('native API and CLI select LWS, serve CRUD, and forward SIGTERM', { skip: !binary, timeout: 30_000 }, async (t) => {
  const dir = await fixture(t, true);
  const { child, exited, startup } = await boot(t, dir, ['--protocol', 'lws', '--port', '0', '--base-url', 'http://127.0.0.1'], { SOLID_SERVER_LWS_OPEN: '1' });
  const root = await fetch(startup.url);
  assert.equal(root.status, 200);
  assert.match(root.headers.get('content-type'), /^application\/lws\+cid/);
  const description = await root.json();
  assert.ok(JSON.stringify(description).includes('Storage'));
  const create = await fetch(startup.url, { method: 'POST', headers: { 'content-type': 'text/plain', slug: 'npm-roundtrip' }, body: 'locally packed native server' });
  assert.equal(create.status, 201, await create.text());
  // The configured public origin differs deliberately from the bound ephemeral listener.
  const location = new URL(create.headers.get('location'), startup.url);
  const resource = `${startup.url}${location.pathname}`;
  const read = await fetch(resource);
  assert.equal(read.status, 200);
  assert.equal(await read.text(), 'locally packed native server');
  const remove = await fetch(resource, { method: 'DELETE' });
  assert.equal(remove.status, 204);
  assert.equal((await fetch(resource)).status, 404);
  child.kill('SIGTERM');
  const [code, signal] = await exited;
  assert.equal(code, 0); assert.equal(signal, null);
  await assert.rejects(fetch(startup.url));

  const { startSolidServer } = await import(pathToFileURL(join(dir, 'src/index.js')));
  const previousOpen = process.env.SOLID_SERVER_LWS_OPEN;
  process.env.SOLID_SERVER_LWS_OPEN = '1';
  let server;
  try {
    server = await startSolidServer({ protocol: 'lws', port: 0, baseUrl: 'http://127.0.0.1' });
  } finally {
    if (previousOpen === undefined) delete process.env.SOLID_SERVER_LWS_OPEN;
    else process.env.SOLID_SERVER_LWS_OPEN = previousOpen;
  }
  t.after(() => server.closeAsync());
  assert.ok(server.child.pid);
  assert.match((await fetch(`http://127.0.0.1:${server.address().port}/`)).headers.get('content-type'), /^application\/lws\+cid/);
  await assert.rejects(startSolidServer({ native: true, port: server.address().port }), /Address already in use|EADDRINUSE/);
  await assert.rejects(startSolidServer({ native: true, ownerWebid: 'https://example.test/alice' }), /wasm host options/);
  await assert.rejects(startSolidServer({ native: true, oidc: true }), /wasm host options/);
  await assert.rejects(startSolidServer({ native: true, data: '' }), /non-empty directory/);
  await assert.rejects(startSolidServer({ native: true, port: 0, baseUrl: 'http://127.0.0.1', data: join(dir, 'data') }), /requires a durable blob/);
  assert.equal((await fetch(`http://127.0.0.1:${server.address().port}/`)).status, 200);
  await server.closeAsync();
});

test('native Solid advertises WebSocketChannel2023 and handles subscription requests', { skip: !binary, timeout: 30_000 }, async (t) => {
  const dir = await fixture(t, true);
  const issuerKeys = await generateKeyPair('ES256', { extractable: true });
  const issuerJwk = { ...await exportJWK(issuerKeys.publicKey), kid: 'npm-test', alg: 'ES256' };
  let issuer;
  const identity = createServer((req, res) => {
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify(req.url === '/jwks' ? { keys: [issuerJwk] } : { issuer, jwks_uri: `${issuer}/jwks` }));
  });
  await new Promise((resolve) => identity.listen(0, '127.0.0.1', resolve));
  issuer = `http://127.0.0.1:${identity.address().port}`;
  t.after(() => new Promise((resolve) => identity.close(resolve)));
  const webid = 'https://id.example/alice#me';
  const { child, exited, startup } = await boot(t, dir, ['--native', '--port', '0', '--base-url', 'http://127.0.0.1'], {
    SOLID_SERVER_TRUSTED_ISSUER: issuer, SOLID_SERVER_ALLOW_LOOPBACK: '1',
    SOLID_SERVER_BIDIRECTIONAL: 'off', SOLID_SERVER_SEED_BENCH: '1', SOLID_SERVER_SEED_BENCH_OWNER: webid,
  });
  const discovery = await fetch(`${startup.url}/.well-known/solid`);
  assert.equal(discovery.status, 200);
  const body = await discovery.text();
  assert.ok(body.includes('http://www.w3.org/ns/solid/notifications#WebSocketChannel2023'), body);
  const subscription = await fetch(`${startup.url}/.notifications/WebSocketChannel2023/`, {
    method: 'POST', headers: { 'content-type': 'application/ld+json' },
    body: JSON.stringify({ type: 'http://www.w3.org/ns/solid/notifications#WebSocketChannel2023', topic: 'http://127.0.0.1/doc' }),
  });
  assert.equal(subscription.status, 401);
  const clientKeys = await generateKeyPair('ES256', { extractable: true });
  const jwk = await exportJWK(clientKeys.publicKey);
  const token = await new SignJWT({ webid, client_id: 'https://app.example/client', cnf: { jkt: await calculateJwkThumbprint(jwk) } })
    .setProtectedHeader({ alg: 'ES256', kid: issuerJwk.kid, typ: 'at+jwt' })
    .setIssuer(issuer).setSubject(webid).setJti(randomUUID()).setAudience('http://127.0.0.1')
    .setIssuedAt().setExpirationTime('5m').sign(issuerKeys.privateKey);
  const proof = await new SignJWT({
    ath: createHash('sha256').update(token).digest('base64url'), htm: 'POST',
    htu: 'http://127.0.0.1/.notifications/WebSocketChannel2023/', jti: randomUUID(),
  }).setProtectedHeader({ alg: 'ES256', typ: 'dpop+jwt', jwk }).setIssuedAt().sign(clientKeys.privateKey);
  const authorized = await fetch(`${startup.url}/.notifications/WebSocketChannel2023/`, {
    method: 'POST', headers: { 'content-type': 'application/ld+json', authorization: `DPoP ${token}`, dpop: proof },
    body: JSON.stringify({ type: 'http://www.w3.org/ns/solid/notifications#WebSocketChannel2023', topic: 'http://127.0.0.1/bench/private/doc' }),
  });
  assert.equal(authorized.status, 200, await authorized.clone().text());
  const channel = await authorized.json();
  assert.equal(channel.type, 'http://www.w3.org/ns/solid/notifications#WebSocketChannel2023');
  assert.match(channel.receiveFrom, /^ws:\/\/127\.0\.0\.1\/\.notifications\/WebSocketChannel2023\/receive/);
  child.kill('SIGTERM');
  assert.deepEqual(await exited, [0, null]);
});

const httpChild = `
const http = require('node:http');
const port = Number(process.env.SOLID_SERVER_BIND.split(':').pop());
const server = http.createServer((req, res) => res.end('native child fixture'));
server.listen(port, '127.0.0.1', () => console.log(JSON.stringify({
  event: 'native-listening', address: '127.0.0.1:' + server.address().port, scheme: 'http',
})));
`;

test('native CLI bounds shutdown when the child ignores SIGTERM', { skip: process.platform === 'win32', timeout: 20_000 }, async (t) => {
  const dir = await fixture(t, `${httpChild}\nprocess.on('SIGTERM', () => {});`);
  const { child, exited, startup } = await boot(t, dir, ['--native', '--port', '0', '--base-url', 'http://127.0.0.1']);
  assert.equal(await (await fetch(startup.url)).text(), 'native child fixture');
  child.kill('SIGTERM');
  await new Promise((resolve) => setTimeout(resolve, 100));
  child.kill('SIGTERM');
  assert.deepEqual(await exited, [137, null]);
  await assert.rejects(fetch(startup.url));
});

test('native CLI forwards SIGINT and preserves the child runtime exit code', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, `${httpChild}\nprocess.on('SIGINT', () => process.exit(23));`);
  const { child, exited, startup } = await boot(t, dir, ['--protocol', 'lws', '--port', '0', '--base-url', 'http://127.0.0.1']);
  assert.equal(await (await fetch(startup.url)).text(), 'native child fixture');
  child.kill('SIGINT');
  assert.deepEqual(await exited, [23, null]);
});

test('SIGTERM during native startup stops the child before readiness', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, "console.error('waiting:' + process.pid); setInterval(() => {}, 1000);");
  const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), '--native'], { env: cleanEnv, stdio: ['ignore', 'pipe', 'pipe'] });
  const exited = once(child, 'close');
  let stdout = ''; let stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
  t.after(() => clearTimeout(timer));
  while (!stderr.includes('waiting:')) {
    assert.equal(child.exitCode, null, stderr);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  const nativePid = Number(/waiting:(\d+)/.exec(stderr)[1]);
  t.after(() => { try { process.kill(nativePid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; } });
  child.kill('SIGTERM');
  assert.deepEqual(await exited, [143, null]);
  assert.equal(stdout, '');
  assert.throws(() => process.kill(nativePid, 0), { code: 'ESRCH' });
});

test('native API abort owns child lifetime before and after readiness', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, httpChild);
  const { startSolidServer } = await import(pathToFileURL(join(dir, 'src/index.js')));
  const cancelled = new AbortController();
  cancelled.abort();
  await assert.rejects(startSolidServer({ native: true, signal: cancelled.signal }), { name: 'AbortError' });
  const active = new AbortController();
  const server = await startSolidServer({ native: true, port: 0, baseUrl: 'http://127.0.0.1', signal: active.signal });
  t.after(() => server.closeAsync());
  const url = `http://127.0.0.1:${server.address().port}`;
  assert.equal(await (await fetch(url)).text(), 'native child fixture');
  active.abort();
  assert.equal((await server.exited).signal, 'SIGTERM');
  await assert.rejects(fetch(url));
});

test('native CLI rejects an incompatible installed package before launch', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, httpChild);
  await writeFile(join(dir, 'node_modules', packageName, 'package.json'), JSON.stringify({ name: packageName, version: '0.0.0' }));
  const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), '--native'], { env: cleanEnv, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const timer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; } }, 2000);
  t.after(() => clearTimeout(timer));
  let stdout = ''; let stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  assert.deepEqual(await once(child, 'close'), [1, null]);
  assert.match(stderr, /must match @sparq-org\/solid-server version/);
  assert.equal(stdout, '');
});

test('native spawn and malformed readiness failures return errors without a listener', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, 'console.log("invalid readiness"); setInterval(() => {}, 1000);');
  const { startSolidServer } = await import(pathToFileURL(join(dir, 'src/index.js')));
  await assert.rejects(startSolidServer({ native: true }), /JSON|Unexpected/);
  const executable = join(dir, 'node_modules', packageName, 'bin/sparq-lws-core');
  await chmod(executable, 0o644);
  await assert.rejects(startSolidServer({ native: true }), /EACCES/);
});


test('native TLS reports its actual bound port and HTTPS scheme', { skip: !binary, timeout: 30_000 }, async (t) => {
  const dir = await fixture(t, true);
  const cert = join(dir, 'cert.pem');
  const key = join(dir, 'key.pem');
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=localhost', '-keyout', key, '-out', cert], { stdio: 'ignore' });
  const { startup } = await boot(t, dir, ['--native', '--port', '0', '--base-url', 'https://localhost'], {
    SOLID_SERVER_TLS_CERT: cert, SOLID_SERVER_TLS_KEY: key,
  });
  assert.match(startup.url, /^https:\/\/127\.0\.0\.1:[1-9]\d*$/);
  const response = await new Promise((resolve, reject) => {
    getHttps(`${startup.url}/.well-known/solid`, { rejectUnauthorized: false }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => { body += chunk; });
      res.on('end', () => resolve({ status: res.statusCode, body }));
    }).on('error', reject);
  });
  assert.equal(response.status, 200);
  assert.match(response.body, /WebSocketChannel2023/);
});

test('native API rejects readiness for the wrong listener and oversized reports', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  for (const source of [
    "console.log(JSON.stringify({ event: 'wrong-event', address: '127.0.0.1:3012', scheme: 'http' })); setInterval(() => {}, 1000);",
    "console.log('x'.repeat(8193)); setInterval(() => {}, 1000);",
  ]) {
    const dir = await fixture(t, source);
    const { startSolidServer } = await import(pathToFileURL(join(dir, 'src/index.js')));
    await assert.rejects(startSolidServer({ native: true, port: 3012 }).then((server) => {
      t.after(() => server.closeAsync());
      return server;
    }), /invalid listener report|report is too large/);
  }
});

test('native CLI handles a signal while its launcher is still loading', { skip: process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, false);
  const launcher = join(dir, 'src/native.js');
  const source = await readFile(launcher, 'utf8');
  await writeFile(launcher, `console.error('loading-launcher'); await new Promise((resolve) => setTimeout(resolve, 250));\n${source}`);
  const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), '--native'], { env: cleanEnv, stdio: ['ignore', 'pipe', 'pipe'] });
  const exited = once(child, 'close');
  let stderr = '';
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const timer = setTimeout(() => child.kill('SIGKILL'), 2000);
  t.after(() => clearTimeout(timer));
  while (!stderr.includes('loading-launcher')) {
    assert.equal(child.exitCode, null, stderr);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  child.kill('SIGINT');
  assert.deepEqual(await exited, [130, null]);
  assert.match(stderr, /startup interrupted by SIGINT/);
  assert.doesNotMatch(stderr, /missing native server package|undefined/);
});
