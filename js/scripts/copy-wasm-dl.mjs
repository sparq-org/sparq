#!/usr/bin/env node
// Copy the `--target web` wasm-pack output of the OWL 2 DL reasoner bundle
// (`crates/sparq-dl-wasm/pkg/`) into the package's `wasm-dl/` dir. `src/owl-dl.ts` imports it
// dynamically on the first `checkOwlDl` call, so the main entry never loads it.
//
// Node, not shell, for the same reason as scripts/copy-wasm.mjs (issue #5414): `npm run
// build` runs from the `prepare` lifecycle, which also runs on Windows.
//
// Nothing is written to stdout: this runs inside `npm pack --dry-run --json`, whose stdout
// guardrails/check-package.mjs parses as JSON.
import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const src = resolve(pkgDir, '..', 'crates', 'sparq-dl-wasm', 'pkg');
const dst = resolve(pkgDir, 'wasm-dl');

const FILES = ['sparq_dl_wasm.js', 'sparq_dl_wasm.d.ts', 'sparq_dl_wasm_bg.wasm', 'sparq_dl_wasm_bg.wasm.d.ts'];

const missing = FILES.filter((f) => !existsSync(resolve(src, f)));
if (missing.length) {
  console.error(
    `copy-wasm-dl: ${src} is missing ${missing.join(', ')} — run \`npm run build:wasm:dl\` ` +
      '(wasm-pack build ../crates/sparq-dl-wasm --target web) first.',
  );
  process.exit(1);
}
rmSync(dst, { recursive: true, force: true });
mkdirSync(dst);
for (const f of FILES) cpSync(resolve(src, f), resolve(dst, f));
