import { mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export function openConsumerEvidence(cacheRoot = process.env.SPARQ_QUERY_TEST_DIR) {
  const cache = cacheRoot ? join(cacheRoot, 'consumer') : undefined;
  if (cache) mkdirSync(cache, { recursive: true, mode: 0o700 });
  const scratch = mkdtempSync(cache ? join(cache, '.run-') : join(tmpdir(), 'sparq-query-consumer-'));
  return {
    read(name) {
      try {
        return JSON.parse(readFileSync(join(cache ?? scratch, name)));
      } catch (error) {
        if (error.code !== 'ENOENT') throw error;
      }
    },
    write(name, evidence) {
      const staged = join(scratch, name);
      writeFileSync(staged, JSON.stringify(evidence), { mode: 0o600 });
      if (cache) renameSync(staged, join(cache, name));
    },
    close() {
      rmSync(scratch, { recursive: true, force: true });
    },
  };
}
