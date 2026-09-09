// [FABLE-5] #2323: integration tests for the `@sparq-org/solid-server/fastify` plugin — a REAL
// Fastify instance driving the REAL wasm pod (no stubs), via fastify's bundled inject()
// (light-my-request), so no listener socket is needed.
//
// Both majors of the `^4.28.0 || ^5.0.0` peer range are exercised: `fastify` (v5) and the
// aliased `fastify-v4` (npm:fastify@^4) are workspace devDependencies installed by the root
// `npm ci`, so a failed import is a broken environment and FAILS the suite — it never skips.
// The ONLY skip is the wasm artifact (`npm run build:lws-wasm`), a build product this suite
// needs like server.test.mjs; the js CI lane builds it in the same job before `npm test`, so
// the skip can only fire locally, never silently in CI.
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import test from 'node:test';

const wasmBuilt = existsSync(new URL('../wasm/sparq_lws_wasm.js', import.meta.url));
const skip = wasmBuilt ? false : 'wasm artifact missing — run `npm run build:lws-wasm` first';

const baseUrl = 'http://127.0.0.1';
const ownerWebid = 'https://id.example/alice#me';
const turtle = '<http://127.0.0.1/card> <http://xmlns.com/foaf/0.1/name> "Ada" .\n';

// Both claimed peer majors, driven through the identical suite below.
const FASTIFY_MAJORS = [
  { expected: /^5\./, label: 'fastify v5', specifier: 'fastify' },
  { expected: /^4\./, label: 'fastify v4', specifier: 'fastify-v4' },
];

async function buildApp(specifier, expected) {
  // Dynamic imports: a missing fastify install must fail INSIDE the test (loud, not a silent
  // skip), and src/fastify.js statically imports the wasm glue via index.js, so neither may
  // load at module scope while the wasm-artifact skip is deciding.
  const { default: Fastify } = await import(specifier);
  const { solidPod } = await import('../src/fastify.js');
  const app = Fastify();
  assert.match(
    app.version,
    expected,
    `${specifier} resolved fastify ${app.version}; the devDependency no longer pins this major`,
  );
  await app.register(solidPod, { baseUrl, ownerWebid });
  await app.ready();
  return app;
}

for (const { expected, label, specifier } of FASTIFY_MAJORS) {
  test(`${label}: plugin round-trips Turtle through the wasm pod`, { skip, timeout: 60_000 }, async () => {
    const app = await buildApp(specifier, expected);
    try {
      const put = await app.inject({
        method: 'PUT',
        url: '/card',
        headers: { 'content-type': 'text/turtle' },
        payload: turtle,
      });
      assert.equal(put.statusCode, 201);

      const get = await app.inject({
        method: 'GET',
        url: '/card',
        headers: { accept: 'text/turtle' },
      });
      assert.equal(get.statusCode, 200);
      assert.equal(get.body, turtle, 'bytes reach wasm unparsed and round-trip exactly');

      const missing = await app.inject({ method: 'GET', url: '/missing' });
      assert.equal(missing.statusCode, 404);
    } finally {
      await app.close();
    }
  });

  test(`${label}: plugin enforces WAC through the pod`, { skip, timeout: 60_000 }, async () => {
    const app = await buildApp(specifier, expected);
    try {
      const locked = '<http://127.0.0.1/locked> <http://xmlns.com/foaf/0.1/name> "Private" .\n';
      const putLocked = await app.inject({
        method: 'PUT',
        url: '/locked',
        headers: { 'content-type': 'text/turtle' },
        payload: locked,
      });
      assert.equal(putLocked.statusCode, 201);

      const acl = `@prefix acl: <http://www.w3.org/ns/auth/acl#> .
<#mallory> a acl:Authorization;
  acl:agent <https://id.example/mallory#me>;
  acl:accessTo <http://127.0.0.1/locked>;
  acl:mode acl:Read .
`;
      const putAcl = await app.inject({
        method: 'PUT',
        url: '/locked.acl',
        headers: { 'content-type': 'text/turtle' },
        payload: acl,
      });
      assert.equal(putAcl.statusCode, 201);

      const denied = await app.inject({
        method: 'GET',
        url: '/locked',
        headers: { accept: 'text/turtle' },
      });
      assert.equal(denied.statusCode, 403, 'the owner is denied by the mallory-only ACL');
    } finally {
      await app.close();
    }
  });

  test(`${label}: plugin maps the body-limit error to the host 413 shape`, { skip, timeout: 60_000 }, async () => {
    const app = await buildApp(specifier, expected);
    try {
      const big = await app.inject({
        method: 'PUT',
        url: '/big',
        headers: { 'content-type': 'text/turtle' },
        payload: Buffer.alloc(2 * 1024 * 1024 + 1),
      });
      assert.equal(big.statusCode, 413);
      assert.equal(big.headers['content-type'], 'text/plain; charset=utf-8');
      assert.equal(big.body, 'request body too large\n');
    } finally {
      await app.close();
    }
  });

  test(`${label}: plugin preserves the /sparql query string`, { skip, timeout: 60_000 }, async () => {
    const app = await buildApp(specifier, expected);
    try {
      const put = await app.inject({
        method: 'PUT',
        url: '/card',
        headers: { 'content-type': 'text/turtle' },
        payload: turtle,
      });
      assert.equal(put.statusCode, 201);

      const query = 'SELECT ?name FROM <http://www.w3.org/ns/solid/sparql#union-default-graph> ' +
        'WHERE { ?s <http://xmlns.com/foaf/0.1/name> ?name }';
      const res = await app.inject({
        method: 'GET',
        url: `/sparql?query=${encodeURIComponent(query)}`,
      });
      assert.equal(res.statusCode, 200, `sparql endpoint answered ${res.statusCode}: ${res.body}`);
      const json = JSON.parse(res.body);
      assert.deepEqual(
        json.results.bindings.map((b) => b.name.value),
        ['Ada'],
        'the query parameter survived request.raw.url intact',
      );
    } finally {
      await app.close();
    }
  });
}
