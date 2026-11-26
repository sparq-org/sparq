import { test } from 'node:test';
import assert from 'node:assert/strict';
import { closeSync, mkdtempSync, openSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openConsumerEvidence } from '../scripts/consumer-evidence.mjs';

test('standalone consumer runs use separate evidence even for the same case', t => {
  const configuredCache = process.env.SPARQ_QUERY_TEST_DIR;
  delete process.env.SPARQ_QUERY_TEST_DIR;
  t.after(() => {
    if (configuredCache !== undefined) process.env.SPARQ_QUERY_TEST_DIR = configuredCache;
  });
  const first = openConsumerEvidence();
  t.after(() => first.close());
  const second = openConsumerEvidence();
  t.after(() => second.close());
  const name = 'same-case.json';
  first.write(name, { cacheKey: 'first' });
  assert.equal(second.read(name), undefined);
  second.write(name, { cacheKey: 'second' });
  assert.deepEqual(first.read(name), { cacheKey: 'first' });
  assert.deepEqual(second.read(name), { cacheKey: 'second' });
});

test('explicit consumer cache publishes a complete replacement while readers retain the old file', t => {
  const root = mkdtempSync(join(tmpdir(), 'sparq-query-evidence-test-'));
  const first = openConsumerEvidence(root);
  const second = openConsumerEvidence(root);
  t.after(() => {
    first.close();
    second.close();
    rmSync(root, { recursive: true, force: true });
  });
  const name = 'same-case.json';
  const directory = join(root, 'consumer');
  const before = { cacheKey: 'old', result: ['before'] };
  const after = { cacheKey: 'new', result: ['after'] };
  first.write(name, before);
  assert.deepEqual(second.read(name), before);
  const reader = openSync(join(directory, name), 'r');
  try {
    second.write(name, after);
    assert.deepEqual(JSON.parse(readFileSync(reader)), before);
    assert.deepEqual(first.read(name), after);
  } finally {
    closeSync(reader);
  }
  first.close();
  second.close();
  assert.deepEqual(readdirSync(directory), [name]);
});
