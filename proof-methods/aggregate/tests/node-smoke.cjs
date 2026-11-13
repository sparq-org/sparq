'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { randomBytes, createPrivateKey, sign } = require('node:crypto');
const { writeFileSync, readFileSync, existsSync } = require('node:fs');
const { resolve } = require('node:path');
const aggregate = require(resolve(process.env.AGGREGATE_PACKAGE));
const evidence = [];
const bytes = (value) => Buffer.byteLength(JSON.stringify(value));
const call = (name, value) => JSON.parse(aggregate[name](JSON.stringify(value)));

function session(n, minimum, countOnly = false) {
  const start = process.hrtime.bigint();
  const keys = Array.from({ length: n }, (_, i) => JSON.parse(aggregate.generateKeys(Buffer.alloc(32, i + 1).toString('base64url'))));
  const parameters = {
    session_id: randomBytes(32).toString('base64url'),
    statistic: countOnly ? 'count-only' : 'count-and-sum',
    min_group: minimum,
    roster: keys.map((k, i) => ({ party_iri: `urn:hospital:${i}`, ed25519_public_key: k.ed25519_public_key, x25519_public_key: k.x25519_public_key })),
  };
  const session = call('newSession', parameters);
  const states = keys.map((k, index) => call('contribute', { session, index, keys: k.secret, input: { count: 2, sum: countOnly ? null : String(10 + index) } }));
  const contributions = states.map((s) => s.contribution);
  const partials = n < 3 ? [] : keys.map((k, index) => call('receiveAndPartial', { session, index, keys: k.secret, contributions, local: states[index].local }));
  const outputs = keys.map((k, index) => {
    const outcome = call('finish', { session, index, keys: k.secret, contributions, local: partials[index]?.local ?? null, messages: partials.flatMap((p) => p.messages.filter((m) => m.recipient === index)) });
    assert.deepEqual(call('verifyOutcome', { session, contributions, outcome }), { verified: true });
    return outcome;
  });
  const elapsed = Number(process.hrtime.bigint() - start) / 1e6;
  evidence.push({ mode: 'steps', parties: n, minimum, statistic: parameters.statistic, elapsed_ms: elapsed, contribution_json_bytes: contributions.map(bytes), share_ciphertext_json_bytes: contributions.map((c) => c.ciphertexts.map((p) => p ? bytes(p) : null)), local_partial_json_bytes: partials.map((p) => bytes(p.local)), partial_message_json_bytes: partials.map((p) => p.messages.map(bytes)), output_json_bytes: outputs.map(bytes), outcome: outputs[0].outcome.kind });
  if (process.env.AGGREGATE_EVIDENCE) writeFileSync(process.env.AGGREGATE_EVIDENCE, JSON.stringify({ node: process.version, runs: evidence }, null, 2));
  return { session, contributions, outputs, partials };
}

test('joint count and sum with every contributor verifying', () => {
  for (const n of [3, 6]) {
    const { outputs } = session(n, n * 2);
    for (const o of outputs) {
      assert.equal(o.outcome.kind, 'release');
      assert.equal(o.outcome.count, n * 2);
      assert.equal(o.outcome.sum, String(10 * n + n * (n - 1) / 2));
    }
  }
});
test('count-only and the disabled minimum', () => {
  const { outputs } = session(3, 0, true);
  assert.equal(outputs[0].outcome.count, 6);
  assert.ok(!Object.hasOwn(outputs[0].outcome, 'sum'));
  assert.ok(!Object.hasOwn(outputs[0].outcome, 'sigma'));
});
test('minimum-group refusal exposes no aggregate openings', () => {
  const { outputs } = session(3, 7);
  for (const o of outputs) {
    assert.equal(o.outcome.kind, 'below-minimum-group');
    assert.deepEqual(Object.keys(o.outcome).sort(), ['kind', 'range_proof']);
  }
});
test('too few contributors is publicly verifiable without partials', () => {
  const { session: s, contributions, outputs, partials } = session(2, 0);
  assert.deepEqual(partials, []);
  assert.equal(outputs[0].outcome.kind, 'too-few-contributors');
  for (const [field, value] of Object.entries({ count: 0, rho: '', sum: '0', sigma: '',
    range_proof: '', verdict: true, limb_commitments: [] })) {
    const outcome = structuredClone(outputs[0]);
    outcome.outcome[field] = value;
    assert.throws(() => call('verifyOutcome', { session: s, contributions, outcome }),
      (e) => JSON.parse(String(e)).check === 'json-encoding',
      `accepted unsigned refusal field ${field}`);
  }
});
test('unattested hospital input is rejected by roster index', () => {
  const f = session(3, 6);
  for (const mutate of [
    (cs) => cs.splice(1, 1),
    (cs) => { cs[1].signature = cs[0].signature; },
    (cs) => cs.push(cs[1]),
  ]) {
    const contributions = structuredClone(f.contributions);
    mutate(contributions);
    assert.throws(() => call('verifyOutcome', { session: f.session, contributions, outcome: f.outputs[0] }), (text) => {
      const e = JSON.parse(String(text));
      assert.equal(e.code, 'input-not-attested');
      assert.equal(e.index, 1);
      assert.ok(['missing-contributor', 'signature', 'duplicate-contributor'].includes(e.check));
      return true;
    });
  }
});
test('JSON errors and invalid fixture seeds are ordinary errors', () => {
  const noticesPath = resolve(process.env.AGGREGATE_PACKAGE, 'THIRD_PARTY_LICENSES');
  assert.ok(existsSync(noticesPath), 'package includes dependency licence notices');
  const notices = readFileSync(noticesPath, 'utf8');
  for (const expected of ['bulletproofs', 'curve25519-dalek', 'ed25519-dalek', 'Copyright', 'Permission is hereby granted']) assert.ok(notices.includes(expected));
  assert.throws(() => aggregate.newSession('{'), (e) => JSON.parse(String(e)).check === 'json-encoding');
  assert.throws(() => aggregate.generateKeys('AA'), (e) => JSON.parse(String(e)).check === 'seed');
  const keys = JSON.parse(aggregate.generateKeys());
  assert.equal(Buffer.from(keys.ed25519_public_key, 'base64url').length, 32);
});

function simulated(options = {}) {
  const start = process.hrtime.bigint();
  const keys = Array.from({ length: options.n ?? 3 }, () => JSON.parse(aggregate.generateKeys()));
  const params = {
    session_id: randomBytes(32).toString('base64url'),
    statistic: options.countOnly ? 'count-only' : 'count-and-sum',
    min_group: options.minimum ?? (options.computeOnly ? 4 : 6),
    max_value_bits: options.bits,
    threshold: options.threshold,
    roster: keys.map((k, i) => ({
      party_iri: `urn:holder:${i}`, ed25519_public_key: k.ed25519_public_key,
      x25519_public_key: k.x25519_public_key, computeOnly: Boolean(options.computeOnly && i === 2),
    })),
  };
  const inputs = keys.map((k, i) => ({ keys: k.secret, input: {
    count: options.computeOnly && i === 2 ? 0 : 2,
    sum: options.countOnly ? null : options.computeOnly && i === 2 ? '0' : options.sum ?? String(10 + i),
  } }));
  const f = JSON.parse(aggregate.simulateSession(JSON.stringify(params), inputs.map((p) => JSON.stringify(p))));
  assert.deepEqual(call('verifyOutcome', { session: f.session, contributions: f.contributions, outcome: f.outcome }), { verified: true });
  assert.equal(f.contributions.length, keys.length);
  assert.equal(f.partials.length, keys.length < 3 ? 0 : keys.length);
  evidence.push({ mode: 'simulation', parties: keys.length, minimum: params.min_group,
    statistic: params.statistic, threshold: params.threshold ?? null,
    elapsed_ms: Number(process.hrtime.bigint() - start) / 1e6,
    contribution_json_bytes: f.contributions.map(bytes),
    share_ciphertext_json_bytes: f.contributions.map((c) => c.ciphertexts.map((p) => p ? bytes(p) : null)),
    local_partial_json_bytes: f.partials.map((p) => bytes(p.local)),
    partial_message_json_bytes: f.partials.map((p) => p.messages.map(bytes)),
    output_json_bytes: [bytes(f.outcome)], outcome: f.outcome.outcome.kind });
  if (process.env.AGGREGATE_EVIDENCE) writeFileSync(process.env.AGGREGATE_EVIDENCE, JSON.stringify({ node: process.version, runs: evidence }, null, 2));
  return { ...f, keys, params, inputs };
}

// Independent canonical encoder lets the smoke test forge a correctly signed bad verdict.
function signThresholdOutcome(outcome, secret) {
  const chunks = [];
  const byteString = (v) => {
    const b = typeof v === 'string' ? Buffer.from(v, 'base64url') : v;
    const len = Buffer.alloc(8); len.writeBigUInt64LE(BigInt(b.length));
    chunks.push(len, b);
  };
  const u32 = (v) => { const b = Buffer.alloc(4); b.writeUInt32LE(v); chunks.push(b); };
  byteString(Buffer.from('sparq-verifiable-aggregate/v1'));
  byteString(Buffer.from('outcome'));
  byteString(outcome.session_id); byteString(outcome.rule_digest); u32(outcome.finisher);
  byteString(outcome.contributions_digest);
  chunks.push(Buffer.from([3, Number(outcome.outcome.verdict)]));
  u32(outcome.outcome.limb_commitments.length);
  for (const point of outcome.outcome.limb_commitments) byteString(point);
  byteString(outcome.outcome.range_proof);
  const key = createPrivateKey({ key: Buffer.concat([
    Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(secret.signing_secret, 'base64url'),
  ]), format: 'der', type: 'pkcs8' });
  return sign(null, Buffer.concat(chunks), key).toString('base64url');
}

test('simulated threshold verdicts open neither total', () => {
  for (const statistic of ['count', 'sum']) {
    const total = statistic === 'count' ? 6 : 33;
    for (const op of ['>=', '<']) {
      for (const value of [total, total + 1]) {
        const f = simulated({ threshold: { statistic, op, value: String(value) } });
        assert.equal(f.outcome.outcome.kind, 'threshold-verdict');
        assert.equal(f.outcome.outcome.verdict, op === '>=' ? total >= value : total < value);
        assert.deepEqual(Object.keys(f.outcome.outcome).sort(), ['kind', 'limb_commitments', 'range_proof', 'verdict']);
        assert.equal(signThresholdOutcome(f.outcome, f.keys[0].secret), f.outcome.signature);
        const forged = structuredClone(f.outcome);
        forged.outcome.verdict = !forged.outcome.verdict;
        assert.throws(() => call('verifyOutcome', { session: f.session, contributions: f.contributions, outcome: forged }), (e) => JSON.parse(String(e)).check === 'signature');
        forged.signature = signThresholdOutcome(forged, f.keys[0].secret);
        assert.throws(() => call('verifyOutcome', { session: f.session, contributions: f.contributions, outcome: forged }), (e) => JSON.parse(String(e)).check === 'threshold-commitment');
      }
    }
  }
  assert.equal(simulated({ minimum: 7, threshold: { statistic: 'sum', op: '>=', value: '1' } }).outcome.outcome.kind, 'below-minimum-group');
  assert.equal(simulated({ n: 2, threshold: { statistic: 'sum', op: '>=', value: '1' } }).outcome.outcome.kind, 'too-few-contributors');
  const large = { bits: 64, sum: '18446744073709551615' };
  assert.equal(simulated(large).outcome.outcome.sum, '55340232221128654845');
  assert.equal(simulated({ ...large, threshold: { statistic: 'sum', op: '>=', value: '18446744073709551615' } }).outcome.outcome.verdict, true);
  assert.equal(simulated({ ...large, threshold: { statistic: 'sum', op: '>=', value: '340282366920938463463374607431768211455' } }).outcome.outcome.verdict, false);
});

test('two holders and a compute-only requester use the same steps', () => {
  const f = simulated({ computeOnly: true });
  assert.equal(f.outcome.outcome.count, 4);
  assert.equal(f.outcome.outcome.sum, '21');
  const index = 2;
  const outcome = call('finish', {
    session: f.session, index, keys: f.keys[index].secret, contributions: f.contributions,
    local: f.partials[index].local,
    messages: f.partials.flatMap((p) => p.messages.filter((m) => m.recipient === index)),
  });
  assert.equal(outcome.finisher, index);
  assert.deepEqual(call('verifyOutcome', { session: f.session, contributions: f.contributions, outcome }), { verified: true });
  assert.throws(() => call('contribute', { session: f.session, index, keys: f.keys[index].secret, input: { count: 1, sum: '0' } }), (e) => JSON.parse(String(e)).check === 'compute-only-input');
  const threshold = simulated({ computeOnly: true, threshold: { statistic: 'sum', op: '>=', value: '21' } });
  assert.equal(threshold.outcome.outcome.verdict, true);
  const requesterOutcome = call('finish', { session: threshold.session, index,
    keys: threshold.keys[index].secret, contributions: threshold.contributions,
    local: threshold.partials[index].local,
    messages: threshold.partials.flatMap((p) => p.messages.filter((m) => m.recipient === index)) });
  assert.equal(requesterOutcome.finisher, index);
  assert.equal(requesterOutcome.outcome.verdict, true);
  assert.deepEqual(call('verifyOutcome', { session: threshold.session,
    contributions: threshold.contributions, outcome: requesterOutcome }), { verified: true });
  assert.equal(simulated({ computeOnly: true, countOnly: true }).outcome.outcome.count, 4);
});

test('simulation preserves attestation checks and validates its party array', () => {
  const six = simulated({ n: 6, minimum: 12 });
  assert.equal(six.outcome.outcome.count, 12);
  assert.equal(six.outcome.outcome.sum, '75');
  const f = simulated();
  assert.throws(() => aggregate.simulateSession(JSON.stringify(f.params), f.inputs.slice(1).map((p) => JSON.stringify(p))), (e) => JSON.parse(String(e)).check === 'simulation-party-count');
  f.inputs[1].keys = JSON.parse(aggregate.generateKeys()).secret;
  assert.throws(() => aggregate.simulateSession(JSON.stringify(f.params), f.inputs.map((p) => JSON.stringify(p))), (e) => {
    const error = JSON.parse(String(e));
    return error.index === 1 && error.check === 'signing-key-mismatch';
  });
});
