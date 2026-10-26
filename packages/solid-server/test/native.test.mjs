import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createHash, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { calculateJwkThumbprint, exportJWK, generateKeyPair, SignJWT } from 'jose';
import { chmod, cp, mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
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
    await writeFile(join(pkg, 'package.json'), JSON.stringify({ name: packageName, version: '0.1.6' }));
    await symlink(resolve(binary), join(pkg, 'bin', `sparq-lws-core${process.platform === 'win32' ? '.exe' : ''}`));
  }
  return dir;
}

async function boot(t, dir, args, env = {}) {
  const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), ...args], {
    env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'],
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
    const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
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

test('CLI preserves the native child exit code during startup', { skip: !binary || process.platform === 'win32', timeout: 10_000 }, async (t) => {
  const dir = await fixture(t, true);
  const executable = join(dir, 'node_modules', packageName, 'bin/sparq-lws-core');
  await rm(executable);
  await writeFile(executable, '#!/bin/sh\nexit 7\n');
  await chmod(executable, 0o755);
  const child = spawn(process.execPath, [join(dir, 'bin/solid-server.js'), '--protocol', 'lws'], { stdio: ['ignore', 'pipe', 'pipe'] });
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
  await assert.rejects(startSolidServer({ native: true, port: server.address().port }), /EADDRINUSE/);
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
