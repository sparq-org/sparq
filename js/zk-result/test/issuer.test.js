import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { issuerKeygen, testIssuerKeygen, issue, prove, verify } from '../index.js';

const graph = '<https://example.org/alice> <https://example.org/degree> <https://example.org/BSc> .';
const query = 'SELECT DISTINCT ?p WHERE { ?p <https://example.org/degree> <https://example.org/BSc> }';
const status = { status_list: 'https://example.org/status', version: 1, index: 0 };

test('production keys survive caller serialization and reject invalid or mixed key material', () => {
  const issuer = issuerKeygen();
  assert.deepEqual(Object.keys(issuer).sort(), ['publicKey', 'secretKeyHex']);
  assert.ok(/^[0-9a-f]{64}$/.test(issuer.secretKeyHex), 'canonical secret encoding');
  const restored = JSON.parse(JSON.stringify(issuer));
  assert.equal(issue(restored, graph, status).issuer, issuer.publicKey);
  const other = issuerKeygen();
  assert.ok(issuer.secretKeyHex !== other.secretKeyHex);
  assert.notEqual(issuer.publicKey, other.publicKey);
  assert.throws(() => issue({ ...issuer, publicKey: other.publicKey }, graph, status), /issuer key differs from secret/);
  for (const secretKeyHex of ['00'.repeat(32), 'ff'.repeat(32), issuer.secretKeyHex.slice(2), 'gg'.repeat(32)]) {
    assert.throws(() => issue({ ...issuer, secretKeyHex }, graph, status), /invalid issuer secret key/);
  }
  const legacy = testIssuerKeygen('a1');
  assert.deepEqual(Object.keys(legacy).sort(), ['publicKey', 'seedHex']);
  assert.deepEqual(testIssuerKeygen('a1'), legacy);
  assert.equal(issue(legacy, graph, status).issuer, legacy.publicKey);
  assert.throws(() => issue({ ...legacy, secretKeyHex: issuer.secretKeyHex }, graph, status));
  assert.throws(() => issue({ ...issuer, seedHex: legacy.seedHex }, graph, status));
  assert.throws(() => issue({ secretKeyHex: legacy.seedHex, publicKey: legacy.publicKey }, graph, status), /invalid issuer secret key/);
  assert.throws(() => issue({ seedHex: issuer.secretKeyHex, publicKey: issuer.publicKey }, graph, status), /test issuer seed/);
});

test('production issuer credentials prove and verify offline and require policy trust', async () => {
  const issuer = JSON.parse(JSON.stringify(issuerKeygen()));
  const credentials = [issue(issuer, graph, status)];
  const policy = {
    trustedIssuers: [issuer.publicKey],
    snapshots: [{ status_list: status.status_list, version: status.version, bits: [0] }],
    minVersion: status.version, maxVersion: status.version,
  };
  const nonce = '0x' + randomBytes(31).toString('hex');
  const { presentation, result } = await prove({ query, credentials, policy, nonce });
  const expected = [{ p: { kind: 'iri', value: 'https://example.org/alice' } }];
  assert.deepEqual(result, expected);
  assert.deepEqual(presentation.issuer_slots, [issuer.publicKey]);
  assert.equal(JSON.stringify(presentation).includes(issuer.secretKeyHex), false);
  const checked = await verify({ expectedQuery: query, presentation, policy, nonce, seenNonces: new Set() });
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.released, expected);

  const untrustedPolicy = { ...policy, trustedIssuers: [issuerKeygen().publicKey] };
  await assert.rejects(prove({ query, credentials, policy: untrustedPolicy, nonce }));
  let accesses = 0;
  const refused = await verify({
    expectedQuery: query, presentation, policy: untrustedPolicy, nonce,
    seenNonces: { has() { accesses++; return false; }, add() { accesses++; } },
  });
  assert.equal(refused.verified, false);
  assert.deepEqual(refused.released, []);
  assert.match(refused.reason, /statement rejected:.*issuer/i);
  assert.equal(accesses, 0);
});
