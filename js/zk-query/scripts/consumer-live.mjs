// Research grade and not externally audited. Native Solid consumer proof contracts.
import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { request, prove, verify, testIssuerKeygen } from '../index.js';
import { consumerCases } from './consumer-cases.mjs';
import { targetFixture } from './solid-targets.mjs';
import { examplePolicy } from './catalogue-example.mjs';
import { liveRuntimeHash } from './live-runtime.mjs';
import { sha256 } from '../internal/runtime.mjs';
import { canonicalGraph, issueFixture } from './test-credentials.mjs';
import { openConsumerEvidence } from './consumer-evidence.mjs';

const table = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
const backend = { type: 'native' };
const evidenceStore = openConsumerEvidence();
after(() => evidenceStore.close());
const runtimeHash = liveRuntimeHash(fileURLToPath(new URL('../../../', import.meta.url)));
const xsd = 'http://www.w3.org/2001/XMLSchema#';

function degreeCases(sample) {
  const projected = !sample.name.endsWith('-hidden');
  const rows = (value, matches) => matches ? [[projected ? value : '<urn:subject>']] : [];
  const strings = [['honest', '"BSc"', true], ['masters', '"MSc"', true], ['phd', '"PhD"', false]];
  const typed = (label, datatype, match, miss, query) => [
    [label + '-match', `"${match}"^^<${xsd}${datatype}>`, true, query],
    [label + '-miss', `"${miss}"^^<${xsd}${datatype}>`, false, query],
  ];
  const variants = sample.name.includes('-typed') ? [
    ...typed('integer', 'integer', '2', '3', sample.query),
    ...typed('date', 'date', '2026-11-13', '2026-11-14',
      sample.query.replace('IN (1, 2)', 'IN ("2026-11-12"^^xsd:date, "2026-11-13"^^xsd:date)')),
  ] : strings;
  return variants.map(([name, object, matches, query = sample.query]) => ({
    name, object, query, expectedRows: rows(object, matches),
  }));
}

function fixture(sample, { name: kase, object }) {
  if (object) {
    const issuer = testIssuerKeygen('40'.repeat(32));
    const iri = 'urn:fictional:degree-recognition:university';
    const graph = canonicalGraph(`<urn:subject> <urn:claim:degree.level> ${object} .\n`).replace('<urn:issuer>', `<${iri}>`);
    return { credentials: [issueFixture(issuer, graph, '60'.repeat(32))], policy: { trustedIssuers: [issuer.publicKey], trustMembers: [{ issuer: iri, verificationMethod: 'urn:method', publicKey: issuer.publicKey }] } };
  }
  const target = sample.name === 'consumer-trial' ? '2' : sample.name === 'consumer-recycled-min' ? '3' : sample.name === 'consumer-insurance-absence' ? '1b' : '1a';
  return targetFixture(target, kase, claims => sample.name === 'consumer-trial'
    ? claims.split('\n').filter(line => !line.includes('<https://w3id.org/jeswr/awp#period')).join('\n')
      + `<urn:subject> <urn:claim:trial.medication-complete> "${kase !== 'incomplete'}"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n` : claims);
}
for (const sample of consumerCases) {
  test(`${sample.name} proves and verifies natively, including its negative branch`, async () => {
    const entry = table.entries.find(e => e.example.name === sample.name);
    assert(entry, 'consumer plan is catalogued');
    const booleanCases = sample.name === 'consumer-trial' ? ['honest', 'excluded', 'incomplete']
      : ['honest', sample.name === 'consumer-insurance-absence' ? 'claim' : sample.name === 'consumer-insurance-count' ? 'incomplete' : 'threshold'];
    const variants = sample.name.startsWith('consumer-degree') ? degreeCases(sample)
      : booleanCases.map((name, index) => ({ name, query: sample.query, expectedRows: [[`"${index === 0}"^^<${xsd}boolean>`]] }));
    if (sample.name === 'consumer-insurance-count') {
      variants.push({ name: 'wrong-issuer', query: sample.query, expectedRows: [] });
    }
    for (const [index, kase] of variants.entries()) {
      const f = fixture(sample, kase);
      const policy = { ...examplePolicy(entry.example, { publicKey: f.policy.trustedIssuers[0] }, 'urn:consumer'), ...f.policy, notBefore: 0, notAfter: 32503680000 };
      const stored = request({ query: kase.query, policy, nonce: (0x71 + index).toString(16).repeat(32) });
      assert.equal(stored.planHash, entry.planHash);
      const evidenceName = sample.name + '-' + kase.name + '.json';
      const cacheKey = sha256(JSON.stringify([runtimeHash, entry.acirSha256, entry.vkSha256, table.sourceBundleSha256, stored.digest, f]));
      let evidence = evidenceStore.read(evidenceName);
      if (evidence?.cacheKey !== cacheKey) {
        evidence = { cacheKey, ...await prove({ request: stored, ...f, backend }) };
        evidenceStore.write(evidenceName, evidence);
      }
      const { presentation, result } = evidence;
      const checked = await verify({ request: stored, presentation, seenNonces: new Set() });
      assert.equal(checked.verified, true, checked.reason);
      assert.deepEqual(checked.released, result);
      assert.deepEqual(checked.released.rows, kase.expectedRows);
      assert.equal(checked.report.rule.query, kase.query);
      assert.equal(checked.report.complete, true);
      const altered = structuredClone(presentation);
      altered.result.rows = result.rows.length ? [] : [['<urn:unexpected>']];
      const refused = await verify({ request: stored, presentation: altered, seenNonces: new Set() });
      assert.equal(refused.verified, false);
      assert.equal(Object.hasOwn(refused, 'report'), false);
    }
    const f = fixture(sample, variants[0]);
    const policy = { ...examplePolicy(entry.example, { publicKey: f.policy.trustedIssuers[0] }, 'urn:consumer'), ...f.policy };
    const variant = sample.name === 'consumer-insurance-absence' || sample.name === 'consumer-trial'
      ? sample.query.replace('NOT EXISTS', 'EXISTS')
      : sample.name === 'consumer-recycled-min' ? sample.query.replace('MIN(', 'MAX(')
      : sample.name.startsWith('consumer-degree') ? sample.query.replace(' IN (', ' NOT IN (')
      : sample.query.replace('>= 5', '> 5');
    assert.notEqual(variant, sample.query);
    assert.throws(() => request({ query: variant, policy, nonce: '73'.repeat(32) }), /missing catalogue plan/);
  });
}
