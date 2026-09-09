// [GPT-6 Astra] Executes unmodified pinned Digital Bazaar RDFC10Sync and NQuads.
// Direct modules avoid loading the unrelated asynchronous setimmediate dependency.
const fs = require('node:fs');
const assert = require('node:assert/strict');
const RDFC10Sync = require('./reference/package/lib/RDFC10Sync');
const NQuads = require('./reference/package/lib/NQuads');
const original = fs.readFileSync('../replay/original.nq', 'utf8');
const renamed = fs.readFileSync('../replay/renamed.nq', 'utf8');
const run = (nq, options) => new RDFC10Sync(options).main(NQuads.parse(nq));
const transform = (s, how) => s.trimEnd().split('\n')[how]().join('\n') + '\n';
let results = {node: process.version, package: 'rdf-canonize@4.0.1', algorithm: 'RDFC-1.0', options: {maxDeepIterations: 16, timeout: 1000}};
for(const [name, input] of Object.entries({original, renamed})) {
  try {results[name + '_default'] = {output: run(input)};}
  catch(e) {results[name + '_default'] = {error: e.message};}
}
const controls = [original, renamed].map(s => s.split('\n').filter(l => l.includes('<http://ex/p>')).join('\n')+'\n');
results.positive_control_equal = run(controls[0]) === run(controls[1]);
assert(results.positive_control_equal);
for(const [name, a, b] of [['exact_pair', original, renamed], ['sorted_lines', transform(original, 'sort'), transform(renamed, 'sort')], ['reverse_lines', transform(original, 'reverse'), transform(renamed, 'reverse')]]) {
  const left=run(a, results.options), right=run(b, results.options);
  results[name]={input_left:a,input_right:b,left,right,equal:left===right};
}
console.log(JSON.stringify(results, null, 2));
