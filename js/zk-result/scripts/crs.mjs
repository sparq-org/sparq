// Research grade and not externally audited.
import { existsSync } from 'node:fs';
const bundled = new URL('../internal/crs.mjs', import.meta.url);
const source = new URL('../../zk-shared/crs.mjs', import.meta.url);
export const { installPinnedCrs } = await import(existsSync(bundled) ? bundled : source);
