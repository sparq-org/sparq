import { readFileSync, writeFileSync, copyFileSync, mkdtempSync, rmSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { installPinnedCrs } from '../../zk-shared/crs.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const assets = fileURLToPath(new URL('../assets/', import.meta.url));
const members = JSON.parse(readFileSync(join(assets, 'members.json')));
const nativePins = JSON.parse(readFileSync(join(root, 'crates/sparq-zk-compose/tests/result_method_keys.json')));
const pins = readFileSync(join(root, '.github/workflows/zk-toolchain.yml'), 'utf8');
const nargoVersion = pins.match(/NARGO_VERSION: "([^"]+)"/)[1];
const bbVersion = pins.match(/BB_VERSION: "([^"]+)"/)[1];
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
const sourceCommit = git('rev-parse', 'HEAD');
const uncommittedSourceChanges = Boolean(git('status', '--porcelain', '--untracked-files=all', '--',
  'zk/compose', 'zk/dual-leaf-v2', '.github/workflows/zk-toolchain.yml',
  'crates/sparq-zk-compose/tests/result_method_keys.json',
  'js/zk-result/scripts/build-circuits.mjs', 'js/zk-shared/crs.mjs',
  'js/zk-result/assets/members.json', 'js/zk-result/assets/crs.pins.json'));
if (execFileSync('nargo', ['--version'], { encoding: 'utf8' }).split('\n')[0] !== `nargo version = ${nargoVersion}` ||
    execFileSync('bb', ['--version'], { encoding: 'utf8' }).trim() !== bbVersion) {
  throw new Error('circuit toolchain does not match repository pins');
}
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const scratch = mkdtempSync(join(tmpdir(), 'sparq-result-circuits-'));
try {
  const configuredHome = process.env.SPARQ_RESULT_COMPILER_HOME;
  const nargoHome = configuredHome ? resolve(configuredHome) : join(scratch, 'nargo-home');
  mkdirSync(nargoHome, { recursive: true, mode: 0o700 });
  const compile = member => execFileSync('nargo', ['compile', '--package', member], {
    cwd: join(root, 'zk/compose'), stdio: 'inherit', env: { ...process.env, HOME: nargoHome },
  });
  installPinnedCrs(JSON.parse(readFileSync(join(assets, 'crs.pins.json'))), join(scratch, 'crs'));
  for (const entry of members) {
    compile(entry.member);
    const compiled = JSON.parse(readFileSync(join(root, 'zk/compose/target', entry.member + '.json')));
    const runtime = Object.fromEntries(['noir_version', 'hash', 'abi', 'bytecode'].map(key => [key, compiled[key]]));
    writeFileSync(join(assets, entry.acir), JSON.stringify(runtime) + '\n');
    execFileSync('bb', ['write_vk', '-b', join(assets, entry.acir), '-o', scratch, '-t', 'noir-recursive', '-c', join(scratch, 'crs')], {
      cwd: root, stdio: 'inherit',
    });
    if (nativePins.target !== 'noir-recursive' ||
        digest(join(scratch, 'vk')) !== nativePins.members[entry.member]?.vk_sha256) {
      throw new Error(`native canonical VK mismatch: ${entry.member}`);
    }
    copyFileSync(join(scratch, 'vk'), join(assets, entry.vk));
  }
  writeFileSync(join(assets, 'provenance.json'), JSON.stringify({
    sourceCommit,
    uncommittedSourceChanges,
    toolchain: { nargo: nargoVersion, bb: bbVersion },
    members: members.map(entry => ({ ...entry, acirSha256: digest(join(assets, entry.acir)), vkSha256: digest(join(assets, entry.vk)) })),
  }, null, 2) + '\n');
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
