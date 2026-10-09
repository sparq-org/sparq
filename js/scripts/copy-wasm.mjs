#!/usr/bin/env node
// Copy the `--target web` wasm-pack output (`crates/sparq-wasm/pkg/`) into the package's
// `wasm/` dir: the Node form of the former `rm -rf wasm && mkdir wasm && cp ... wasm/`.
//
// Node, not shell (issue #5414): `npm run build` is invoked by the `prepare` lifecycle on a
// git-pinned install, which also runs on Windows, where npm runs scripts through `cmd.exe`
// and `rm`/`cp` do not exist (cf. scripts/copy-wasm-node.mjs, the `--target nodejs` sibling).
//
// Nothing is written to stdout: this runs inside `npm pack --dry-run --json`, whose stdout
// guardrails/check-package.mjs parses as JSON.
import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const src = resolve(pkgDir, '..', 'crates', 'sparq-wasm', 'pkg');
const dst = resolve(pkgDir, 'wasm');

// All four are required, as they were for `cp` (which failed on any missing source).
const FILES = ['sparq_wasm.js', 'sparq_wasm.d.ts', 'sparq_wasm_bg.wasm', 'sparq_wasm_bg.wasm.d.ts'];

rmSync(dst, { recursive: true, force: true });
mkdirSync(dst);
const missing = FILES.filter((f) => !existsSync(resolve(src, f)));
if (missing.length) {
  console.error(
    `copy-wasm: ${src} is missing ${missing.join(', ')} — run \`npm run build:wasm\` ` +
      '(wasm-pack build ../crates/sparq-wasm --target web) first.',
  );
  process.exit(1);
}
for (const f of FILES) cpSync(resolve(src, f), resolve(dst, f));
