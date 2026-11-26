// Research grade and not externally audited. Maintainer publication lock.
import { spawnSync } from 'node:child_process';
import { closeSync, existsSync, linkSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const pendingPath = assets => join(assets, '..', '.catalogue-pending');
const lockDescriptor = `
import fcntl
try:
    fcntl.flock(3, fcntl.LOCK_EX | fcntl.LOCK_NB)
except BlockingIOError:
    raise SystemExit(75)
`;

function rollbackCatalogue(assets) {
  const pending = pendingPath(assets);
  const manifest = join(pending, 'manifest.json');
  if (existsSync(manifest)) {
    for (const name of JSON.parse(readFileSync(manifest, 'utf8'))) {
      // A staged file disappears only once its replacement has reached assets.
      if (existsSync(join(pending, name))) continue;
      const original = join(pending, 'previous', name);
      const destination = join(assets, name);
      if (existsSync(original)) {
        const restore = join(pending, 'restore');
        rmSync(restore, { force: true });
        linkSync(original, restore);
        renameSync(restore, destination);
      } else {
        rmSync(destination, { force: true });
      }
    }
    unlinkSync(manifest);
  }
  rmSync(pending, { recursive: true, force: true });
}

export function recoverCatalogue(assets) {
  if (existsSync(pendingPath(assets))) withCatalogueLock(assets, () => {});
}

export function publishCatalogue(assets, staging, names) {
  const backup = join(staging, 'previous');
  mkdirSync(backup);
  for (const name of names) {
    const path = join(assets, name);
    if (existsSync(path)) linkSync(path, join(backup, name));
  }
  writeFileSync(join(staging, 'manifest.json'), JSON.stringify(names));
  const pending = pendingPath(assets);
  renameSync(staging, pending);
  try {
    for (const name of names) renameSync(join(pending, name), join(assets, name));
    // Removing the manifest commits publication. Cleanup can resume independently.
    unlinkSync(join(pending, 'manifest.json'));
  } catch (error) {
    rollbackCatalogue(assets);
    throw error;
  }
  rmSync(pending, { recursive: true, force: true });
}

export function withCatalogueLock(assets, publish) {
  const fd = openSync(join(assets, '..', '.catalogue-publish.lock'), 'a', 0o600);
  try {
    // The child locks the inherited open file description, which stays open here.
    const result = spawnSync('python3', ['-c', lockDescriptor], { stdio: ['ignore', 'ignore', 'pipe', fd], encoding: 'utf8' });
    if (result.error) throw result.error;
    if (result.status === 75) throw new Error('catalogue publication already in progress; retry the build');
    if (result.status !== 0) throw new Error(`could not lock catalogue publication: ${result.stderr}`);
    rollbackCatalogue(assets);
    return publish();
  } finally {
    closeSync(fd);
  }
}
