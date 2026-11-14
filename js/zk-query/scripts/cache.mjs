// Research grade and not externally audited. Maintainer source staging only.
import { lstatSync, mkdirSync, unlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export function stageSource(root, name, text) {
  const parts = name.split('/');
  if (!parts.length || parts.some(part => !part || part === '.' || part === '..') || !/^[a-zA-Z0-9_./-]+$/.test(name)) throw new Error('catalogue source path rejected');
  let path = root;
  if (lstatSync(path).isSymbolicLink()) throw new Error('catalogue cache symbolic link rejected');
  for (const [index, part] of parts.entries()) {
    path = join(path, part);
    const info = lstatSync(path, { throwIfNoEntry: false });
    if (info?.isSymbolicLink()) throw new Error('catalogue cache symbolic link rejected');
    if (index < parts.length - 1) mkdirSync(path, { recursive: true });
    else if (info) unlinkSync(path);
  }
  writeFileSync(path, text, { flag: 'wx' });
}
