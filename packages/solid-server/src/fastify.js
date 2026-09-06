// [FABLE-5] #2323: first-party Fastify plugin over the transport-agnostic pod dispatcher.
// `fastify` itself is an OPTIONAL peer dependency — this module never imports it, it only
// receives an instance via `fastify.register(solidPod, { baseUrl, ownerWebid, oidc })`, so the
// core package keeps its single runtime dependency. Registered without `fastify-plugin` on
// purpose: the plugin's content-type parsers and error handler stay encapsulated in its own
// context and never leak onto sibling application routes.
import { MAX_BODY_BYTES } from './http.js';
import { createSolidPod } from './index.js';

const TEXT_PLAIN = 'text/plain; charset=utf-8';

/** Group flat name/value response pairs into the object shape `reply.headers()` accepts,
 *  with repeated fields (`Link`, `Vary`, ...) preserved as array values. */
function groupResponseHeaders(flat) {
  const grouped = {};
  for (let index = 0; index < flat.length; index += 2) {
    const name = flat[index].toLowerCase();
    const value = flat[index + 1];
    const existing = grouped[name];
    if (existing === undefined) {
      grouped[name] = value;
    } else if (Array.isArray(existing)) {
      existing.push(value);
    } else {
      grouped[name] = [existing, value];
    }
  }
  return grouped;
}

/**
 * Fastify plugin: mount one in-memory wasm Solid pod on this Fastify context.
 *
 * ```js
 * import { solidPod } from '@sparq-org/solid-server/fastify';
 * await fastify.register(solidPod, { baseUrl, ownerWebid, oidc });
 * ```
 *
 * Everything is mechanical over `createSolidPod().dispatch()`:
 * - all content-type parsers are replaced with a single `'*'` buffer parser so JSON-LD/N3/
 *   Turtle bytes reach wasm unparsed, capped at the host's 2 MiB ceiling;
 * - `FST_ERR_CTP_BODY_TOO_LARGE` is mapped to the host's plain-text 413 shape;
 * - one catch-all `all('/*')` route (`exposeHeadRoutes: false` — HEAD is already in the set)
 *   forwards `request.raw.url` + `request.raw.rawHeaders`, so repeated request headers and
 *   `/sparql?query=` targets survive intact;
 * - response headers go out grouped with array values, so repeated `Link`/`Vary` fields are
 *   preserved;
 * - no `@fastify/cors` — the wasm router owns CORS.
 * The pod is freed on `fastify.close()`.
 */
export async function solidPod(fastify, options = {}) {
  const pod = await createSolidPod({
    baseUrl: options.baseUrl,
    oidc: options.oidc,
    ownerWebid: options.ownerWebid,
  });

  fastify.addHook('onClose', async () => {
    pod.free();
  });

  fastify.removeAllContentTypeParsers();
  fastify.addContentTypeParser(
    '*',
    { bodyLimit: MAX_BODY_BYTES, parseAs: 'buffer' },
    (_request, payload, done) => {
      done(null, payload);
    },
  );

  fastify.setErrorHandler((error, _request, reply) => {
    if (error?.code === 'FST_ERR_CTP_BODY_TOO_LARGE') {
      reply.status(413).header('content-type', TEXT_PLAIN).send('request body too large\n');
      return;
    }
    reply.send(error);
  });

  fastify.all('/*', { exposeHeadRoutes: false }, async (request, reply) => {
    const result = await pod.dispatch({
      body: request.body ?? undefined,
      method: request.raw.method ?? 'GET',
      rawHeaders: request.raw.rawHeaders,
      url: request.raw.url ?? '/',
    });
    reply.status(result.status);
    reply.headers(groupResponseHeaders(result.headers));
    return reply.send(result.body);
  });
}

export default solidPod;
