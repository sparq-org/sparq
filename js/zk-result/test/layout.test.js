import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fixture } from './helpers/fixtures.js';
import { filterFixture } from './helpers/filters.js';

const host = createRequire(import.meta.url)('../wasm/sparq_zk_result_wasm.cjs');
const read = name => readFileSync(new URL(name, import.meta.url), 'utf8');

function expression(text, K, F) {
  return text.split('+').reduce((sum, term) => {
    const match = /^(\d*)([KF]?)$/.exec(term.trim());
    assert.ok(match && (match[1] || match[2]), `invalid documented expression: ${text}`);
    return sum + Number(match[1] || 1) * (match[2] === 'K' ? K : match[2] === 'F' ? F : 1);
  }, 0);
}

function flatten(value) {
  return Array.isArray(value) ? value.flatMap(flatten) : [BigInt(value)];
}

test('README public input order, positions and counts match reconstruction for every bundled member', () => {
  const readme = read('../README.md');
  const section = readme.split('<!-- public-input-layout:start -->')[1]?.split('<!-- public-input-layout:end -->')[0];
  assert.ok(section, 'documented layout table must exist');
  const layout = section.split('\n').filter(line => /^\| \d/.test(line)).map(line => {
    const [, start, field, words] = line.split('|').map(s => s.trim().replaceAll('`', ''));
    return { start, field, words };
  });
  assert.ok(layout.length, 'documented layout must not be empty');
  const total = /The total is `([^`]+)` words/.exec(readme)?.[1];
  assert.ok(total, 'documented total must exist');
  const members = JSON.parse(read('../assets/members.json'));
  assert.ok(members.length, 'bundled registry must not be empty');
  for (const entry of members) {
    const [, capacity, predicates, depth] = /^result_v5_k(\d)_n16_p3_r4_(f0|f1_date|f2_i64)_d(\d+)$/.exec(entry.member);
    const K = Number(capacity), F = predicates === 'f0' ? 0 : predicates === 'f1_date' ? 1 : 2;
    const f = F === 0 ? fixture(K === 2) : filterFixture(F === 1 ? 'date' : 'integer', K === 2, Number(depth));
    const prepared = JSON.parse(host.prepare(f.query, f.credentials.map(c => JSON.stringify(c)), JSON.stringify(f.policy), f.nonce, '{}'));
    assert.equal(prepared.member, entry.member);
    const reconstructed = JSON.parse(host.reconstructPublicInputs(f.query, JSON.stringify(prepared.presentationSkeleton), JSON.stringify(f.policy), f.nonce));
    assert.equal(reconstructed.member, entry.member);
    const abi = JSON.parse(read(`../assets/${entry.acir}`)).abi.parameters.filter(p => p.visibility === 'public');
    const active = layout.filter(row => expression(row.words, K, F) !== 0);
    assert.deepEqual(active.map(row => row.field), abi.map(p => p.name), entry.member);
    let offset = 0;
    const documentedInputs = [];
    for (const row of active) {
      assert.equal(expression(row.start, K, F), offset, `${entry.member}: ${row.field} position`);
      const words = flatten(prepared.witnessInputs[row.field]);
      assert.equal(words.length, expression(row.words, K, F), `${entry.member}: ${row.field} count`);
      assert.deepEqual(reconstructed.publicInputs.slice(offset, offset + words.length).map(BigInt), words,
        `${entry.member}: ${row.field} public values`);
      offset += words.length;
      documentedInputs.push(...words);
    }
    assert.equal(offset, reconstructed.publicInputs.length, `${entry.member}: total count`);
    assert.equal(expression(total, K, F), offset, `${entry.member}: documented total`);
    assert.deepEqual(reconstructed.publicInputs.map(BigInt), documentedInputs, entry.member);
  }
});
