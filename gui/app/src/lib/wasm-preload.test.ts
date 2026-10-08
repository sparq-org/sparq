// #2422 — the layout's engine preload hints must name exactly the URLs the runtime loader
// requests, or the browser downloads the engine wasm twice.

import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { enginePreloadHrefs, readWasmManifest } from "./wasm-preload.js";

test("preload hints use the content-hashed names from the manifest", () => {
  const manifest = {
    "sparq_wasm.js": "sparq_wasm-0123abcd.js",
    "sparq_wasm_bg.wasm": "sparq_wasm_bg-89abcdef.wasm",
  };
  assert.deepEqual(enginePreloadHrefs(manifest, "/app"), {
    glue: "/app/wasm/sparq_wasm-0123abcd.js",
    wasm: "/app/wasm/sparq_wasm_bg-89abcdef.wasm",
  });
});

test("preload hints fall back to the unhashed names without a manifest", () => {
  assert.deepEqual(enginePreloadHrefs(null, ""), {
    glue: "/wasm/sparq_wasm.js",
    wasm: "/wasm/sparq_wasm_bg.wasm",
  });
});

test("readWasmManifest reads public/wasm/wasm-manifest.json and tolerates its absence", () => {
  const dir = mkdtempSync(join(tmpdir(), "sparq-wasm-preload-"));
  assert.equal(readWasmManifest(dir), null);
  mkdirSync(join(dir, "wasm"));
  writeFileSync(
    join(dir, "wasm", "wasm-manifest.json"),
    JSON.stringify({ "sparq_wasm_bg.wasm": "sparq_wasm_bg-1.wasm" }),
  );
  assert.deepEqual(readWasmManifest(dir), { "sparq_wasm_bg.wasm": "sparq_wasm_bg-1.wasm" });
});
