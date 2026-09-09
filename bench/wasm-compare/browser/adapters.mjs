// [FABLE-5] sq-hmd7l.17 — Node-side library adapters for the cross-LIBRARY
// comparison (compare.mjs). Each adapter exposes the surface documented in
// compare-workload.mjs. Competitor packages are GATHER-ONLY installs (see
// COMPETITOR_PINS / INSTALL_HINT there) — a missing package yields
// `{ missing: true, reason }` so the orchestrator can skip WITH NOTICE,
// never fabricate.

import path from "node:path";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { INSTALL_HINT } from "./compare-workload.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..", "..");

export const LIBRARIES = ["sparq", "oxigraph", "n3js-quadstore"];

export async function pkgVersion(name) {
  try {
    const raw = await readFile(path.join(HERE, "node_modules", name, "package.json"), "utf8");
    return JSON.parse(raw).version;
  } catch {
    return null;
  }
}

function missing(library, pkgs, err) {
  return {
    missing: true,
    library,
    reason:
      `${library}: cannot import ${pkgs} (${String(err?.message ?? err).split("\n")[0]}) — ` +
      `gather-only install in bench/wasm-compare/browser/: \`${INSTALL_HINT}\``,
  };
}

/** sparq — the shipped @sparq-org/sparq web bundle, run under Node. */
async function sparqAdapter() {
  const gluePath = path.join(REPO, "js", "wasm", "sparq_wasm.js");
  let glue;
  try {
    glue = await import(gluePath);
  } catch (err) {
    return {
      missing: true,
      library: "sparq",
      reason:
        `sparq: shipped bundle not found at js/wasm/ (${String(err?.message ?? err).split("\n")[0]}) — ` +
        "build it: repo root `npm ci --ignore-scripts`, then `cd js && npm run build:wasm`",
    };
  }
  const wasmBytes = await readFile(path.join(REPO, "js", "wasm", "sparq_wasm_bg.wasm"));
  await glue.default({ module_or_path: await WebAssembly.compile(wasmBytes) });
  return {
    library: "sparq",
    version: JSON.parse(await readFile(path.join(REPO, "js", "package.json"), "utf8")).version + " (local shipped bundle)",
    // Store.load constructs the store: newStore is a no-op handle.
    newStore: () => ({}),
    load: (handle, text, format) => {
      handle.store = glue.Store.load(text, format === "ntriples" ? "ntriples" : "turtle");
    },
    size: (handle) => handle.store.size,
    queryCount: (handle, sparql) => JSON.parse(handle.store.query(sparql)).results.bindings.length,
    free: (handle) => handle.store?.free?.(),
  };
}

/** oxigraph — the official npm WASM package (Node build), pinned gather-only. */
async function oxigraphAdapter() {
  let oxigraph;
  try {
    oxigraph = await import("oxigraph");
  } catch (err) {
    return missing("oxigraph", "oxigraph", err);
  }
  const FORMATS = { ntriples: "application/n-triples", turtle: "text/turtle" };
  return {
    library: "oxigraph",
    version: await pkgVersion("oxigraph"),
    newStore: () => new oxigraph.Store(),
    load: (store, text, format) => store.load(text, { format: FORMATS[format] }),
    size: (store) => store.size,
    queryCount: (store, sparql) => store.query(sparql).length,
  };
}

/** n3js-quadstore — N3.js parse + quadstore (memory-level) + quadstore-comunica SPARQL. */
async function n3QuadstoreAdapter() {
  let n3, Quadstore, Engine, MemoryLevel;
  try {
    n3 = await import("n3");
    ({ Quadstore } = await import("quadstore"));
    ({ Engine } = await import("quadstore-comunica"));
    ({ MemoryLevel } = await import("memory-level"));
  } catch (err) {
    return missing("n3js-quadstore", "n3/quadstore/quadstore-comunica/memory-level", err);
  }
  const FORMATS = { ntriples: "N-Triples", turtle: "Turtle" };
  const versions = await Promise.all(["n3", "quadstore", "quadstore-comunica", "memory-level"].map(pkgVersion));
  return {
    library: "n3js-quadstore",
    version: `n3 ${versions[0]} + quadstore ${versions[1]} + quadstore-comunica ${versions[2]} + memory-level ${versions[3]}`,
    newStore: async () => {
      const store = new Quadstore({ backend: new MemoryLevel(), dataFactory: n3.DataFactory });
      await store.open();
      return { store, engine: new Engine(store) };
    },
    // load = parse (N3.js) + put (quadstore): the same text→queryable-store
    // boundary the wasm engines' load measures.
    load: async ({ store }, text, format) => {
      const quads = new n3.Parser({ format: FORMATS[format] }).parse(text);
      const CHUNK = 10_000;
      for (let i = 0; i < quads.length; i += CHUNK) {
        await store.multiPut(quads.slice(i, i + CHUNK));
      }
    },
    size: async ({ store }) => {
      const { items } = await store.get({});
      return items.length;
    },
    queryCount: async ({ engine }, sparql) => {
      const stream = await engine.queryBindings(sparql);
      return await new Promise((resolve, reject) => {
        let count = 0;
        stream.on("data", () => count++);
        stream.on("end", () => resolve(count));
        stream.on("error", reject);
      });
    },
    free: ({ store }) => store?.close?.(),
  };
}

/** Builds the adapter for `library`, or `{ missing: true, reason }`. */
export async function makeAdapter(library) {
  switch (library) {
    case "sparq":
      return sparqAdapter();
    case "oxigraph":
      return oxigraphAdapter();
    case "n3js-quadstore":
      return n3QuadstoreAdapter();
    default:
      throw new Error(`unknown library '${library}' (expected ${LIBRARIES.join("/")})`);
  }
}
