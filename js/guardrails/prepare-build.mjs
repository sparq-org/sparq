#!/usr/bin/env node
// `prepare` lifecycle for the `@sparq-org/sparq` git-pin path. [OPUS-4.8] sq-bkag.
//
// npm runs `prepare` (a) when a consumer installs this package from a GIT URL
// (`github:sparq-org/sparq#<sha>` with `directory: "js"`), and (b) on a maintainer's
// plain `npm install` in a source checkout. It does NOT run when installing the
// published REGISTRY tarball (that already ships dist/ + wasm/).
//
// The git-pin consumer needs dist/ + wasm/ BUILT on install, because both are
// .gitignored and absent from a git checkout. So here we build — but only when
// it is both NECESSARY and POSSIBLE, to avoid breaking the maintainer's
// `npm ci` (which installs wasm-pack AFTER deps, and runs an explicit build):
//
//   * Skip when the wasm artifact is already present (already built, or the
//     maintainer will build explicitly) AND dist/ exists — nothing to do.
//   * Skip when the Rust source crate is not reachable (e.g. a published tarball
//     edge case) — there is nothing to build from.
//   * Skip when wasm-pack is unavailable: emit a LOUD, actionable error telling
//     the git-pin consumer exactly what to install, and exit non-zero so the
//     broken-binding state is impossible to miss. (cf. sq-bkag git-pin guard.)

import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const wasmArtifact = resolve(pkgDir, 'wasm', 'sparq_wasm_bg.wasm');
// [OPUS-5] sq-2hk — the `--target nodejs` CommonJS sibling is part of a complete
// build too, so a tree that has wasm/ + dist/ but no wasm-node/ is NOT "already
// built": skipping there would hand a git-pin consumer a package whose
// `./wasm-node` export resolves to nothing.
const wasmNodeArtifact = resolve(pkgDir, 'wasm-node', 'sparq_wasm_bg.wasm');
const distEntry = resolve(pkgDir, 'dist', 'index.js');
const sourceCrate = resolve(pkgDir, '..', 'crates', 'sparq-wasm', 'Cargo.toml');

// Already built (or shipping a built tree) — nothing to do.
if (existsSync(wasmArtifact) && existsSync(wasmNodeArtifact) && existsSync(distEntry)) {
  process.exit(0);
}

// No source crate to build from — not a git/source checkout; leave it be.
if (!existsSync(sourceCrate)) {
  process.exit(0);
}

const hasWasmPack = spawnSync('wasm-pack', ['--version'], { stdio: 'ignore' }).status === 0;
if (!hasWasmPack) {
  console.error(
    [
      'prepare: @sparq-org/sparq is pinned from a git build but cannot build the wasm engine.',
      'A git-pinned install must compile crates/sparq-wasm to WebAssembly on install, but',
      '`wasm-pack` was not found on PATH. Install the toolchain, then reinstall:',
      '',
      '    rustup target add wasm32-unknown-unknown',
      '    cargo install wasm-pack --locked',
      '',
      'Or depend on the published @sparq-org/sparq registry tarball (ships prebuilt dist/ + wasm/).',
    ].join('\n'),
  );
  process.exit(1);
}

// STDERR, not stdout, for the same reason as scripts/copy-wasm-node.mjs (#3396): `prepare`
// runs inside `npm pack --dry-run --json`, whose STDOUT is a machine-read data channel —
// guardrails/check-package.mjs captures it and parses it as JSON. A progress line on stdout
// lands ahead of that JSON and breaks the parse, which the guardrail then misreports as the
// package being unpackable. CI does not hit it today only because `prepack` has already
// built wasm/ + wasm-node/ + dist/ by then, so the silent early-exit above fires first.
console.error('prepare: building @sparq-org/sparq (wasm-pack + tsc) for the git-pin install...');
// [OPUS-4.8] sq-gl3cf — `shell: true` so this resolves the `npm` launcher on WINDOWS too.
// Without it, execFileSync('npm', …) does no PATHEXT/`.cmd` resolution and dies with
// `spawnSync npm ENOENT` (errno -4058) on Windows — which is what failed the win-x64
// `GUI desktop bundle` row in release.yml at root `npm install` (run 27890097353): the
// root install runs this `prepare` hook for the @sparq-org/sparq workspace member, and its
// crash also triggered npm's rollback `EPERM rmdir node_modules/next` cleanup warnings.
// `shell: true` spawns via `cmd /c` on Windows (resolves `npm.cmd`) and `/bin/sh -c` on
// POSIX (unchanged behavior there). The args are a fixed literal list with no shell
// metacharacters, so there is no injection surface.
execFileSync('npm', ['run', 'build'], { cwd: pkgDir, stdio: 'inherit', shell: true });
