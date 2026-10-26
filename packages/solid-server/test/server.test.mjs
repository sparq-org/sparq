// [GPT-5.6] sq-6xasp.9: real wasm listener and npx acceptance tests.
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createConnection } from 'node:net';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { startSolidServer } from '../src/index.js';

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoRoot = resolve(packageDir, '../..');
const ownerWebid = 'https://id.example/alice#me';
const baseUrl = 'http://127.0.0.1';
const turtle = '<http://127.0.0.1/card> <http://xmlns.com/foaf/0.1/name> "Ada" .\n';

function listenerUrl(server) {
  const address = server.address();
  assert.ok(address && typeof address !== 'string');
  return `http://127.0.0.1:${address.port}`;
}

async function stopChild(child) {
  if (!child.pid) return;
  const target = process.platform === 'win32' ? child.pid : -child.pid;
  const signal = (name) => {
    try {
      process.kill(target, name);
      return true;
    } catch (error) {
      if (error.code === 'ESRCH') return false;
      throw error;
    }
  };
  const exit = child.exitCode === null && child.signalCode === null
    ? once(child, 'exit')
    : Promise.resolve();
  signal('SIGTERM');
  await Promise.race([exit, new Promise((resolve) => setTimeout(resolve, 2_000))]);
  if (signal('SIGKILL')) {
    await exit;
  }
}

async function waitForStartup(child) {
  let stdout = '';
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', (chunk) => {
    stderr += chunk;
  });
  child.stdout.setEncoding('utf8');
  for await (const chunk of child.stdout) {
    stdout += chunk;
    const newline = stdout.indexOf('\n');
    if (newline === -1) continue;
    const line = stdout.slice(0, newline);
    const startup = JSON.parse(line);
    assert.equal(startup.event, 'listening');
    assert.equal(startup.auth, 'fixed-owner-local-only');
    return startup;
  }
  throw new Error(`solid-server exited before readiness: ${stderr}`);
}

test('startSolidServer boots a loopback listener', { timeout: 30_000 }, async () => {
  const server = await startSolidServer({ port: 0, baseUrl, ownerWebid });
  try {
    const response = await fetch(`${listenerUrl(server)}/missing`);
    assert.equal(response.status, 404);
  } finally {
    await server.closeAsync();
  }
});

test('wasm CLI exits on a second SIGINT while graceful close waits for a request', { skip: process.platform === 'win32', timeout: 15_000 }, async (t) => {
  const child = spawn(process.execPath, [join(packageDir, 'bin/solid-server.js'), '--port', '0', '--base-url', baseUrl], {
    detached: true,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('SPARQ_SOLID_'))),
  });
  t.after(() => stopChild(child));
  let stderr = '';
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const exited = once(child, 'close');
  const startup = await waitForStartup(child);
  const { hostname, port } = new URL(startup.url);
  const socket = createConnection({ host: hostname, port: Number(port) });
  t.after(() => socket.destroy());
  await once(socket, 'connect');
  const continued = once(socket, 'data');
  socket.write(`PUT /unfinished HTTP/1.1\r\nHost: ${hostname}:${port}\r\nContent-Type: text/plain\r\nContent-Length: 10\r\nExpect: 100-continue\r\n\r\nx`);
  assert.match((await continued)[0].toString(), /HTTP\/1\.1 100 Continue/);

  child.kill('SIGINT');
  // Wait until close() has stopped accepting connections, while the body stays unfinished.
  const deadline = Date.now() + 2_000;
  while (true) {
    const probe = createConnection({ host: hostname, port: Number(port) });
    const error = await new Promise((resolve) => {
      probe.once('connect', () => resolve(null));
      probe.once('error', resolve);
    });
    probe.destroy();
    if (error) {
      assert.equal(error.code, 'ECONNREFUSED');
      break;
    }
    assert.ok(Date.now() < deadline, 'first SIGINT must start graceful close');
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.equal(child.exitCode, null, 'unfinished request must keep graceful close pending');
  assert.equal(child.signalCode, null);
  assert.equal(socket.destroyed, false);

  child.kill('SIGINT');
  let timer;
  try {
    const result = await Promise.race([
      exited,
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('second SIGINT did not exit promptly')), 2_000); }),
    ]);
    assert.deepEqual(result, [130, null]);
    assert.equal(stderr, '');
  } finally {
    clearTimeout(timer);
  }
});

test('npx boots, keeps one pod, round-trips Turtle, and enforces WAC', { timeout: 60_000 }, async () => {
  // Private npm cache: test files run concurrently, and this file and smoke.test.mjs
  // both `npx --package <packageDir>` — the same npx cache key. Sharing ~/.npm/_npx
  // makes the two npm installs race on the same symlink (EEXIST on cold CI caches).
  const npmCache = await mkdtemp(join(tmpdir(), 'solid-server-npx-'));
  const child = spawn(
    'npx',
    [
      '--yes',
      '--package',
      packageDir,
      'solid-server',
      '--port',
      '0',
      '--base-url',
      baseUrl,
      '--owner-webid',
      ownerWebid,
    ],
    {
      cwd: repoRoot,
      detached: process.platform !== 'win32',
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, npm_config_cache: npmCache },
    },
  );

  try {
    const startup = await waitForStartup(child);

    const put = await fetch(`${startup.url}/card`, {
      method: 'PUT',
      headers: { 'content-type': 'text/turtle' },
      body: turtle,
    });
    assert.equal(put.status, 201);

    const get = await fetch(`${startup.url}/card`, {
      headers: { accept: 'text/turtle' },
    });
    assert.equal(get.status, 200);
    assert.equal(await get.text(), turtle);

    const locked = '<http://127.0.0.1/locked> <http://xmlns.com/foaf/0.1/name> "Private" .\n';
    const putLocked = await fetch(`${startup.url}/locked`, {
      method: 'PUT',
      headers: { 'content-type': 'text/turtle' },
      body: locked,
    });
    assert.equal(putLocked.status, 201);

    const acl = `@prefix acl: <http://www.w3.org/ns/auth/acl#> .
<#mallory> a acl:Authorization;
  acl:agent <https://id.example/mallory#me>;
  acl:accessTo <http://127.0.0.1/locked>;
  acl:mode acl:Read .
`;
    const putAcl = await fetch(`${startup.url}/locked.acl`, {
      method: 'PUT',
      headers: { 'content-type': 'text/turtle' },
      body: acl,
    });
    assert.equal(putAcl.status, 201);

    const denied = await fetch(`${startup.url}/locked`, {
      headers: { accept: 'text/turtle' },
    });
    assert.equal(denied.status, 403);
  } finally {
    await stopChild(child);
    await rm(npmCache, { recursive: true, force: true });
  }
});
