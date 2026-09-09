// [OPUS-4.8] sq-55w5a (#981) — publish a SELF-HOSTED, ESM-importable artifact that exposes
// the named `Dataset` (and the rest of the `@sparq-org/sparq` RDF/JS surface) from the project's
// OWN GitHub Pages origin, so the literal `<script type="module">` snippet the maintainer's
// issue asked for works WITHOUT a third-party ESM CDN:
//
//     <script type="module">
//       import { Dataset } from "https://sparq.jeswr.org/wasm/sparq.js";
//       const ds = await Dataset.fromString('<a> <b> <c> .', 'ntriples'); // wasm lazy-fetched HERE
//     </script>
//
// WHY a bundle (not a raw file copy). The `@sparq-org/sparq` source (js/src) is many ESM modules
// with bare imports (`fzstd` for zstd ingest, `@rdfjs/types` for types) that a browser
// `<script type="module">` from a static origin cannot resolve. esbuild collapses the whole
// graph into ONE self-contained file, inlining `fzstd` (~8 kB, pure JS) and erasing the
// type-only `@rdfjs/types` import — so the artifact has zero runtime dependency on any CDN.
//
// WHY the wasm binary stays OUT of the bundle (the #981 lazy-load posture). The wasm-pack
// `--target web` glue (`sparq_wasm.js`) is marked EXTERNAL and rewritten to a sibling
// `./sparq_wasm.js` specifier. So:
//   * the ~MB `sparq_wasm_bg.wasm` is NEVER inlined into `sparq.js` — importing the module is
//     cheap; the binary is fetched LAZILY by the first `await Dataset.…` (which awaits the
//     engine's memoised `init()`), exactly as the npm-package path already behaves;
//   * the glue resolves `sparq_wasm_bg.wasm` via its own `import.meta.url`, i.e. relative to
//     `/sparq/wasm/`, so the sibling-file layout `sync-wasm.mjs` already produces just works.
//
// `node:`-builtin dynamic imports (`node:fs/promises`, `node:zlib`) are marked external: they
// live behind an `isNode()` guard in the source and are never evaluated in a browser, so an
// unresolved external `node:` specifier in dead-on-browser code is harmless — and keeping them
// external lets the SAME artifact be `import`ed from Node too.
import { access } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";

const here = dirname(fileURLToPath(import.meta.url));
const entry = join(here, "..", "..", "js", "src", "index.ts");
const dest = join(here, "..", "public", "wasm", "sparq.js");
const glue = join(here, "..", "public", "wasm", "sparq_wasm.js");

// The bundle re-exports the wasm-pack glue's init/Store via a SIBLING specifier; the glue must
// already be synced next to it (sync-wasm.mjs runs first in the prebuild chain).
try {
  await access(glue);
} catch {
  console.error(
    `[bundle-wasm-esm] ${glue} not found.\n` +
      `Run scripts/sync-wasm.mjs first (the prebuild chain does this).`,
  );
  process.exit(1);
}

// Rewrite the source's `../wasm/sparq_wasm.js` (relative to js/src) to a sibling `./sparq_wasm.js`
// (relative to the emitted /public/wasm/sparq.js) and keep it EXTERNAL so the heavy wasm binary
// is lazy-loaded, never inlined. The plugin matches the exact path the js/ source imports.
const externaliseGlue = {
  name: "externalise-wasm-glue",
  setup(b) {
    b.onResolve({ filter: /(^|\/)sparq_wasm\.js$/ }, () => ({
      path: "./sparq_wasm.js",
      external: true,
    }));
  },
};

await build({
  entryPoints: [entry],
  outfile: dest,
  bundle: true,
  format: "esm",
  platform: "browser",
  target: "es2022",
  // Keep the Node-only dynamic imports external — they sit behind an isNode() guard and never
  // run in a browser, but staying external means the artifact also works when imported in Node.
  external: ["node:fs/promises", "node:zlib"],
  plugins: [externaliseGlue],
  legalComments: "none",
  minify: true,
  sourcemap: false,
  // A small banner so the published artifact is self-describing for anyone who opens it.
  banner: {
    js:
      "/* sparq — RDF/JS Dataset + SPARQL engine (Rust→WASM), self-hosted ESM build.\n" +
      "   import { Dataset } from \"https://sparq.jeswr.org/wasm/sparq.js\"\n" +
      "   The ~MB engine wasm is fetched lazily by the first `await Dataset.…`, not by this import.\n" +
      "   Docs: https://sparq.jeswr.org/surface/javascript-wasm  •  npm: @sparq-org/sparq */",
  },
});

console.log(`[bundle-wasm-esm] wrote self-hosted ESM entry → public/wasm/sparq.js`);
