# Issue 6475: single complete-outcome scratch candidate — NO-GO

This GPT-6 Astra xhigh experiment does not propose a production patch. Frozen prior evidence established the exact relabel/order defect in unmodified Rust 0.15.3 and JS 5.0.0. The candidate compares completed outcomes after equal-hash/top-level and equal-minimum-path/recursive issuer choices. It repairs the saved 12 variants but newly exhausts four positive W3C fixtures. No limit changes or second candidate followed.

Primary upstream base: digitalbazaar/rdf-canonize 5.0.0, git 9502eee92d87e8685d96b9d325477d767c86fb83. Package provenance and exact hashes follow. The prior full source/consumer/spec packet is immutable, SHA256 0f943bfa5b625e878be443586d0f4758da4612fd85737f23d499161f7538cd1f. This supplement omits that unchanged Rust bridge/consumer context. No accepted specification erratum or approved repair is claimed.

The public source references are https://github.com/digitalbazaar/rdf-canonize/tree/9502eee92d87e8685d96b9d325477d767c86fb83 and https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-nd-quads-algorithm and #canon-algorithm. Existing related-hash/path construction is retained. The new complete-document comparison is experimental.

## Decision and reasoning

```json
{
  "decision": "NO-GO: four previously passing positive W3C fixtures exhaust fixed experimental work limits",
  "scope": "One scratch complete-outcome equal-hash/equal-path candidate; fixed protocol and no second candidate",
  "results": "Saved 12/12 unify; first-only control restores two outputs. 96 seeded variants and 12 component variants complete consistently. Baseline meets all 86 W3C expected outcomes; candidate preserves 81/85 positives and retains negative failure; four positives newly exhaust.",
  "completion_argument": [
    "Top-level equal-hash candidates retain complete ordered temporary issuers. Each selected issuer is applied, then every remaining hash group continues to a full dataset output before comparison. Already fully covered issuers have no canonical-assignment effect and are skipped.",
    "Recursive equal-minimum-path issuers are retained by exact ordered issuer-state identity. Choice-prefix replay branches at every encountered recursive/top choice; later groups recompute from each selected state. This intends to preserve nested ties rather than merely sort top-level results.",
    "A minimum is returned only when the pending choice tree is empty. Any error or limit returns no output/map, even when leaves were observed. test059c concretely exercises partial-search rejection after 36 leaves.",
    "UTF-8 byte comparison chooses among completed documents; the returned map belongs to that same completed document, and reconstruction checks map/output agreement. This is empirical consistency evidence, not a correctness proof for the candidate algorithm."
  ],
  "smallest_obstruction": "test059c is a 254-byte positive parsing fixture containing six interchangeable disconnected blank-node components. At 64 executions, 36 completed leaves had one distinct document, but remaining branches could not be assumed equivalent. Original JS5 completes under the same 4000 deep-iteration diagnostic option.",
  "recommended_next_step": "Stop this candidate. If root authorizes a further design experiment, use test059c to justify a sound structural quotient or memoization of equivalent complete continuation states before further breadth. Prove remaining-state equivalence rather than assuming equal hashes or identical observed leaves; separately address recursive test044c/045c/046c cost. Do not increase budgets, discard recursive alternatives, return a partial minimum, or weaken the success property.",
  "limits": [
    "No production patch, dependency change, commit, build, new install, API call or gate/property/hold change. Prototype is not approved or ready for Rust port.",
    "Complete means all branches exposed by this candidate finished, not a proof that the candidate exposes every semantically relevant RDFC outcome. Equivalence to a complete canonical algorithm is unproved.",
    "No successful case completed after a recursive multi-issuer equal-path choice; test044c/test045c/test046c reached those hooks but exhausted before any leaf. Recursive alternative correctness remains unvalidated.",
    "Output/map consistency is verified by reconstructing the entire canonical document from each completed map. Exact map equality across renamings is not required for legitimate automorphisms; tested W3C expected maps remained unchanged.",
    "Existing lexicographic path pruning is retained. JS uses strict greater-than pruning. Rust comparison details and lexical b10/b2 temporary ordering remain separate future port concerns.",
    "The 4000 global HNDQ-entry cap counts replay work; it is stricter than granting every replay an independent 4000 budget. 64 executions and 1000 ms are fixed diagnostic caps, not proposed public API defaults. No limits were tuned.",
    "Negative test074c still errors without output/map, but the experimental error type/text differs from the baseline. This is not error-API compatibility proof.",
    "The implemented disableRecursive hook was not executed. Only firstOnly was executed as a negative control.",
    "Seeded corpus is eight 3-quad graphs with 12 variants each, not exhaustive differential testing. Additional component variants are four per shape, not all permutations.",
    "Protocol timing measures finite diagnostic execution, not performance benchmarking. No throughput/canonical performance claim.",
    "No accepted W3C erratum or standard-level correction is established by these results."
  ],
  "summary": "summary.json",
  "provenance": "provenance.json",
  "raw_results": "results.json",
  "exact_commands": "commands.json",
  "scratch_only": true,
  "no_commands_pending": true
}
```

## Generated executed summary

```json
{
  "decision": "NO-GO: four previously passing positive W3C fixtures exhaust fixed experimental work limits",
  "commands": {
    "count": 218,
    "all_process_exit_zero": true,
    "note": "case.cjs returns JSON for algorithm errors; process exit zero is not a canonicalization success"
  },
  "groups": {
    "saved": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "saved-control": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 2,
      "all_complete_maps_consistent": true
    },
    "W3C": {
      "cases": 86,
      "complete": 81,
      "errors": 5,
      "distinct_complete_outputs": 43,
      "all_complete_maps_consistent": true
    },
    "two-distinct-predicate-components": {
      "cases": 4,
      "complete": 4,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "two-same-predicate-components": {
      "cases": 4,
      "complete": 4,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "saved-plus-cycle": {
      "cases": 4,
      "complete": 4,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus0": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus1": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus2": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus3": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus4": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus5": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus6": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    },
    "corpus7": {
      "cases": 12,
      "complete": 12,
      "errors": 0,
      "distinct_complete_outputs": 1,
      "all_complete_maps_consistent": true
    }
  },
  "W3C": {
    "entries": 86,
    "baseline_expected_outcomes": 86,
    "positive_unchanged": 81,
    "positive_total": 85,
    "negative_still_errors": 1,
    "new_positive_failures": [
      {
        "name": "test044c",
        "error": "global HNDQ budget",
        "stats": {
          "executions": 22,
          "hndqCalls": 4001,
          "leaves": 0,
          "choices": 21,
          "topComplete": 0,
          "topPartial": 0,
          "topCrossGroup": 0,
          "topOverlapping": 0,
          "recursive": 21,
          "maxPending": 30
        },
        "partialOutcomes": 0
      },
      {
        "name": "test045c",
        "error": "global HNDQ budget",
        "stats": {
          "executions": 36,
          "hndqCalls": 4001,
          "leaves": 0,
          "choices": 35,
          "topComplete": 0,
          "topPartial": 0,
          "topCrossGroup": 0,
          "topOverlapping": 0,
          "recursive": 35,
          "maxPending": 56
        },
        "partialOutcomes": 0
      },
      {
        "name": "test046c",
        "error": "global HNDQ budget",
        "stats": {
          "executions": 22,
          "hndqCalls": 4001,
          "leaves": 0,
          "choices": 21,
          "topComplete": 0,
          "topPartial": 0,
          "topCrossGroup": 0,
          "topOverlapping": 0,
          "recursive": 21,
          "maxPending": 30
        },
        "partialOutcomes": 0
      },
      {
        "name": "test059c",
        "error": "execution budget",
        "stats": {
          "executions": 64,
          "hndqCalls": 1152,
          "leaves": 36,
          "choices": 28,
          "topComplete": 0,
          "topPartial": 28,
          "topCrossGroup": 28,
          "topOverlapping": 0,
          "recursive": 0,
          "maxPending": 16
        },
        "partialOutcomes": 1
      }
    ]
  },
  "recursive_coverage": {
    "completed_cases_with_recursive_choices": 0,
    "cases_with_recursive_choices": [
      "test044c",
      "test045c",
      "test046c"
    ]
  },
  "execution_seconds": 22.085896084
}
```

## Exact protocol

```json
{
  "candidate": "enumerate equal-hash top choices and equal-path recursive issuers; compare complete outputs only",
  "max_hndq_calls_across_replays": 4000,
  "max_executions_per_input": 64,
  "total_deadline_ms_per_input": 1000,
  "external_seconds_per_case": 3,
  "whole_run_seconds": 140,
  "corpus": "8 seeded3quad datasets over4 bnodes,2predicates,default/bnode graphs; all6ordersx2 bijective labels",
  "seed": 25717,
  "controls": "first-only on all12 saved variants",
  "suite": "all86 packagedW3C entries",
  "no_retries": true
}
```

## Source identity

```json
{
  "author": "GPT-6 Astra, xhigh; scratch experiment only",
  "baseline_package": "rdf-canonize 5.0.0",
  "baseline_gitHead": "9502eee92d87e8685d96b9d325477d767c86fb83",
  "baseline_npm_sha512": "sha512-g8OUrgMXAR9ys/ZuJVfBr05sPPoMA7nHIVs8VEvg9QwM5W4GR2qSFEEHjsyHF1eWlBaf8Ev40WNjQFQ+nJTO3w==",
  "candidate_changed_package_files": [
    "lib/RDFC10Sync.js"
  ],
  "candidate_package_file_count": 14,
  "node_binary_sha256": "27db838bb204ef7c21df2931f5656e4c8fb32e6e947f363a402b49714d32b5b1",
  "node_version": "24.19.0 (previous frozen reference-v5 tooling evidence; not rerun in this freeze)",
  "source_sha256": {
    "candidate/lib/RDFC10Sync.js": "82711fe47bb6e29c224abec5ca1d0c306261d2b7af02b0e86a92c3a3f02282c6",
    "explore.cjs": "00c170088de05427d9db092b7a061d107021339f7b865026a59ce8e98194f6bd",
    "case.cjs": "c9d8b375c6af2b28bb1096fb600385cb6c3f07cdfb39f7fbb6afce018c1b98d1",
    "run.py": "e82cc40eb4ac4cbb8809f5830450c08c26b8f7355f6c4bded13c7b90c6228892",
    "candidate.diff": "29a5bfbdb547e7da2a2f627f8e0d1a53730418fb5ea4dd9323496e4d014322c0"
  },
  "baseline_RDFC10Sync_sha256": "0f08af51ac41dc730f146cc1eb12b07d913f67ffbf1498d9b124e1cef589a02d",
  "production_HEAD": "e53464c73f31f7aca800f3867ac054c36408e346",
  "production_status_porcelain": "",
  "production_modified": false,
  "previous_frozen_evidence_packet_sha256": "0f943bfa5b625e878be443586d0f4758da4612fd85737f23d499161f7538cd1f"
}
```

## Exact executed source delta

```diff
--- rdf-canonize5.0.0/lib/RDFC10Sync.js
+++ scratch/lib/RDFC10Sync.js
@@ -151,7 +151,16 @@
       // 6.3) For each result in the hash path list,
       // lexicographically-sorted by the hash in result:
       hashPathList.sort(_stringHashCompare);
-      for(const result of hashPathList) {
+      // [GPT-6 Astra] Scratch candidate: explore only equal-hash order choices.
+      for(let begin = 0; begin < hashPathList.length;) {
+        let end = begin + 1;
+        while(end < hashPathList.length && hashPathList[end].hash === hashPathList[begin].hash) ++end;
+        const remaining = hashPathList.slice(begin, end);
+        while(remaining.length > 0) {
+          const useful = remaining.filter(result => result.issuer.getOldIds().some(id => !this.canonicalIssuer.hasId(id)));
+          if(useful.length === 0) break;
+          const result = this.chooseEqual(useful, {stage:'top', crossGroup:useful.some(r=>r.issuer.getOldIds().some(id=>!idList.includes(id)&&!this.canonicalIssuer.hasId(id))), complete:useful.every(r => [...this.blankNodeInfo.keys()].every(id => this.canonicalIssuer.hasId(id) || r.issuer.hasId(id))), partial:useful.some(r => [...this.blankNodeInfo.keys()].some(id => !this.canonicalIssuer.hasId(id) && !r.issuer.hasId(id))), overlap:useful.some((a,i)=>useful.slice(i+1).some(b=>a.issuer.getOldIds().some(id=>b.issuer.hasId(id))))});
+          remaining.splice(remaining.indexOf(result), 1);
         // 6.3.1) For each blank node identifier, existing identifier,
         // that was issued a temporary identifier by identifier issuer
         // in result, issue a canonical identifier, in the same order,
@@ -161,6 +170,8 @@
         for(const id of oldIds) {
           this.canonicalIssuer.getId(id);
         }
+        }
+        begin = end;
       }
     }
 
@@ -271,6 +282,7 @@
 
   // 4.8) Hash N-Degree Quads
   hashNDegreeQuads(id, issuer) {
+    this.experimentalTick();
     if(this.remainingDeepIterations === 0) {
       throw new Error(
         `Maximum deep iterations exceeded (${this.maxDeepIterations}).`);
@@ -298,6 +310,7 @@
 
       // 5.3) Create an unset chosen issuer variable.
       let chosenIssuer;
+      let chosenIssuers = [];
 
       // 5.4) For each permutation of blank node list:
       const permuter = new Permuter(hashToRelated.get(hash));
@@ -396,8 +409,12 @@
         if(chosenPath.length === 0 || path < chosenPath) {
           chosenPath = path;
           chosenIssuer = issuerCopy;
+          chosenIssuers = [issuerCopy];
+        } else if(path === chosenPath && !chosenIssuers.some(x => JSON.stringify([...x._existing]) === JSON.stringify([...issuerCopy._existing]))) {
+          chosenIssuers.push(issuerCopy);
         }
       }
+      chosenIssuer = this.chooseEqual(chosenIssuers, {stage:'recursive', partial:true});
 
       // 5.5) Append chosen path to data to hash.
       md.update(chosenPath);

```

## explore.cjs

```javascript
// [GPT-6 Astra] Experimental complete-outcome search; never production or cache code.
const Candidate = require('./candidate/lib/RDFC10Sync');
const NQuads = require('./candidate/lib/NQuads');
const LIMITS = Object.freeze({hndqCalls:4000, executions:64, milliseconds:1000});
class Choice extends Error {constructor(count, context){super('choice');this.count=count;this.context=context;}}
class Exhausted extends Error {}
function reconstruct(quads, map) {
  const term = x => x.termType==='BlankNode'?{...x,value:map[x.value]}:x;
  return NQuads.serialize(quads.map(q=>({...q,subject:term(q.subject),object:term(q.object),graph:term(q.graph)})));
}
function explore(input, {hash='SHA256', disableRecursive=false, firstOnly=false}={}) {
  const quads=NQuads.parse(input), pending=[[]], started=Date.now();
  const stats={executions:0,hndqCalls:0,leaves:0,choices:0,topComplete:0,topPartial:0,topCrossGroup:0,topOverlapping:0,recursive:0,maxPending:1};
  let best=null;const distinct=new Set();
  try {
    while(pending.length) {
      if(stats.executions>=LIMITS.executions) throw new Exhausted('execution budget');
      const prefix=pending.pop();let position=0;
      const map=new Map(), instance=new Candidate({canonicalIdMap:map,messageDigestAlgorithm:hash,maxDeepIterations:4000,timeout:1000});
      ++stats.executions;
      instance.experimentalTick=()=>{
        if(++stats.hndqCalls>LIMITS.hndqCalls) throw new Exhausted('global HNDQ budget');
        if(Date.now()-started>LIMITS.milliseconds) throw new Exhausted('total deadline');
      };
      instance.chooseEqual=(options,context)=>{
        if(options.length===0) throw new Error('empty choice');
        if(options.length===1 || firstOnly || (disableRecursive&&context.stage==='recursive')) return options[0];
        if(position===prefix.length) throw new Choice(options.length,context);
        const chosen=prefix[position++];
        if(chosen>=options.length) throw new Error('invalid replay prefix');
        return options[chosen];
      };
      try {
        const output=instance.main(quads);
        if(position!==prefix.length) throw new Error('unused replay prefix');
        const issuer=Object.fromEntries(map);
        if(reconstruct(quads,issuer)!==output) throw new Error('output/map mismatch');
        ++stats.leaves;distinct.add(output);
        if(best===null || Buffer.compare(Buffer.from(output),Buffer.from(best.output))<0) best={output,map:issuer};
      } catch(error) {
        if(!(error instanceof Choice)) throw error;
        ++stats.choices;
        const c=error.context;
        if(c.stage==='recursive')++stats.recursive;
        if(c.stage==='top') {if(c.complete)++stats.topComplete;if(c.partial)++stats.topPartial;if(c.crossGroup)++stats.topCrossGroup;if(c.overlap)++stats.topOverlapping;}
        if(pending.length+error.count>LIMITS.executions) throw new Exhausted('pending execution budget');
        for(let i=error.count-1;i>=0;--i)pending.push([...prefix,i]);
        stats.maxPending=Math.max(stats.maxPending,pending.length);
      }
      if(Date.now()-started>LIMITS.milliseconds)throw new Exhausted('total deadline');
    }
    return {status:'complete',...best,distinctOutcomes:distinct.size,mapConsistent:true,stats};
  } catch(error) {
    // Never return a minimum from an incomplete search, even when a leaf exists.
    return {status:'error',error:error.message,kind:error instanceof Exhausted?'experimental_budget':error.name,stats,partialOutcomes:distinct.size};
  }
}
module.exports={explore,LIMITS};

```

## case.cjs

```javascript
// [GPT-6 Astra] One isolated input; result collection is not an invariance assertion.
const fs=require('node:fs');
const {explore,LIMITS}=require('./explore.cjs');
const [file,hash='SHA256',control='candidate']=process.argv.slice(2);
const input=fs.readFileSync(file,'utf8');
const Baseline=require('../reference-v5/reference/package/lib/RDFC10Sync');
const NQuads=require('../reference-v5/reference/package/lib/NQuads');
let baseline;
try {const map=new Map();const output=new Baseline({canonicalIdMap:map,messageDigestAlgorithm:hash,maxDeepIterations:4000,timeout:1000}).main(NQuads.parse(input));baseline={status:'complete',output,map:Object.fromEntries(map)};}
catch(e){baseline={status:'error',error:e.message};}
console.log(JSON.stringify({limits:LIMITS,baseline,result:explore(input,{hash,disableRecursive:control==='disable-recursive',firstOnly:control==='first-only'})}));

```

## candidate/lib/RDFC10Sync.js

```javascript
/*!
 * Copyright (c) 2016-2023 Digital Bazaar, Inc. All rights reserved.
 */
'use strict';

const IdentifierIssuer = require('./IdentifierIssuer');
// FIXME: do not import; convert to requiring a
// hash factory
const MessageDigest = require('./MessageDigest');
const Permuter = require('./Permuter');
const NQuads = require('./NQuads');

module.exports = class RDFC10Sync {
  constructor({
    createMessageDigest = null,
    messageDigestAlgorithm = 'sha256',
    canonicalIdMap = new Map(),
    maxWorkFactor = 1,
    maxDeepIterations = -1,
    timeout = 0
  } = {}) {
    this.name = 'RDFC-1.0';
    this.blankNodeInfo = new Map();
    this.canonicalIssuer = new IdentifierIssuer('c14n', canonicalIdMap);
    this.createMessageDigest = createMessageDigest ||
      (() => new MessageDigest(messageDigestAlgorithm));
    this.maxWorkFactor = maxWorkFactor;
    this.maxDeepIterations = maxDeepIterations;
    this.remainingDeepIterations = 0;
    this.timeout = timeout;
    if(timeout > 0) {
      this.startTime = Date.now();
    }
    this.quads = null;
  }

  // 4.4) Normalization Algorithm
  main(dataset) {
    this.quads = dataset;

    // 1) Create the normalization state.
    // 2) For every quad in input dataset:
    for(const quad of dataset) {
      // 2.1) For each blank node that occurs in the quad, add a reference
      // to the quad using the blank node identifier in the blank node to
      // quads map, creating a new entry if necessary.
      this._addBlankNodeQuadInfo({quad, component: quad.subject});
      this._addBlankNodeQuadInfo({quad, component: quad.object});
      this._addBlankNodeQuadInfo({quad, component: quad.graph});
    }

    // 3) Create a list of non-normalized blank node identifiers
    // non-normalized identifiers and populate it using the keys from the
    // blank node to quads map.
    // Note: We use a map here and it was generated during step 2.

    // 4) `simple` flag is skipped -- loop is optimized away. This optimization
    // is permitted because there was a typo in the hash first degree quads
    // algorithm in the RDFC-1.0 spec that was implemented widely making it
    // such that it could not be fixed; the result was that the loop only
    // needs to be run once and the first degree quad hashes will never change.
    // 5.1-5.2 are skipped; first degree quad hashes are generated just once
    // for all non-normalized blank nodes.

    // 5.3) For each blank node identifier identifier in non-normalized
    // identifiers:
    const hashToBlankNodes = new Map();
    const nonNormalized = [...this.blankNodeInfo.keys()];
    for(const id of nonNormalized) {
      // steps 5.3.1 and 5.3.2:
      this._hashAndTrackBlankNode({id, hashToBlankNodes});
    }

    // 5.4) For each hash to identifier list mapping in hash to blank
    // nodes map, lexicographically-sorted by hash:
    const hashes = [...hashToBlankNodes.keys()].sort();
    // optimize away second sort, gather non-unique hashes in order as we go
    const nonUnique = [];
    for(const hash of hashes) {
      // 5.4.1) If the length of identifier list is greater than 1,
      // continue to the next mapping.
      const idList = hashToBlankNodes.get(hash);
      if(idList.length > 1) {
        nonUnique.push(idList);
        continue;
      }

      // 5.4.2) Use the Issue Identifier algorithm, passing canonical
      // issuer and the single blank node identifier in identifier
      // list, identifier, to issue a canonical replacement identifier
      // for identifier.
      const id = idList[0];
      this.canonicalIssuer.getId(id);

      // Note: These steps are skipped, optimized away since the loop
      // only needs to be run once.
      // 5.4.3) Remove identifier from non-normalized identifiers.
      // 5.4.4) Remove hash from the hash to blank nodes map.
      // 5.4.5) Set simple to true.
    }

    if(this.maxDeepIterations < 0) {
      // calculate maxDeepIterations if not explicit
      if(this.maxWorkFactor === 0) {
        this.maxDeepIterations = 0;
      } else if(this.maxWorkFactor === Infinity) {
        this.maxDeepIterations = Infinity;
      } else {
        const nonUniqueCount =
          nonUnique.reduce((count, v) => count + v.length, 0);
        this.maxDeepIterations = nonUniqueCount ** this.maxWorkFactor;
      }
    }
    // handle any large inputs as Infinity
    if(this.maxDeepIterations > Number.MAX_SAFE_INTEGER) {
      this.maxDeepIterations = Infinity;
    }
    this.remainingDeepIterations = this.maxDeepIterations;

    // 6) For each hash to identifier list mapping in hash to blank nodes map,
    // lexicographically-sorted by hash:
    // Note: sort optimized away, use `nonUnique`.
    for(const idList of nonUnique) {
      // 6.1) Create hash path list where each item will be a result of
      // running the Hash N-Degree Quads algorithm.
      const hashPathList = [];

      // 6.2) For each blank node identifier identifier in identifier list:
      for(const id of idList) {
        // 6.2.1) If a canonical identifier has already been issued for
        // identifier, continue to the next identifier.
        if(this.canonicalIssuer.hasId(id)) {
          continue;
        }

        // 6.2.2) Create temporary issuer, an identifier issuer
        // initialized with the prefix _:b.
        const issuer = new IdentifierIssuer('b');

        // 6.2.3) Use the Issue Identifier algorithm, passing temporary
        // issuer and identifier, to issue a new temporary blank node
        // identifier for identifier.
        issuer.getId(id);

        // 6.2.4) Run the Hash N-Degree Quads algorithm, passing
        // temporary issuer, and append the result to the hash path list.
        const result = this.hashNDegreeQuads(id, issuer);
        hashPathList.push(result);
      }

      // 6.3) For each result in the hash path list,
      // lexicographically-sorted by the hash in result:
      hashPathList.sort(_stringHashCompare);
      // [GPT-6 Astra] Scratch candidate: explore only equal-hash order choices.
      for(let begin = 0; begin < hashPathList.length;) {
        let end = begin + 1;
        while(end < hashPathList.length && hashPathList[end].hash === hashPathList[begin].hash) ++end;
        const remaining = hashPathList.slice(begin, end);
        while(remaining.length > 0) {
          const useful = remaining.filter(result => result.issuer.getOldIds().some(id => !this.canonicalIssuer.hasId(id)));
          if(useful.length === 0) break;
          const result = this.chooseEqual(useful, {stage:'top', crossGroup:useful.some(r=>r.issuer.getOldIds().some(id=>!idList.includes(id)&&!this.canonicalIssuer.hasId(id))), complete:useful.every(r => [...this.blankNodeInfo.keys()].every(id => this.canonicalIssuer.hasId(id) || r.issuer.hasId(id))), partial:useful.some(r => [...this.blankNodeInfo.keys()].some(id => !this.canonicalIssuer.hasId(id) && !r.issuer.hasId(id))), overlap:useful.some((a,i)=>useful.slice(i+1).some(b=>a.issuer.getOldIds().some(id=>b.issuer.hasId(id))))});
          remaining.splice(remaining.indexOf(result), 1);
        // 6.3.1) For each blank node identifier, existing identifier,
        // that was issued a temporary identifier by identifier issuer
        // in result, issue a canonical identifier, in the same order,
        // using the Issue Identifier algorithm, passing canonical
        // issuer and existing identifier.
        const oldIds = result.issuer.getOldIds();
        for(const id of oldIds) {
          this.canonicalIssuer.getId(id);
        }
        }
        begin = end;
      }
    }

    /* Note: At this point all blank nodes in the set of RDF quads have been
    assigned canonical identifiers, which have been stored in the canonical
    issuer. Here each quad is updated by assigning each of its blank nodes
    its new identifier. */

    // 7) For each quad, quad, in input dataset:
    const normalized = [];
    for(const quad of this.quads) {
      // 7.1) Create a copy, quad copy, of quad and replace any existing
      // blank node identifiers using the canonical identifiers
      // previously issued by canonical issuer.
      // Note: We optimize away the copy here.
      const nQuad = NQuads.serializeQuadComponents(
        this._componentWithCanonicalId(quad.subject),
        quad.predicate,
        this._componentWithCanonicalId(quad.object),
        this._componentWithCanonicalId(quad.graph)
      );
      // 7.2) Add quad copy to the normalized dataset.
      normalized.push(nQuad);
    }

    // sort normalized output
    normalized.sort();

    // 8) Return the normalized dataset.
    return normalized.join('');
  }

  // 4.6) Hash First Degree Quads
  hashFirstDegreeQuads(id) {
    // 1) Initialize nquads to an empty list. It will be used to store quads in
    // N-Quads format.
    const nquads = [];

    // 2) Get the list of quads `quads` associated with the reference blank node
    // identifier in the blank node to quads map.
    const info = this.blankNodeInfo.get(id);
    const quads = info.quads;

    // 3) For each quad `quad` in `quads`:
    for(const quad of quads) {
      // 3.1) Serialize the quad in N-Quads format with the following special
      // rule:

      // 3.1.1) If any component in quad is an blank node, then serialize it
      // using a special identifier as follows:
      // 3.1.2) If the blank node's existing blank node identifier matches
      // the reference blank node identifier then use the blank node
      // identifier _:a, otherwise, use the blank node identifier _:z.
      nquads.push(NQuads.serializeQuadComponents(
        this.modifyFirstDegreeComponent(id, quad.subject, 'subject'),
        quad.predicate,
        this.modifyFirstDegreeComponent(id, quad.object, 'object'),
        this.modifyFirstDegreeComponent(id, quad.graph, 'graph')
      ));
    }

    // 4) Sort nquads in lexicographical order.
    nquads.sort();

    // 5) Return the hash that results from passing the sorted, joined nquads
    // through the hash algorithm.
    const md = this.createMessageDigest();
    for(const nquad of nquads) {
      md.update(nquad);
    }
    info.hash = md.digest();
    return info.hash;
  }

  // 4.7) Hash Related Blank Node
  hashRelatedBlankNode(related, quad, issuer, position) {
    // 1) Initialize a string input to the value of position.
    // Note: We use a hash object instead.
    const md = this.createMessageDigest();
    md.update(position);

    // 2) If position is not g, append <, the value of the predicate in quad,
    // and > to input.
    if(position !== 'g') {
      md.update(this.getRelatedPredicate(quad));
    }

    // 3) Set the identifier to use for related, preferring first the canonical
    // identifier for related if issued, second the identifier issued by issuer
    // if issued, and last, if necessary, the result of the Hash First Degree
    // Quads algorithm, passing related.
    let id;
    if(this.canonicalIssuer.hasId(related)) {
      id = '_:' + this.canonicalIssuer.getId(related);
    } else if(issuer.hasId(related)) {
      id = '_:' + issuer.getId(related);
    } else {
      id = this.blankNodeInfo.get(related).hash;
    }

    // 4) Append identifier to input.
    md.update(id);

    // 5) Return the hash that results from passing input through the hash
    // algorithm.
    return md.digest();
  }

  // 4.8) Hash N-Degree Quads
  hashNDegreeQuads(id, issuer) {
    this.experimentalTick();
    if(this.remainingDeepIterations === 0) {
      throw new Error(
        `Maximum deep iterations exceeded (${this.maxDeepIterations}).`);
    }
    this.remainingDeepIterations--;

    // 1) Create a hash to related blank nodes map for storing hashes that
    // identify related blank nodes.
    // Note: 2) and 3) handled within `createHashToRelated`
    const md = this.createMessageDigest();
    const hashToRelated = this.createHashToRelated(id, issuer);

    // 4) Create an empty string, data to hash.
    // Note: We created a hash object `md` above instead.

    // 5) For each related hash to blank node list mapping in hash to related
    // blank nodes map, sorted lexicographically by related hash:
    const hashes = [...hashToRelated.keys()].sort();
    for(const hash of hashes) {
      // 5.1) Append the related hash to the data to hash.
      md.update(hash);

      // 5.2) Create a string chosen path.
      let chosenPath = '';

      // 5.3) Create an unset chosen issuer variable.
      let chosenIssuer;
      let chosenIssuers = [];

      // 5.4) For each permutation of blank node list:
      const permuter = new Permuter(hashToRelated.get(hash));
      let i = 0;
      while(permuter.hasNext()) {
        const permutation = permuter.next();
        // Note: batch permutations 3 at a time
        if(++i % 3 === 0) {
          if(this.timeout > 0 && Date.now() - this.startTime > this.timeout) {
            throw new Error('Canonize timeout.');
          }
        }

        // 5.4.1) Create a copy of issuer, issuer copy.
        let issuerCopy = issuer.clone();

        // 5.4.2) Create a string path.
        let path = '';

        // 5.4.3) Create a recursion list, to store blank node identifiers
        // that must be recursively processed by this algorithm.
        const recursionList = [];

        // 5.4.4) For each related in permutation:
        let nextPermutation = false;
        for(const related of permutation) {
          // 5.4.4.1) If a canonical identifier has been issued for
          // related, append it to path.
          if(this.canonicalIssuer.hasId(related)) {
            path += '_:' + this.canonicalIssuer.getId(related);
          } else {
            // 5.4.4.2) Otherwise:
            // 5.4.4.2.1) If issuer copy has not issued an identifier for
            // related, append related to recursion list.
            if(!issuerCopy.hasId(related)) {
              recursionList.push(related);
            }
            // 5.4.4.2.2) Use the Issue Identifier algorithm, passing
            // issuer copy and related and append the result to path.
            path += '_:' + issuerCopy.getId(related);
          }

          // 5.4.4.3) If chosen path is not empty and the length of path
          // is greater than or equal to the length of chosen path and
          // path is lexicographically greater than chosen path, then
          // skip to the next permutation.
          // Note: Comparing path length to chosen path length can be optimized
          // away; only compare lexicographically.
          if(chosenPath.length !== 0 && path > chosenPath) {
            nextPermutation = true;
            break;
          }
        }

        if(nextPermutation) {
          continue;
        }

        // 5.4.5) For each related in recursion list:
        for(const related of recursionList) {
          // 5.4.5.1) Set result to the result of recursively executing
          // the Hash N-Degree Quads algorithm, passing related for
          // identifier and issuer copy for path identifier issuer.
          const result = this.hashNDegreeQuads(related, issuerCopy);

          // 5.4.5.2) Use the Issue Identifier algorithm, passing issuer
          // copy and related and append the result to path.
          path += '_:' + issuerCopy.getId(related);

          // 5.4.5.3) Append <, the hash in result, and > to path.
          path += `<${result.hash}>`;

          // 5.4.5.4) Set issuer copy to the identifier issuer in
          // result.
          issuerCopy = result.issuer;

          // 5.4.5.5) If chosen path is not empty and the length of path
          // is greater than or equal to the length of chosen path and
          // path is lexicographically greater than chosen path, then
          // skip to the next permutation.
          // Note: Comparing path length to chosen path length can be optimized
          // away; only compare lexicographically.
          if(chosenPath.length !== 0 && path > chosenPath) {
            nextPermutation = true;
            break;
          }
        }

        if(nextPermutation) {
          continue;
        }

        // 5.4.6) If chosen path is empty or path is lexicographically
        // less than chosen path, set chosen path to path and chosen
        // issuer to issuer copy.
        if(chosenPath.length === 0 || path < chosenPath) {
          chosenPath = path;
          chosenIssuer = issuerCopy;
          chosenIssuers = [issuerCopy];
        } else if(path === chosenPath && !chosenIssuers.some(x => JSON.stringify([...x._existing]) === JSON.stringify([...issuerCopy._existing]))) {
          chosenIssuers.push(issuerCopy);
        }
      }
      chosenIssuer = this.chooseEqual(chosenIssuers, {stage:'recursive', partial:true});

      // 5.5) Append chosen path to data to hash.
      md.update(chosenPath);

      // 5.6) Replace issuer, by reference, with chosen issuer.
      issuer = chosenIssuer;
    }

    // 6) Return issuer and the hash that results from passing data to hash
    // through the hash algorithm.
    return {hash: md.digest(), issuer};
  }

  // helper for modifying component during Hash First Degree Quads
  modifyFirstDegreeComponent(id, component) {
    if(component.termType !== 'BlankNode') {
      return component;
    }
    /* Note: A mistake in the RDFC-1.0 spec that made its way into
    implementations (and therefore must stay to avoid interop breakage)
    resulted in an assigned canonical ID, if available for
    `component.value`, not being used in place of `_:a`/`_:z`, so
    we don't use it here. */
    return {
      termType: 'BlankNode',
      value: component.value === id ? 'a' : 'z'
    };
  }

  // helper for getting a related predicate
  getRelatedPredicate(quad) {
    return `<${quad.predicate.value}>`;
  }

  // helper for creating hash to related blank nodes map
  createHashToRelated(id, issuer) {
    // 1) Create a hash to related blank nodes map for storing hashes that
    // identify related blank nodes.
    const hashToRelated = new Map();

    // 2) Get a reference, quads, to the list of quads in the blank node to
    // quads map for the key identifier.
    const quads = this.blankNodeInfo.get(id).quads;

    // 3) For each quad in quads:
    for(const quad of quads) {
      // 3.1) For each component in quad, if component is the subject, object,
      // or graph name and it is a blank node that is not identified by
      // identifier:
      // steps 3.1.1 and 3.1.2 occur in helpers:
      this._addRelatedBlankNodeHash({
        quad, component: quad.subject, position: 's',
        id, issuer, hashToRelated
      });
      this._addRelatedBlankNodeHash({
        quad, component: quad.object, position: 'o',
        id, issuer, hashToRelated
      });
      this._addRelatedBlankNodeHash({
        quad, component: quad.graph, position: 'g',
        id, issuer, hashToRelated
      });
    }

    return hashToRelated;
  }

  _hashAndTrackBlankNode({id, hashToBlankNodes}) {
    // 5.3.1) Create a hash, hash, according to the Hash First Degree
    // Quads algorithm.
    const hash = this.hashFirstDegreeQuads(id);

    // 5.3.2) Add hash and identifier to hash to blank nodes map,
    // creating a new entry if necessary.
    const idList = hashToBlankNodes.get(hash);
    if(!idList) {
      hashToBlankNodes.set(hash, [id]);
    } else {
      idList.push(id);
    }
  }

  _addBlankNodeQuadInfo({quad, component}) {
    if(component.termType !== 'BlankNode') {
      return;
    }
    const id = component.value;
    const info = this.blankNodeInfo.get(id);
    if(info) {
      info.quads.add(quad);
    } else {
      this.blankNodeInfo.set(id, {quads: new Set([quad]), hash: null});
    }
  }

  _addRelatedBlankNodeHash(
    {quad, component, position, id, issuer, hashToRelated}) {
    if(!(component.termType === 'BlankNode' && component.value !== id)) {
      return;
    }
    // 3.1.1) Set hash to the result of the Hash Related Blank Node
    // algorithm, passing the blank node identifier for component as
    // related, quad, path identifier issuer as issuer, and position as
    // either s, o, or g based on whether component is a subject, object,
    // graph name, respectively.
    const related = component.value;
    const hash = this.hashRelatedBlankNode(
      related, quad, issuer, position);

    // 3.1.2) Add a mapping of hash to the blank node identifier for
    // component to hash to related blank nodes map, adding an entry as
    // necessary.
    const entries = hashToRelated.get(hash);
    if(entries) {
      entries.push(related);
    } else {
      hashToRelated.set(hash, [related]);
    }
  }

  // canonical ids for 7.1
  _componentWithCanonicalId(component) {
    if(component.termType === 'BlankNode' &&
      !component.value.startsWith(this.canonicalIssuer.prefix)) {
      // create new BlankNode
      return {
        termType: 'BlankNode',
        value: this.canonicalIssuer.getId(component.value)
      };
    }
    return component;
  }
};

function _stringHashCompare(a, b) {
  return a.hash < b.hash ? -1 : a.hash > b.hash ? 1 : 0;
}

```

## Executed selected evidence, including all four positive regressions

Full 218-case input/output/map/command records remain in the private evidence bundle; these selected bodies contain all four new failures and their exact expected and unchanged baseline outputs. No host paths or environment/build logs are included.

```json
[
  {
    "name": "saved-original-012",
    "group": "saved",
    "input": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": null,
    "expected": null,
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
      "map": {
        "b0": "c14n0",
        "b4": "c14n1",
        "b2": "c14n2",
        "b3": "c14n3"
      }
    },
    "result": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n",
      "map": {
        "b0": "c14n0",
        "b4": "c14n1",
        "b3": "c14n2",
        "b2": "c14n3"
      },
      "distinctOutcomes": 2,
      "mapConsistent": true,
      "stats": {
        "executions": 3,
        "hndqCalls": 12,
        "leaves": 2,
        "choices": 1,
        "topComplete": 1,
        "topPartial": 0,
        "topCrossGroup": 0,
        "topOverlapping": 1,
        "recursive": 0,
        "maxPending": 2
      }
    }
  },
  {
    "name": "saved-original-012-first-only",
    "group": "saved-control",
    "input": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n",
    "hash": "SHA256",
    "control": "first-only",
    "kind": null,
    "expected": null,
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
      "map": {
        "b0": "c14n0",
        "b4": "c14n1",
        "b2": "c14n2",
        "b3": "c14n3"
      }
    },
    "result": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
      "map": {
        "b0": "c14n0",
        "b4": "c14n1",
        "b2": "c14n2",
        "b3": "c14n3"
      },
      "distinctOutcomes": 1,
      "mapConsistent": true,
      "stats": {
        "executions": 1,
        "hndqCalls": 4,
        "leaves": 1,
        "choices": 0,
        "topComplete": 0,
        "topPartial": 0,
        "topCrossGroup": 0,
        "topOverlapping": 0,
        "recursive": 0,
        "maxPending": 1
      }
    }
  },
  {
    "name": "test021c",
    "group": "W3C",
    "input": "_:e0 <http://example.org/vocab#next> _:e1 .\n_:e1 <http://example.org/vocab#next> _:e0 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": "rdfc:RDFC10EvalTest",
    "expected": "_:c14n0 <http://example.org/vocab#next> _:c14n1 .\n_:c14n1 <http://example.org/vocab#next> _:c14n0 .\n",
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://example.org/vocab#next> _:c14n1 .\n_:c14n1 <http://example.org/vocab#next> _:c14n0 .\n",
      "map": {
        "e0": "c14n0",
        "e1": "c14n1"
      }
    },
    "result": {
      "status": "complete",
      "output": "_:c14n0 <http://example.org/vocab#next> _:c14n1 .\n_:c14n1 <http://example.org/vocab#next> _:c14n0 .\n",
      "map": {
        "e0": "c14n0",
        "e1": "c14n1"
      },
      "distinctOutcomes": 1,
      "mapConsistent": true,
      "stats": {
        "executions": 3,
        "hndqCalls": 12,
        "leaves": 2,
        "choices": 1,
        "topComplete": 1,
        "topPartial": 0,
        "topCrossGroup": 0,
        "topOverlapping": 1,
        "recursive": 0,
        "maxPending": 2
      }
    }
  },
  {
    "name": "test033c",
    "group": "W3C",
    "input": "_:e0 <http://example.org/vocab#prop> _:e1 .\n_:e2 <http://example.org/vocab#prop> _:e3 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": "rdfc:RDFC10EvalTest",
    "expected": "_:c14n0 <http://example.org/vocab#prop> _:c14n1 .\n_:c14n2 <http://example.org/vocab#prop> _:c14n3 .\n",
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://example.org/vocab#prop> _:c14n1 .\n_:c14n2 <http://example.org/vocab#prop> _:c14n3 .\n",
      "map": {
        "e0": "c14n0",
        "e1": "c14n1",
        "e2": "c14n2",
        "e3": "c14n3"
      }
    },
    "result": {
      "status": "complete",
      "output": "_:c14n0 <http://example.org/vocab#prop> _:c14n1 .\n_:c14n2 <http://example.org/vocab#prop> _:c14n3 .\n",
      "map": {
        "e0": "c14n0",
        "e1": "c14n1",
        "e2": "c14n2",
        "e3": "c14n3"
      },
      "distinctOutcomes": 1,
      "mapConsistent": true,
      "stats": {
        "executions": 3,
        "hndqCalls": 12,
        "leaves": 2,
        "choices": 1,
        "topComplete": 0,
        "topPartial": 1,
        "topCrossGroup": 1,
        "topOverlapping": 0,
        "recursive": 0,
        "maxPending": 2
      }
    }
  },
  {
    "name": "test044c",
    "group": "W3C",
    "input": "_:e0 <http://example.org/vocab#p> _:e1 .\n_:e0 <http://example.org/vocab#p> _:e2 .\n_:e0 <http://example.org/vocab#p> _:e3 .\n_:e1 <http://example.org/vocab#p> _:e0 .\n_:e1 <http://example.org/vocab#p> _:e3 .\n_:e1 <http://example.org/vocab#p> _:e4 .\n_:e2 <http://example.org/vocab#p> _:e0 .\n_:e2 <http://example.org/vocab#p> _:e4 .\n_:e2 <http://example.org/vocab#p> _:e5 .\n_:e3 <http://example.org/vocab#p> _:e0 .\n_:e3 <http://example.org/vocab#p> _:e1 .\n_:e3 <http://example.org/vocab#p> _:e5 .\n_:e4 <http://example.org/vocab#p> _:e1 .\n_:e4 <http://example.org/vocab#p> _:e2 .\n_:e4 <http://example.org/vocab#p> _:e5 .\n_:e5 <http://example.org/vocab#p> _:e3 .\n_:e5 <http://example.org/vocab#p> _:e2 .\n_:e5 <http://example.org/vocab#p> _:e4 .\n_:e6 <http://example.org/vocab#p> _:e7 .\n_:e6 <http://example.org/vocab#p> _:e8 .\n_:e6 <http://example.org/vocab#p> _:e9 .\n_:e7 <http://example.org/vocab#p> _:e6 .\n_:e7 <http://example.org/vocab#p> _:e10 .\n_:e7 <http://example.org/vocab#p> _:e11 .\n_:e8 <http://example.org/vocab#p> _:e6 .\n_:e8 <http://example.org/vocab#p> _:e10 .\n_:e8 <http://example.org/vocab#p> _:e11 .\n_:e9 <http://example.org/vocab#p> _:e6 .\n_:e9 <http://example.org/vocab#p> _:e10 .\n_:e9 <http://example.org/vocab#p> _:e11 .\n_:e10 <http://example.org/vocab#p> _:e7 .\n_:e10 <http://example.org/vocab#p> _:e8 .\n_:e10 <http://example.org/vocab#p> _:e9 .\n_:e11 <http://example.org/vocab#p> _:e7 .\n_:e11 <http://example.org/vocab#p> _:e8 .\n_:e11 <http://example.org/vocab#p> _:e9 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": "rdfc:RDFC10EvalTest",
    "expected": "_:c14n0 <http://example.org/vocab#p> _:c14n1 .\n_:c14n0 <http://example.org/vocab#p> _:c14n2 .\n_:c14n0 <http://example.org/vocab#p> _:c14n3 .\n_:c14n1 <http://example.org/vocab#p> _:c14n0 .\n_:c14n1 <http://example.org/vocab#p> _:c14n4 .\n_:c14n1 <http://example.org/vocab#p> _:c14n5 .\n_:c14n10 <http://example.org/vocab#p> _:c14n7 .\n_:c14n10 <http://example.org/vocab#p> _:c14n8 .\n_:c14n10 <http://example.org/vocab#p> _:c14n9 .\n_:c14n11 <http://example.org/vocab#p> _:c14n7 .\n_:c14n11 <http://example.org/vocab#p> _:c14n8 .\n_:c14n11 <http://example.org/vocab#p> _:c14n9 .\n_:c14n2 <http://example.org/vocab#p> _:c14n0 .\n_:c14n2 <http://example.org/vocab#p> _:c14n3 .\n_:c14n2 <http://example.org/vocab#p> _:c14n5 .\n_:c14n3 <http://example.org/vocab#p> _:c14n0 .\n_:c14n3 <http://example.org/vocab#p> _:c14n2 .\n_:c14n3 <http://example.org/vocab#p> _:c14n4 .\n_:c14n4 <http://example.org/vocab#p> _:c14n1 .\n_:c14n4 <http://example.org/vocab#p> _:c14n3 .\n_:c14n4 <http://example.org/vocab#p> _:c14n5 .\n_:c14n5 <http://example.org/vocab#p> _:c14n1 .\n_:c14n5 <http://example.org/vocab#p> _:c14n2 .\n_:c14n5 <http://example.org/vocab#p> _:c14n4 .\n_:c14n6 <http://example.org/vocab#p> _:c14n7 .\n_:c14n6 <http://example.org/vocab#p> _:c14n8 .\n_:c14n6 <http://example.org/vocab#p> _:c14n9 .\n_:c14n7 <http://example.org/vocab#p> _:c14n10 .\n_:c14n7 <http://example.org/vocab#p> _:c14n11 .\n_:c14n7 <http://example.org/vocab#p> _:c14n6 .\n_:c14n8 <http://example.org/vocab#p> _:c14n10 .\n_:c14n8 <http://example.org/vocab#p> _:c14n11 .\n_:c14n8 <http://example.org/vocab#p> _:c14n6 .\n_:c14n9 <http://example.org/vocab#p> _:c14n10 .\n_:c14n9 <http://example.org/vocab#p> _:c14n11 .\n_:c14n9 <http://example.org/vocab#p> _:c14n6 .\n",
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://example.org/vocab#p> _:c14n1 .\n_:c14n0 <http://example.org/vocab#p> _:c14n2 .\n_:c14n0 <http://example.org/vocab#p> _:c14n3 .\n_:c14n1 <http://example.org/vocab#p> _:c14n0 .\n_:c14n1 <http://example.org/vocab#p> _:c14n4 .\n_:c14n1 <http://example.org/vocab#p> _:c14n5 .\n_:c14n10 <http://example.org/vocab#p> _:c14n7 .\n_:c14n10 <http://example.org/vocab#p> _:c14n8 .\n_:c14n10 <http://example.org/vocab#p> _:c14n9 .\n_:c14n11 <http://example.org/vocab#p> _:c14n7 .\n_:c14n11 <http://example.org/vocab#p> _:c14n8 .\n_:c14n11 <http://example.org/vocab#p> _:c14n9 .\n_:c14n2 <http://example.org/vocab#p> _:c14n0 .\n_:c14n2 <http://example.org/vocab#p> _:c14n3 .\n_:c14n2 <http://example.org/vocab#p> _:c14n5 .\n_:c14n3 <http://example.org/vocab#p> _:c14n0 .\n_:c14n3 <http://example.org/vocab#p> _:c14n2 .\n_:c14n3 <http://example.org/vocab#p> _:c14n4 .\n_:c14n4 <http://example.org/vocab#p> _:c14n1 .\n_:c14n4 <http://example.org/vocab#p> _:c14n3 .\n_:c14n4 <http://example.org/vocab#p> _:c14n5 .\n_:c14n5 <http://example.org/vocab#p> _:c14n1 .\n_:c14n5 <http://example.org/vocab#p> _:c14n2 .\n_:c14n5 <http://example.org/vocab#p> _:c14n4 .\n_:c14n6 <http://example.org/vocab#p> _:c14n7 .\n_:c14n6 <http://example.org/vocab#p> _:c14n8 .\n_:c14n6 <http://example.org/vocab#p> _:c14n9 .\n_:c14n7 <http://example.org/vocab#p> _:c14n10 .\n_:c14n7 <http://example.org/vocab#p> _:c14n11 .\n_:c14n7 <http://example.org/vocab#p> _:c14n6 .\n_:c14n8 <http://example.org/vocab#p> _:c14n10 .\n_:c14n8 <http://example.org/vocab#p> _:c14n11 .\n_:c14n8 <http://example.org/vocab#p> _:c14n6 .\n_:c14n9 <http://example.org/vocab#p> _:c14n10 .\n_:c14n9 <http://example.org/vocab#p> _:c14n11 .\n_:c14n9 <http://example.org/vocab#p> _:c14n6 .\n",
      "map": {
        "e0": "c14n0",
        "e2": "c14n1",
        "e3": "c14n2",
        "e1": "c14n3",
        "e4": "c14n4",
        "e5": "c14n5",
        "e6": "c14n6",
        "e7": "c14n7",
        "e8": "c14n8",
        "e9": "c14n9",
        "e10": "c14n10",
        "e11": "c14n11"
      }
    },
    "result": {
      "status": "error",
      "error": "global HNDQ budget",
      "kind": "experimental_budget",
      "stats": {
        "executions": 22,
        "hndqCalls": 4001,
        "leaves": 0,
        "choices": 21,
        "topComplete": 0,
        "topPartial": 0,
        "topCrossGroup": 0,
        "topOverlapping": 0,
        "recursive": 21,
        "maxPending": 30
      },
      "partialOutcomes": 0
    }
  },
  {
    "name": "test045c",
    "group": "W3C",
    "input": "_:e0 <http://example.org/vocab#p> _:e1 .\n_:e0 <http://example.org/vocab#p> _:e2 .\n_:e0 <http://example.org/vocab#p> _:e3 .\n_:e1 <http://example.org/vocab#p> _:e0 .\n_:e1 <http://example.org/vocab#p> _:e4 .\n_:e1 <http://example.org/vocab#p> _:e5 .\n_:e2 <http://example.org/vocab#p> _:e0 .\n_:e2 <http://example.org/vocab#p> _:e4 .\n_:e2 <http://example.org/vocab#p> _:e5 .\n_:e3 <http://example.org/vocab#p> _:e0 .\n_:e3 <http://example.org/vocab#p> _:e4 .\n_:e3 <http://example.org/vocab#p> _:e5 .\n_:e4 <http://example.org/vocab#p> _:e1 .\n_:e4 <http://example.org/vocab#p> _:e2 .\n_:e4 <http://example.org/vocab#p> _:e3 .\n_:e5 <http://example.org/vocab#p> _:e1 .\n_:e5 <http://example.org/vocab#p> _:e2 .\n_:e5 <http://example.org/vocab#p> _:e3 .\n_:e6 <http://example.org/vocab#p> _:e7 .\n_:e6 <http://example.org/vocab#p> _:e8 .\n_:e6 <http://example.org/vocab#p> _:e9 .\n_:e7 <http://example.org/vocab#p> _:e6 .\n_:e7 <http://example.org/vocab#p> _:e9 .\n_:e7 <http://example.org/vocab#p> _:e10 .\n_:e8 <http://example.org/vocab#p> _:e6 .\n_:e8 <http://example.org/vocab#p> _:e10 .\n_:e8 <http://example.org/vocab#p> _:e11 .\n_:e9 <http://example.org/vocab#p> _:e6 .\n_:e9 <http://example.org/vocab#p> _:e7 .\n_:e9 <http://example.org/vocab#p> _:e11 .\n_:e10 <http://example.org/vocab#p> _:e7 .\n_:e10 <http://example.org/vocab#p> _:e8 .\n_:e10 <http://example.org/vocab#p> _:e11 .\n_:e11 <http://example.org/vocab#p> _:e9 .\n_:e11 <http://example.org/vocab#p> _:e8 .\n_:e11 <http://example.org/vocab#p> _:e10 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": "rdfc:RDFC10EvalTest",
    "expected": "_:c14n0 <http://example.org/vocab#p> _:c14n1 .\n_:c14n0 <http://example.org/vocab#p> _:c14n2 .\n_:c14n0 <http://example.org/vocab#p> _:c14n3 .\n_:c14n1 <http://example.org/vocab#p> _:c14n0 .\n_:c14n1 <http://example.org/vocab#p> _:c14n4 .\n_:c14n1 <http://example.org/vocab#p> _:c14n5 .\n_:c14n10 <http://example.org/vocab#p> _:c14n7 .\n_:c14n10 <http://example.org/vocab#p> _:c14n8 .\n_:c14n10 <http://example.org/vocab#p> _:c14n9 .\n_:c14n11 <http://example.org/vocab#p> _:c14n7 .\n_:c14n11 <http://example.org/vocab#p> _:c14n8 .\n_:c14n11 <http://example.org/vocab#p> _:c14n9 .\n_:c14n2 <http://example.org/vocab#p> _:c14n0 .\n_:c14n2 <http://example.org/vocab#p> _:c14n3 .\n_:c14n2 <http://example.org/vocab#p> _:c14n5 .\n_:c14n3 <http://example.org/vocab#p> _:c14n0 .\n_:c14n3 <http://example.org/vocab#p> _:c14n2 .\n_:c14n3 <http://example.org/vocab#p> _:c14n4 .\n_:c14n4 <http://example.org/vocab#p> _:c14n1 .\n_:c14n4 <http://example.org/vocab#p> _:c14n3 .\n_:c14n4 <http://example.org/vocab#p> _:c14n5 .\n_:c14n5 <http://example.org/vocab#p> _:c14n1 .\n_:c14n5 <http://example.org/vocab#p> _:c14n2 .\n_:c14n5 <http://example.org/vocab#p> _:c14n4 .\n_:c14n6 <http://example.org/vocab#p> _:c14n7 .\n_:c14n6 <http://example.org/vocab#p> _:c14n8 .\n_:c14n6 <http://example.org/vocab#p> _:c14n9 .\n_:c14n7 <http://example.org/vocab#p> _:c14n10 .\n_:c14n7 <http://example.org/vocab#p> _:c14n11 .\n_:c14n7 <http://example.org/vocab#p> _:c14n6 .\n_:c14n8 <http://example.org/vocab#p> _:c14n10 .\n_:c14n8 <http://example.org/vocab#p> _:c14n11 .\n_:c14n8 <http://example.org/vocab#p> _:c14n6 .\n_:c14n9 <http://example.org/vocab#p> _:c14n10 .\n_:c14n9 <http://example.org/vocab#p> _:c14n11 .\n_:c14n9 <http://example.org/vocab#p> _:c14n6 .\n",
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://example.org/vocab#p> _:c14n1 .\n_:c14n0 <http://example.org/vocab#p> _:c14n2 .\n_:c14n0 <http://example.org/vocab#p> _:c14n3 .\n_:c14n1 <http://example.org/vocab#p> _:c14n0 .\n_:c14n1 <http://example.org/vocab#p> _:c14n4 .\n_:c14n1 <http://example.org/vocab#p> _:c14n5 .\n_:c14n10 <http://example.org/vocab#p> _:c14n7 .\n_:c14n10 <http://example.org/vocab#p> _:c14n8 .\n_:c14n10 <http://example.org/vocab#p> _:c14n9 .\n_:c14n11 <http://example.org/vocab#p> _:c14n7 .\n_:c14n11 <http://example.org/vocab#p> _:c14n8 .\n_:c14n11 <http://example.org/vocab#p> _:c14n9 .\n_:c14n2 <http://example.org/vocab#p> _:c14n0 .\n_:c14n2 <http://example.org/vocab#p> _:c14n3 .\n_:c14n2 <http://example.org/vocab#p> _:c14n5 .\n_:c14n3 <http://example.org/vocab#p> _:c14n0 .\n_:c14n3 <http://example.org/vocab#p> _:c14n2 .\n_:c14n3 <http://example.org/vocab#p> _:c14n4 .\n_:c14n4 <http://example.org/vocab#p> _:c14n1 .\n_:c14n4 <http://example.org/vocab#p> _:c14n3 .\n_:c14n4 <http://example.org/vocab#p> _:c14n5 .\n_:c14n5 <http://example.org/vocab#p> _:c14n1 .\n_:c14n5 <http://example.org/vocab#p> _:c14n2 .\n_:c14n5 <http://example.org/vocab#p> _:c14n4 .\n_:c14n6 <http://example.org/vocab#p> _:c14n7 .\n_:c14n6 <http://example.org/vocab#p> _:c14n8 .\n_:c14n6 <http://example.org/vocab#p> _:c14n9 .\n_:c14n7 <http://example.org/vocab#p> _:c14n10 .\n_:c14n7 <http://example.org/vocab#p> _:c14n11 .\n_:c14n7 <http://example.org/vocab#p> _:c14n6 .\n_:c14n8 <http://example.org/vocab#p> _:c14n10 .\n_:c14n8 <http://example.org/vocab#p> _:c14n11 .\n_:c14n8 <http://example.org/vocab#p> _:c14n6 .\n_:c14n9 <http://example.org/vocab#p> _:c14n10 .\n_:c14n9 <http://example.org/vocab#p> _:c14n11 .\n_:c14n9 <http://example.org/vocab#p> _:c14n6 .\n",
      "map": {
        "e6": "c14n0",
        "e8": "c14n1",
        "e9": "c14n2",
        "e7": "c14n3",
        "e10": "c14n4",
        "e11": "c14n5",
        "e0": "c14n6",
        "e1": "c14n7",
        "e2": "c14n8",
        "e3": "c14n9",
        "e4": "c14n10",
        "e5": "c14n11"
      }
    },
    "result": {
      "status": "error",
      "error": "global HNDQ budget",
      "kind": "experimental_budget",
      "stats": {
        "executions": 36,
        "hndqCalls": 4001,
        "leaves": 0,
        "choices": 35,
        "topComplete": 0,
        "topPartial": 0,
        "topCrossGroup": 0,
        "topOverlapping": 0,
        "recursive": 35,
        "maxPending": 56
      },
      "partialOutcomes": 0
    }
  },
  {
    "name": "test046c",
    "group": "W3C",
    "input": "_:e0 <http://example.org/vocab#p> _:e1 .\n_:e0 <http://example.org/vocab#p> _:e2 .\n_:e0 <http://example.org/vocab#p> _:e3 .\n_:e1 <http://example.org/vocab#p> _:e0 .\n_:e1 <http://example.org/vocab#p> _:e9 .\n_:e1 <http://example.org/vocab#p> _:e8 .\n_:e2 <http://example.org/vocab#p> _:e3 .\n_:e2 <http://example.org/vocab#p> _:e8 .\n_:e2 <http://example.org/vocab#p> _:e0 .\n_:e3 <http://example.org/vocab#p> _:e0 .\n_:e3 <http://example.org/vocab#p> _:e2 .\n_:e3 <http://example.org/vocab#p> _:e9 .\n_:e4 <http://example.org/vocab#p> _:e5 .\n_:e4 <http://example.org/vocab#p> _:e6 .\n_:e4 <http://example.org/vocab#p> _:e7 .\n_:e5 <http://example.org/vocab#p> _:e10 .\n_:e5 <http://example.org/vocab#p> _:e4 .\n_:e5 <http://example.org/vocab#p> _:e11 .\n_:e6 <http://example.org/vocab#p> _:e4 .\n_:e6 <http://example.org/vocab#p> _:e11 .\n_:e6 <http://example.org/vocab#p> _:e10 .\n_:e7 <http://example.org/vocab#p> _:e10 .\n_:e7 <http://example.org/vocab#p> _:e11 .\n_:e7 <http://example.org/vocab#p> _:e4 .\n_:e8 <http://example.org/vocab#p> _:e1 .\n_:e8 <http://example.org/vocab#p> _:e2 .\n_:e8 <http://example.org/vocab#p> _:e9 .\n_:e9 <http://example.org/vocab#p> _:e8 .\n_:e9 <http://example.org/vocab#p> _:e3 .\n_:e9 <http://example.org/vocab#p> _:e1 .\n_:e10 <http://example.org/vocab#p> _:e6 .\n_:e10 <http://example.org/vocab#p> _:e7 .\n_:e10 <http://example.org/vocab#p> _:e5 .\n_:e11 <http://example.org/vocab#p> _:e5 .\n_:e11 <http://example.org/vocab#p> _:e6 .\n_:e11 <http://example.org/vocab#p> _:e7 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": "rdfc:RDFC10EvalTest",
    "expected": "_:c14n0 <http://example.org/vocab#p> _:c14n1 .\n_:c14n0 <http://example.org/vocab#p> _:c14n2 .\n_:c14n0 <http://example.org/vocab#p> _:c14n3 .\n_:c14n1 <http://example.org/vocab#p> _:c14n0 .\n_:c14n1 <http://example.org/vocab#p> _:c14n4 .\n_:c14n1 <http://example.org/vocab#p> _:c14n5 .\n_:c14n10 <http://example.org/vocab#p> _:c14n7 .\n_:c14n10 <http://example.org/vocab#p> _:c14n8 .\n_:c14n10 <http://example.org/vocab#p> _:c14n9 .\n_:c14n11 <http://example.org/vocab#p> _:c14n7 .\n_:c14n11 <http://example.org/vocab#p> _:c14n8 .\n_:c14n11 <http://example.org/vocab#p> _:c14n9 .\n_:c14n2 <http://example.org/vocab#p> _:c14n0 .\n_:c14n2 <http://example.org/vocab#p> _:c14n3 .\n_:c14n2 <http://example.org/vocab#p> _:c14n5 .\n_:c14n3 <http://example.org/vocab#p> _:c14n0 .\n_:c14n3 <http://example.org/vocab#p> _:c14n2 .\n_:c14n3 <http://example.org/vocab#p> _:c14n4 .\n_:c14n4 <http://example.org/vocab#p> _:c14n1 .\n_:c14n4 <http://example.org/vocab#p> _:c14n3 .\n_:c14n4 <http://example.org/vocab#p> _:c14n5 .\n_:c14n5 <http://example.org/vocab#p> _:c14n1 .\n_:c14n5 <http://example.org/vocab#p> _:c14n2 .\n_:c14n5 <http://example.org/vocab#p> _:c14n4 .\n_:c14n6 <http://example.org/vocab#p> _:c14n7 .\n_:c14n6 <http://example.org/vocab#p> _:c14n8 .\n_:c14n6 <http://example.org/vocab#p> _:c14n9 .\n_:c14n7 <http://example.org/vocab#p> _:c14n10 .\n_:c14n7 <http://example.org/vocab#p> _:c14n11 .\n_:c14n7 <http://example.org/vocab#p> _:c14n6 .\n_:c14n8 <http://example.org/vocab#p> _:c14n10 .\n_:c14n8 <http://example.org/vocab#p> _:c14n11 .\n_:c14n8 <http://example.org/vocab#p> _:c14n6 .\n_:c14n9 <http://example.org/vocab#p> _:c14n10 .\n_:c14n9 <http://example.org/vocab#p> _:c14n11 .\n_:c14n9 <http://example.org/vocab#p> _:c14n6 .\n",
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://example.org/vocab#p> _:c14n1 .\n_:c14n0 <http://example.org/vocab#p> _:c14n2 .\n_:c14n0 <http://example.org/vocab#p> _:c14n3 .\n_:c14n1 <http://example.org/vocab#p> _:c14n0 .\n_:c14n1 <http://example.org/vocab#p> _:c14n4 .\n_:c14n1 <http://example.org/vocab#p> _:c14n5 .\n_:c14n10 <http://example.org/vocab#p> _:c14n7 .\n_:c14n10 <http://example.org/vocab#p> _:c14n8 .\n_:c14n10 <http://example.org/vocab#p> _:c14n9 .\n_:c14n11 <http://example.org/vocab#p> _:c14n7 .\n_:c14n11 <http://example.org/vocab#p> _:c14n8 .\n_:c14n11 <http://example.org/vocab#p> _:c14n9 .\n_:c14n2 <http://example.org/vocab#p> _:c14n0 .\n_:c14n2 <http://example.org/vocab#p> _:c14n3 .\n_:c14n2 <http://example.org/vocab#p> _:c14n5 .\n_:c14n3 <http://example.org/vocab#p> _:c14n0 .\n_:c14n3 <http://example.org/vocab#p> _:c14n2 .\n_:c14n3 <http://example.org/vocab#p> _:c14n4 .\n_:c14n4 <http://example.org/vocab#p> _:c14n1 .\n_:c14n4 <http://example.org/vocab#p> _:c14n3 .\n_:c14n4 <http://example.org/vocab#p> _:c14n5 .\n_:c14n5 <http://example.org/vocab#p> _:c14n1 .\n_:c14n5 <http://example.org/vocab#p> _:c14n2 .\n_:c14n5 <http://example.org/vocab#p> _:c14n4 .\n_:c14n6 <http://example.org/vocab#p> _:c14n7 .\n_:c14n6 <http://example.org/vocab#p> _:c14n8 .\n_:c14n6 <http://example.org/vocab#p> _:c14n9 .\n_:c14n7 <http://example.org/vocab#p> _:c14n10 .\n_:c14n7 <http://example.org/vocab#p> _:c14n11 .\n_:c14n7 <http://example.org/vocab#p> _:c14n6 .\n_:c14n8 <http://example.org/vocab#p> _:c14n10 .\n_:c14n8 <http://example.org/vocab#p> _:c14n11 .\n_:c14n8 <http://example.org/vocab#p> _:c14n6 .\n_:c14n9 <http://example.org/vocab#p> _:c14n10 .\n_:c14n9 <http://example.org/vocab#p> _:c14n11 .\n_:c14n9 <http://example.org/vocab#p> _:c14n6 .\n",
      "map": {
        "e0": "c14n0",
        "e1": "c14n1",
        "e2": "c14n2",
        "e3": "c14n3",
        "e9": "c14n4",
        "e8": "c14n5",
        "e4": "c14n6",
        "e5": "c14n7",
        "e6": "c14n8",
        "e7": "c14n9",
        "e10": "c14n10",
        "e11": "c14n11"
      }
    },
    "result": {
      "status": "error",
      "error": "global HNDQ budget",
      "kind": "experimental_budget",
      "stats": {
        "executions": 22,
        "hndqCalls": 4001,
        "leaves": 0,
        "choices": 21,
        "topComplete": 0,
        "topPartial": 0,
        "topCrossGroup": 0,
        "topOverlapping": 0,
        "recursive": 21,
        "maxPending": 30
      },
      "partialOutcomes": 0
    }
  },
  {
    "name": "test059c",
    "group": "W3C",
    "input": "<urn:ex:s> <urn:ex:p> <urn:ex:o> <urn:ex:g> .\n_:s0 <urn:ex:p> _:o0 _:g0 .\n_:s1 <urn:ex:p> _:o1 _:g1 .\n_:s2 <urn:ex:p> _:o2 _:g2 .\n_:s3 <urn:ex:p> _:o3 _:g3 .\n_:s4 <urn:ex:p> _:o4 _:g4 .\n_:s5 <urn:ex:p> _:o5 _:g5 .\n_:s6 <urn:ex:p> <urn:ex:o> <urn:ex:g> .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": "rdfc:RDFC10EvalTest",
    "expected": "<urn:ex:s> <urn:ex:p> <urn:ex:o> <urn:ex:g> .\n_:c14n0 <urn:ex:p> <urn:ex:o> <urn:ex:g> .\n_:c14n1 <urn:ex:p> _:c14n3 _:c14n2 .\n_:c14n10 <urn:ex:p> _:c14n12 _:c14n11 .\n_:c14n13 <urn:ex:p> _:c14n15 _:c14n14 .\n_:c14n16 <urn:ex:p> _:c14n18 _:c14n17 .\n_:c14n4 <urn:ex:p> _:c14n6 _:c14n5 .\n_:c14n7 <urn:ex:p> _:c14n9 _:c14n8 .\n",
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "<urn:ex:s> <urn:ex:p> <urn:ex:o> <urn:ex:g> .\n_:c14n0 <urn:ex:p> <urn:ex:o> <urn:ex:g> .\n_:c14n1 <urn:ex:p> _:c14n3 _:c14n2 .\n_:c14n10 <urn:ex:p> _:c14n12 _:c14n11 .\n_:c14n13 <urn:ex:p> _:c14n15 _:c14n14 .\n_:c14n16 <urn:ex:p> _:c14n18 _:c14n17 .\n_:c14n4 <urn:ex:p> _:c14n6 _:c14n5 .\n_:c14n7 <urn:ex:p> _:c14n9 _:c14n8 .\n",
      "map": {
        "s6": "c14n0",
        "s0": "c14n1",
        "g0": "c14n2",
        "o0": "c14n3",
        "s1": "c14n4",
        "g1": "c14n5",
        "o1": "c14n6",
        "s2": "c14n7",
        "g2": "c14n8",
        "o2": "c14n9",
        "s3": "c14n10",
        "g3": "c14n11",
        "o3": "c14n12",
        "s4": "c14n13",
        "g4": "c14n14",
        "o4": "c14n15",
        "s5": "c14n16",
        "g5": "c14n17",
        "o5": "c14n18"
      }
    },
    "result": {
      "status": "error",
      "error": "execution budget",
      "kind": "experimental_budget",
      "stats": {
        "executions": 64,
        "hndqCalls": 1152,
        "leaves": 36,
        "choices": 28,
        "topComplete": 0,
        "topPartial": 28,
        "topCrossGroup": 28,
        "topOverlapping": 0,
        "recursive": 0,
        "maxPending": 16
      },
      "partialOutcomes": 1
    }
  },
  {
    "name": "two-distinct-predicate-components-r0-n0",
    "group": "two-distinct-predicate-components",
    "input": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n_:d4 <http://other/q> _:d2 _:d3 .\n_:d4 <http://other/p> _:d0 .\n_:d0 <http://other/q> _:d3 _:d2 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": null,
    "expected": null,
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n5 _:c14n4 .\n_:c14n1 <http://other/p> _:c14n3 .\n_:c14n1 <http://other/q> _:c14n6 _:c14n7 .\n_:c14n2 <http://ex/p> _:c14n0 .\n_:c14n2 <http://ex/q> _:c14n4 _:c14n5 .\n_:c14n3 <http://other/q> _:c14n7 _:c14n6 .\n",
      "map": {
        "b0": "c14n0",
        "d4": "c14n1",
        "b4": "c14n2",
        "d0": "c14n3",
        "b2": "c14n4",
        "b3": "c14n5",
        "d2": "c14n6",
        "d3": "c14n7"
      }
    },
    "result": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n4 _:c14n5 .\n_:c14n1 <http://other/p> _:c14n3 .\n_:c14n1 <http://other/q> _:c14n6 _:c14n7 .\n_:c14n2 <http://ex/p> _:c14n0 .\n_:c14n2 <http://ex/q> _:c14n5 _:c14n4 .\n_:c14n3 <http://other/q> _:c14n7 _:c14n6 .\n",
      "map": {
        "b0": "c14n0",
        "d4": "c14n1",
        "b4": "c14n2",
        "d0": "c14n3",
        "b3": "c14n4",
        "b2": "c14n5",
        "d2": "c14n6",
        "d3": "c14n7"
      },
      "distinctOutcomes": 4,
      "mapConsistent": true,
      "stats": {
        "executions": 7,
        "hndqCalls": 52,
        "leaves": 4,
        "choices": 3,
        "topComplete": 2,
        "topPartial": 1,
        "topCrossGroup": 0,
        "topOverlapping": 3,
        "recursive": 0,
        "maxPending": 3
      }
    }
  },
  {
    "name": "two-same-predicate-components-r0-n0",
    "group": "two-same-predicate-components",
    "input": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n_:d4 <http://ex/q> _:d2 _:d3 .\n_:d4 <http://ex/p> _:d0 .\n_:d0 <http://ex/q> _:d3 _:d2 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": null,
    "expected": null,
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n2 <http://ex/p> _:c14n3 .\n_:c14n2 <http://ex/q> _:c14n1 _:c14n0 .\n_:c14n3 <http://ex/q> _:c14n0 _:c14n1 .\n_:c14n6 <http://ex/p> _:c14n7 .\n_:c14n6 <http://ex/q> _:c14n5 _:c14n4 .\n_:c14n7 <http://ex/q> _:c14n4 _:c14n5 .\n",
      "map": {
        "b3": "c14n0",
        "b2": "c14n1",
        "b4": "c14n2",
        "b0": "c14n3",
        "d3": "c14n4",
        "d2": "c14n5",
        "d4": "c14n6",
        "d0": "c14n7"
      }
    },
    "result": {
      "status": "complete",
      "output": "_:c14n2 <http://ex/p> _:c14n3 .\n_:c14n2 <http://ex/q> _:c14n1 _:c14n0 .\n_:c14n3 <http://ex/q> _:c14n0 _:c14n1 .\n_:c14n6 <http://ex/p> _:c14n7 .\n_:c14n6 <http://ex/q> _:c14n5 _:c14n4 .\n_:c14n7 <http://ex/q> _:c14n4 _:c14n5 .\n",
      "map": {
        "b3": "c14n0",
        "b2": "c14n1",
        "b4": "c14n2",
        "b0": "c14n3",
        "d3": "c14n4",
        "d2": "c14n5",
        "d4": "c14n6",
        "d0": "c14n7"
      },
      "distinctOutcomes": 1,
      "mapConsistent": true,
      "stats": {
        "executions": 3,
        "hndqCalls": 48,
        "leaves": 2,
        "choices": 1,
        "topComplete": 0,
        "topPartial": 1,
        "topCrossGroup": 1,
        "topOverlapping": 0,
        "recursive": 0,
        "maxPending": 2
      }
    }
  },
  {
    "name": "saved-plus-cycle-r0-n0",
    "group": "saved-plus-cycle",
    "input": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n_:u0 <http://ex/r> _:u1 .\n_:u1 <http://ex/r> _:u0 .\n",
    "hash": "SHA256",
    "control": "candidate",
    "kind": null,
    "expected": null,
    "limits": {
      "hndqCalls": 4000,
      "executions": 64,
      "milliseconds": 1000
    },
    "baseline": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n4 <http://ex/r> _:c14n5 .\n_:c14n5 <http://ex/r> _:c14n4 .\n",
      "map": {
        "b0": "c14n0",
        "b4": "c14n1",
        "b2": "c14n2",
        "b3": "c14n3",
        "u0": "c14n4",
        "u1": "c14n5"
      }
    },
    "result": {
      "status": "complete",
      "output": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n4 <http://ex/r> _:c14n5 .\n_:c14n5 <http://ex/r> _:c14n4 .\n",
      "map": {
        "b0": "c14n0",
        "b4": "c14n1",
        "b3": "c14n2",
        "b2": "c14n3",
        "u0": "c14n4",
        "u1": "c14n5"
      },
      "distinctOutcomes": 2,
      "mapConsistent": true,
      "stats": {
        "executions": 7,
        "hndqCalls": 52,
        "leaves": 4,
        "choices": 3,
        "topComplete": 2,
        "topPartial": 1,
        "topCrossGroup": 0,
        "topOverlapping": 3,
        "recursive": 0,
        "maxPending": 3
      }
    }
  }
]
```
