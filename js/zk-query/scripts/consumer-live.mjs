// Research grade and not externally audited. Native Solid consumer proof contracts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { request, prove, verify, testIssuerKeygen } from '../index.js';
import { consumerCases } from './consumer-cases.mjs';
import { targetFixture } from './solid-targets.mjs';
import { examplePolicy } from './catalogue-example.mjs';
import { liveRuntimeHash } from './live-runtime.mjs';
import { sha256 } from '../internal/runtime.mjs';
import { canonicalGraph, issueFixture } from './test-credentials.mjs';

const table = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
const backend = { type: 'native' };
const evidenceDir = join(process.env.SPARQ_QUERY_TEST_DIR || join(tmpdir(), 'sparq-query-consumer-evidence'), 'consumer');
mkdirSync(evidenceDir, { recursive: true, mode: 0o700 });
const runtimeHash = liveRuntimeHash(fileURLToPath(new URL('../../../', import.meta.url)));
function fixture(sample, kase) {
  if (sample.name.startsWith('consumer-degree')) {
    const issuer = testIssuerKeygen('40'.repeat(32));
    const iri = 'urn:fictional:degree-recognition:university';
    const graph = canonicalGraph(`<urn:subject> <urn:claim:degree.level> "${kase === 'masters' ? 'MSc' : 'BSc'}" .\n`).replace('<urn:issuer>', `<${iri}>`);
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
    const variants = sample.name.startsWith('consumer-degree') ? ['honest', 'masters']
      : sample.name === 'consumer-trial' ? ['honest', 'excluded', 'incomplete']
      : ['honest', sample.name === 'consumer-insurance-absence' ? 'claim' : sample.name === 'consumer-insurance-count' ? 'incomplete' : 'threshold'];
    for (const [index, kase] of variants.entries()) {
      const f = fixture(sample, kase);
      const policy = { ...examplePolicy(entry.example, { publicKey: f.policy.trustedIssuers[0] }, 'urn:consumer'), ...f.policy, notBefore: 0, notAfter: 32503680000 };
      const stored = request({ query: sample.query, policy, nonce: (index ? '72' : '71').repeat(32) });
      const cachePath = join(evidenceDir, sample.name + '-' + kase + '.json');
      const cacheKey = sha256(JSON.stringify([runtimeHash, entry.acirSha256, entry.vkSha256, table.sourceBundleSha256, stored.digest, f]));
      let evidence = existsSync(cachePath) ? JSON.parse(readFileSync(cachePath)) : undefined;
      if (evidence?.cacheKey !== cacheKey) {
        evidence = { cacheKey, ...await prove({ request: stored, ...f, backend }) };
        writeFileSync(cachePath, JSON.stringify(evidence), { mode: 0o600 });
      }
      const { presentation, result } = evidence;
      const checked = await verify({ request: stored, presentation, seenNonces: new Set() });
      assert.equal(checked.verified, true, checked.reason);
      assert.deepEqual(checked.released, result);
      assert.equal(result.rows.length, 1);
      assert.match(result.rows[0][0], sample.name === 'consumer-degree-hidden' ? /urn:subject/ : sample.name === 'consumer-degree' ? new RegExp(kase === 'masters' ? 'MSc' : 'BSc') : new RegExp(index ? '^"false"' : '^"true"'));
      assert.equal(checked.report.rule.query, sample.query);
      assert.equal(checked.report.complete, true);
      const altered = structuredClone(presentation);
      altered.result.rows = [];
      const refused = await verify({ request: stored, presentation: altered, seenNonces: new Set() });
      assert.equal(refused.verified, false);
      assert.equal(Object.hasOwn(refused, 'report'), false);
    }
    const f = fixture(sample, 'honest');
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
