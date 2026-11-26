// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, symlinkSync, linkSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { catalogueBuildDigest, stageSource } from '../scripts/cache.mjs';

test('a warm catalogue cache is reused only for identical build inputs', () => {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-query-cache-test-'));
  const packageName = 'query_same_plan';
  const files = { 'Nargo.toml': 'manifest', 'src/main.nr': 'fn main() {}' };
  const work = (files, bundle, toolchain) => join(directory, catalogueBuildDigest(files, bundle, toolchain));
  const program = work => join(work, 'target', `${packageName}.json`);
  const old = work(files, 'old bundle', 'pinned toolchain');
  try {
    mkdirSync(join(old, 'target'), { recursive: true });
    writeFileSync(program(old), 'previous program');
    writeFileSync(join(old, 'vk'), 'previous key');

    const reordered = { 'src/main.nr': files['src/main.nr'], 'Nargo.toml': files['Nargo.toml'] };
    assert.equal(work(reordered, 'old bundle', 'pinned toolchain'), old);
    const revised = { ...files, 'src/main.nr': 'fn main(target: [u32; 4]) {}' };
    for (const changed of [
      work(revised, 'old bundle', 'pinned toolchain'),
      work(files, 'indexed scan bundle', 'pinned toolchain'),
      work(files, 'old bundle', 'new toolchain'),
    ]) {
      assert.notEqual(changed, old);
      assert.equal(existsSync(program(changed)), false);
      assert.equal(existsSync(join(changed, 'vk')), false);
    }
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('catalogue staging cannot follow cached source links or escape its directory', () => {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-query-cache-test-'));
  const work = join(directory, 'work'), outside = join(directory, 'outside');
  mkdirSync(work); mkdirSync(outside);
  writeFileSync(join(outside, 'sentinel.nr'), 'original');
  try {
    symlinkSync(outside, join(work, 'zk'));
    const linkedRoot = join(directory, 'linked-root'); symlinkSync(outside, linkedRoot);
    assert.throws(() => stageSource(linkedRoot, 'sentinel.nr', 'changed'), /symbolic link rejected/);
    assert.throws(() => stageSource(work, 'zk/sentinel.nr', 'changed'), /symbolic link rejected/);
    assert.equal(readFileSync(join(outside, 'sentinel.nr'), 'utf8'), 'original');
    assert.throws(() => stageSource(work, '../outside/sentinel.nr', 'changed'), /source path rejected/);
    symlinkSync(join(outside, 'absent.nr'), join(work, 'dangling.nr'));
    assert.throws(() => stageSource(work, 'dangling.nr', 'changed'), /symbolic link rejected/);
    assert.equal(existsSync(join(outside, 'absent.nr')), false);
    linkSync(join(outside, 'sentinel.nr'), join(work, 'linked.nr'));
    stageSource(work, 'linked.nr', 'replacement');
    assert.equal(readFileSync(join(outside, 'sentinel.nr'), 'utf8'), 'original');
    assert.equal(readFileSync(join(work, 'linked.nr'), 'utf8'), 'replacement');
    stageSource(work, 'generated/src/main.nr', 'fn main() {}');
    assert.equal(readFileSync(join(work, 'generated/src/main.nr'), 'utf8'), 'fn main() {}');
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
