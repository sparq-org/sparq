import { readFileSync, writeFileSync, copyFileSync, mkdtempSync, rmSync, renameSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
const root = fileURLToPath(new URL('../../../', import.meta.url));
const assets = fileURLToPath(new URL('../assets/', import.meta.url));
const members = JSON.parse(readFileSync(join(assets, 'members.json')));
const pins = readFileSync(join(root, '.github/workflows/zk-toolchain.yml'), 'utf8');
const nargoVersion = pins.match(/NARGO_VERSION: "([^"]+)"/)[1];
const bbVersion = pins.match(/BB_VERSION: "([^"]+)"/)[1];
const bindgenVersion = readFileSync(join(root, 'Cargo.lock'), 'utf8').match(/name = "wasm-bindgen"\nversion = "([^"]+)"/)[1];
if (execFileSync('nargo', ['--version'], { encoding: 'utf8' }).split('\n')[0] !== `nargo version = ${nargoVersion}` ||
    execFileSync('bb', ['--version'], { encoding: 'utf8' }).trim() !== bbVersion ||
    execFileSync('wasm-bindgen', ['--version'], { encoding: 'utf8' }).trim() !== `wasm-bindgen ${bindgenVersion}`) {
  throw new Error('build toolchain does not match repository pins');
}
const run = (cmd, args, cwd = root) => execFileSync(cmd, args, { cwd, stdio: 'inherit' });
for (const entry of members) {
  run('nargo', ['compile', '--package', entry.member], join(root, 'zk/compose'));
  const compiled = JSON.parse(readFileSync(join(root, 'zk/compose/target', entry.member + '.json')));
  const runtime = Object.fromEntries(['noir_version', 'hash', 'abi', 'bytecode'].map(key => [key, compiled[key]]));
  writeFileSync(join(assets, entry.acir), JSON.stringify(runtime) + '\n');
  const scratch = mkdtempSync(join(tmpdir(), 'sparq-result-vk-'));
  try {
    run('bb', ['write_vk', '-b', join(assets, entry.acir), '-o', scratch, '-t', 'noir-recursive', '-c', join(assets, 'crs')]);
    copyFileSync(join(scratch, 'vk'), join(assets, entry.vk));
  } finally { rmSync(scratch, { recursive: true, force: true }); }
}
run('cargo', ['build', '-p', 'sparq-zk-result-wasm', '--target', 'wasm32-unknown-unknown', '--release', '-j', '2']);
const target = resolve(root, process.env.CARGO_TARGET_DIR || 'target');
const wasm = fileURLToPath(new URL('../wasm/', import.meta.url));
run('wasm-bindgen', [join(target, 'wasm32-unknown-unknown/release/sparq_zk_result_wasm.wasm'), '--target', 'nodejs', '--out-dir', wasm, '--out-name', 'sparq_zk_result_wasm']);
renameSync(join(wasm, 'sparq_zk_result_wasm.js'), join(wasm, 'sparq_zk_result_wasm.cjs'));
// Assert that the built host embeds the exact keys shipped beside the circuits.
const { createRequire } = await import('node:module');
const { createHash } = await import('node:crypto');
const host = createRequire(import.meta.url)(join(wasm, 'sparq_zk_result_wasm.cjs'));
for (const entry of members) {
  const digest = createHash('sha256').update(readFileSync(join(assets, entry.vk))).digest('hex');
  if (host.canonicalVkHash(entry.member) !== digest) throw new Error('canonical VK mismatch');
}

const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const hostSources = [
  'crates/sparq-zk-result-wasm/src/lib.rs', 'crates/sparq-zk-result-wasm/build.rs',
  'crates/sparq-zk-result-wasm/Cargo.toml', 'crates/sparq-zk-compose/src/result.rs',
  'crates/sparq-zk-compose/src/result/signed.rs', 'crates/sparq-zk-compose/src/nonce.rs',
  'crates/sparq-zk-compose/src/verifier_wasm.rs', 'Cargo.lock', '.cargo/config.toml',
];
const provenance = {
  sourceCommit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  uncommittedSourceChanges: Boolean(execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim()),
  toolchain: { nargo: nargoVersion, bb: bbVersion, wasmBindgen: bindgenVersion },
  members: members.map(entry => ({ ...entry, acirSha256: digest(join(assets, entry.acir)), vkSha256: digest(join(assets, entry.vk)) })),
  crs: Object.fromEntries([['bn254_g1.dat', 'g1.dat'], ['bn254_g2.dat', 'g2.dat']].map(([name, remote]) => [name, { source: `https://crs.aztec-cdn.foundation/${remote}`, sha256: digest(join(assets, 'crs', name)) }])),
  host: Object.fromEntries(['sparq_zk_result_wasm_bg.wasm', 'sparq_zk_result_wasm.cjs'].map(name => [name, digest(join(wasm, name))])),
  hostSourceFiles: Object.fromEntries(hostSources.map(name => [name, digest(join(root, name))])),
};
writeFileSync(join(assets, 'provenance.json'), JSON.stringify(provenance, null, 2) + '\n');
