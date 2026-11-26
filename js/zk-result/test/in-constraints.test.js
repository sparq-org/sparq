import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Noir } from '@noir-lang/noir_js';
import { testIssuerKeygen, testIssue } from '../index.js';
import { inFixture } from './helpers/in.js';

const host = createRequire(import.meta.url)('../wasm/sparq_zk_result_wasm.cjs');
const prepare = f => JSON.parse(host.prepare(f.query, f.credentials.map(c => JSON.stringify(c)), JSON.stringify(f.policy), f.nonce, '{}'));

test('scoped IN admits one pattern per declared graph in preparation and reconstruction', () => {
  const f = inFixture(true);
  const prepared = prepare(f);
  const query = f.query.replace(/^ +\?h vcq:verifiedIssuer[^\n]+\n/m, '').replace('GRAPH ?h', 'GRAPH ?g');
  const reason = /scoped IN requires one triple pattern per graph variable/;
  assert.throws(() => prepare({ ...f, query }), reason);
  const presentation = { ...prepared.presentationSkeleton, query };
  assert.throws(() => host.reconstructPublicInputs(query, JSON.stringify(presentation), JSON.stringify(f.policy), f.nonce), reason);
});

for (const two of [false, true]) {
  test(`hidden IN checks both active rows with ${two ? 'joined' : 'single'} credentials`, async () => {
    const f = inFixture(two, 'BSc', 'MSc');
    const prepared = prepare(f);
    const member = `result_v5_k${two ? 2 : 1}_n16_p3_r4_f1_in_d10`;
    assert.equal(prepared.member, member);
    const noir = new Noir(JSON.parse(readFileSync(new URL(`../assets/${member}.json`, import.meta.url))));
    const rows = prepared.presentationSkeleton.rows;
    assert.deepEqual(new Set(rows), new Set([
      { subject: { kind: 'iri', value: 'urn:subject' } },
      { subject: { kind: 'iri', value: 'urn:subject:second' } },
    ]));
    assert.equal(Number(prepared.witnessInputs.result_count), 2);
    await noir.execute(prepared.witnessInputs);

    const firstDegree = rows[0].subject.value === 'urn:subject' ? 'BSc' : 'MSc';
    const query = f.query.replace('"BSc", "MSc"', `"${firstDegree}"`);
    const singleton = prepare({ ...f, query });
    assert.deepEqual(singleton.presentationSkeleton.rows, [rows[0]]);
    await noir.execute(singleton.witnessInputs);

    const witness = { ...prepared.witnessInputs,
      in_filter_count: singleton.witnessInputs.in_filter_count,
      in_filter_constants: singleton.witnessInputs.in_filter_constants,
    };
    await assert.rejects(noir.execute(witness), /IN membership is false/);
  });

  test(`hidden IN constraints retain authentication and policy with ${two ? 'joined' : 'single'} credentials`, async () => {
    const prepared = prepare(inFixture(two));
    const member = `result_v5_k${two ? 2 : 1}_n16_p3_r4_f1_in_d10`;
    assert.equal(prepared.member, member);
    const noir = new Noir(JSON.parse(readFileSync(new URL(`../assets/${member}.json`, import.meta.url))));
    await noir.execute(prepared.witnessInputs);
    const fullList = structuredClone(prepared.witnessInputs);
    fullList.in_filter_count = 16;
    const [match, miss] = fullList.in_filter_constants;
    fullList.in_filter_constants.fill(miss);
    fullList.in_filter_constants[15] = match;
    await noir.execute(fullList);
    fullList.in_filter_constants[15] = miss;
    await assert.rejects(noir.execute(fullList), /IN membership is false/);
    for (const [change, reason] of [
      [w => { w.in_filter_constants[0] = w.in_filter_constants[1]; }, /IN membership is false/],
      [w => { w.values[0][Number(w.in_filter_var) - 1] = w.in_filter_constants[1]; }, /joined variable mismatch/],
      [w => { w.accepted_root = '0x01'; }, /credential status outside verifier policy/],
      [w => { w.salts[two ? 1 : 0] = '0x01'; }, /challenge reduction not bound to its preimage/],
      [w => { w.filter_count = 1; }, /filter capacity/],
      [w => { w.in_filter_count = 0; }, /IN list capacity/],
      [w => { w.in_filter_count = 17; }, /IN list capacity/],
      [w => { w.in_filter_var = 0; }, /IN variable capacity/],
      [w => { w.in_filter_var = 7; }, /IN variable capacity/],
      [w => { w.in_filter_constants[15] = w.in_filter_constants[0]; }, /noncanonical IN padding/],
      [w => { w.in_filter_var = 6; }, /unbound IN variable/],
      [w => { w.projection[Number(w.in_filter_var) - 1] = true; }, /IN member requires hidden operand/],
    ]) {
      const witness = structuredClone(prepared.witnessInputs);
      change(witness);
      await assert.rejects(noir.execute(witness), reason);
    }
  });
}

test('IN normalization retains typed numeric admission and comparisons', () => {
  for (const [constant, lower, lowerLexical] of [
    ['21', '18', '18'],
    ['"2000-01-01"^^<http://www.w3.org/2001/XMLSchema#date>', '"1999-12-31"^^<http://www.w3.org/2001/XMLSchema#date>', '1999-12-31'],
  ]) {
    const f = inFixture();
    const issuer = testIssuerKeygen('a1');
    const object = constant === '21' ? '"21"^^<http://www.w3.org/2001/XMLSchema#integer>' : constant;
    f.credentials = [testIssue({ issuer,
      graphNQuads: `<urn:subject> <urn:claim:degree.level> ${object} .`,
      status: { status_list: 'urn:list', version: 1, index: 0 }, seedHex: 'c3' })];
    const comparison = `?value >= ${constant}`;
    f.query = `SELECT DISTINCT ?value WHERE { ?s <urn:claim:degree.level> ?value . FILTER(?value IN (${constant}, ${lower}) && ${comparison}) }`;
    const prepared = prepare(f);
    assert.equal(prepared.member, 'result_v5_k1_n16_p3_r4_f0_d10');
    const changed = structuredClone(prepared.presentationSkeleton);
    changed.rows[0].value.value = lowerLexical;
    assert.throws(() => host.reconstructPublicInputs(f.query, JSON.stringify(changed), JSON.stringify(f.policy), f.nonce), /public FILTER is false/);
    const failing = { ...f, query: f.query.replace('>=', '<') };
    assert.throws(() => prepare(failing), /selected credential capacity/);
    for (const invalid of ['-1', '"01"^^<http://www.w3.org/2001/XMLSchema#integer>', '"2000-01-01Z"^^<http://www.w3.org/2001/XMLSchema#date>']) {
      assert.throws(() => prepare({ ...f, query: f.query.replace(comparison, `?value >= ${invalid}`) }), /outside disclosure fragment/);
    }
  }
});
