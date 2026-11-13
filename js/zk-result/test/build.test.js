import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { installPinnedCrs } from '../scripts/crs.mjs';

const hash = bytes => createHash('sha256').update(bytes).digest('hex');
function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-crs-test-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const content = [Buffer.from('first point fixture'), Buffer.from('second point fixture')];
  const pins = Object.fromEntries(content.map((bytes, i) => [`bn254_g${i + 1}.dat`, {
    url: `https://crs.aztec-cdn.foundation/g${i + 1}.dat`, bytes: bytes.length, sha256: hash(bytes),
  }]));
  return { directory, destination: join(directory, 'crs'), content, pins };
}

test('CRS installation publishes the complete pinned downloads', t => {
  const f = fixture(t), requests = [];
  installPinnedCrs(f.pins, f.destination, (pin, path) => {
    requests.push(pin.url);
    writeFileSync(path, f.content[requests.length - 1]);
  });
  assert.deepEqual(requests, Object.values(f.pins).map(pin => pin.url));
  for (const [i, name] of Object.keys(f.pins).entries()) {
    assert.deepEqual(readFileSync(join(f.destination, name)), f.content[i]);
  }
  assert.deepEqual(readdirSync(f.directory), ['crs']);
});

test('a CRS hash mismatch leaves the previous installation untouched', t => {
  const f = fixture(t);
  mkdirSync(f.destination);
  writeFileSync(join(f.destination, 'previous'), 'existing installation');
  let index = 0;
  assert.throws(() => installPinnedCrs(f.pins, f.destination, (pin, path) => {
    const bytes = Buffer.from(f.content[index++]);
    if (index === f.content.length) bytes[0] ^= 1;
    writeFileSync(path, bytes);
  }), /CRS SHA-256 mismatch/);
  assert.deepEqual(readdirSync(f.destination), ['previous']);
  assert.equal(readFileSync(join(f.destination, 'previous'), 'utf8'), 'existing installation');
  assert.deepEqual(readdirSync(f.directory), ['crs']);
});

test('short and overlong CRS downloads reject even if their digest matches', t => {
  for (const delta of [-1, 1]) {
    const f = fixture(t);
    const bytes = delta < 0 ? f.content[0].subarray(1) : Buffer.concat([f.content[0], Buffer.from('!')]);
    f.pins['bn254_g1.dat'].sha256 = hash(bytes);
    assert.throws(() => installPinnedCrs(f.pins, f.destination, (pin, path) => {
      writeFileSync(path, pin.url.endsWith('/g1.dat') ? bytes : f.content[1]);
    }), /CRS length mismatch/);
    assert.equal(existsSync(f.destination), false);
    assert.deepEqual(readdirSync(f.directory), []);
  }
});
