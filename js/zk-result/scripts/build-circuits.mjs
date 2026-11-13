import { readFileSync, writeFileSync, copyFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { installPinnedCrs } from './crs.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const assets = fileURLToPath(new URL('../assets/', import.meta.url));
const members = JSON.parse(readFileSync(join(assets, 'members.json')));
const pins = readFileSync(join(root, '.github/workflows/zk-toolchain.yml'), 'utf8');
const nargoVersion = pins.match(/NARGO_VERSION: "([^"]+)"/)[1];
const bbVersion = pins.match(/BB_VERSION: "([^"]+)"/)[1];
if (execFileSync('nargo', ['--version'], { encoding: 'utf8' }).split('\n')[0] !== `nargo version = ${nargoVersion}` ||
    execFileSync('bb', ['--version'], { encoding: 'utf8' }).trim() !== bbVersion) {
  throw new Error('circuit toolchain does not match repository pins');
}
const run = (cmd, args, cwd = root) => execFileSync(cmd, args, { cwd, stdio: 'inherit' });
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const scratch = mkdtempSync(join(tmpdir(), 'sparq-result-circuits-'));
try {
  installPinnedCrs(JSON.parse(readFileSync(join(assets, 'crs.pins.json'))), join(scratch, 'crs'));
  for (const entry of members) {
    run('nargo', ['compile', '--package', entry.member], join(root, 'zk/compose'));
    const compiled = JSON.parse(readFileSync(join(root, 'zk/compose/target', entry.member + '.json')));
    const runtime = Object.fromEntries(['noir_version', 'hash', 'abi', 'bytecode'].map(key => [key, compiled[key]]));
    writeFileSync(join(assets, entry.acir), JSON.stringify(runtime) + '\n');
    run('bb', ['write_vk', '-b', join(assets, entry.acir), '-o', scratch, '-t', 'noir-recursive', '-c', join(scratch, 'crs')]);
    copyFileSync(join(scratch, 'vk'), join(assets, entry.vk));
  }
  writeFileSync(join(assets, 'provenance.json'), JSON.stringify({
    sourceCommit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
    uncommittedSourceChanges: Boolean(execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim()),
    toolchain: { nargo: nargoVersion, bb: bbVersion },
    members: members.map(entry => ({ ...entry, acirSha256: digest(join(assets, entry.acir)), vkSha256: digest(join(assets, entry.vk)) })),
  }, null, 2) + '\n');
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
