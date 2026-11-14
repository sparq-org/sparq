// Research grade and not externally audited. Proof callbacks isolate host ordering.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
const fixture = JSON.parse(readFileSync(new URL('./fixtures/authenticated.json', import.meta.url)));
const input = JSON.stringify({ request: fixture.request, presentation: fixture.presentation });
test('Rust verification checks the statement and proof before final clock and consumption', () => {
  const events = [];
  const result = JSON.parse(host.verifyWith(input, () => { events.push('proof'); return true; }, () => { events.push('consume'); return 'fresh'; }, () => { events.push('clock'); return 1; }));
  assert.equal(result.verified, true); assert.deepEqual(events, ['clock', 'proof', 'clock', 'consume']);
  assert.equal(result.report.fresh, fixture.request.nonce);
});
test('failed proof or expiry during proof verification cannot consume', () => {
  let consumed = 0, clocks = 0;
  const store = () => { consumed++; return 'fresh'; };
  assert.equal(JSON.parse(host.verifyWith(input, () => false, store, () => 1)).verified, false);
  assert.equal(JSON.parse(host.verifyWith(input, () => true, store, () => ++clocks === 1 ? 1 : Number.MAX_SAFE_INTEGER)).verified, false);
  assert.equal(consumed, 0);
});
test('wrong stored request cannot reach the proof callback', () => {
  const r = fixture.request;
  const stored = JSON.parse(host.hostCall('request', JSON.stringify({ query: r.query, policy: r.policy, nonce: '08'.repeat(32), verificationKey: r.verificationKey, clock: 1 })));
  let proofs = 0, consumed = 0;
  const result = JSON.parse(host.verifyWith(JSON.stringify({ request: stored, presentation: fixture.presentation }), () => { proofs++; return true; }, () => { consumed++; return 'fresh'; }, () => 1));
  assert.equal(result.verified, false); assert.match(result.reason, /request digest/); assert.equal(proofs, 0); assert.equal(consumed, 0);
});
test('Rust rejects an unknown plan or wrong key even when called without the ESM catalogue', () => {
  const r = fixture.request;
  const call = (query, verificationKey) => host.hostCall('request', JSON.stringify({ query, policy: r.policy, nonce: r.nonce, verificationKey, clock: 1 }));
  assert.throws(() => call(r.query, '00'), /verification key mismatch/);
  assert.throws(() => call('SELECT ?s WHERE {?s <urn:uncatalogued> ?o}', r.verificationKey), /missing catalogue plan/);
});
