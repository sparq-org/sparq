// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cpSync, symlinkSync, mkdtempSync, readFileSync, writeFileSync, rmSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { request, testIssuerKeygen } from '../index.js';
test('every catalogue example derives its pinned plan and leaves no stale assets', () => {
  const directory = new URL('../assets/', import.meta.url);
  const table = JSON.parse(readFileSync(new URL('catalogue.json', directory)));
  const issuer = testIssuerKeygen('00'.repeat(32));
  for (const entry of table.entries) {
    const stored = request({ query: entry.example.query, policy: { bounds: entry.example.bounds, ...(entry.example.credentialQuadBounds === undefined ? {} : { credentialQuadBounds: entry.example.credentialQuadBounds }), trustedIssuers: [issuer.publicKey], audience: 'urn:sparq:test:catalogue' }, nonce: '09'.repeat(32) });
    assert.equal(stored.planHash, entry.planHash);
    assert.equal(stored.verificationKey, readFileSync(new URL(entry.vk, directory), 'utf8').trim());
  }
  const keys = readdirSync(directory).filter(name => /^[a-f0-9]{64}\.vk\.hex$/.test(name));
  assert.deepEqual(keys.sort(), table.entries.map(entry => entry.vk).sort());
  for (const name of readdirSync(directory).filter(name => /^[a-f0-9]{64}\.json$/.test(name))) assert(table.entries.some(entry => entry.acir === name), 'stale catalogue program');
});
test('catalogue, binary pins and keys fail closed when their stored digests differ', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'sparq-query-catalogue-test-'));
  const packageDir = fileURLToPath(new URL('../', import.meta.url));
  try {
    for (const name of ['index.js', 'package.json', 'internal']) cpSync(join(packageDir, name), join(directory, name), { recursive: true });
    for (const name of ['node_modules', 'wasm']) symlinkSync(join(packageDir, name), join(directory, name));
    cpSync(join(packageDir, 'assets'), join(directory, 'assets'), { recursive: true, filter: path => !path.endsWith('.dat') && !path.endsWith('sources.json') && (!path.endsWith('.json') || /catalogue|pins/.test(path)) });
    const api = await import(pathToFileURL(join(directory, 'index.js')));
    const fixture = JSON.parse(readFileSync(new URL('./fixtures/authenticated.json', import.meta.url)));
    const input = { query: fixture.request.query, policy: fixture.request.policy, nonce: fixture.request.nonce };
    assert.equal(api.request(input).planHash, fixture.request.planHash);
    for (const name of ['catalogue.json', 'toolchain.pins.json', `${fixture.request.planHash}.vk.hex`]) {
      const path = join(directory, 'assets', name), original = readFileSync(path);
      writeFileSync(path, name.endsWith('.hex') ? Buffer.from('00\n') : Buffer.concat([original, Buffer.from(' ')]));
      assert.throws(() => api.request(input), /digest rejected|pin data rejected/);
      writeFileSync(path, original);
    }
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
