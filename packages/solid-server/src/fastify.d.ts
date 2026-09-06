// [FABLE-5] #2323: types for the `@sparq-org/solid-server/fastify` subpath plugin.
// Typed against a structural stand-in rather than fastify's own types so that `fastify`
// can stay an OPTIONAL peer dependency (no type-level import of an absent package).
import type { SolidPodOptions } from './index.js';

export type { SolidPodOptions };

/**
 * Fastify plugin: mount one in-memory wasm Solid pod on this Fastify context.
 * Register with `await fastify.register(solidPod, { baseUrl, ownerWebid, oidc })`.
 * The `instance` parameter is the FastifyInstance (v4/v5); typed structurally here.
 */
export function solidPod(
  instance: object,
  options: SolidPodOptions,
): Promise<void>;

export default solidPod;
