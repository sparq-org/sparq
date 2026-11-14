// Research grade and not externally audited. Shared live backend contracts.
import assert from 'node:assert/strict';
import { request, prove, verify } from '../index.js';
export async function proofContract({ stored, result, credentials, backend }) {
  const seen = new Set();
  const checked = await verify({ request: stored, presentation: result.presentation, seenNonces: seen });
  assert.equal(checked.verified, true, checked.reason);
  assert.deepEqual(checked.released, result.result);
  assert.deepEqual(checked.report.rule, { query: stored.query, planHash: stored.planHash });
  assert.deepEqual(checked.report.issuers, stored.policy.trustedIssuers);
  assert.deepEqual(checked.report.trustMembers, stored.policy.trustMembers);
  assert.deepEqual(checked.report.subject, []);
  assert.equal(checked.report.complete, true);
  assert.equal(checked.report.fresh, stored.nonce);
  assert.deepEqual([...seen], [stored.nonce]);
  const refused = answer => { assert.equal(answer.verified, false); assert.equal('report' in answer, false); };
  refused(await verify({ request: stored, presentation: result.presentation }));
  refused(await verify({ request: stored, presentation: result.presentation, seenNonces: seen }));
  const wrong = request({ query: stored.query, policy: stored.policy, nonce: '08'.repeat(32) });
  const untouched = new Set();
  refused(await verify({ request: wrong, presentation: result.presentation, seenNonces: untouched }));
  const altered = structuredClone(result.presentation);
  if ('boolean' in altered.result) altered.result.boolean = !altered.result.boolean;
  else altered.result.rows = altered.result.rows.length ? [] : [[null]];
  refused(await verify({ request: stored, presentation: altered, seenNonces: untouched }));
  assert.equal(untouched.size, 0);
  const badKey = { ...stored, verificationKey: '00' };
  await assert.rejects(prove({ request: badKey, credentials, backend }), /request reconstruction|key mismatch/);
  const missing = { ...stored, query: 'SELECT ?s WHERE {?s <urn:uncatalogued> ?o}' };
  await assert.rejects(prove({ request: missing, credentials, backend }), /request reconstruction|missing catalogue plan/);
  assert.throws(() => request({ query: missing.query, policy: stored.policy, nonce: stored.nonce }), /missing catalogue plan/);
  return checked;
}
