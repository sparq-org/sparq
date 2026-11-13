// Research grade and not externally audited.
import { copyFileSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
const source = fileURLToPath(new URL('./', import.meta.url));
export const sharedFiles = ['runtime.mjs', 'crs.mjs', 'offline.py', 'package-pins.json'];
export function assertPinnedPackages(directory) {
  const pins = JSON.parse(readFileSync(join(source, 'package-pins.json')));
  const manifest = JSON.parse(readFileSync(join(directory, 'package.json')));
  const lock = JSON.parse(readFileSync(join(directory, 'package-lock.json')));
  for (const [name, version] of Object.entries(manifest.dependencies)) {
    if (pins[name] !== version || lock.packages[`node_modules/${name}`]?.version !== version) throw new Error(`pinned package mismatch: ${name}`);
  }
  if (manifest.bundleDependencies?.slice().sort().join() !== Object.keys(manifest.dependencies).sort().join()) throw new Error('bundled dependency set rejected');
}
export function syncShared(directory) {
  assertPinnedPackages(directory);
  const target = join(directory, 'internal');
  mkdirSync(target, { recursive: true });
  for (const name of sharedFiles) copyFileSync(join(source, name), join(target, name));
}
