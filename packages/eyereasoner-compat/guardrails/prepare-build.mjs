#!/usr/bin/env node
// [FABLE-5] sq-ohnj1 — `prepare` lifecycle for the @sparq-org/eyereasoner-compat git-pin path.
// Mirrors js/guardrails/prepare-build.mjs: build dist/ + wasm/ on a git-URL install (both are
// .gitignored) when it is NECESSARY and POSSIBLE; skip cleanly otherwise. Does NOT run when a
// consumer installs the published registry tarball (that already ships dist/ + wasm/).
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const wasmArtifact = resolve(pkgDir, 'wasm', 'sparq_reason_wasm_bg.wasm');
const distEntry = resolve(pkgDir, 'dist', 'index.js');
const sourceCrate = resolve(pkgDir, '..', '..', 'crates', 'sparq-reason-wasm', 'Cargo.toml');

if (existsSync(wasmArtifact) && existsSync(distEntry)) process.exit(0); // already built
if (!existsSync(sourceCrate)) process.exit(0); // not a source/git checkout — nothing to build

const hasWasmPack = spawnSync('wasm-pack', ['--version'], { stdio: 'ignore' }).status === 0;
if (!hasWasmPack) {
  console.error(
    [
      'prepare: @sparq-org/eyereasoner-compat is pinned from a git build but cannot build its',
      'wasm engine — `wasm-pack` was not found on PATH. Install the toolchain, then reinstall:',
      '',
      '    rustup target add wasm32-unknown-unknown',
      '    cargo install wasm-pack --locked --version =0.15.0',
      '',
      // [OPUS-5] #5777: same pin as js/guardrails/prepare-build.mjs — the `with: version:`
      // input to jetli/wasm-pack-action in .github/workflows/js.yml. `=0.15.0` is cargo's
      // exact requirement; a bare `0.15.0` would float across the 0.15.x range, and each
      // wasm-pack release carries its own wasm-bindgen CLI. Held in step with the workflow
      // by scripts/tests/test_js_wasm_pack_install.py.
      'That is the wasm-pack CI installs; an unpinned one bundles a wasm-bindgen CLI no lane tests.',
      '',
      'Or depend on the published @sparq-org/eyereasoner-compat registry tarball (ships prebuilt).',
    ].join('\n'),
  );
  process.exit(1);
}

console.log('prepare: building @sparq-org/eyereasoner-compat (wasm-pack + tsc) for the git-pin install...');
// `shell: true` so `npm` resolves to `npm.cmd` on Windows (see js/guardrails/prepare-build.mjs).
execFileSync('npm', ['run', 'build'], { cwd: pkgDir, stdio: 'inherit', shell: true });
