// Research grade and not externally audited.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdtempSync, renameSync, rmSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { syncShared } from '../../zk-shared/sync.mjs';
import { installPinnedCrs } from '../../zk-shared/crs.mjs';
const directory = fileURLToPath(new URL('../', import.meta.url));
const root = fileURLToPath(new URL('../../../', import.meta.url));
const version = readFileSync(join(root, 'Cargo.lock'), 'utf8').match(/name = "wasm-bindgen"\nversion = "([^"]+)"/)[1];
if (execFileSync('wasm-bindgen', ['--version'], { encoding: 'utf8' }).trim() !== `wasm-bindgen ${version}`) throw new Error('binding generator does not match Cargo.lock');
syncShared(directory);
const staging = mkdtempSync(join(directory, '.host-'));
try {
  execFileSync('cargo', ['build', '--locked', '-p', 'sparq-zk-query-wasm', '--target', 'wasm32-unknown-unknown', '--release'], { cwd: root, stdio: 'inherit' });
  const target = resolve(root, process.env.CARGO_TARGET_DIR || 'target');
  const built = join(staging, 'wasm');
  execFileSync('wasm-bindgen', [join(target, 'wasm32-unknown-unknown/release/sparq_zk_query_wasm.wasm'), '--target', 'nodejs', '--out-dir', built, '--out-name', 'sparq_zk_query_wasm'], { cwd: root, stdio: 'inherit' });
  renameSync(join(built, 'sparq_zk_query_wasm.js'), join(built, 'sparq_zk_query_wasm.cjs'));
  const pins = JSON.parse(readFileSync(join(directory, 'assets/crs.pins.json')));
  installPinnedCrs(pins, join(staging, 'crs'));
  const metadata = { sourceCommit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(), uncommittedSourceChanges: Boolean(execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim()), wasmBindgen: version };
  writeFileSync(join(built, 'provenance.json'), JSON.stringify(metadata, null, 2) + '\n');
  rmSync(join(directory, 'wasm'), { recursive: true, force: true });
  renameSync(built, join(directory, 'wasm'));
  mkdirSync(join(directory, 'assets'), { recursive: true });
  rmSync(join(directory, 'assets/crs'), { recursive: true, force: true });
  renameSync(join(staging, 'crs'), join(directory, 'assets/crs'));
} finally { rmSync(staging, { recursive: true, force: true }); }
