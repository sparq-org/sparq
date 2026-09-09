<!-- [GPT-5.6] sq-6xasp.9/.10: honest public-package README for the Node wasm host. -->
# @sparq-org/solid-server

A local-development [Solid](https://solidproject.org/) LDP + WAC server backed by
sparq WebAssembly. Node owns the loopback HTTP listener; one long-lived wasm
`SolidServer` owns the in-memory pod.

> This v1 package is not a production server. Its default mode treats every
> request as the configured owner without authentication. Solid-OIDC verification
> is opt-in and does not add production transport or persistence hardening.

## 🚀 Quickstart

The published npm package includes its wasm artifact:

```sh
npx @sparq-org/solid-server \
  --port 3000 \
  --base-url http://127.0.0.1:3000 \
  --owner-webid https://id.example/alice#me
```

From a source checkout, build the artifact first:

```sh
npm --workspace @sparq-org/solid-server run build:lws-wasm
npx --yes --package ./packages/solid-server solid-server --port 3000
```

Use it from Node:

```js
import { startSolidServer } from '@sparq-org/solid-server';

const server = await startSolidServer({
  port: 3000,
  baseUrl: 'http://127.0.0.1:3000',
  ownerWebid: 'https://id.example/alice#me',
  oidc: true,
});

// Later: await server.closeAsync();
```

Options may also come from `SPARQ_SOLID_PORT` (or `PORT`),
`SPARQ_SOLID_BASE_URL`, and `SPARQ_SOLID_OWNER_WEBID`.

## ✨ Scope

- PUT/GET/POST/PATCH/DELETE and WAC run through the real `sparq-lws-core`
  router over in-wasm metadata and blob stores.
- The listener binds only `127.0.0.1`; `baseUrl` is the public pod origin and
  may differ when a local proxy terminates TLS.
- Storage is process-local and disappears on shutdown.
- Authentication defaults to fixed-owner local mode. Set `oidc: true` in the
  Node API to require and verify Solid-OIDC access tokens plus request-bound DPoP
  proofs with `@solid/access-token-verifier`; missing, invalid, expired, replayed,
  or bearer-only credentials stay anonymous. The `npx` command remains fixed-owner.
- OIDC verification runs in Node, not wasm. `baseUrl` must match the public URL
  used in DPoP proofs; the verifier dereferences the WebID and issuer JWKS.
- TLS, persistent storage, notifications, PoP, and native networking remain
  outside the wasm module.
- [GPT-5.6] The shipped full build includes query-only GET/POST `/sparql` for
  SELECT, ASK, and CONSTRUCT. Its dataset contains one named graph per resource
  the authenticated WebID may read; the default graph is empty. SPARQL Update
  is not exposed.
- `npm run build:lws-wasm-core` produces the pure Solid tier with `/sparql` and
  the embedded query engine compiled out.

## 📚 API

`startSolidServer({ port, baseUrl, ownerWebid, oidc })` resolves to a Node
`http.Server` after it is listening. The package adds
`server.closeAsync(): Promise<void>` for clean shutdown. One server instance
reuses one wasm pod, so writes remain visible until shutdown. `ownerWebid`
provisions the root WAC policy; in OIDC mode it is not an authentication claim.

### Bring your own transport

`createSolidPod({ baseUrl, ownerWebid, oidc })` resolves to the same pod behind a
transport-agnostic dispatcher, with no listener attached. `dispatch({ method, url,
rawHeaders, body })` resolves to `{ status, headers, body }` and owns the whole host
contract — the 2 MiB body ceiling (413), wasm-trap recycle (503, never a poisoned
instance), response copy + free, and repeated headers preserved flat in both
directions. Call `pod.free()` when done.

For apps already on [Fastify](https://fastify.dev/) (an optional peer dependency),
register the first-party plugin — it mounts the pod as a catch-all route, keeps
body bytes unparsed for wasm, and maps the body-limit error to the host 413 shape:

```js
import Fastify from 'fastify';
import { solidPod } from '@sparq-org/solid-server/fastify';

const fastify = Fastify();
await fastify.register(solidPod, { baseUrl, ownerWebid });
```

Do not add `@fastify/cors` in front of the pod: the wasm router owns CORS. The
building blocks (`SolidServer` from the root or the `./wasm` subpath, plus the
header/body helpers) are exported for assembling other hosts downstream.

See the [JavaScript/Wasm usage skill](../../skills/javascript-wasm/SKILL.md)
for the raw request adapter and host contract.

## License

[MIT](../../LICENSE).
