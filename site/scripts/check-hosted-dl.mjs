#!/usr/bin/env node
// #7031 — guard the self-hosted ESM entry's lazy OWL 2 DL bundle. `/wasm/sparq.js` (from
// scripts/bundle-wasm-esm.mjs) exports `checkOwlDl`, which dynamically imports the sibling
// `./sparq_dl_wasm.js`; that glue then fetches `sparq_dl_wasm_bg.wasm` next to itself. Both must
// be deployed beside the entry (scripts/sync-wasm.mjs copies them), or the first call fails.
//
// Usage: node scripts/check-hosted-dl.mjs [dir]   (default: public/wasm; Pages passes out/wasm)
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dir = resolve(process.argv[2] ?? join(here, "..", "public", "wasm"));
const failures = [];
const entry = join(dir, "sparq.js");
const glue = join(dir, "sparq_dl_wasm.js");
for (const f of [entry, glue, join(dir, "sparq_dl_wasm_bg.wasm")]) {
  if (!existsSync(f)) failures.push(`missing ${f}`);
}
if (failures.length === 0) {
  if (!/import\(\s*["']\.\/sparq_dl_wasm\.js["']\s*\)/.test(readFileSync(entry, "utf8"))) {
    failures.push(`${entry} does not dynamically import "./sparq_dl_wasm.js"`);
  }
  if (!readFileSync(glue, "utf8").includes("sparq_dl_wasm_bg.wasm")) {
    failures.push(`${glue} does not load "sparq_dl_wasm_bg.wasm" relative to itself`);
  }
}
if (failures.length) {
  console.error(`[check-hosted-dl] ${failures.join("\n[check-hosted-dl] ")}`);
  process.exit(1);
}
console.log(`[check-hosted-dl] ${dir}: sparq.js lazily imports the DL bundle deployed beside it`);
