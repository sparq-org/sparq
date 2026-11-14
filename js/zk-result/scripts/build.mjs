import { readFileSync, writeFileSync, mkdtempSync, rmSync, renameSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { installPinnedCrs } from './crs.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const assets = fileURLToPath(new URL('../assets/', import.meta.url));
const wasm = fileURLToPath(new URL('../wasm', import.meta.url));
const members = JSON.parse(readFileSync(join(assets, 'members.json')));
const pins = JSON.parse(readFileSync(join(assets, 'crs.pins.json')));
const bindgenVersion = readFileSync(join(root, 'Cargo.lock'), 'utf8').match(/name = "wasm-bindgen"\nversion = "([^"]+)"/)[1];
if (execFileSync('wasm-bindgen', ['--version'], { encoding: 'utf8' }).trim() !== `wasm-bindgen ${bindgenVersion}`) {
  throw new Error('binding generator does not match Cargo.lock');
}
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, stdio: 'inherit' });
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const staging = mkdtempSync(join(assets, '..', '.host-'));
try {
  run('cargo', ['build', '--locked', '-p', 'sparq-zk-result-wasm', '--target', 'wasm32-unknown-unknown', '--release', '-j', '2']);
  const target = resolve(root, process.env.CARGO_TARGET_DIR || 'target');
  const built = join(staging, 'wasm');
  run('wasm-bindgen', [join(target, 'wasm32-unknown-unknown/release/sparq_zk_result_wasm.wasm'), '--target', 'nodejs', '--out-dir', built, '--out-name', 'sparq_zk_result_wasm']);
  renameSync(join(built, 'sparq_zk_result_wasm.js'), join(built, 'sparq_zk_result_wasm.cjs'));
  const host = createRequire(import.meta.url)(join(built, 'sparq_zk_result_wasm.cjs'));
  for (const entry of members) {
    if (host.canonicalVkHash(entry.member) !== digest(join(assets, entry.vk))) throw new Error('canonical VK mismatch');
  }
  installPinnedCrs(pins, join(staging, 'crs'));
  const hostSources = [
    'crates/sparq-zk-result-wasm/src/lib.rs', 'crates/sparq-zk-result-wasm/build.rs',
    'crates/sparq-zk-result-wasm/Cargo.toml', 'crates/sparq-zk-compose/src/result.rs',
    'crates/sparq-zk-compose/src/planner.rs', 'crates/sparq-zk-compose/src/planner/issuer_scope.rs',
    'crates/sparq-zk-compose/src/result/signed.rs', 'crates/sparq-zk-compose/src/nonce.rs',
    'crates/sparq-zk-compose/src/verifier_wasm.rs', 'Cargo.lock', '.cargo/config.toml',
    'js/zk-result/scripts/build.mjs', 'js/zk-result/scripts/crs.mjs', 'js/zk-result/index.js',
    'js/zk-result/assets/members.json', 'js/zk-result/assets/crs.pins.json',
  ];
  const provenance = {
    sourceCommit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
    uncommittedSourceChanges: Boolean(execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim()),
    wasmBindgen: bindgenVersion,
    members: members.map(entry => ({ ...entry, acirSha256: digest(join(assets, entry.acir)), vkSha256: digest(join(assets, entry.vk)) })),
    crs: pins,
    host: Object.fromEntries(['sparq_zk_result_wasm_bg.wasm', 'sparq_zk_result_wasm.cjs'].map(name => [name, digest(join(built, name))])),
    hostSourceFiles: Object.fromEntries(hostSources.map(name => [name, digest(join(root, name))])),
  };
  writeFileSync(join(built, 'provenance.json'), JSON.stringify(provenance, null, 2) + '\n');
  // Keep failed downloads and partial bindings out of the package directories.
  rmSync(wasm, { recursive: true, force: true });
  renameSync(built, wasm);
  mkdirSync(assets, { recursive: true });
  rmSync(join(assets, 'crs'), { recursive: true, force: true });
  renameSync(join(staging, 'crs'), join(assets, 'crs'));
} finally {
  rmSync(staging, { recursive: true, force: true });
}
