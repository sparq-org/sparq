// [GPT-6 Astra] Three bounded existing W3C fixture controls; no conformance-suite claim.
const fs = require('node:fs');
const assert = require('node:assert/strict');
const RDFC10Sync = require('./reference/package/lib/RDFC10Sync');
const NQuads = require('./reference/package/lib/NQuads');
const root = './upstream-traced/tests/';
const manifest = JSON.parse(fs.readFileSync(root+'manifest.jsonld', 'utf8'));
const results = [];
for(const id of ['#test021c', '#test058c', '#test073c']) {
  const fixture = manifest.entries.find(x => x.id === id);
  const input = fs.readFileSync(root+fixture.action, 'utf8');
  const expected = fs.readFileSync(root+fixture.result, 'utf8');
  const output = new RDFC10Sync({maxDeepIterations:16, timeout:1000}).main(NQuads.parse(input));
  results.push({id, name:fixture.name, action:fixture.action, result:fixture.result, input, expected, output, pass:expected===output});
  assert.equal(output, expected);
}
console.log(JSON.stringify({passed:results.length, results}, null, 2));
