// [GPT-5.6] sq-6xasp.9/.10: loopback Node listener with optional verified identity into wasm.
// [SONNET-4.6] sq-250si: trap recovery — catch WebAssembly.RuntimeError and recycle the instance.
import { readFile } from 'node:fs/promises';

import initWasm, { SolidServer } from '../wasm/sparq_lws_wasm.js';
import { createOidcAuthenticator } from './auth.js';
import { createPodDispatcher } from './dispatch.js';
import { attachTrapRecoveryHandler, isWasmTrap } from './trap-recovery.js';

export { attachTrapRecoveryHandler, isWasmTrap };
// [FABLE-5] #2323: the host building blocks, so framework/browser hosts can be assembled
// downstream without vendor-patching the tarball. `SolidServer` is the raw wasm pod class
// (construct only after wasm init — `createSolidPod` handles init for Node callers; browser
// hosts should import the `./wasm` subpath and run its own init with fetched bytes).
export { createPodDispatcher, SolidServer };
export {
  MAX_BODY_BYTES,
  RequestBodyTooLargeError,
  copyWasmResponse,
  flattenRequestHeaders,
  readRequestBody,
  writeNodeResponse,
} from './http.js';

const DEFAULT_PORT = 3000;
const DEFAULT_OWNER_WEBID = 'https://example.invalid/profile/card#me';
let wasmReady;

async function init() {
  if (!wasmReady) {
    wasmReady = (async () => {
      const bytes = await readFile(new URL('../wasm/sparq_lws_wasm_bg.wasm', import.meta.url));
      await initWasm({ module_or_path: bytes });
    })();
    wasmReady.catch(() => {
      wasmReady = undefined;
    });
  }
  return wasmReady;
}

function normalizePort(value) {
  const port = value ?? DEFAULT_PORT;
  if (!Number.isInteger(port) || port < 0 || port > 65_535) {
    throw new TypeError('port must be an integer from 0 through 65535');
  }
  return port;
}

function normalizeBaseUrl(value, port) {
  if (value === undefined && port === 0) {
    throw new TypeError('baseUrl is required when port is 0');
  }
  const url = new URL(value ?? `http://127.0.0.1:${port}`);
  if (!['http:', 'https:'].includes(url.protocol)) {
    throw new TypeError('baseUrl must use http or https');
  }
  if (url.username || url.password || url.search || url.hash || url.pathname !== '/') {
    throw new TypeError('baseUrl must be an origin without credentials, path, query, or fragment');
  }
  return url.origin;
}

function normalizeOwnerWebid(value) {
  const ownerWebid = value ?? DEFAULT_OWNER_WEBID;
  const url = new URL(ownerWebid);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
    throw new TypeError('ownerWebid must be an http(s) WebID without credentials');
  }
  return ownerWebid;
}

function normalizeOidc(value) {
  if (value === undefined || value === false) return false;
  if (value === true) return true;
  throw new TypeError('oidc must be a boolean');
}

/**
 * Start a loopback-only Solid/LDP development server.
 *
 * The default fixed-owner mode is deliberately not authentication. With `oidc: true`, the Node
 * host verifies each credential and anonymous requests remain anonymous.
 * The returned Node server owns one in-memory wasm pod until `server.close()` completes.
 *
 * ## Trap recovery (sq-250si)
 *
 * A Rust panic or allocation failure inside the wasm module raises a `WebAssembly.RuntimeError`
 * that permanently poisons the current `SolidServer` instance. When this happens, the host
 * automatically frees the poisoned instance, allocates a fresh one (state loss is acceptable for
 * the ephemeral development server), and responds to the triggering request with HTTP 503. The
 * next request is served by the new instance — a single trap no longer bricks the process
 * indefinitely. Trap events are logged to `console.error`.
 */
/**
 * Create one in-memory wasm Solid pod behind a transport-agnostic dispatcher, without any
 * listener attached — for hosts that already own a transport (the `./fastify` plugin, an
 * embedded harness, a service-worker `fetch` handler bundled for Node).
 *
 * [FABLE-5] #2323: `dispatch({ method, url, rawHeaders, body })` resolves to
 * `{ status, headers, body }` and owns the whole host contract — the 2 MiB body ceiling (413),
 * trap recycle (503, a later request is never served by a poisoned instance), wasm-response
 * copy + free, and repeated-header preservation in both directions. `url` is the origin-form
 * request target including any query string; `rawHeaders` are flat name/value pairs. Call
 * `pod.free()` when done — the pod holds wasm linear memory until then.
 *
 * `baseUrl` is required: with no listener there is no port to derive it from, and OIDC DPoP
 * proofs bind to it. `ownerWebid`/`oidc` behave exactly as in `startSolidServer` — the default
 * fixed-owner mode is deliberately not authentication.
 */
export async function createSolidPod(options = {}) {
  if (options.baseUrl === undefined) {
    throw new TypeError('baseUrl is required (there is no listener port to derive it from)');
  }
  const baseUrl = normalizeBaseUrl(options.baseUrl, DEFAULT_PORT);
  const ownerWebid = normalizeOwnerWebid(options.ownerWebid);
  const oidc = normalizeOidc(options.oidc);

  await init();

  const authenticate = oidc ? createOidcAuthenticator(baseUrl) : async () => ownerWebid;
  const { dispatch, free } = createPodDispatcher(
    () => new SolidServer(baseUrl, ownerWebid),
    { authenticate },
  );
  return { baseUrl, dispatch, free, ownerWebid };
}

export async function startSolidServer(options = {}) {
  const protocol = options.protocol ?? 'solid';
  if (!['solid', 'lws'].includes(protocol)) {
    throw new TypeError('protocol must be solid or lws');
  }
  if (options.native !== undefined && typeof options.native !== 'boolean') {
    throw new TypeError('native must be a boolean');
  }
  if (protocol === 'lws' || options.native) {
    const { startNativeServer } = await import('./native.js');
    return startNativeServer(options, { normalizePort, normalizeBaseUrl });
  }
  if (options.signal !== undefined) {
    throw new TypeError('signal requires --native or --protocol lws');
  }
  const port = normalizePort(options.port);
  const baseUrl = normalizeBaseUrl(options.baseUrl, port);
  const ownerWebid = normalizeOwnerWebid(options.ownerWebid);
  const oidc = normalizeOidc(options.oidc);

  await init();

  const authenticate = oidc ? createOidcAuthenticator(baseUrl) : async () => ownerWebid;
  const makePod = () => new SolidServer(baseUrl, ownerWebid);

  const { server, freePod } = attachTrapRecoveryHandler(makePod, { authenticate });

  try {
    await new Promise((resolve, reject) => {
      const onError = (error) => {
        server.off('listening', onListening);
        reject(error);
      };
      const onListening = () => {
        server.off('error', onError);
        resolve();
      };
      server.once('error', onError);
      server.once('listening', onListening);
      server.listen(port, '127.0.0.1');
    });
  } catch (error) {
    freePod();
    throw error;
  }

  return server;
}
