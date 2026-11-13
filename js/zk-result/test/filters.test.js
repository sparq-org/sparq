import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { prove, verify } from '../index.js';
import { filterFixture } from './helpers/filters.js';

const host = createRequire(import.meta.url)('../wasm/sparq_zk_result_wasm.cjs');
const released = [{ p: { kind: 'iri', value: 'https://example.org/alice' } }];
const preparedMember = f => JSON.parse(host.prepare(f.query, f.credentials.map(c => JSON.stringify(c)), JSON.stringify(f.policy), f.nonce, '{}')).member;
const check = (f, presentation, changes = {}) => verify({ expectedQuery: f.query, presentation, policy: f.policy, nonce: f.nonce, seenNonces: new Set(), ...changes });

for (const two of [false, true]) {
  test(`hidden date comparison proves and verifies with ${two ? 'joined' : 'single'} credentials`, async () => {
    const f = filterFixture('date', two);
    assert.equal(preparedMember(f), `result_v5_k${two ? 2 : 1}_n16_p3_r4_f1_date_d10`);
    const { presentation, result } = await prove(f);
    assert.deepEqual(result, released);
    const checked = await check(f, presentation);
    assert.equal(checked.verified, true, checked.reason);
    assert.deepEqual(checked.released, released);
    let touched = false;
    const changed = await check(f, presentation, {
      expectedQuery: f.query.replace('2008-11-13', '2008-11-14'),
      seenNonces: { has() { touched = true; return false; }, add() { touched = true; } },
    });
    assert.equal(changed.verified, false);
    assert.deepEqual(changed.released, []);
    assert.match(changed.reason, /statement rejected:.*query differs/);
    assert.equal(touched, false);
  });

  for (const depth of [10, 17, 20]) {
    test(`hidden full-width integer comparison proves and verifies with ${two ? 'joined' : 'single'} credentials at depth ${depth}`, async () => {
      const f = filterFixture('integer', two, depth);
      assert.equal(preparedMember(f), `result_v5_k${two ? 2 : 1}_n16_p3_r4_f2_i64_d${depth}`);
      const { presentation, result } = await prove(f);
      assert.deepEqual(result, released);
      const checked = await check(f, presentation);
      assert.equal(checked.verified, true, checked.reason);
      assert.deepEqual(checked.released, released);
    });
  }
}

test('negative integer constants retain the native unsigned refusal', async () => {
  for (const constant of ['-1', '"-1"^^xsd:integer']) {
    const f = filterFixture('integer');
    f.query = f.query.replace('18446744073709551614', constant);
    await assert.rejects(prove(f), /outside disclosure fragment: (FILTER expression|canonical nonnegative integer FILTER bound)/);
  }
});

test('missing filter registry entries refuse both proving and verification', async () => {
  for (const type of ['date', 'integer']) {
    const f = filterFixture(type);
    const { presentation } = await prove(f);
    const member = preparedMember(f);
    const path = new URL('../assets/members.json', import.meta.url);
    const original = readFileSync(path);
    try {
      writeFileSync(path, JSON.stringify(JSON.parse(original).filter(entry => entry.member !== member)));
      await assert.rejects(prove(f), /canonical member or VK rejected/);
      const checked = await check(f, presentation);
      assert.deepEqual(checked, { verified: false, released: [], reason: 'canonical member or VK rejected' });
    } finally { writeFileSync(path, original); }
  }
});
