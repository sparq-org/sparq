/**
 * Environment-aware initialisation of the wasm-pack (`--target web`) module:
 * in Node the wasm bytes are read from disk (Node's `fetch` cannot load
 * `file:` URLs); in the browser/Deno the default `fetch`-relative-to-module
 * path is used. Initialisation is idempotent and memoised.
 */
// [OPUS-4.8] sq-jpki.2 — `Store`/`WasmStore` come from the wasm-pack build artifact
// `../wasm/sparq_wasm.js` (the d.ts the published `@sparq-org/sparq` SHIPS — `js/wasm/` is in the
// publish `files` allowlist). That d.ts is NOT a hand-written mirror: it is the wasm-pack
// GENERATED surface, and it is kept BYTE-IDENTICAL to `@sparq/client`'s tracked
// `src/generated/sparq_wasm.d.ts` by `@sparq/client`'s `check:wasm-types` guard. So `js/`,
// the site, and the (proposed) GUI all type against ONE canonical generated `Store` surface
// (research/gui-design.md §0/§4) — `js/` simply references the COPY it ships so the published
// package stays self-contained (a TS consumer of `@sparq-org/sparq` must not need the private
// `@sparq/client` workspace package). `src/wasm-conformance.ts` (dev-only, `noEmit`) asserts
// at compile time that this shipped artifact type is structurally identical to the shared
// `@sparq/client` surface, so the single-source-of-truth relationship is CI-guarded from the
// `js/` side too and the two copies cannot silently diverge.
import initWasm, { Store as WasmStore, canonicalizeNQuads } from '../wasm/sparq_wasm.js';

let ready: Promise<void> | undefined;

declare const process: { versions?: { node?: string } } | undefined;

export function init(): Promise<void> {
  if (!ready) {
    ready = (async () => {
      if (typeof process !== 'undefined' && process?.versions?.node) {
        const { readFile } = await import('node:fs/promises');
        const bytes = await readFile(new URL('../wasm/sparq_wasm_bg.wasm', import.meta.url));
        await initWasm({ module_or_path: bytes });
      } else {
        await initWasm();
      }
    })();
    // Allow a retry if initialisation failed (e.g. transient fetch error).
    ready.catch(() => {
      ready = undefined;
    });
  }
  return ready;
}

// [OPUS-4.8] sq-1dd5t (#1047): RDFC-1.0 (RDF Dataset Canonicalization) free function the
// RDF/JS `Dataset` uses for isomorphism-aware toCanonical / equals / contains. Available
// because the published `@sparq-org/sparq` wasm bundle (js `build:wasm`) enables the `canon`
// Cargo feature; the engine must be `init()`-ed before calling it (it is, by the time a
// `Dataset` instance exists — the async factories await `init()`).
export { WasmStore, canonicalizeNQuads };
