// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cpSync, symlinkSync, mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL, fileURLToPath } from 'node:url';
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
