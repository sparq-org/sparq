// Research grade and not externally audited. Requires the pinned native executable.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { copyPinnedBinary } from '../native.mjs';
import pins from '../assets/toolchain.pins.json' with { type: 'json' };
test('the pinned executable also has to report the expected version', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'sparq-query-version-test-'));
  const binaries = pins.platforms[`${process.platform}-${process.arch}`];
  const version = binaries.nargo.version;
  try {
    await copyPinnedBinary('nargo', process.env.SPARQ_NARGO, join(directory, 'nargo'));
    binaries.nargo.version = 'deliberately mismatched expected version';
    await assert.rejects(copyPinnedBinary('nargo', process.env.SPARQ_NARGO, join(directory, 'other')), /binary version mismatch/);
  } finally { binaries.nargo.version = version; await rm(directory, { recursive: true, force: true }); }
});

test('native proving gives nargo a private home when the parent home is a file', async () => {
  const { writeFile, readFile } = await import('node:fs/promises');
  const { execFile } = await import('node:child_process');
  const { promisify } = await import('node:util');
  const directory = await mkdtemp(join(tmpdir(), 'sparq-query-home-test-'));
  const parentHome = join(directory, 'parent-home');
  await writeFile(parentHome, 'parent home sentinel');
  try {
    const moduleUrl = new URL('../index.js', import.meta.url).href;
    const fixturesUrl = new URL('../scripts/test-credentials.mjs', import.meta.url).href;
    const program = `
      import assert from 'node:assert/strict';
      import { request, prove, verify, testIssuerKeygen } from ${JSON.stringify(moduleUrl)};
      import { canonicalGraph, issueFixture, trustMembers } from ${JSON.stringify(fixturesUrl)};
      const issuer = testIssuerKeygen('00'.repeat(32));
      const stored = request({ query: 'ASK {}', policy: {
        bounds: { credentials: 1, quadsPerCredential: 2, rowsPerTable: 1 },
        trustedIssuers: [issuer.publicKey], trustMembers: trustMembers(issuer), audience: 'urn:home-test'
      }, nonce: '19'.repeat(32) });
      const { presentation } = await prove({ request: stored, credentials: [issueFixture(issuer, canonicalGraph(''), '01'.repeat(32))] });
      const checked = await verify({ request: stored, presentation, seenNonces: new Set() });
      assert.equal(checked.verified, true, checked.reason);
      assert.equal(checked.released.boolean, true);
    `;
    await promisify(execFile)(process.execPath, ['--input-type=module', '-e', program], { env: { ...process.env, HOME: parentHome }, timeout: 19 * 60 * 1000 });
    assert.equal(await readFile(parentHome, 'utf8'), 'parent home sentinel');
  } finally { await rm(directory, { recursive: true, force: true }); }
});
