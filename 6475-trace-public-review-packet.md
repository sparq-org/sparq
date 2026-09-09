# Issue6475: upstream canonicalization trace for independent soundness review

Author: OpenAI GPT-6 Astra, actual xhigh lane. This is diagnosis, not an independently reviewed repair.
Production source remains exact e53464c73f31f7aca800f3867ac054c36408e346. No production edit or candidate correction.

Please assess whether the loss of quad correlation below is a counterexample to the normative RDFC1.0 algorithm, a Rust-only implementation error, or an incomplete diagnosis. Do not treat the standard-level explanation as established. Recommend the smallest responsible next step while preserving canonical-output compatibility, invariance testing, and bounded computation. No policy or gate weakening is proposed.

## Findings

- Scratch-only instrumentation leaves direct upstream output byte-identical to phase1 and Linux CI.
- b2 and b3 have the same first-degree hash 166b37005906f602b8c3e82241f325ff2d22e757fe1ec677be8f8684c7805218; anchors b0/b4 are c14n0/c14n1.
- All four top-level N-degree calls (b2,b3,zz3,zz4) have identical preimage bytes and SHA256 a812f907ae7c594b7fd02586a57c254860f399e64e6b3bbf13fa0e56e38f81e8. This is information loss before hashing, not a cryptographic collision.
- Related hashes encode s+q+anchor, g+other and o+q+other independently. Grouping discards which anchor shares the quad with each object/graph role.
- Stably sorting tied hash results preserves BTreeMap input-label order, choosing b2 before b3 and zz3 before zz4. The temporary issuers are swapped; canonical outputs differ.
- Unmodified Digital Bazaar rdf-canonize4.0.1 exactly preserves relabeling for the provided input order, but reversing only quad order changes canonical output for each dataset.
- The reference default budget rejects both inputs at2 deep iterations. Reported ordering defect uses explicit maxDeepIterations16 and timeout1000ms only in this isolated diagnostic; no production budget changed.
- Reference one-edge positive control and existing W3C test021c/test058c/test073c pass. This is not full conformance validation.

The sole p edge fixes b4 and b0, so swapping b2/b3 while retaining these anchors is not an automorphism. Both nodes see the same multiset of independently hashed related positions, despite distinct quad correlations. The tie is therefore not justified merely by the existence of equal hashes.

Normative correspondence (provisional): §4.7.3 steps1–5 build related hashes; §4.8.3 steps3.1.1–3.1.2 place them independently into Hn; §4.4.3 steps5.2–5.3 sort N-degree results by hash and assign issuer order. The selected path appears to follow these steps. The contract expects isomorphic datasets to canonicalize identically.

- conformance: https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#conformance
- canonicalization: https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#canon-algorithm
- related: https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-related-algorithm
- ndegree: https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-nd-quads-algorithm
- upstream_rust: https://github.com/zkp-ld/rdf-canon/tree/v0.15.3
- reference: https://github.com/digitalbazaar/rdf-canonize/tree/7d06569ac53441bc4786943d2336c98cbb29d1bb
- reference_metadata: https://registry.npmjs.org/rdf-canonize/4.0.1

The >= pruning discrepancy cannot explain this input: all related groups are singletons and no prior chosen path exists. Temporary IDs never exceed b1. Dataset multiplicity and bridge parsing are also absent. These observations rule out the tempting small fixes, not every possible implementation defect.

## Limits and next step

- One pinned reference version, not a latest-version survey.
- Reference synchronous internal module invoked directly to avoid unrelated async dependency loading; actual unmodified source and package archive retained.
- Reference has independent implementation structure but may share standard lineage; not an independent mathematical proof.
- No candidate correction, Rust upstream fixture suite, broader randomized campaign, full workspace, or engine build run.
- Only three packaged W3C fixtures executed, on JS reference.
- No standard-level claim accepted before independent review.

Adding quad context would change the specified hash algorithm. Sorting original labels, changing input ordering, or silently rejecting the saved shape does not establish conformant canonicalization. Tied hashes can also represent genuine symmetry; blanket tie rejection is not justified.

Have independent Opus review exact trace, topology and primary-source correspondence. If confirmed, root can prepare an upstream/W3C reproducer and choose a compatibility-preserving repair or explicitly reviewed fail-closed contract; do not weaken the invariance property.

The independent reference package is rdf-canonize4.0.1, npm gitHead7d06569ac53441bc4786943d2336c98cbb29d1bb; archive integrity matched the registry SHA512. Its RDFC10Sync, NQuads, IdentifierIssuer, Permuter and built-in Node crypto code are unchanged. Direct synchronous invocation is the same algorithm selected by its _canonizeSync RDFC1.0 branch, without loading async setimmediate. Reference default rejection and bounded diagnostic outcomes are deliberately separate.

## harness/src/main.rs

```text
// [GPT-6 Astra] Issue #6475: pinned upstream replay of the saved CI counterexample.
// No sparq crate, bridge, parser, proptest, or engine is linked in this harness.
use oxrdf::{BlankNode, GraphName, NamedNode, Quad};
use std::error::Error;

fn dataset(labels: &[&str; 6]) -> Result<Vec<Quad>, Box<dyn Error>> {
    let b = labels
        .iter()
        .map(|label| BlankNode::new(*label))
        .collect::<Result<Vec<_>, _>>()?;
    let p = NamedNode::new("http://ex/p")?;
    let q = NamedNode::new("http://ex/q")?;
    Ok(vec![
        Quad::new(
            b[4].clone(),
            q.clone(),
            b[2].clone(),
            GraphName::BlankNode(b[3].clone()),
        ),
        Quad::new(b[4].clone(), p, b[0].clone(), GraphName::DefaultGraph),
        Quad::new(
            b[0].clone(),
            q,
            b[3].clone(),
            GraphName::BlankNode(b[2].clone()),
        ),
    ])
}

fn main() -> Result<(), Box<dyn Error>> {
    let original = dataset(&["b0", "b1", "b2", "b3", "b4", "b5"])?;
    let renamed = dataset(&["zz0", "zz1", "zz4", "zz3", "zz5", "zz2"])?;
    for (name, quads) in [("original", &original), ("renamed", &renamed)] {
        for quad in quads {
            println!("input_{name}: {quad} .");
        }
    }
    let control_original = rdf_canon::canonicalize_quads(&original[1..2])?;
    let control_renamed = rdf_canon::canonicalize_quads(&renamed[1..2])?;
    println!(
        "single_edge_control_equal={}",
        control_original == control_renamed
    );
    assert_eq!(
        control_original, control_renamed,
        "positive control must canonicalize"
    );
    let left = rdf_canon::canonicalize_quads(&original)?;
    let right = rdf_canon::canonicalize_quads(&renamed)?;
    println!("original_canonical={left:?}");
    println!("renamed_canonical={right:?}");
    println!("relabel_invariant={}", left == right);
    if left != right {
        return Err(
            "saved relabeling pair violates canonicalization invariance in direct rdf-canon".into(),
        );
    }
    Ok(())
}
```

## instrumentation.diff

```text
--- rdf-canon-0.15.3/src/canon.rs
+++ upstream-traced/src/canon.rs
@@ -1,3 +1,4 @@
+// [GPT-6 Astra] Scratch-only tracing; upstream algorithm and dependency cache unchanged.
 use crate::{
     counter::{HndqCallCounter, SimpleHndqCallCounter},
     error::CanonicalizationError,
@@ -164,6 +165,7 @@
 
         // 4) Increment identifier counter.
         self.increment();
+        eprintln!("TRACE issue prefix={} existing={} issued={}", self.identifier_prefix, existing_identifier, issued_identifier);
 
         // 5) Return issued identifier.
         issued_identifier
@@ -252,6 +254,7 @@
         let span_ca_3_1 = debug_span!("", indent = 1).entered();
 
         let hash = hash_first_degree_quads::<D>(&state, n).unwrap();
+        eprintln!("TRACE h1 node={} hash={}", n, hash);
 
         #[cfg(feature = "log")]
         span_ca_3_1.exit();
@@ -645,6 +648,7 @@
     debug!(indent = 1, "input: \"{}\"", input);
 
     // 5) Return the hash that results from passing input through the hash algorithm.
+    eprintln!("TRACE related node={} quad={:?} input={:?}", related, quad.to_string(), input);
     let output = hash::<D>(input);
 
     #[cfg(feature = "log")]
@@ -869,6 +873,8 @@
     #[cfg(feature = "log")]
     span_hndq_3.exit();
 
+    eprintln!("TRACE hn node={} groups={:?}", identifier, h_n);
+
     // 4) Create an empty string, data to hash.
     let mut data_to_hash = Vec::<String>::new();
 
@@ -1113,6 +1119,7 @@
     .entered();
 
     let hash = hash::<D>(data_to_hash.join(""));
+    eprintln!("TRACE nd node={} data={:?} hash={} issuer={:?}", identifier, data_to_hash.join(""), hash, issuer.issued_identifiers_map.iter().collect::<BTreeMap<_, _>>());
 
     #[cfg(feature = "log")]
     {
```

## replay.log

```text
input_original: _:b4 <http://ex/q> _:b2 _:b3 .
input_original: _:b4 <http://ex/p> _:b0 .
input_original: _:b0 <http://ex/q> _:b3 _:b2 .
input_renamed: _:zz5 <http://ex/q> _:zz4 _:zz3 .
input_renamed: _:zz5 <http://ex/p> _:zz0 .
input_renamed: _:zz0 <http://ex/q> _:zz3 _:zz4 .
single_edge_control_equal=true
original_canonical="_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
renamed_canonical="_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
relabel_invariant=false
```

## reference-replay.cjs

```text
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
```

## reference-replay.json

```text
{
  "node": "v24.19.0",
  "package": "rdf-canonize@4.0.1",
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

## topology-proof.json

```text
{
  "original": [
    [
      "b4",
      "q",
      "b2",
      "b3"
    ],
    [
      "b4",
      "p",
      "b0",
      null
    ],
    [
      "b0",
      "q",
      "b3",
      "b2"
    ]
  ],
  "swap_b2_b3_fixed_anchors": [
    [
      "b4",
      "q",
      "b3",
      "b2"
    ],
    [
      "b4",
      "p",
      "b0",
      null
    ],
    [
      "b0",
      "q",
      "b2",
      "b3"
    ]
  ],
  "same_dataset": false,
  "anchor_reason": "The sole p edge fixes its subject b4 and object b0; swapping b2 and b3 is not an automorphism."
}
```

## Relevant trace: H1, Hn, N-degree preimages and canonical issuance

```text
TRACE h1 node=b0 hash=0dffb953cf2396662e6f44c4dac2121efe5f2e50feea75409ee8836c320a2ba5
TRACE h1 node=b4 hash=70eaaa62710908bd8ce3f926f8a971e4ae0029557623d6e71c895aa708ade5fa
TRACE issue prefix=c14n existing=b0 issued=c14n0
TRACE issue prefix=c14n existing=b4 issued=c14n1
TRACE h1 node=zz0 hash=0dffb953cf2396662e6f44c4dac2121efe5f2e50feea75409ee8836c320a2ba5
TRACE h1 node=zz5 hash=70eaaa62710908bd8ce3f926f8a971e4ae0029557623d6e71c895aa708ade5fa
TRACE issue prefix=c14n existing=zz0 issued=c14n0
TRACE issue prefix=c14n existing=zz5 issued=c14n1
TRACE h1 node=b0 hash=84e1a25aeaed612753af5589745abe7e1bacb42361d2ef457c79e7242325302a
TRACE h1 node=b2 hash=166b37005906f602b8c3e82241f325ff2d22e757fe1ec677be8f8684c7805218
TRACE h1 node=b3 hash=166b37005906f602b8c3e82241f325ff2d22e757fe1ec677be8f8684c7805218
TRACE h1 node=b4 hash=ae1d96e8ca946868bcebe52e0c02d06500b308d60b2ccd25d6f657da4ca29782
TRACE issue prefix=c14n existing=b0 issued=c14n0
TRACE issue prefix=c14n existing=b4 issued=c14n1
TRACE hn node=b2 groups={"0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e": ["b3"], "a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9": ["b3"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["b0"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["b4"]}
TRACE hn node=b3 groups={"48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da": ["b2"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["b0"], "e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065": ["b2"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["b4"]}
TRACE nd node=b3 data="48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da_:b0dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065_:b0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55 issuer={"b2": "b0", "b3": "b1"}
TRACE nd node=b2 data="0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e_:b1_:b1<e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55>a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9_:b1dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=a812f907ae7c594b7fd02586a57c254860f399e64e6b3bbf13fa0e56e38f81e8 issuer={"b2": "b0", "b3": "b1"}
TRACE hn node=b3 groups={"0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e": ["b2"], "a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9": ["b2"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["b0"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["b4"]}
TRACE hn node=b2 groups={"48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da": ["b3"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["b0"], "e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065": ["b3"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["b4"]}
TRACE nd node=b2 data="48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da_:b0dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065_:b0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55 issuer={"b2": "b1", "b3": "b0"}
TRACE nd node=b3 data="0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e_:b1_:b1<e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55>a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9_:b1dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=a812f907ae7c594b7fd02586a57c254860f399e64e6b3bbf13fa0e56e38f81e8 issuer={"b2": "b1", "b3": "b0"}
TRACE issue prefix=c14n existing=b2 issued=c14n2
TRACE issue prefix=c14n existing=b3 issued=c14n3
TRACE h1 node=zz0 hash=84e1a25aeaed612753af5589745abe7e1bacb42361d2ef457c79e7242325302a
TRACE h1 node=zz3 hash=166b37005906f602b8c3e82241f325ff2d22e757fe1ec677be8f8684c7805218
TRACE h1 node=zz4 hash=166b37005906f602b8c3e82241f325ff2d22e757fe1ec677be8f8684c7805218
TRACE h1 node=zz5 hash=ae1d96e8ca946868bcebe52e0c02d06500b308d60b2ccd25d6f657da4ca29782
TRACE issue prefix=c14n existing=zz0 issued=c14n0
TRACE issue prefix=c14n existing=zz5 issued=c14n1
TRACE hn node=zz3 groups={"0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e": ["zz4"], "a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9": ["zz4"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["zz0"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["zz5"]}
TRACE hn node=zz4 groups={"48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da": ["zz3"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["zz0"], "e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065": ["zz3"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["zz5"]}
TRACE nd node=zz4 data="48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da_:b0dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065_:b0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55 issuer={"zz3": "b0", "zz4": "b1"}
TRACE nd node=zz3 data="0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e_:b1_:b1<e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55>a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9_:b1dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=a812f907ae7c594b7fd02586a57c254860f399e64e6b3bbf13fa0e56e38f81e8 issuer={"zz3": "b0", "zz4": "b1"}
TRACE hn node=zz4 groups={"0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e": ["zz3"], "a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9": ["zz3"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["zz0"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["zz5"]}
TRACE hn node=zz3 groups={"48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da": ["zz4"], "dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203": ["zz0"], "e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065": ["zz4"], "fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25": ["zz5"]}
TRACE nd node=zz3 data="48dd923ee6c17016cd4b881684cf668df66e4a103a0b6cd1a131b75eeda4b2da_:b0dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0e8b6351e6f2671494b76ca4a68aaf202f11f0e407bee1ce7e69a95908c886065_:b0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55 issuer={"zz3": "b1", "zz4": "b0"}
TRACE nd node=zz4 data="0feea0387efd853c14bd3f73580df4bfabaaab7c8cc23d7e9450cf8c74aae72e_:b1_:b1<e0920fb2a9003fe3a7feb7836f356bcf0e8d8d4e3c5fffd09c33b2f73b0eaf55>a62ddaca740d6a4674973d915e71813364dd497a676ad565e2c61394ac5b77b9_:b1dad1b426b8ad7a67e066020401940210bb554b2736faa7a37bc420d3ca168203_:c14n0fcc967479af6bb9a287d2d9275ccd98eb6df35c189e83804ade5579269547f25_:c14n1" hash=a812f907ae7c594b7fd02586a57c254860f399e64e6b3bbf13fa0e56e38f81e8 issuer={"zz3": "b1", "zz4": "b0"}
TRACE issue prefix=c14n existing=zz3 issued=c14n2
TRACE issue prefix=c14n existing=zz4 issued=c14n3
```

## Rust first-degree grouping and unique anchors — rdf-canon0.15.3 src/canon.rs

```text
254:         let hash = hash_first_degree_quads::<D>(&state, n).unwrap();
255: 
256:         #[cfg(feature = "log")]
257:         span_ca_3_1.exit();
258: 
259:         // 3.2) Add h_f(n) and n to hash to blank nodes map, including repetitions, creating a new entry if necessary.
260:         state
261:             .hash_to_blank_node_map
262:             .entry(hash)
263:             .or_default()
264:             .push(n.clone());
265:     }
266: 
267:     #[cfg(feature = "log")]
268:     span_ca_3.exit();
269: 
270:     // 4) For each hash to identifier list map entry in hash to blank nodes map, code point ordered by hash:
271:     // TODO: check if the ordering in `BTreeMap` is actually in **Unicode code point order**
272:     #[cfg(feature = "log")]    
273:     let span_ca_4 = debug_span!(
274:         "ca.4",
275:         message = "log point: Create canonical replacements for hashes mapping to a single node (4.4.3 (4))."
276:     )
277:     .entered();
278:     #[cfg(feature = "log")]
279:     debug!("with:");
280: 
281:     let mut new_hash_to_blank_node_map = state.hash_to_blank_node_map.clone();
282:     for (hash, identifier_list) in state.hash_to_blank_node_map.iter() {
283:         // 4.1) If identifier list has more than one entry, continue to the next mapping.
284:         if identifier_list.len() > 1 {
285:             continue;
286:         }
287:         let identifier = &identifier_list[0];
288: 
289:         #[cfg(feature = "log")]
290:         {
291:             debug!(indent = 1, "- identifier: {}", identifier);
292:             debug!(indent = 2, "hash: {}", hash);
293:         }
294: 
295:         // 4.2) Use the Issue Identifier algorithm, passing canonical issuer and the single blank node identifier,
296:         // identifier in identifier list to issue a canonical replacement identifier for identifier.
297:         let _canonical_identifier = state.canonical_issuer.issue(identifier);
298: 
299:         #[cfg(feature = "log")]
300:         debug!(indent = 2, "canonical label: {}", _canonical_identifier);
301: 
302:         // 4.3) Remove the map entry for hash from the hash to blank nodes map.
303:         new_hash_to_blank_node_map.remove(hash);
304:     }
305:     state.hash_to_blank_node_map = new_hash_to_blank_node_map;
306: 
307:     #[cfg(feature = "log")]
308:     span_ca_4.exit();
309: 
310:     // 5) For each hash to identifier list map entry in hash to blank nodes map, code point ordered by hash:
```

## Rust hash-path construction, stable sort and issuer consumption — rdf-canon0.15.3 src/canon.rs

```text
327:         // 5.1) Create hash path list where each item will be a result of running the Hash N-Degree Quads algorithm.
328:         let mut hash_path_list = Vec::<HashNDegreeQuadsResult>::new();
329: 
330:         // 5.2) For each blank node identifier n in identifier list:
331:         #[cfg(feature = "log")]
332:         let span_ca_5_2 = debug_span!(
333:             "ca.5.2",
334:             message =
335:                 "log point: Calculate hashes for identifiers with shared hashes (4.4.3 (5.2)).",
336:             indent = 2
337:         )
338:         .entered();
339:         #[cfg(feature = "log")]
340:         debug!("with:");
341: 
342:         for n in identifier_list {
343:             #[cfg(feature = "log")]
344:             debug!(indent = 1, "- identifier: {}", n);
345: 
346:             // 5.2.1) If a canonical identifier has already been issued for n, continue to the next blank node
347:             // identifier.
348:             if state.canonical_issuer.get(n).is_some() {
349:                 continue;
350:             }
351: 
352:             // 5.2.2) Create temporary issuer, an identifier issuer initialized with the prefix b.
353:             let mut temporary_issuer = IdentifierIssuer::new("b");
354: 
355:             // 5.2.3) Use the Issue Identifier algorithm, passing temporary issuer and n, to issue a new temporary
356:             // blank node identifier b_n to n.
357:             temporary_issuer.issue(n);
358: 
359:             // 5.2.4) Run the Hash N-Degree Quads algorithm, passing the canonicalization state, n for identifier,
360:             // and temporary issuer, appending the result to the hash path list.
361:             #[cfg(feature = "log")]
362:             let span_ca_5_2_4 = debug_span!("", indent = 1).entered();
363: 
364:             let result = hash_n_degree_quads::<D>(
365:                 &state,
366:                 n.clone(),
367:                 &temporary_issuer,
368:                 &mut hndq_call_counter,
369:             )?;
370: 
371:             #[cfg(feature = "log")]
372:             span_ca_5_2_4.exit();
373: 
374:             hash_path_list.push(result);
375:         }
376: 
377:         #[cfg(feature = "log")]
378:         span_ca_5_2.exit();
379: 
380:         // 5.3) For each result in the hash path list, code point ordered by the hash in result:
381: 
382:         #[cfg(feature = "log")]
383:         let span_ca_5_3 = debug_span!(
384:             "ca.5.3",
385:             message = "log point: Canonical identifiers for temporary identifiers (4.4.3 (5.3)).",
386:             indent = 2
387:         )
388:         .entered();
389: 
390:         // TODO: check if the `sort()` here is actually in **Unicode code point order**
391:         hash_path_list.sort();
392: 
393:         #[cfg(feature = "log")]
394:         {
395:             fn has_duplicates_in_hash_path_list(l: &Vec<HashNDegreeQuadsResult>) -> bool {
396:                 if l.is_empty() {
397:                     return false;
398:                 }
399:                 for i in 0..(l.len() - 1) {
400:                     if l[i].hash == l[i + 1].hash {
401:                         return true;
402:                     }
403:                 }
404:                 false
405:             }
406:             if has_duplicates_in_hash_path_list(&hash_path_list) {
407:                 info!("has duplicate hashes: true");
408:             }
409:         }
410: 
411:         #[cfg(feature = "log")]
412:         if !hash_path_list.is_empty() {
413:             debug!("with:");
414:         }
415: 
416:         for result in hash_path_list.iter() {
417:             #[cfg(feature = "log")]
418:             {
419:                 debug!(indent = 1, "- result: {}", result.hash);
420:                 debug!(
421:                     indent = 2,
422:                     "issuer: {}",
423:                     result.issuer.serialize_issued_identifiers_map()
424:                 );
425:             }
426: 
427:             // 5.3.1) For each blank node identifier, existing identifier, that was issued a temporary identifier
428:             // by identifier issuer in result, issue a canonical identifier, in the same order, using the Issue
429:             // Identifier algorithm, passing canonical issuer and existing identifier.
430: 
431:             #[cfg(feature = "log")]
432:             let span_ca_5_3_1 = debug_span!("ca.5.3.1", indent = 2).entered();
433: 
434:             // Retrieve the existing identifiers in the order of the temporarily issued identifiers.
435:             let temporarily_issued_identifiers_map = &result.issuer.issued_identifiers_map;
436:             let inverted_map: BTreeMap<_, _> = temporarily_issued_identifiers_map
437:                 .iter()
438:                 .map(|(k, v)| (v, k))
439:                 .collect();
440:             for existing_identifier in inverted_map.into_values() {
441:                 #[cfg(feature = "log")]
442:                 debug!("- existing identifier: {}", existing_identifier);
443: 
444:                 let _canonical_identifier = state.canonical_issuer.issue(existing_identifier);
445: 
446:                 #[cfg(feature = "log")]
447:                 debug!(indent = 1, "cid: {}", _canonical_identifier);
448:             }
449:
```

## Rust related hash and result ordering — rdf-canon0.15.3 src/canon.rs

```text
602: fn hash_related_blank_node<D: Digest>(
603:     state: &CanonicalizationState,
604:     related: &String,
605:     quad: &Quad,
606:     issuer: &IdentifierIssuer,
607:     position: HashRelatedBlankNodePosition,
608: ) -> Result<String, CanonicalizationError> {
609:     #[cfg(feature = "log")]
610:     {
611:         debug!("- position: {}", position.serialize());
612:         debug!(indent = 1, "related: {}", related);
613:     }
614: 
615:     // 1) Initialize a string input to the value of position.
616:     let input = match position {
617:         HashRelatedBlankNodePosition::Graph => position.serialize().to_string(),
618:         // 2) If position is not g, append <, the value of the predicate in quad, and > to input.
619:         _ => format!("{}{}", position.serialize(), quad.predicate),
620:     };
621: 
622:     // 3) If there is a canonical identifier for related, or an identifier issued by issuer,
623:     // append the string _:, followed by that identifier (using the canonical identifier if
624:     // present, otherwise the one issued by issuer) to input.
625: 
626:     #[cfg(feature = "log")]
627:     let span_hrbn_3 = debug_span!("").entered();
628: 
629:     let identifier = match state.canonical_issuer.get(related) {
630:         Some(id) => format!("_:{id}"),
631:         None => match issuer.get(related) {
632:             Some(id) => format!("_:{id}"),
633:             // 4) Otherwise, append the result of the Hash First Degree Quads algorithm,
634:             // passing related to input.
635:             None => hash_first_degree_quads::<D>(state, related)?,
636:         },
637:     };
638: 
639:     #[cfg(feature = "log")]
640:     span_hrbn_3.exit();
641: 
642:     let input = format!("{input}{identifier}");
643: 
644:     #[cfg(feature = "log")]
645:     debug!(indent = 1, "input: \"{}\"", input);
646: 
647:     // 5) Return the hash that results from passing input through the hash algorithm.
648:     let output = hash::<D>(input);
649: 
650:     #[cfg(feature = "log")]
651:     debug!(indent = 1, "hash: {}", output);
652: 
653:     Ok(output)
654: }
655: 
656: #[derive(PartialEq, Eq, Debug)]
657: struct HashNDegreeQuadsResult {
658:     hash: String,
659:     issuer: IdentifierIssuer,
660: }
661: 
662: impl PartialOrd for HashNDegreeQuadsResult {
663:     fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
664:         self.hash.partial_cmp(&other.hash)
665:     }
666: }
667: 
668: impl Ord for HashNDegreeQuadsResult {
669:     fn cmp(&self, other: &Self) -> std::cmp::Ordering {
670:         self.hash.cmp(&other.hash)
671:     }
672: }
673: 
674: /// **4.8 Hash N-Degree Quads**
675: ///   This algorithm calculates a hash for a given blank node across the quads in a dataset
676: ///   in which that blank node is a component for which the hash does not uniquely identify
677: ///   that blank node. This is done by expanding the search from quads directly referencing
678: ///   that blank node (the mention set), to those quads which contain nodes which are also
679: ///   components of quads in the mention set, called the gossip path. This process proceeds
680: ///   in every greater degrees of indirection until a unique hash is obtained.
681: /// **4.8.3 Algorithm**
682: ///   The inputs to this algorithm are the canonicalization state, the identifier for the
683: ///   blank node to recursively hash quads for, and path identifier issuer which is an
684: ///   identifier issuer that issues temporary blank node identifiers. The output from this
685: ///   algorithm will be a hash and the identifier issuer used to help generate it.
```

## Rust complete N-degree processing body — rdf-canon0.15.3 src/canon.rs

```text
707:     // Check call limit and halt if necessary to avoid poison input
708:     call_counter.add(&identifier)?;
709: 
710:     let mut issuer = path_identifier_issuer.clone();
711: 
712:     // 1) Create a new map Hn for relating hashes to related blank nodes.
713:     let mut h_n = BTreeMap::<String, Vec<String>>::new();
714: 
715:     // 2) Get a reference, quads, to the list of quads from the map entry for identifier
716:     // in the blank node to quads map.
717:     #[cfg(feature = "log")]
718:     let span_hndq_2 = debug_span!(
719:         "hndq.2",
720:         message = "log point: Quads for identifier (4.8.3 (2))."
721:     )
722:     .entered();
723: 
724:     let quads = match state.get_quads_for_blank_node(&identifier) {
725:         Some(q) => q,
726:         None => return Err(CanonicalizationError::QuadsNotExist),
727:     };
728: 
729:     #[cfg(feature = "log")]
730:     {
731:         debug!("quads:");
732:         for quad in quads {
733:             debug!(indent = 1, "- {}", quad.to_string().trim_end());
734:         }
735:     }
736:     #[cfg(feature = "log")]
737:     span_hndq_2.exit();
738: 
739:     // 3) For each quad in quads:
740:     #[cfg(feature = "log")]
741:     let span_hndq_3 = debug_span!(
742:         "hndq.3",
743:         message = "log point: Hash N-Degree Quads function (4.8.3 (3))."
744:     )
745:     .entered();
746:     #[cfg(feature = "log")]
747:     debug!("with:");
748: 
749:     for quad in quads {
750:         #[cfg(feature = "log")]
751:         debug!(indent = 1, "- quad: {}", quad.to_string().trim_end());
752:         #[cfg(feature = "log")]
753:         let span_hndq_3_1 = debug_span!(
754:             "hndq.3.1",
755:             message = "log point: Hash related bnode component (4.8.3 (3.1)).",
756:             indent = 2
757:         )
758:         .entered();
759:         #[cfg(feature = "log")]
760:         let mut span_hndq_3_1_flag = false;
761: 
762:         // 3.1) For each component in quad, where component is the subject, object, or graph name,
763:         // and it is a blank node that is not identified by identifier:
764:         if let Subject::BlankNode(bnode) = &quad.subject {
765:             let bnode_id = bnode.as_str().to_string();
766:             if bnode_id != identifier {
767:                 // 3.1.1) Set hash to the result of the Hash Related Blank Node algorithm, passing
768:                 // the blank node identifier for component as related, quad, issuer, and position
769:                 // as either s, o, or g based on whether component is a subject, object, graph name,
770:                 // respectively.
771: 
772:                 #[cfg(feature = "log")]
773:                 if !span_hndq_3_1_flag {
774:                     debug!("with:");
775:                     span_hndq_3_1_flag = true;
776:                 }
777: 
778:                 let hash = hash_related_blank_node::<D>(
779:                     state,
780:                     &bnode_id,
781:                     quad,
782:                     &issuer,
783:                     HashRelatedBlankNodePosition::Subject,
784:                 )?;
785: 
786:                 // 3.1.2) Add a mapping of hash to the blank node identifier for component to Hn,
787:                 // adding an entry as necessary.
788:                 h_n.entry(hash)
789:                     .or_default()
790:                     .push(bnode_id);
791:             };
792:         };
793:         // 3.1) For each component in quad, where component is the subject, object, or graph name,
794:         // and it is a blank node that is not identified by identifier:
795:         if let Term::BlankNode(bnode) = &quad.object {
796:             let bnode_id = bnode.as_str().to_string();
797:             if bnode_id != identifier {
798:                 // 3.1.1) Set hash to the result of the Hash Related Blank Node algorithm, passing
799:                 // the blank node identifier for component as related, quad, issuer, and position
800:                 // as either s, o, or g based on whether component is a subject, object, graph name,
801:                 // respectively.
802: 
803:                 #[cfg(feature = "log")]
804:                 if !span_hndq_3_1_flag {
805:                     debug!("with:");
806:                     span_hndq_3_1_flag = true;
807:                 }
808: 
809:                 let hash = hash_related_blank_node::<D>(
810:                     state,
811:                     &bnode_id,
812:                     quad,
813:                     &issuer,
814:                     HashRelatedBlankNodePosition::Object,
815:                 )?;
816: 
817:                 // 3.1.2) Add a mapping of hash to the blank node identifier for component to Hn,
818:                 // adding an entry as necessary.
819:                 h_n.entry(hash)
820:                     .or_default()
821:                     .push(bnode_id);
822:             };
823:         };
824:         // 3.1) For each component in quad, where component is the subject, object, or graph name,
825:         // and it is a blank node that is not identified by identifier:
826:         if let GraphName::BlankNode(bnode) = &quad.graph_name {
827:             let bnode_id = bnode.as_str().to_string();
828:             if bnode_id != identifier {
829:                 // 3.1.1) Set hash to the result of the Hash Related Blank Node algorithm, passing
830:                 // the blank node identifier for component as related, quad, issuer, and position
831:                 // as either s, o, or g based on whether component is a subject, object, graph name,
832:                 // respectively.
833: 
834:                 #[cfg(feature = "log")]
835:                 if !span_hndq_3_1_flag {
836:                     debug!("with:");
837:                 }
838: 
839:                 let hash = hash_related_blank_node::<D>(
840:                     state,
841:                     &bnode_id,
842:                     quad,
843:                     &issuer,
844:                     HashRelatedBlankNodePosition::Graph,
845:                 )?;
846: 
847:                 // 3.1.2) Add a mapping of hash to the blank node identifier for component to Hn,
848:                 // adding an entry as necessary.
849:                 h_n.entry(hash)
850:                     .or_default()
851:                     .push(bnode_id);
852:             };
853:         };
854: 
855:         #[cfg(feature = "log")]
856:         span_hndq_3_1.exit();
857:     }
858: 
859:     #[cfg(feature = "log")]
860:     {
861:         debug!("Hash to bnodes:");
862:         for (hash, bnodes) in h_n.iter() {
863:             debug!(indent = 1, "{}:", hash);
864:             for bnode in bnodes.iter() {
865:                 debug!(indent = 2, "- {}", bnode);
866:             }
867:         }
868:     }
869:     #[cfg(feature = "log")]
870:     span_hndq_3.exit();
871: 
872:     // 4) Create an empty string, data to hash.
873:     let mut data_to_hash = Vec::<String>::new();
874: 
875:     // 5) For each related hash to blank node list mapping in Hn, code point ordered by related hash:
876:     // TODO: check if keys in BTreeMap is actually sorted in **code point order**
877: 
878:     #[cfg(feature = "log")]
879:     let span_hndq_5 = debug_span!(
880:         "hndq.5",
881:         message = "log point: Hash N-Degree Quads function (4.8.3 (5)), entering loop."
882:     )
883:     .entered();
884:     #[cfg(feature = "log")]
885:     debug!("with:");
886: 
887:     for (related_hash, blank_node_list) in h_n {
888:         #[cfg(feature = "log")]
889:         {
890:             debug!(indent = 1, "- related hash: {}", related_hash);
891:             debug!(indent = 2, "data to hash: \"{}\"", data_to_hash.join(""));
892:         }
893: 
894:         // 5.1) Append the related hash to the data to hash.
895:         data_to_hash.push(related_hash);
896: 
897:         // 5.2) Create a string chosen path.
898:         let mut chosen_path = String::new();
899: 
900:         // 5.3) Create an unset chosen issuer variable.
901:         let mut chosen_issuer = IdentifierIssuer::new("UNSET");
902: 
903:         // 5.4) For each permutation p of blank node list:
904: 
905:         #[cfg(feature = "log")]
906:         let span_hndq_5_4 = debug_span!(
907:             "hndq.5.4",
908:             message = "log point: Hash N-Degree Quads function (4.8.3 (5.4)), entering loop.",
909:             indent = 2
910:         )
911:         .entered();
912: 
913:         #[cfg(feature = "log")]
914:         let mut span_hndq_5_4_flag = false;
915: 
916:         'perm_loop: for p in blank_node_list.iter().permutations(blank_node_list.len()) {
917:             #[cfg(feature = "log")]
918:             {
919:                 if !span_hndq_5_4_flag {
920:                     debug!("with:");
921:                     span_hndq_5_4_flag = true;
922:                 }
923:                 debug!(indent = 1, "- perm: {:?}", p);
924:             }
925: 
926:             // 5.4.1) Create a copy of issuer, issuer copy.
927:             let mut issuer_copy = issuer.clone();
928: 
929:             // 5.4.2) Create a string path.
930:             let mut path_vec = Vec::<String>::new();
931: 
932:             // 5.4.3) Create a recursion list, to store blank node identifiers that must be
933:             // recursively processed by this algorithm.
934:             let mut recursion_list = Vec::<&String>::new();
935: 
936:             // 5.4.4) For each related in p:
937:             #[cfg(feature = "log")]
938:             let span_hndq_5_4_4 = debug_span!(
939:                 "hndq.5.4.4",
940:                 message = "log point: Hash N-Degree Quads function (4.8.3 (5.4.4)), entering loop.",
941:                 indent = 2
942:             )
943:             .entered();
944:             #[cfg(feature = "log")]
945:             debug!("with:");
946: 
947:             for related in p {
948:                 #[cfg(feature = "log")]
949:                 debug!(indent = 1, "- related: {}", related);
950: 
951:                 if let Some(canonical_identifier) = state.canonical_issuer.get(related) {
952:                     // 5.4.4.1) If a canonical identifier has been issued for related by
953:                     // canonical issuer, append the string _:, followed by the canonical
954:                     // identifier for related, to path.
955:                     path_vec.push(format!("_:{canonical_identifier}"));
956:                 } else {
957:                     // 5.4.4.2) Otherwise:
958:                     // 5.4.4.2.1) If issuer copy has not issued an identifier for
959:                     // related, append related to recursion list.
960:                     if issuer_copy.get(related).is_none() {
961:                         recursion_list.push(related);
962:                     }
963:                     // 5.4.4.2.2) Use the Issue Identifier algorithm, passing issuer
964:                     // copy and related, and append the string _:, followed by the result,
965:                     // to path.
966:                     path_vec.push(format!("_:{}", issuer_copy.issue(related)));
967:                 }
968: 
969:                 // 5.4.4.3) If chosen path is not empty and the length of path is greater
970:                 // than or equal to the length of chosen path and path is greater than
971:                 // chosen path when considering code point order, then skip to the next
972:                 // permutation p.
973:                 let path = path_vec.join("");
974: 
975:                 #[cfg(feature = "log")]
976:                 debug!(indent = 2, "path: \"{}\"", path);
977: 
978:                 if !chosen_path.is_empty() && path.len() >= chosen_path.len() && path >= chosen_path
979:                 {
980:                     continue 'perm_loop;
981:                 }
982:             }
983: 
984:             #[cfg(feature = "log")]
985:             span_hndq_5_4_4.exit();
986: 
987:             // 5.4.5) For each related in recursion list:
988: 
989:             #[cfg(feature = "log")]
990:                 let span_hndq_5_4_5 = debug_span!(
991:                 "hndq.5.4.5",
992:                 message = "log point: Hash N-Degree Quads function (4.8.3 (5.4.5)), before possible recursion.",
993:                 indent = 2
994:             )
995:             .entered();
996:             #[cfg(feature = "log")]
997:             {
998:                 debug!("recursion list: {:?}", recursion_list);
999:                 debug!("path: {:?}", chosen_path);
1000:                 if !recursion_list.is_empty() {
1001:                     debug!("with:");
1002:                 }
1003:             }
1004: 
1005:             for related in recursion_list {
1006:                 #[cfg(feature = "log")]
1007:                 debug!(indent = 1, "- related: {}", related);
1008: 
1009:                 // 5.4.5.1) Set result to the result of recursively executing the Hash
1010:                 // N-Degree Quads algorithm, passing the canonicalization state, related
1011:                 // for identifier, and issuer copy for path identifier issuer.
1012: 
1013:                 #[cfg(feature = "log")]
1014:                 let span_hndq_5_4_5_1 = debug_span!("", indent = 1).entered();
1015: 
1016:                 let result =
1017:                     hash_n_degree_quads::<D>(state, related.clone(), &issuer_copy, call_counter)?;
1018: 
1019:                 #[cfg(feature = "log")]
1020:                 span_hndq_5_4_5_1.exit();
1021: 
1022:                 // 5.4.5.2) Use the Issue Identifier algorithm, passing issuer copy and
1023:                 // related; append the string _:, followed by the result, to path.
1024:                 path_vec.push(format!("_:{}", issuer_copy.issue(related)));
1025: 
1026:                 // 5.4.5.3) Append <, the hash in result, and > to path.
1027:                 path_vec.push("<".to_string());
1028:                 path_vec.push(result.hash);
1029:                 path_vec.push(">".to_string());
1030: 
1031:                 // 5.4.5.4) Set issuer copy to the identifier issuer in result.
1032: 
1033:                 #[cfg(feature="log")]
1034:                 let span_hndq_5_4_5_4 = debug_span!(
1035:                     "hndq.5.4.5.4",
1036:                     message = "log point: Hash N-Degree Quads function (4.8.3 (5.4.5.4)), combine result of recursion.",
1037:                     indent = 2
1038:                 ).entered();
1039: 
1040:                 issuer_copy = result.issuer;
1041:                 let path = path_vec.join("");
1042: 
1043:                 #[cfg(feature = "log")]
1044:                 {
1045:                     debug!("path: \"{}\"", path);
1046:                     debug!(
1047:                         "issuer copy: {}",
1048:                         issuer_copy.serialize_issued_identifiers_map()
1049:                     );
1050:                 }
1051:                 #[cfg(feature = "log")]
1052:                 span_hndq_5_4_5_4.exit();
1053: 
1054:                 // 5.4.5.5) If chosen path is not empty and the length of path is greater
1055:                 // than or equal to the length of chosen path and path is greater than
1056:                 // chosen path when considering code point order, then skip to the next p.
1057:                 if !chosen_path.is_empty() && path.len() >= chosen_path.len() && path >= chosen_path
1058:                 {
1059:                     continue 'perm_loop;
1060:                 }
1061:             }
1062: 
1063:             #[cfg(feature = "log")]
1064:             span_hndq_5_4_5.exit();
1065: 
1066:             // 5.4.6) If chosen path is empty or path is less than chosen path when
1067:             // considering code point order, set chosen path to path and chosen issuer to
1068:             // issuer copy.
1069:             let path = path_vec.join("");
1070:             if chosen_path.is_empty() || path < chosen_path {
1071:                 chosen_path = path;
1072:                 chosen_issuer = issuer_copy;
1073:             }
1074:         }
1075: 
1076:         #[cfg(feature = "log")]
1077:         span_hndq_5_4.exit();
1078: 
1079:         // 5.5) Append chosen path to data to hash.
1080: 
1081:         #[cfg(feature = "log")]
1082:         let span_hndq_5_5 = debug_span!(
1083:             "hndq.5.5",
1084:             message = "log point: Hash N-Degree Quads function (4.8.3 (5.5). End of current loop with Hn hashes.",
1085:             indent = 2
1086:         )
1087:         .entered();
1088:         #[cfg(feature = "log")]
1089:         debug!("chosen path: \"{}\"", chosen_path);
1090: 
1091:         data_to_hash.push(chosen_path);
1092: 
1093:         #[cfg(feature = "log")]
1094:         debug!("data to hash: \"{}\"", data_to_hash.join(""));
1095:         #[cfg(feature = "log")]
1096:         span_hndq_5_5.exit();
1097: 
1098:         // 5.6) Replace issuer, by reference, with chosen issuer.
1099:         issuer = chosen_issuer;
1100:     }
1101: 
1102:     #[cfg(feature = "log")]
1103:     span_hndq_5.exit();
1104: 
1105:     // 6) Return issuer and the hash that results from passing data to hash through the
1106:     // hash algorithm.
1107: 
1108:     #[cfg(feature = "log")]
1109:     let span_hndq_6 = debug_span!(
1110:         "hndq.6",
1111:         message = "log point: Leaving Hash N-Degree Quads function (4.8.3 (6))."
1112:     )
1113:     .entered();
1114: 
1115:     let hash = hash::<D>(data_to_hash.join(""));
1116: 
1117:     #[cfg(feature = "log")]
1118:     {
1119:         debug!("hash: {}", hash);
1120:         debug!("issuer: {}", issuer.serialize_issued_identifiers_map());
1121:     }
1122:     #[cfg(feature = "log")]
1123:     span_hndq_6.exit();
1124: 
1125:     Ok(HashNDegreeQuadsResult { hash, issuer })
1126: }
1127: 
1128: /// **5. Serialization**
1129: ///   The serialized canonical form of a canonicalized dataset is an N-Quads document [N-QUADS]
1130: ///   created by representing each quad from the canonicalized dataset in canonical n-quads form,
1131: ///   sorting them into code point order, and concatenating them.
1132: ///   (Note that each canonical N-Quads statement ends with a new line, so no additional separators
1133: ///    are needed in the concatenation.)
1134: ///   The resulting document has a media type of application/n-quads, as described in
1135: ///   C. N-Quads Internet Media Type, File Extension and Macintosh File Type of [N-QUADS].
1136: ///
1137: ///   When serializing quads in canonical n-quads form, components which are blank nodes MUST be
1138: ///   serialized using the canonical label associated with each blank node from the issued
1139: ///   identifiers map component of the canonicalized dataset.
1140: pub fn serialize(dataset: &Dataset) -> String {
```

## Reference hash-path sort and issuer consumer — rdf-canonize4.0.1 lib/RDFC10Sync.js

```text
127: 
128:       // 6.2) For each blank node identifier identifier in identifier list:
129:       for(const id of idList) {
130:         // 6.2.1) If a canonical identifier has already been issued for
131:         // identifier, continue to the next identifier.
132:         if(this.canonicalIssuer.hasId(id)) {
133:           continue;
134:         }
135: 
136:         // 6.2.2) Create temporary issuer, an identifier issuer
137:         // initialized with the prefix _:b.
138:         const issuer = new IdentifierIssuer('b');
139: 
140:         // 6.2.3) Use the Issue Identifier algorithm, passing temporary
141:         // issuer and identifier, to issue a new temporary blank node
142:         // identifier for identifier.
143:         issuer.getId(id);
144: 
145:         // 6.2.4) Run the Hash N-Degree Quads algorithm, passing
146:         // temporary issuer, and append the result to the hash path list.
147:         const result = this.hashNDegreeQuads(id, issuer);
148:         hashPathList.push(result);
149:       }
150: 
151:       // 6.3) For each result in the hash path list,
152:       // lexicographically-sorted by the hash in result:
153:       hashPathList.sort(_stringHashCompare);
154:       for(const result of hashPathList) {
155:         // 6.3.1) For each blank node identifier, existing identifier,
156:         // that was issued a temporary identifier by identifier issuer
157:         // in result, issue a canonical identifier, in the same order,
158:         // using the Issue Identifier algorithm, passing canonical
159:         // issuer and existing identifier.
160:         const oldIds = result.issuer.getOldIds();
161:         for(const id of oldIds) {
162:           this.canonicalIssuer.getId(id);
163:         }
164:       }
165:     }
```

## Reference related hash — rdf-canonize4.0.1 lib/RDFC10Sync.js

```text
239:   hashRelatedBlankNode(related, quad, issuer, position) {
240:     // 1) Initialize a string input to the value of position.
241:     // Note: We use a hash object instead.
242:     const md = this.createMessageDigest();
243:     md.update(position);
244: 
245:     // 2) If position is not g, append <, the value of the predicate in quad,
246:     // and > to input.
247:     if(position !== 'g') {
248:       md.update(this.getRelatedPredicate(quad));
249:     }
250: 
251:     // 3) Set the identifier to use for related, preferring first the canonical
252:     // identifier for related if issued, second the identifier issued by issuer
253:     // if issued, and last, if necessary, the result of the Hash First Degree
254:     // Quads algorithm, passing related.
255:     let id;
256:     if(this.canonicalIssuer.hasId(related)) {
257:       id = '_:' + this.canonicalIssuer.getId(related);
258:     } else if(issuer.hasId(related)) {
259:       id = '_:' + issuer.getId(related);
260:     } else {
261:       id = this.blankNodeInfo.get(related).hash;
262:     }
263: 
264:     // 4) Append identifier to input.
265:     md.update(id);
266: 
267:     // 5) Return the hash that results from passing input through the hash
268:     // algorithm.
269:     return md.digest();
270:   }
```

## Reference grouping and related helper — rdf-canonize4.0.1 lib/RDFC10Sync.js

```text
436:   createHashToRelated(id, issuer) {
437:     // 1) Create a hash to related blank nodes map for storing hashes that
438:     // identify related blank nodes.
439:     const hashToRelated = new Map();
440: 
441:     // 2) Get a reference, quads, to the list of quads in the blank node to
442:     // quads map for the key identifier.
443:     const quads = this.blankNodeInfo.get(id).quads;
444: 
445:     // 3) For each quad in quads:
446:     for(const quad of quads) {
447:       // 3.1) For each component in quad, if component is the subject, object,
448:       // or graph name and it is a blank node that is not identified by
449:       // identifier:
450:       // steps 3.1.1 and 3.1.2 occur in helpers:
451:       this._addRelatedBlankNodeHash({
452:         quad, component: quad.subject, position: 's',
453:         id, issuer, hashToRelated
454:       });
455:       this._addRelatedBlankNodeHash({
456:         quad, component: quad.object, position: 'o',
457:         id, issuer, hashToRelated
458:       });
459:       this._addRelatedBlankNodeHash({
460:         quad, component: quad.graph, position: 'g',
461:         id, issuer, hashToRelated
462:       });
463:     }
464: 
465:     return hashToRelated;
466:   }
467: 
468:   _hashAndTrackBlankNode({id, hashToBlankNodes}) {
469:     // 5.3.1) Create a hash, hash, according to the Hash First Degree
470:     // Quads algorithm.
471:     const hash = this.hashFirstDegreeQuads(id);
472: 
473:     // 5.3.2) Add hash and identifier to hash to blank nodes map,
474:     // creating a new entry if necessary.
475:     const idList = hashToBlankNodes.get(hash);
476:     if(!idList) {
477:       hashToBlankNodes.set(hash, [id]);
478:     } else {
479:       idList.push(id);
480:     }
481:   }
482: 
483:   _addBlankNodeQuadInfo({quad, component}) {
484:     if(component.termType !== 'BlankNode') {
485:       return;
486:     }
487:     const id = component.value;
488:     const info = this.blankNodeInfo.get(id);
489:     if(info) {
490:       info.quads.add(quad);
491:     } else {
492:       this.blankNodeInfo.set(id, {quads: new Set([quad]), hash: null});
493:     }
494:   }
495: 
496:   _addRelatedBlankNodeHash(
497:     {quad, component, position, id, issuer, hashToRelated}) {
498:     if(!(component.termType === 'BlankNode' && component.value !== id)) {
499:       return;
500:     }
501:     // 3.1.1) Set hash to the result of the Hash Related Blank Node
502:     // algorithm, passing the blank node identifier for component as
503:     // related, quad, path identifier issuer as issuer, and position as
504:     // either s, o, or g based on whether component is a subject, object,
505:     // graph name, respectively.
506:     const related = component.value;
507:     const hash = this.hashRelatedBlankNode(
508:       related, quad, issuer, position);
509: 
510:     // 3.1.2) Add a mapping of hash to the blank node identifier for
511:     // component to hash to related blank nodes map, adding an entry as
512:     // necessary.
513:     const entries = hashToRelated.get(hash);
514:     if(entries) {
515:       entries.push(related);
516:     } else {
517:       hashToRelated.set(hash, [related]);
518:     }
519:   }
520: 
521:   // canonical ids for 7.1
522:   _componentWithCanonicalId(component) {
```

## Packet boundary

This packet intentionally omits host paths, environment, compiler/build logs and private operational metadata. Those remain in local hashed evidence. Complete unmodified reference sources, original Rust package/archive provenance, full trace and exact executable are retained locally; only the listed load-bearing source ranges are embedded here. No fresh survey of W3C errata or latest reference versions is claimed.
