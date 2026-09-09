// [FABLE-5] sq-hmd7l.17 — browser side of the cross-LIBRARY comparison.
//
// `?lib=` selects the library; both are wasm-bindgen web-target modules
// instantiated the same way, then driven through the SAME reduced workload
// (compare-workload.mjs → the sq-3ul2n.1 generators + query shapes + oracle):
//   sparq    — the shipped @sparq-org/sparq bundle (/js/wasm/).
//   oxigraph — the pinned npm package's web build, served from the harness
//              dir's gather-only node_modules (/nm/oxigraph/web.js).
// n3js-quadstore is a Node-runtime column (per bench/competitors.json): the
// quadstore stack needs a bundler for browser use, so the orchestrator skips
// it here WITH NOTICE rather than measuring something else.
//
// Results land on `window.__WASM_COMPARE_RESULT__` (+ `__WASM_COMPARE_DONE__`)
// for compare.mjs to collect. All timings ADVISORY / NON-CANONICAL.

import { runCompareWorkload } from "../compare-workload.mjs";

const logEl = document.getElementById("log");
const log = (msg) => {
  logEl.textContent += `\n${msg}`;
  console.log(`[compare] ${msg}`);
};

async function makeBrowserAdapter(library) {
  if (library === "sparq") {
    const glue = await import("/js/wasm/sparq_wasm.js");
    await glue.default(); // fetch + instantiateStreaming of /js/wasm/sparq_wasm_bg.wasm
    return {
      library,
      newStore: () => ({}),
      load: (handle, text, format) => {
        handle.store = glue.Store.load(text, format === "ntriples" ? "ntriples" : "turtle");
      },
      size: (handle) => handle.store.size,
      queryCount: (handle, sparql) => JSON.parse(handle.store.query(sparql)).results.bindings.length,
      free: (handle) => handle.store?.free?.(),
    };
  }
  if (library === "oxigraph") {
    const oxigraph = await import("/nm/oxigraph/web.js");
    await oxigraph.default(); // fetches /nm/oxigraph/web_bg.wasm
    const FORMATS = { ntriples: "application/n-triples", turtle: "text/turtle" };
    return {
      library,
      newStore: () => new oxigraph.Store(),
      load: (store, text, format) => store.load(text, { format: FORMATS[format] }),
      size: (store) => store.size,
      queryCount: (store, sparql) => store.query(sparql).length,
    };
  }
  throw new Error(`unsupported browser library '${library}'`);
}

async function main() {
  const params = new URLSearchParams(location.search);
  const library = params.get("lib") ?? "sparq";
  const quick = params.get("quick") === "1";
  const adapter = await makeBrowserAdapter(library);
  log(`library ${library} instantiated`);
  const { rows, skipped } = await runCompareWorkload({ adapter, quick, log });
  return { rows, skipped, library };
}

main()
  .then((result) => {
    window.__WASM_COMPARE_RESULT__ = { ok: true, ...result };
    window.__WASM_COMPARE_DONE__ = true;
    log("done ✓");
  })
  .catch((err) => {
    window.__WASM_COMPARE_RESULT__ = { ok: false, error: String(err?.stack ?? err) };
    window.__WASM_COMPARE_DONE__ = true;
    log(`FAILED: ${err}`);
  });
