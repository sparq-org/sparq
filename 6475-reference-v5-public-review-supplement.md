# Issue6475: current rdf-canonize5.0.0 reference supplement

OpenAI GPT-6 Astra, actual xhigh lane. Complements the frozen phase2 public packet SHA256 1f862c034d10297101c8c334a8c70866c1ce1dc99143063a86b04adde233259d; it does not revise that packet or assert a standard-level verdict.

The package version and gitHead match tag9502eee92d87e8685d96b9d325477d767c86fb83. The unmodified archive matches npm SHA512 integrity and SHA1. Source is extracted task-locally without running install/lifecycle scripts. Node24.19 executes the same direct synchronous module seam as phase2. The explicit diagnostic options remain maxDeepIterations16 and timeout1000ms; defaults are recorded separately without tuning.

All6 quad orders across both original and renamed labels return2 distinct canonical strings under those diagnostic options. Same-order relabeling agrees. All12 default cases instead reject at2 deep iterations. The one-edge control and packaged W3C test021c,058c,073c pass. The tested algorithm/helper source files are byte-identical to4.0.1. Thus5.0.0 does not fix the observed ordering defect; the normative interpretation remains for independent review.

## Identity, conclusions, complete output association and limits

```json
{
  "author": "OpenAI GPT-6 Astra; actual xhigh lane",
  "package": "rdf-canonize5.0.0",
  "gitHead": "9502eee92d87e8685d96b9d325477d767c86fb83",
  "metadata_url": "https://registry.npmjs.org/rdf-canonize/5.0.0",
  "archive_url": "https://registry.npmjs.org/rdf-canonize/-/rdf-canonize-5.0.0.tgz",
  "archive_sha256": "ad8ef5d1a3400a152668de5f3ab4611b1b3b2347bb237542cd8e3c7835707f7a",
  "archive_integrity": "sha512-g8OUrgMXAR9ys/ZuJVfBr05sPPoMA7nHIVs8VEvg9QwM5W4GR2qSFEEHjsyHF1eWlBaf8Ev40WNjQFQ+nJTO3w==",
  "integrity_verified": true,
  "node": "v24.19.0",
  "algorithm": "unmodified RDFC10Sync directly; no async loader, install, native addon or lifecycle script",
  "diagnostic_options": {
    "maxDeepIterations": 16,
    "timeout": 1000
  },
  "defaults": "No options passed; maxWorkFactor1 calculates2 deep iterations for the saved pair. All12 orders rejected with the same documented error.",
  "summary": {
    "default_errors": 12,
    "diagnostic_successes": 12,
    "distinct_outputs": 2,
    "same_order_relabel_equal": true,
    "order_invariant": false
  },
  "single_edge_positive_control": true,
  "existing_W3C_fixture_controls": {
    "passed": 3,
    "ids": [
      "#test021c",
      "#test058c",
      "#test073c"
    ]
  },
  "outputs": {
    "d426d4355240eeb9ba0426c80989fbd4f98f258dfe470e9e2041132307bcef86": {
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
      "cases": [
        {
          "labeling": "original",
          "order": [
            0,
            1,
            2
          ]
        },
        {
          "labeling": "original",
          "order": [
            0,
            2,
            1
          ]
        },
        {
          "labeling": "original",
          "order": [
            1,
            0,
            2
          ]
        },
        {
          "labeling": "renamed",
          "order": [
            0,
            1,
            2
          ]
        },
        {
          "labeling": "renamed",
          "order": [
            0,
            2,
            1
          ]
        },
        {
          "labeling": "renamed",
          "order": [
            1,
            0,
            2
          ]
        }
      ]
    },
    "7807795c1390a9b22935165bfb5d0a3f93af026ae4d1266ea96c43990864ed50": {
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n",
      "cases": [
        {
          "labeling": "original",
          "order": [
            1,
            2,
            0
          ]
        },
        {
          "labeling": "original",
          "order": [
            2,
            0,
            1
          ]
        },
        {
          "labeling": "original",
          "order": [
            2,
            1,
            0
          ]
        },
        {
          "labeling": "renamed",
          "order": [
            1,
            2,
            0
          ]
        },
        {
          "labeling": "renamed",
          "order": [
            2,
            0,
            1
          ]
        },
        {
          "labeling": "renamed",
          "order": [
            2,
            1,
            0
          ]
        }
      ]
    }
  },
  "exact_phase2_cases_byte_identical": true,
  "algorithm_source_comparison": "RDFC10Sync.js, RDFC10.js, IdentifierIssuer.js, MessageDigest.js, NQuads.js and Permuter.js are each byte-identical to4.0.1. Complete per-file hashes retained.",
  "conclusion": "Current5.0.0 reference does not remove the observed order dependence under the fixed bounded diagnostic options. This corroborates phase2 behavior; standard-level soundness interpretation remains provisional for independent review.",
  "limits": [
    "Default rejection is not canonicalization success.",
    "Same-order relabeling agrees; the independently observed defect here is quad-order dependence.",
    "Only the declared counterexample/orders and three W3C controls were executed, not full conformance or proof.",
    "Direct synchronous module invocation was inspected and uses built-in Node crypto; normal async public API was not executed.",
    "No production edit, dependency change, gate weakening, remote mutation, installation or new model call."
  ]
}
```

## reference-replay.cjs

```text
// [GPT-6 Astra] Executes unmodified pinned Digital Bazaar RDFC10Sync and NQuads.
// Direct modules avoid loading the unrelated asynchronous setimmediate dependency.
const fs = require('node:fs');
const assert = require('node:assert/strict');
const RDFC10Sync = require('./reference/package/lib/RDFC10Sync');
const NQuads = require('./reference/package/lib/NQuads');
const original = fs.readFileSync('./original.nq', 'utf8');
const renamed = fs.readFileSync('./renamed.nq', 'utf8');
const run = (nq, options) => new RDFC10Sync(options).main(NQuads.parse(nq));
const transform = (s, how) => s.trimEnd().split('\n')[how]().join('\n') + '\n';
let results = {node: process.version, package: 'rdf-canonize@5.0.0', algorithm: 'RDFC-1.0', options: {maxDeepIterations: 16, timeout: 1000}};
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
const orders = [[0,1,2],[0,2,1],[1,0,2],[1,2,0],[2,0,1],[2,1,0]];
results.all_orders = [];
for(const [labeling, input] of Object.entries({original, renamed})) {
  const lines = input.trimEnd().split('\n');
  for(const order of orders) {
    const ordered = order.map(i => lines[i]).join('\n')+'\n';
    let defaultResult;
    try {defaultResult = {output:run(ordered)};}
    catch(e) {defaultResult = {error:e.message};}
    const output = run(ordered, results.options);
    results.all_orders.push({labeling, order, input:ordered, defaultResult, output});
  }
}
results.summary = {
  default_errors:results.all_orders.filter(x=>x.defaultResult.error).length,
  diagnostic_successes:results.all_orders.length,
  distinct_outputs:new Set(results.all_orders.map(x=>x.output)).size,
  same_order_relabel_equal:orders.every((_,i)=>results.all_orders[i].output===results.all_orders[i+6].output),
  order_invariant:new Set(results.all_orders.map(x=>x.output)).size===1
};
console.log(JSON.stringify(results, null, 2));
```

## reference-replay.json

```text
{
  "node": "v24.19.0",
  "package": "rdf-canonize@5.0.0",
  "algorithm": "RDFC-1.0",
  "options": {
    "maxDeepIterations": 16,
    "timeout": 1000
  },
  "original_default": {
    "error": "Maximum deep iterations exceeded (2)."
  },
  "renamed_default": {
    "error": "Maximum deep iterations exceeded (2)."
  },
  "positive_control_equal": true,
  "exact_pair": {
    "input_left": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n",
    "input_right": "_:zz5 <http://ex/q> _:zz4 _:zz3 .\n_:zz5 <http://ex/p> _:zz0 .\n_:zz0 <http://ex/q> _:zz3 _:zz4 .\n",
    "left": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
    "right": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
    "equal": true
  },
  "sorted_lines": {
    "input_left": "_:b0 <http://ex/q> _:b3 _:b2 .\n_:b4 <http://ex/p> _:b0 .\n_:b4 <http://ex/q> _:b2 _:b3 .\n",
    "input_right": "_:zz0 <http://ex/q> _:zz3 _:zz4 .\n_:zz5 <http://ex/p> _:zz0 .\n_:zz5 <http://ex/q> _:zz4 _:zz3 .\n",
    "left": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n",
    "right": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n",
    "equal": true
  },
  "reverse_lines": {
    "input_left": "_:b0 <http://ex/q> _:b3 _:b2 .\n_:b4 <http://ex/p> _:b0 .\n_:b4 <http://ex/q> _:b2 _:b3 .\n",
    "input_right": "_:zz0 <http://ex/q> _:zz3 _:zz4 .\n_:zz5 <http://ex/p> _:zz0 .\n_:zz5 <http://ex/q> _:zz4 _:zz3 .\n",
    "left": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n",
    "right": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n",
    "equal": true
  },
  "all_orders": [
    {
      "labeling": "original",
      "order": [
        0,
        1,
        2
      ],
      "input": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
    },
    {
      "labeling": "original",
      "order": [
        0,
        2,
        1
      ],
      "input": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n_:b4 <http://ex/p> _:b0 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
    },
    {
      "labeling": "original",
      "order": [
        1,
        0,
        2
      ],
      "input": "_:b4 <http://ex/p> _:b0 .\n_:b4 <http://ex/q> _:b2 _:b3 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
    },
    {
      "labeling": "original",
      "order": [
        1,
        2,
        0
      ],
      "input": "_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n_:b4 <http://ex/q> _:b2 _:b3 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
    },
    {
      "labeling": "original",
      "order": [
        2,
        0,
        1
      ],
      "input": "_:b0 <http://ex/q> _:b3 _:b2 .\n_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
    },
    {
      "labeling": "original",
      "order": [
        2,
        1,
        0
      ],
      "input": "_:b0 <http://ex/q> _:b3 _:b2 .\n_:b4 <http://ex/p> _:b0 .\n_:b4 <http://ex/q> _:b2 _:b3 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
    },
    {
      "labeling": "renamed",
      "order": [
        0,
        1,
        2
      ],
      "input": "_:zz5 <http://ex/q> _:zz4 _:zz3 .\n_:zz5 <http://ex/p> _:zz0 .\n_:zz0 <http://ex/q> _:zz3 _:zz4 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
    },
    {
      "labeling": "renamed",
      "order": [
        0,
        2,
        1
      ],
      "input": "_:zz5 <http://ex/q> _:zz4 _:zz3 .\n_:zz0 <http://ex/q> _:zz3 _:zz4 .\n_:zz5 <http://ex/p> _:zz0 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
    },
    {
      "labeling": "renamed",
      "order": [
        1,
        0,
        2
      ],
      "input": "_:zz5 <http://ex/p> _:zz0 .\n_:zz5 <http://ex/q> _:zz4 _:zz3 .\n_:zz0 <http://ex/q> _:zz3 _:zz4 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
    },
    {
      "labeling": "renamed",
      "order": [
        1,
        2,
        0
      ],
      "input": "_:zz5 <http://ex/p> _:zz0 .\n_:zz0 <http://ex/q> _:zz3 _:zz4 .\n_:zz5 <http://ex/q> _:zz4 _:zz3 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
    },
    {
      "labeling": "renamed",
      "order": [
        2,
        0,
        1
      ],
      "input": "_:zz0 <http://ex/q> _:zz3 _:zz4 .\n_:zz5 <http://ex/q> _:zz4 _:zz3 .\n_:zz5 <http://ex/p> _:zz0 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
    },
    {
      "labeling": "renamed",
      "order": [
        2,
        1,
        0
      ],
      "input": "_:zz0 <http://ex/q> _:zz3 _:zz4 .\n_:zz5 <http://ex/p> _:zz0 .\n_:zz5 <http://ex/q> _:zz4 _:zz3 .\n",
      "defaultResult": {
        "error": "Maximum deep iterations exceeded (2)."
      },
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
    }
  ],
  "summary": {
    "default_errors": 12,
    "diagnostic_successes": 12,
    "distinct_outputs": 2,
    "same_order_relabel_equal": true,
    "order_invariant": false
  }
}
```

## reference-fixtures.cjs

```text
// [GPT-6 Astra] Three bounded existing W3C fixture controls; no conformance-suite claim.
const fs = require('node:fs');
const assert = require('node:assert/strict');
const RDFC10Sync = require('./reference/package/lib/RDFC10Sync');
const NQuads = require('./reference/package/lib/NQuads');
const root = './fixtures/';
const manifest = JSON.parse(fs.readFileSync(root+'manifest.jsonld', 'utf8'));
const results = [];
for(const id of ['#test021c', '#test058c', '#test073c']) {
  const fixture = manifest.entries.find(x => x.id === id);
  const input = fs.readFileSync(root+fixture.action.replace('rdfc10/',''), 'utf8');
  const expected = fs.readFileSync(root+fixture.result.replace('rdfc10/',''), 'utf8');
  const output = new RDFC10Sync({maxDeepIterations:16, timeout:1000}).main(NQuads.parse(input));
  results.push({id, name:fixture.name, action:fixture.action, result:fixture.result, input, expected, output, pass:expected===output});
  assert.equal(output, expected);
}
console.log(JSON.stringify({passed:results.length, results}, null, 2));
```

## reference-fixtures.json

```text
{
  "passed": 3,
  "results": [
    {
      "id": "#test021c",
      "name": "blank node - circle of 2",
      "action": "rdfc10/test021-in.nq",
      "result": "rdfc10/test021-rdfc10.nq",
      "input": "_:e0 <http://example.org/vocab#next> _:e1 .\n_:e1 <http://example.org/vocab#next> _:e0 .\n",
      "expected": "_:c14n0 <http://example.org/vocab#next> _:c14n1 .\n_:c14n1 <http://example.org/vocab#next> _:c14n0 .\n",
      "output": "_:c14n0 <http://example.org/vocab#next> _:c14n1 .\n_:c14n1 <http://example.org/vocab#next> _:c14n0 .\n",
      "pass": true
    },
    {
      "id": "#test058c",
      "name": "unnamed graph with blank node objects",
      "action": "rdfc10/test058-in.nq",
      "result": "rdfc10/test058-rdfc10.nq",
      "input": "<https://example.com/1> <https://example.com/2> _:e0 _:e3 .\n<https://example.com/1> <https://example.com/2> _:e1 _:e3 .\n",
      "expected": "<https://example.com/1> <https://example.com/2> _:c14n1 _:c14n0 .\n<https://example.com/1> <https://example.com/2> _:c14n2 _:c14n0 .\n",
      "output": "<https://example.com/1> <https://example.com/2> _:c14n1 _:c14n0 .\n<https://example.com/1> <https://example.com/2> _:c14n2 _:c14n0 .\n",
      "pass": true
    },
    {
      "id": "#test073c",
      "name": "dataset - referencing graph name",
      "action": "rdfc10/test073-in.nq",
      "result": "rdfc10/test073-rdfc10.nq",
      "input": "<http://example.org/test> <http://example.org/vocab#A> _:e0 .\n<http://example.org/test> <http://example.org/vocab#B> _:e0 .\n<http://example.org/test> <http://example.org/vocab#embed> _:e0 .\n<http://example.org/test> <http://example.org/vocab#graph> _:g1 .\n<http://example.org/test> <http://example.org/vocab#A> _:e0  _:g1 .\n<http://example.org/test> <http://example.org/vocab#B> _:e0 _:g1 .\n<http://example.org/test> <http://example.org/vocab#embed> _:e0 _:g1 .\n",
      "expected": "<http://example.org/test> <http://example.org/vocab#A> _:c14n1 .\n<http://example.org/test> <http://example.org/vocab#A> _:c14n1 _:c14n0 .\n<http://example.org/test> <http://example.org/vocab#B> _:c14n1 .\n<http://example.org/test> <http://example.org/vocab#B> _:c14n1 _:c14n0 .\n<http://example.org/test> <http://example.org/vocab#embed> _:c14n1 .\n<http://example.org/test> <http://example.org/vocab#embed> _:c14n1 _:c14n0 .\n<http://example.org/test> <http://example.org/vocab#graph> _:c14n0 .\n",
      "output": "<http://example.org/test> <http://example.org/vocab#A> _:c14n1 .\n<http://example.org/test> <http://example.org/vocab#A> _:c14n1 _:c14n0 .\n<http://example.org/test> <http://example.org/vocab#B> _:c14n1 .\n<http://example.org/test> <http://example.org/vocab#B> _:c14n1 _:c14n0 .\n<http://example.org/test> <http://example.org/vocab#embed> _:c14n1 .\n<http://example.org/test> <http://example.org/vocab#embed> _:c14n1 _:c14n0 .\n<http://example.org/test> <http://example.org/vocab#graph> _:c14n0 .\n",
      "pass": true
    }
  ]
}
```

## source-comparison.json

```text
[
  {
    "path": "LICENSE",
    "new_sha256": "ce809f30d8416b87c46a5affa6e5f8cdad868df31045986ea510bed856b397aa",
    "old_sha256": "ce809f30d8416b87c46a5affa6e5f8cdad868df31045986ea510bed856b397aa",
    "identical": true
  },
  {
    "path": "README.md",
    "new_sha256": "72815c66b35797b99eb0fcbd5eaec602ebdf27f640a8aa2cc1c86ddb2f71f068",
    "old_sha256": "5cc7fd73a9e6f645c3192d417978078967ef1512091a774c99671e487b1b7bb8",
    "identical": false
  },
  {
    "path": "index.js",
    "new_sha256": "f255d71cfe0cc54bd85753e8227bb6b0071cb12deee963cd3a216f07e17d14a8",
    "old_sha256": "f255d71cfe0cc54bd85753e8227bb6b0071cb12deee963cd3a216f07e17d14a8",
    "identical": true
  },
  {
    "path": "lib/IdentifierIssuer.js",
    "new_sha256": "83aca58cc739075dd5aebd25bf5fac8d41ed4f008add508604a409c1989da9ee",
    "old_sha256": "83aca58cc739075dd5aebd25bf5fac8d41ed4f008add508604a409c1989da9ee",
    "identical": true
  },
  {
    "path": "lib/MessageDigest-webcrypto.js",
    "new_sha256": "d05a577b14e39cdbf49b3f72d7dc2a127073e6413350c3cdb76df713c6d0a3bd",
    "old_sha256": "d05a577b14e39cdbf49b3f72d7dc2a127073e6413350c3cdb76df713c6d0a3bd",
    "identical": true
  },
  {
    "path": "lib/MessageDigest.js",
    "new_sha256": "3b211d9b672c1b440cd4f47f70dec96898669c51902d4912599e4f448de52ea9",
    "old_sha256": "3b211d9b672c1b440cd4f47f70dec96898669c51902d4912599e4f448de52ea9",
    "identical": true
  },
  {
    "path": "lib/NQuads.js",
    "new_sha256": "7555c8cdff0dd702ff0c0ef836200fda382746b02e0b9f657c012d4c2c2b3a32",
    "old_sha256": "7555c8cdff0dd702ff0c0ef836200fda382746b02e0b9f657c012d4c2c2b3a32",
    "identical": true
  },
  {
    "path": "lib/Permuter.js",
    "new_sha256": "4a4e9e3d1c461d1937da98e9edd258d0b95be7cfa402be2c3d75aaeeb21e24d8",
    "old_sha256": "4a4e9e3d1c461d1937da98e9edd258d0b95be7cfa402be2c3d75aaeeb21e24d8",
    "identical": true
  },
  {
    "path": "lib/RDFC10.js",
    "new_sha256": "454a1158dd18a1559f7259573ffd9748999921cd18064c3ed48ab2b135446a98",
    "old_sha256": "454a1158dd18a1559f7259573ffd9748999921cd18064c3ed48ab2b135446a98",
    "identical": true
  },
  {
    "path": "lib/RDFC10Sync.js",
    "new_sha256": "0f08af51ac41dc730f146cc1eb12b07d913f67ffbf1498d9b124e1cef589a02d",
    "old_sha256": "0f08af51ac41dc730f146cc1eb12b07d913f67ffbf1498d9b124e1cef589a02d",
    "identical": true
  },
  {
    "path": "lib/index.js",
    "new_sha256": "8bc236cb8cea6a8e6fb8cf46e826bae7bb6618883c83413000e16d45d57726ac",
    "old_sha256": "cb35db662298276d79cc31cba6c88d7d6a443ddece4814b173daad83ece17269",
    "identical": false
  },
  {
    "path": "lib/platform-browser.js",
    "new_sha256": "03db9714392fb29c13dae8aae3d8f2b4ce7a91e7b7c17bccdb2f59043f715e9c",
    "old_sha256": "03db9714392fb29c13dae8aae3d8f2b4ce7a91e7b7c17bccdb2f59043f715e9c",
    "identical": true
  },
  {
    "path": "lib/platform.js",
    "new_sha256": "5b670b95cf78d897a8fcc664b1aa8f13d58589c33032d63dd6a827871ed20d1d",
    "old_sha256": "5b670b95cf78d897a8fcc664b1aa8f13d58589c33032d63dd6a827871ed20d1d",
    "identical": true
  },
  {
    "path": "package.json",
    "new_sha256": "4fea40db683ef464ac7e36227d7c80e7076b6dc92e262c9d342058e4b1cf36cf",
    "old_sha256": "e4c57203b59332365eb04d8f3267ee951b8644ba3c6f77fab8cd2035ad7c3dab",
    "identical": false
  }
]
```

## wrapper-4-to-5.diff

```text
--- rdf-canonize4.0.1/lib/index.js
+++ rdf-canonize5.0.0/lib/index.js
@@ -37,12 +37,6 @@
 const RDFC10 = require('./RDFC10');
 const RDFC10Sync = require('./RDFC10Sync');
 
-// optional native support
-let rdfCanonizeNative;
-try {
-  rdfCanonizeNative = require('rdf-canonize-native');
-} catch(e) {}
-
 // return a dataset from input dataset or n-quads
 function _inputToDataset(input, options) {
   if(options.inputFormat) {
@@ -79,20 +73,6 @@
 // expose helpers
 exports.NQuads = require('./NQuads');
 exports.IdentifierIssuer = require('./IdentifierIssuer');
-
-/**
- * Get or set native API.
- *
- * @param {object} [api] - The native API.
- *
- * @returns {object} - The currently set native API.
- */
-exports._rdfCanonizeNative = function(api) {
-  if(api) {
-    rdfCanonizeNative = api;
-  }
-  return rdfCanonizeNative;
-};
 
 /**
  * Asynchronously canonizes an RDF dataset.
@@ -118,7 +98,6 @@
  *     falsy for a JSON dataset.
  *   {string} [format] - The format of the output. Omit or use
  *     'application/n-quads' for a N-Quads string.
- *   {boolean} [useNative=false] - Use native implementation.
  *   {number} [maxWorkFactor=1] - Control of the maximum number of times to run
  *     deep comparison algorithms (such as the N-Degree Hash Quads algorithm
  *     used in RDFC-1.0) before bailing out and throwing an error; this is a
@@ -147,19 +126,6 @@
 exports.canonize = async function(input, options = {}) {
   const dataset = _inputToDataset(input, options);
   _checkOutputFormat(options);
-
-  if(options.useNative) {
-    if(!rdfCanonizeNative) {
-      throw new Error('rdf-canonize-native not available');
-    }
-    if(options.createMessageDigest) {
-      throw new Error(
-        '"createMessageDigest" cannot be used with "useNative".');
-    }
-    return new Promise((resolve, reject) =>
-      rdfCanonizeNative.canonize(dataset, options, (err, canonical) =>
-        err ? reject(err) : resolve(canonical)));
-  }
 
   if(!('algorithm' in options)) {
     throw new Error('No RDF Dataset Canonicalization algorithm specified.');
@@ -202,7 +168,6 @@
  *     falsy for a JSON dataset.
  *   {string} [format] - The format of the output. Omit or use
  *     'application/n-quads' for a N-Quads string.
- *   {boolean} [useNative=false] - Use native implementation.
  *   {number} [maxWorkFactor=1] - Control of the maximum number of times to run
  *     deep comparison algorithms (such as the N-Degree Hash Quads algorithm
  *     used in RDFC-1.0) before bailing out and throwing an error; this is a
@@ -236,16 +201,6 @@
   const dataset = _inputToDataset(input, options);
   _checkOutputFormat(options);
 
-  if(options.useNative) {
-    if(!rdfCanonizeNative) {
-      throw new Error('rdf-canonize-native not available');
-    }
-    if(options.createMessageDigest) {
-      throw new Error(
-        '"createMessageDigest" cannot be used with "useNative".');
-    }
-    return rdfCanonizeNative.canonizeSync(dataset, options);
-  }
   if(!('algorithm' in options)) {
     throw new Error('No RDF Dataset Canonicalization algorithm specified.');
   }
```

This source-only supplement omits host paths, environment and operational/build logs. Full unmodified package source, archive, commands and integrity evidence remain in its local hashed bundle. No W3C or upstream issue status is claimed here.
