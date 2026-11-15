import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { createRequire } from 'node:module';
import * as aggregate from 'sparq-verifiable-aggregate';

const commonjs = createRequire(import.meta.url)('sparq-verifiable-aggregate');
const call = (name, request) => JSON.parse(aggregate[name](JSON.stringify(request)));

test('package import selects the ESM entry and shares the CommonJS bindings', () => {
  assert.ok(import.meta.resolve('sparq-verifiable-aggregate').endsWith('/index.mjs'));
  for (const name of ['newSession', 'generateKeys', 'contribute', 'receiveAndPartial',
    'finish', 'verifyOutcome', 'simulateSession']) {
    assert.equal(typeof aggregate[name], 'function', name);
    assert.equal(aggregate[name], commonjs[name], name);
  }
});

test('ESM consumers route a full session through the installed package', () => {
  const keys = Array.from({ length: 3 }, () => JSON.parse(aggregate.generateKeys()));
  const session = call('newSession', {
    session_id: randomBytes(32).toString('base64url'),
    statistic: 'count-only', min_group: 6,
    roster: keys.map((k, i) => ({ party_iri: `urn:party:${i}`,
      ed25519_public_key: k.ed25519_public_key, x25519_public_key: k.x25519_public_key })),
  });
  const states = keys.map((k, index) => call('contribute', {
    session, index, keys: k.secret, input: { count: 2 },
  }));
  const contributions = states.map((s) => s.contribution);
  const partials = keys.map((k, index) => call('receiveAndPartial', {
    session, index, keys: k.secret, contributions, local: states[index].local,
  }));
  const index = 2;
  const outcome = call('finish', {
    session, index, keys: keys[index].secret, contributions, local: partials[index].local,
    messages: partials.flatMap((p) => p.messages.filter((m) => m.recipient === index)),
  });
  assert.equal(outcome.outcome.kind, 'release');
  assert.equal(outcome.outcome.count, 6);
  assert.deepEqual(call('verifyOutcome', { session, contributions, outcome }), { verified: true });
});
