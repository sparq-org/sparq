// Research grade and not externally audited. Maintainer publication lock.
import { mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';

export function withCatalogueLock(assets, publish) {
  const lock = join(assets, '..', '.catalogue.lock');
  try {
    mkdirSync(lock);
  } catch (error) {
    if (error.code !== 'EEXIST') throw error;
    throw new Error('catalogue publication already in progress; retry the build');
  }
  try {
    publish();
  } finally {
    rmdirSync(lock);
  }
}
