#!/usr/bin/env node
// [GPT-5.6] sq-hcy7c: refuse to pack @sparq-org/solid-server without its wasm runtime.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { NATIVE_PLATFORMS } from '../src/native-platforms.js';

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const expectedName = '@sparq-org/solid-server';
const failures = [];
const fail = (message) => failures.push(message);
const manifest = JSON.parse(readFileSync(resolve(packageDir, 'package.json'), 'utf8'));
// Native platform packages are pinned only at publish time, by native/pin.mjs, in a dispatch
// that also publishes them. Either none is pinned or all are, each at exactly this version.
const pins = manifest.optionalDependencies ?? {};
if (Object.keys(pins).length > 0) {
  const names = Object.keys(NATIVE_PLATFORMS).map((platform) => `@sparq-org/solid-server-native-${platform}`);
  for (const name of names) {
    if (pins[name] !== manifest.version) fail(`${name} must be an optional dependency at exactly ${manifest.version}`);
  }
  for (const name of Object.keys(pins)) {
    if (!names.includes(name)) fail(`unexpected optional dependency ${name}`);
  }
}
try {
  execFileSync(process.execPath, [resolve(packageDir, 'guardrails/check-native-resolution.mjs')], {
    cwd: packageDir, stdio: ['ignore', 'pipe', 'inherit'],
  });
} catch (error) {
  fail(`offline native resolution check failed: ${error.message}`);
}

if (manifest.name !== expectedName) {
  fail(`package name is "${manifest.name}", expected "${expectedName}"`);
}

const prepack = manifest.scripts?.prepack ?? '';
const buildPosition = prepack.indexOf('build:lws-wasm');
const checkPosition = prepack.indexOf('check:package');
if (buildPosition === -1) {
  fail('prepack must run build:lws-wasm so an ad-hoc publish cannot omit the wasm runtime');
}
if (checkPosition === -1) {
  fail('prepack must run check:package so publishing verifies the packed file list');
}
if (buildPosition !== -1 && checkPosition !== -1 && buildPosition > checkPosition) {
  fail('prepack must build the wasm runtime before checking the packed file list');
}

if (!(manifest.files ?? []).includes('wasm')) {
  fail('the files allowlist must include "wasm"');
}

let packedFiles;
try {
  const output = execFileSync('npm', ['pack', '--dry-run', '--json', '--ignore-scripts'], {
    cwd: packageDir,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  packedFiles = JSON.parse(output)[0]?.files ?? [];
} catch (error) {
  fail(`npm pack --dry-run failed: ${error.message}`);
}

if (packedFiles) {
  const packed = new Map(packedFiles.map((file) => [file.path, file]));
  const requireFile = (path, description) => {
    const file = packed.get(path);
    if (!file) {
      fail(`packed tarball is missing ${description} (${path})`);
    } else if (!Number.isFinite(file.size) || file.size <= 0) {
      fail(`packed ${description} is empty (${path})`);
    }
  };

  requireFile((manifest.main ?? '').replace(/^\.\//, ''), 'JavaScript entrypoint');
  requireFile((manifest.types ?? '').replace(/^\.\//, ''), 'type entrypoint');
  for (const [name, path] of Object.entries(manifest.bin ?? {})) {
    requireFile(path.replace(/^\.\//, ''), `executable "${name}"`);
  }
  requireFile('wasm/sparq_lws_wasm.js', 'wasm JavaScript glue');
  requireFile('wasm/sparq_lws_wasm_bg.wasm', 'wasm runtime');
}

if (failures.length > 0) {
  console.error(`check:package FAILED for ${expectedName}:\n${failures.map((message) => `  - ${message}`).join('\n')}`);
  process.exit(1);
}

console.log(`check:package OK — ${expectedName} ships its entrypoints and non-empty wasm runtime.`);
