// [SONNET-4.6] sq-6xasp.5: end-to-end smoke test — npx-boot the @sparq-org/solid-server wasm
// package then run a full Solid LDP CREATE → READ → UPDATE → DELETE + WAC-denied round-trip.
//
// DESIGN (research/wasm-solid-server-npm-design.md phase 5):
//   • boot via `npx --package <dir> solid-server` on an ephemeral OS port (port=0);
//   • issue a sequence of LDP operations asserting status codes + body;
//   • confirm WAC enforcement (a request to a resource whose ACL restricts access → 403).
//
// INVARIANT: this test lives in packages/solid-server/test/ and is picked up by the existing
// `npm test` runner (`node --test test/*.test.mjs`).  It runs inside the existing
// `solid-server package (build wasm + node:test + package guardrail)` step of .github/workflows/
// js.yml — no new CI job or required check is added.  It does NOT gate the native Rust workspace.
//
// NON-VACUITY: if the server is not booted (the `waitForStartup` step is removed and replaced with
// a direct URL like http://127.0.0.1:1) every fetch throws ECONNREFUSED and the test FAILS with an
// unhandled rejection surfaced by node:test.  Likewise, mutating any expected status code (e.g.
// `assert.equal(create.status, 200)` instead of 201) causes node:test to report a failure.
// These failure modes are confirmed in the non-vacuity section of the PR description.
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoRoot = resolve(packageDir, '../..');

// ---------------------------------------------------------------------------
// Helpers (shared with server.test.mjs by design — kept local for isolation)
// ---------------------------------------------------------------------------

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
  const exit =
    child.exitCode === null && child.signalCode === null
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
    return startup;
  }
  throw new Error(`solid-server exited before readiness: ${stderr}`);
}

// ---------------------------------------------------------------------------
// Smoke: full LDP CRUD + WAC round-trip through npx-booted wasm server
// ---------------------------------------------------------------------------

test(
  'smoke: npx-boot wasm server + LDP CREATE/READ/UPDATE/DELETE + WAC denial',
  { timeout: 90_000 },
  async () => {
    // [SONNET-4.6] sq-6xasp.5: boot via the package's npx entry on an OS-assigned port.
    const ownerWebid = 'https://id.example/alice#me';
    const baseUrl = 'http://127.0.0.1';

    // Private npm cache: this file and server.test.mjs run concurrently and share the
    // npx cache key for <packageDir>; a shared ~/.npm/_npx races on the install symlink.
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
      const base = startup.url; // e.g. "http://127.0.0.1:<port>"

      // -----------------------------------------------------------------------
      // CREATE: PUT a Turtle resource — expect 201 Created
      // -----------------------------------------------------------------------
      const originalTurtle =
        `<${base}/doc> <http://xmlns.com/foaf/0.1/name> "Smoke" .\n`;

      const create = await fetch(`${base}/doc`, {
        method: 'PUT',
        headers: { 'content-type': 'text/turtle' },
        body: originalTurtle,
      });
      assert.equal(create.status, 201, `CREATE: expected 201, got ${create.status}`);

      // -----------------------------------------------------------------------
      // READ: GET the resource — expect 200 + exact body round-trip
      // -----------------------------------------------------------------------
      const read = await fetch(`${base}/doc`, {
        headers: { accept: 'text/turtle' },
      });
      assert.equal(read.status, 200, `READ: expected 200, got ${read.status}`);
      assert.equal(
        await read.text(),
        originalTurtle,
        'READ: body does not match the PUTted Turtle',
      );

      // -----------------------------------------------------------------------
      // UPDATE: N3 Patch — insert a second triple, expect 204 No Content.
      //
      // The server accepts `text/n3` with a solid:inserts graph. The patch adds a
      // dc:title triple to /doc without removing the foaf:name one.
      // -----------------------------------------------------------------------
      const patch = `\
@prefix solid: <http://www.w3.org/ns/solid/terms#> .
@prefix dc: <http://purl.org/dc/terms/> .
_:p solid:inserts { <${base}/doc> dc:title "Smoke-doc" . } .
`;
      const update = await fetch(`${base}/doc`, {
        method: 'PATCH',
        headers: { 'content-type': 'text/n3' },
        body: patch,
      });
      assert.equal(update.status, 204, `UPDATE: expected 204, got ${update.status}`);

      // Verify the insert landed — the GET body must now contain both triples.
      const afterUpdate = await fetch(`${base}/doc`, {
        headers: { accept: 'text/turtle' },
      });
      assert.equal(afterUpdate.status, 200, `after-UPDATE GET: expected 200, got ${afterUpdate.status}`);
      const updatedBody = await afterUpdate.text();
      assert.ok(
        updatedBody.includes('Smoke-doc') || updatedBody.includes('dc:title'),
        `after-UPDATE GET: expected dc:title in body, got: ${updatedBody.slice(0, 400)}`,
      );

      // -----------------------------------------------------------------------
      // WAC: create a resource whose ACL restricts access to a third-party agent
      // only, then confirm an unauthenticated (= owner) request is denied — 403.
      //
      // In fixed-owner mode all requests are authenticated as ownerWebid.
      // An ACL that grants access to a different agent (mallory) but NOT to the
      // owner should return 403.
      // -----------------------------------------------------------------------
      const lockedTurtle =
        `<${base}/locked> <http://xmlns.com/foaf/0.1/name> "Locked" .\n`;

      const createLocked = await fetch(`${base}/locked`, {
        method: 'PUT',
        headers: { 'content-type': 'text/turtle' },
        body: lockedTurtle,
      });
      assert.equal(
        createLocked.status,
        201,
        `WAC setup CREATE: expected 201, got ${createLocked.status}`,
      );

      const lockAcl = `\
@prefix acl: <http://www.w3.org/ns/auth/acl#> .
<#mallory> a acl:Authorization;
  acl:agent <https://id.example/mallory#me>;
  acl:accessTo <${base}/locked>;
  acl:mode acl:Read .
`;
      const putAcl = await fetch(`${base}/locked.acl`, {
        method: 'PUT',
        headers: { 'content-type': 'text/turtle' },
        body: lockAcl,
      });
      assert.equal(
        putAcl.status,
        201,
        `WAC setup PUT ACL: expected 201, got ${putAcl.status}`,
      );

      const denied = await fetch(`${base}/locked`, {
        headers: { accept: 'text/turtle' },
      });
      assert.equal(
        denied.status,
        403,
        `WAC: expected 403 for locked resource, got ${denied.status}`,
      );

      // -----------------------------------------------------------------------
      // DELETE: remove /doc — expect 204 No Content
      // -----------------------------------------------------------------------
      const del = await fetch(`${base}/doc`, { method: 'DELETE' });
      assert.equal(del.status, 204, `DELETE: expected 204, got ${del.status}`);

      // Confirm the resource is gone — 404 after DELETE.
      const afterDelete = await fetch(`${base}/doc`, {
        headers: { accept: 'text/turtle' },
      });
      assert.equal(
        afterDelete.status,
        404,
        `after-DELETE GET: expected 404, got ${afterDelete.status}`,
      );
    } finally {
      await stopChild(child);
      await rm(npmCache, { recursive: true, force: true });
    }
  },
);
