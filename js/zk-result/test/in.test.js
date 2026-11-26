import { test } from 'node:test';
import assert from 'node:assert/strict';
import { prove, verify, testIssue, testIssuerKeygen } from '../index.js';
const degreeQuery = `PREFIX vcq: <https://w3id.org/sparq/vcq#>
PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
SELECT DISTINCT ?value WHERE {
  ?g vcq:verifiedIssuer <urn:fictional:degree-recognition:university> .
  GRAPH ?g { ?subject <urn:claim:degree.level> ?value . FILTER(?value IN ("BSc", "MSc")) }
}`;

test('single issuer degree IN proves and verifies both alternatives and refuses substitutions', async () => {
  const issuer = testIssuerKeygen('a1');
  const status = { status_list: 'urn:list', version: 1, index: 0 };
  const policy = { issuers: { 'urn:fictional:degree-recognition:university': issuer.publicKey }, snapshots: [{ status_list: 'urn:list', version: 1, bits: [0] }], minVersion: 1, maxVersion: 1 };
  for (const value of ['BSc', 'MSc']) {
    const credentials = [testIssue({ issuer, graphNQuads: `<urn:subject> <urn:claim:degree.level> "${value}" .`, status, seedHex: 'c3' })];
    const nonce = '0x1234';
    const { presentation, result } = await prove({ query: degreeQuery, credentials, policy, nonce });
    assert.equal(result[0].value.value, value);
    const checked = await verify({ expectedQuery: degreeQuery, presentation, policy, nonce, seenNonces: new Set() });
    assert.equal(checked.verified, true, checked.reason);
    assert.deepEqual(checked.released, result);
    assert.deepEqual(checked.report.issuers, ['urn:fictional:degree-recognition:university']);
    const bad = structuredClone(presentation);
    bad.rows[0].value.value = 'PhD';
    const refused = await verify({ expectedQuery: degreeQuery, presentation: bad, policy, nonce, seenNonces: new Set() });
    assert.equal(refused.verified, false);
    assert.deepEqual(refused.released, []);
    assert.match(refused.reason, /public IN FILTER is false/);
  }
  const credentials = [testIssue({ issuer, graphNQuads: '<urn:subject> <urn:claim:degree.level> "PhD" .', status, seedHex: 'd4' })];
  const query = degreeQuery.replace(' FILTER(?value IN ("BSc", "MSc"))', '');
  const nonce = '0x5678';
  const { presentation } = await prove({ query, credentials, policy, nonce });
  const control = await verify({ expectedQuery: query, presentation, policy, nonce, seenNonces: new Set() });
  assert.equal(control.verified, true, control.reason);
  const checked = await verify({ expectedQuery: degreeQuery, presentation: { ...presentation, query: degreeQuery }, policy, nonce, seenNonces: new Set() });
  assert.equal(checked.verified, false);
  assert.deepEqual(checked.released, []);
  assert.match(checked.reason, /public IN FILTER is false/);
});

for (const two of [false, true]) {
  test(`hidden degree IN proves both alternatives with ${two ? 'joined' : 'single'} credentials`, async () => {
    const { inFixture } = await import('./helpers/in.js');
    for (const value of ['BSc', 'MSc']) {
      const f = inFixture(two, value);
      const { presentation, result } = await prove(f);
      assert.deepEqual(Object.keys(result[0]), ['subject']);
      assert.equal(result[0].subject.value, 'urn:subject');
      const checked = await verify({ expectedQuery: f.query, presentation, policy: f.policy, nonce: f.nonce, seenNonces: new Set() });
      assert.equal(checked.verified, true, checked.reason);
      assert.deepEqual(checked.released, result);
      const changed = await verify({ expectedQuery: f.query.replace('"MSc"', '"PhD"'), presentation: { ...presentation, query: f.query.replace('"MSc"', '"PhD"') }, policy: f.policy, nonce: f.nonce, seenNonces: new Set() });
      assert.equal(changed.verified, false);
      assert.deepEqual(changed.released, []);
    }
    await assert.rejects(prove(inFixture(two, 'PhD')));
  });
}
