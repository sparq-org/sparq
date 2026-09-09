# Issue6475: completed baseline and production seam evidence

OpenAI GPT-6 Astra, actual xhigh lane. Production remains clean at e53464c73f31f7aca800f3867ac054c36408e346. This supplements the previously reviewed trace/reference evidence; no patch exists or is approved.

## Completed scope

The unmodified Rust12-case matrix is order-invariant for this particular input but label-dependent: each labeling has one canonical output across all6orders, and the two labels give different outputs. Both inputs succeed at the Rust default4000 global-call budget and exactly at4; limits0..3 return HndqCallLimitExceeded. JS first-appearance behavior is not contradicted by a pure renaming: a bijection with quadorder fixed preserves that abstract order.

All86 packaged W3C entries were executed:64canonical-document,21issuer-map,1negative. Rust defaults pass86; JS5 diagnostic4000-call/1s pass86; JS default workfactor1 with1s wall cap passes68 and rejects18positive fixtures. No skips, unknown types, parse errors or external timeouts occurred. The full baseline is from the pinned package manifest; it is not a fresh-current W3C download, a full Rust unit-test execution or a full Sparq workspace run. Exact expected/output bytes and per-case raw command results remain in the hashed bundle.

## Sparq seam and CI connection

```json
{
  "main": "e53464c73f31f7aca800f3867ac054c36408e346",
  "pr_head": "86b8dfa232ad7d315dd56df28759ac5fca7b365c",
  "merge_group": "2adb86d91345deb51e95b11af4e2b5572343c236",
  "failed_source_job": "run34409424033/job102661090898, opt-in group g20, cargo test -p sparq-canon --features concept",
  "property": "canonical_output_invariant_under_bnode_relabeling at tests/proptest_canon_determinism.rs:370\u2013384;256cases unchanged; materialize preserves quad order and component roles with injective label map.",
  "seam": "canonicalize_quads(lib.rs310) -> serialize_quads/default435 or feature-lowcopy -> parse_02(495) -> rdf_canon::canonicalize_quads; default HNDQ budget retained.",
  "dependency": "Cargo.toml declares compatible version constraints0.15.3/0.2.4/0.1.8, not exact-equals pins. Cargo.lock locks registry rdf-canon0.15.3,oxrdf0.2.4,oxttl0.1.8. Sparq native models use oxrdf0.3.3/oxttl0.2.3. No relevant source patch override found in root Cargo.toml/.cargo.",
  "identity": "Three seam/test/manifest files byte-identical to frozen failing CI source."
}
```

## Budget and secondary issuer-order lead

```json
{
  "B5": {
    "default": 4000,
    "semantics": "SimpleHndqCallCounter increments a single global counter on entering hash_n_degree_quads; counter>limit errors. None/default selects4000; the public API uses Simple, not PerNode. This is a call bound, not a deadline/strict full-work bound.",
    "error": "CanonicalizationError::HndqCallLimitExceeded(usize); Sparq maps it to CanonError::Canonicalization(String), losing enum detail but propagating failure.",
    "counterexample": "Both renamings require exactly4 HNDQ entry calls: limits0..3 error,4/default succeed. An ambiguity rejection is demonstrably a new rejection for this accepted input.",
    "accepted_set_limit": "Current acceptance characterized for this fixed matrix and all packaged fixtures, not an exhaustive mathematical set of all inputs."
  },
  "B6": {
    "finding": "Source restores temporary issuer order by BTreeMap keyed on bN strings, although issuer map is HashMap and issuance uses integer counter. At11+ labels lexical b10 precedes b2, differing from chronological issuance.",
    "status": "Independent source lead warrants a dedicated small fixture if root prioritizes it. No11+ temporary-issuer counterexample was constructed in this phase; not asserted as the cause of6475 or a demonstrated output failure. All packaged baseline fixtures still passed."
  },
  "N2": "Pure bijective relabeling with the quad sequence held fixed preserves abstract first-appearance order. No fabricated relabeling was used to claim otherwise; JS order dependence remains the distinct test."
}
```

## Actual canonical-byte consumers

```json
[
  {
    "path": "crates/sparq-canon/src/lib.rs",
    "lines": "112\u2013114,310\u2013352,365\u2013412",
    "role": "Public dataset canonical bytes, digest of exact canonical bytes, and issuer maps delegate through shared bridge to rdf-canon. SHA256 default; parameterized profiles share algorithm.",
    "input": "General RDF1.1 quads including blank graph names; saved shape reachable."
  },
  {
    "path": "crates/sparq-canon/src/concept.rs",
    "lines": "272\u2013279",
    "role": "Opt-in concept record digest/URN verification hashes canonical record bytes through digest_quads_with.",
    "input": "Caller-supplied quads; canonical-byte/error contract matters."
  },
  {
    "path": "crates/sparq-wasm/src/canon.rs; js/src/dataset.ts",
    "lines": "31\u201332;283,397\u2013398",
    "role": "Optional wasm canonicalizeNQuads forwards errors as JsError; RDF/JS Dataset.equals compares canonical strings and toCanonical returns them.",
    "input": "General datasets; actual exported path affected. Native/wasm execution not rerun here."
  },
  {
    "path": "crates/sparq-vc/src/suite.rs",
    "lines": "309\u2013320",
    "role": "hash_data signs/verifies SHA256(canonical proof configuration)||SHA256(canonical document).",
    "input": "Per-document triples only; this exact mixed-graph counterexample is not demonstrated reachable."
  },
  {
    "path": "crates/sparq-zk/src/canon.rs; commit.rs; dual_leaf.rs",
    "lines": "23\u201325;239\u2013258;648\u2013680",
    "role": "Reexported graph canonicalization defines canonical triple order, leaf order and Poseidon2 commitment inputs.",
    "input": "Per-graph triples; current dataset counterexample not a demonstrated exploit or reachability result."
  },
  {
    "path": "crates/sparq-zk/src/trace.rs",
    "lines": "82\u2013107",
    "role": "Issued canonical labels map source triples to committed canonical leaf indices.",
    "input": "Per-graph triples; output/map agreement must be preserved."
  },
  {
    "path": "crates/sparq-zk/src/vc_bridge.rs; vc_bridge_sd.rs",
    "lines": "318\u2013340;147\u2013148",
    "role": "Canonical proof/document strings feed signature hashData and selective-disclosure host verifier interface.",
    "input": "Triples; same qualification as VC above."
  },
  {
    "path": "crates/sparq-difftest/src/iso.rs",
    "lines": "61,266\u2013271",
    "role": "Separate direct rdf-canon caller for differential isomorphism checks, bypasses Sparq bridge with explicit100000-call budget.",
    "input": "Comparison encoding; a Sparq seam-only repair would not repair this independent algorithm consumer."
  },
  {
    "path": "crates/sparq-bench/src/update_fuzz.rs; crates/sparq-canon/src/rdf12.rs",
    "lines": "774\u2013784;195\u2013197,318\u2013332",
    "role": "Update-fuzz comparison uses the separate nonstandard RDF1.2 implementation via ground-term guard, not upstream rdf-canon.",
    "input": "Separate implementation; standard dependency repair will not automatically change it. This phase did not replay the failure there."
  }
]
```

## Real repair options and compatibility boundaries

```json
[
  {
    "option": "Bounded comparison of complete equal-hash candidate outcomes at the shared canonicalizer",
    "assessment": "Most directly aligned with preserving the existing success/invariance property, but still a design proposal. Preserve all distinct-hash behavior and genuine symmetry. When tied complete candidate issuers produce distinct full canonical documents, a deterministic document-order choice could remove input-order dependence. Partial issuers/ties across later groups need a sound completion strategy under the existing budget; do not improvise a local original-label tie break.",
    "compatibility": "This is a canonical-output extension for an underdetermined case, not a proven existing RDFC1.0 requirement. Must test byte preservation over this baseline, property matrix, issuer/output consistency, poison limits, then obtain actual patch review. No source is approved or changed."
  },
  {
    "option": "Detect divergent complete tied outcomes and return a typed ambiguity error",
    "assessment": "Could prevent inconsistent canonical bytes and preserve genuine symmetric outputs; requires complete-outcome proof and bounded work. A blanket equal-hash rejection is disproved as a safe policy by successful symmetric W3C controls.",
    "compatibility": "Changes currently accepted input to error. Current Sparq property calls expect success, so this alone would NOT repair the gate. No relaxation, exception or quarantine of that property is authorized."
  },
  {
    "option": "Replace or switch canonicalizer",
    "assessment": "Not evidence-supported as a small fix: JS5 shares the underdetermination and its default budget also rejects18 positive W3C fixtures. A separate complete canonical-labelling algorithm would be larger and could change standard bytes.",
    "compatibility": "Do not switch implementation merely to make this one relabeling order pass."
  }
]
```

No error-only proposal can make the current success property pass; no property weakening is proposed. The output-selecting option requires real algorithm review and byte-level compatibility evidence, not a claim that the standard already prescribes that tie breaker. Do not treat source-only B6 as a reproduced bug or let it substitute for this task.

## matrix-summary.json

```text
{
  "cases": 12,
  "accepted_under_Rust_defaults": 12,
  "quad_order_invariant_for_this_pair": true,
  "relabel_invariant_for_this_pair": false,
  "canonical_outputs": {
    "original": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
    "renamed": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
  },
  "budget_threshold": {
    "each_renaming_limits0through3": "HndqCallLimitExceeded(limit)",
    "limit4": "success",
    "default": "success at global limit4000"
  },
  "generalization": "Only this concrete pair tested; no general order-invariance proof."
}
```

## rust-budget-cases.json

```text
[
  {
    "labeling": "original",
    "limit": 0,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 0.",
      "error_debug": "HndqCallLimitExceeded(0)"
    }
  },
  {
    "labeling": "original",
    "limit": 1,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 1.",
      "error_debug": "HndqCallLimitExceeded(1)"
    }
  },
  {
    "labeling": "original",
    "limit": 2,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 2.",
      "error_debug": "HndqCallLimitExceeded(2)"
    }
  },
  {
    "labeling": "original",
    "limit": 3,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 3.",
      "error_debug": "HndqCallLimitExceeded(3)"
    }
  },
  {
    "labeling": "original",
    "limit": 4,
    "result": {
      "value": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
    }
  },
  {
    "labeling": "renamed",
    "limit": 0,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 0.",
      "error_debug": "HndqCallLimitExceeded(0)"
    }
  },
  {
    "labeling": "renamed",
    "limit": 1,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 1.",
      "error_debug": "HndqCallLimitExceeded(1)"
    }
  },
  {
    "labeling": "renamed",
    "limit": 2,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 2.",
      "error_debug": "HndqCallLimitExceeded(2)"
    }
  },
  {
    "labeling": "renamed",
    "limit": 3,
    "result": {
      "error": "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of 3.",
      "error_debug": "HndqCallLimitExceeded(3)"
    }
  },
  {
    "labeling": "renamed",
    "limit": 4,
    "result": {
      "value": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
    }
  }
]
```

## suite-counts.json

```text
[
  {
    "runtime": "rust",
    "budget": "default",
    "types": {
      "rdfc:RDFC10EvalTest": {
        "total": 64,
        "pass": 64
      },
      "rdfc:RDFC10MapTest": {
        "total": 21,
        "pass": 21
      },
      "rdfc:RDFC10NegativeEvalTest": {
        "total": 1,
        "pass": 1
      }
    },
    "skips": 0,
    "external_timeouts": 0,
    "nonJSON_errors": 0
  },
  {
    "runtime": "js",
    "budget": "default",
    "types": {
      "rdfc:RDFC10EvalTest": {
        "total": 64,
        "pass": 46
      },
      "rdfc:RDFC10MapTest": {
        "total": 21,
        "pass": 21
      },
      "rdfc:RDFC10NegativeEvalTest": {
        "total": 1,
        "pass": 1
      }
    },
    "skips": 0,
    "external_timeouts": 0,
    "nonJSON_errors": 0
  },
  {
    "runtime": "js",
    "budget": "4000",
    "types": {
      "rdfc:RDFC10EvalTest": {
        "total": 64,
        "pass": 64
      },
      "rdfc:RDFC10MapTest": {
        "total": 21,
        "pass": 21
      },
      "rdfc:RDFC10NegativeEvalTest": {
        "total": 1,
        "pass": 1
      }
    },
    "skips": 0,
    "external_timeouts": 0,
    "nonJSON_errors": 0
  }
]
```

## suite-summary.json

```text
{
  "rust-default": {
    "total": 86,
    "passed": 86,
    "failed_or_limited": []
  },
  "js-default": {
    "total": 86,
    "passed": 68,
    "failed_or_limited": [
      {
        "id": "#test021c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (2).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test022c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (2).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test023c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test024c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test025c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test026c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test027c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test028c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test029c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test044c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (12).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test045c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (12).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test046c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (12).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test064c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test065c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test066c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test067c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test068c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      },
      {
        "id": "#test069c",
        "type": "rdfc:RDFC10EvalTest",
        "result": {
          "error": "Maximum deep iterations exceeded (3).",
          "error_debug": "Error"
        }
      }
    ]
  },
  "js-4000": {
    "total": 86,
    "passed": 86,
    "failed_or_limited": []
  }
}
```

## protocol.json

```text
{
  "suite": "all86 entries of packaged rdf-canon0.15.3 tests/manifest.jsonld",
  "rust": "upstream defaults:4000 global HNDQ calls",
  "js_modes": [
    "default work factor1",
    "diagnostic fixed4000 deep iterations"
  ],
  "js_case_timeout_ms": 1000,
  "subprocess_timeout_seconds": 2,
  "whole_execution_cap_seconds": 120,
  "floor_bytes": 6442450944,
  "retries": 0,
  "note": "JS baseline default work factor is retained; an explicit1s case timeout is added and separately reported."
}
```

## harness/Cargo.toml

```text
[package]
name = "canon-relabel-replay"
version = "0.0.0"
edition = "2024"
rust-version = "1.85"
publish = false

[workspace]

[dependencies]
rdf-canon = "=0.15.3"
oxrdf = "=0.2.4"
oxttl = "=0.1.8"
sha2 = "=0.10.9"
serde_json = "=1.0.150"
serde = "=1.0.228"

[profile.dev]
debug = 0
incremental = false
```

## harness/src/main.rs

```text
// [GPT-6 Astra] Scratch-only baseline driver; upstream packages are unmodified.
use oxrdf::Quad;
use oxttl::NQuadsParser;
use rdf_canon::{CanonicalizationError, CanonicalizationOptions};
use serde_json::{Value, json};
use sha2::{Sha256, Sha384};
use std::{env, fs::File};

fn result_json<T: serde::Serialize>(result: Result<T, CanonicalizationError>) -> Value {
    match result {
        Ok(value) => json!({"value": value}),
        Err(error) => json!({"error": error.to_string(), "error_debug": format!("{error:?}")}),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        return Err("expected input.nq output|map SHA256|SHA384 default|limit".into());
    }
    let quads: Vec<Quad> = NQuadsParser::new()
        .for_reader(File::open(&args[1])?)
        .collect::<Result<_, _>>()?;
    let options = CanonicalizationOptions {
        hndq_call_limit: if args[4] == "default" {
            None
        } else {
            Some(args[4].parse()?)
        },
    };
    let result = match (args[2].as_str(), args[3].as_str()) {
        ("output", "SHA256") => result_json(rdf_canon::canonicalize_quads_with::<Sha256>(
            &quads, &options,
        )),
        ("output", "SHA384") => result_json(rdf_canon::canonicalize_quads_with::<Sha384>(
            &quads, &options,
        )),
        ("map", "SHA256") => result_json(rdf_canon::issue_quads_with::<Sha256>(&quads, &options)),
        ("map", "SHA384") => result_json(rdf_canon::issue_quads_with::<Sha384>(&quads, &options)),
        _ => return Err("unsupported mode/hash".into()),
    };
    println!("{result}");
    Ok(())
}
```

## js-case.cjs

```text
// [GPT-6 Astra] Unmodified pinned synchronous reference; fixed per-case options.
const fs = require('node:fs');
const RDFC10Sync = require('./reference/package/lib/RDFC10Sync');
const NQuads = require('./reference/package/lib/NQuads');
const [file, mode, hash, budget] = process.argv.slice(2);
const map = new Map();
const options = {canonicalIdMap:map, messageDigestAlgorithm:hash, timeout:1000};
if(budget !== 'default') options.maxDeepIterations = Number(budget);
try {
  const value = new RDFC10Sync(options).main(NQuads.parse(fs.readFileSync(file, 'utf8')));
  console.log(JSON.stringify({value:mode==='map'?Object.fromEntries(map):value}));
} catch(e) {console.log(JSON.stringify({error:e.message, error_debug:e.name}));}
```

## ci/counterexample.json

```text
{
  "proptest_seed": "cc 14c0b82fcdfee2504e7e213e7c440c3f891a8ba23ad553051fd08ab226c0cf2a",
  "spec": [
    {
      "subject_bnode": 4,
      "predicate": 1,
      "object_bnode": 2,
      "graph_bnode": 3
    },
    {
      "subject_bnode": 4,
      "predicate": 0,
      "object_bnode": 0,
      "graph": "default"
    },
    {
      "subject_bnode": 0,
      "predicate": 1,
      "object_bnode": 3,
      "graph_bnode": 2
    }
  ],
  "permutation": [
    0,
    1,
    4,
    3,
    5,
    2
  ],
  "original_nquads": "_:b4 <http://ex/q> _:b2 _:b3 .\n_:b4 <http://ex/p> _:b0 .\n_:b0 <http://ex/q> _:b3 _:b2 .\n",
  "renamed_nquads": "_:zz5 <http://ex/q> _:zz4 _:zz3 .\n_:zz5 <http://ex/p> _:zz0 .\n_:zz0 <http://ex/q> _:zz3 _:zz4 .\n",
  "logged_left": "_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n",
  "logged_right": "_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n",
  "evidence": "Spec, permutation, seed and output strings copied from exact failed job. The two N-Quads inputs are source-derived transcriptions of materialize/base_label, not a new executed Rust reproduction."
}
```

## ci/seed.txt

```text
cc 14c0b82fcdfee2504e7e213e7c440c3f891a8ba23ad553051fd08ab226c0cf2a
```

## crates/sparq-canon/src/lib.rs:259–278

```text
259: pub struct CanonicalGraph {
260:     /// Canonical N-Quads lines in canonical (code point) order, one triple
261:     /// each, no trailing newline. `lines[i]` is the canonical serialization of
262:     /// `triples[i]`.
263:     pub lines: Vec<String>,
264:     /// The canonical triples (blank-node labels are `c14nN`), same order as
265:     /// [`Self::lines`].
266:     pub triples: Vec<Triple>,
267: }
268: 
269: impl CanonicalGraph {
270:     /// The canonical N-Quads document (joined lines, each terminated by `\n`).
271:     pub fn to_nquads(&self) -> String {
272:         let mut s = String::new();
273:         for l in &self.lines {
274:             s.push_str(l);
275:             s.push('\n');
276:         }
277:         s
278:     }
```

## crates/sparq-canon/src/lib.rs:304–413

```text
304: pub fn canonicalize(dataset: &[Quad]) -> Result<String, CanonError> {
305:     canonicalize_quads(dataset)
306: }
307: 
308: /// Alias of [`canonicalize`] for callers that prefer the explicit `_quads`
309: /// name (mirrors [`rdf_canon::canonicalize_quads`]).
310: pub fn canonicalize_quads(dataset: &[Quad]) -> Result<String, CanonError> {
311:     let quads02 = bridge_to_02(dataset)?;
312:     rdf_canon::canonicalize_quads(&quads02).map_err(|e| CanonError::Canonicalization(e.to_string()))
313: }
314: 
315: /// Like [`canonicalize_quads`] but parameterized over the RDFC-1.0 hash
316: /// function `D` (the spec default is SHA-256; e.g. `sha2::Sha384` selects the
317: /// SHA-384 profile). Uses the default HNDQ call limit.
318: pub fn canonicalize_quads_with<D: Digest>(dataset: &[Quad]) -> Result<String, CanonError> {
319:     let quads02 = bridge_to_02(dataset)?;
320:     let opts = rdf_canon::CanonicalizationOptions::default();
321:     rdf_canon::canonicalize_quads_with::<D>(&quads02, &opts)
322:         .map_err(|e| CanonError::Canonicalization(e.to_string()))
323: }
324: 
325: /// Returns the digest bytes of the exact canonical N-Quads document produced
326: /// by [`canonicalize_quads`].
327: ///
328: /// `D` selects only the final digest algorithm; canonicalization retains the
329: /// RDFC-1.0 default hash profile. Every canonical byte is hashed, including the
330: /// final trailing newline when the dataset is non-empty. [GPT-5.6] sq-ddws7.
331: pub fn digest_quads_with<D: Digest>(dataset: &[Quad]) -> Result<Vec<u8>, CanonError> {
332:     let c = canonicalize_quads(dataset)?;
333:     let mut h = D::new();
334:     h.update(c.as_bytes());
335:     Ok(h.finalize().to_vec())
336: }
337: 
338: /// Like [`issue_quads`] but parameterized over the RDFC-1.0 hash function `D`.
339: pub fn issue_quads_with<D: Digest>(
340:     dataset: &[Quad],
341: ) -> Result<HashMap<String, String>, CanonError> {
342:     let quads02 = bridge_to_02(dataset)?;
343:     let opts = rdf_canon::CanonicalizationOptions::default();
344:     let map = rdf_canon::issue_quads_with::<D>(&quads02, &opts)
345:         .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
346:     Ok(map.into_iter().collect())
347: }
348: 
349: /// Returns the RDFC-1.0 **issued-identifier map** for a dataset: input
350: /// blank-node label → canonical `c14nN` label. Cheap relative to a full
351: /// canonicalization-and-reparse when only the relabelling is needed.
352: pub fn issued_identifiers(dataset: &[Quad]) -> Result<HashMap<String, String>, CanonError> {
353:     issue_quads(dataset)
354: }
355: 
356: /// Alias of [`issued_identifiers`] (mirrors [`rdf_canon::issue_quads`]).
357: pub fn issue_quads(dataset: &[Quad]) -> Result<HashMap<String, String>, CanonError> {
358:     let quads02 = bridge_to_02(dataset)?;
359:     let map = rdf_canon::issue_quads(&quads02)
360:         .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
361:     Ok(map.into_iter().collect())
362: }
363: 
364: // ---------------------------------------------------------------------------
365: // Single-graph API (a default-graph-only dataset). What the ZK per-graph
366: // commitment pipeline consumes; kept here so the bridge is single-sourced.
367: // ---------------------------------------------------------------------------
368: 
369: /// Canonicalizes a slice of triples (one graph's content, treated as the
370: /// default graph of a single-graph dataset) into a [`CanonicalGraph`].
371: pub fn canonicalize_triples(triples: &[Triple]) -> Result<CanonicalGraph, CanonError> {
372:     for t in triples {
373:         if contains_triple_term(t) {
374:             return Err(CanonError::TripleTerm);
375:         }
376:     }
377:     let quads02 = bridge_triples_to_02(triples)?;
378:     let canonical = rdf_canon::canonicalize_quads(&quads02)
379:         .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
380:     parse_canonical(&canonical)
381: }
382: 
383: /// Canonicalizes the content of a [`sparq_core::Graph`] into a
384: /// [`CanonicalGraph`].
385: pub fn canonicalize_graph_content(g: &Graph) -> Result<CanonicalGraph, CanonError> {
386:     let triples = graph_triples(g)?;
387:     canonicalize_triples(&triples)
388: }
389: 
390: /// The RDFC-1.0 issued-identifier map for a single graph's content (input
391: /// blank-node label → canonical `c14nN` label).
392: pub fn issue_triples(triples: &[Triple]) -> Result<HashMap<String, String>, CanonError> {
393:     for t in triples {
394:         if contains_triple_term(t) {
395:             return Err(CanonError::TripleTerm);
396:         }
397:     }
398:     let quads02 = bridge_triples_to_02(triples)?;
399:     let map = rdf_canon::issue_quads(&quads02)
400:         .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
401:     Ok(map.into_iter().collect())
402: }
403: 
404: /// Materializes a store graph's triples as oxrdf [`Triple`]s.
405: pub fn graph_triples(g: &Graph) -> Result<Vec<Triple>, CanonError> {
406:     let mut out = Vec::with_capacity(g.len());
407:     for t in g.iter_ids() {
408:         let s = g.dict.term(t[0]);
409:         let p = g.dict.term(t[1]);
410:         let o = g.dict.term(t[2]);
411:         out.push(terms_to_triple(s, p, o)?);
412:     }
413:     Ok(out)
```

## crates/sparq-canon/src/lib.rs:424–455

```text
424: fn bridge_to_02(dataset: &[Quad]) -> Result<Vec<oxrdf02::Quad>, CanonError> {
425:     let doc = serialize_quads(dataset)?;
426:     parse_02(&doc)
427: }
428: 
429: #[cfg(not(feature = "bridge-lowcopy"))]
430: fn serialize_quads(dataset: &[Quad]) -> Result<String, CanonError> {
431:     serialize_quads_default(dataset)
432: }
433: 
434: #[cfg(any(not(feature = "bridge-lowcopy"), test))]
435: fn serialize_quads_default(dataset: &[Quad]) -> Result<String, CanonError> {
436:     let mut doc = String::new();
437:     for q in dataset {
438:         if matches!(q.object, oxrdf::Term::Triple(_)) {
439:             return Err(CanonError::TripleTerm);
440:         }
441:         match &q.graph_name {
442:             GraphName::DefaultGraph => {
443:                 doc.push_str(&format!("{} {} {} .\n", q.subject, q.predicate, q.object));
444:             }
445:             g => {
446:                 doc.push_str(&format!(
447:                     "{} {} {} {} .\n",
448:                     q.subject, q.predicate, q.object, g
449:                 ));
450:             }
451:         }
452:     }
453:     Ok(doc)
454: }
455:
```

## crates/sparq-canon/src/lib.rs:495–510

```text
495: fn parse_02(doc: &str) -> Result<Vec<oxrdf02::Quad>, CanonError> {
496:     let mut quads02 = Vec::new();
497:     for item in oxttl01::NQuadsParser::new().for_reader(doc.as_bytes()) {
498:         quads02.push(item.map_err(|e| CanonError::Bridge(e.to_string()))?);
499:     }
500:     Ok(quads02)
501: }
502: 
503: fn terms_to_triple(s: oxrdf::Term, p: oxrdf::Term, o: oxrdf::Term) -> Result<Triple, CanonError> {
504:     use oxrdf::Term;
505:     let subject = match s {
506:         Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
507:         Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
508:         other => return Err(CanonError::Bridge(format!("invalid subject term: {other}"))),
509:     };
510:     let predicate = match p {
```

## crates/sparq-canon/tests/proptest_canon_determinism.rs:140–169

```text
140: fn materialize(spec: &[QuadSpec], label: &dyn Fn(usize) -> String) -> Vec<Quad> {
141:     spec.iter()
142:         .map(|q| {
143:             let subject = match q.subject {
144:                 NodeSpec::Bnode(i) => {
145:                     NamedOrBlankNode::BlankNode(BlankNode::new_unchecked(label(i)))
146:                 }
147:                 NodeSpec::Iri(i) => NamedOrBlankNode::NamedNode(iri(i)),
148:             };
149:             let object = match q.object {
150:                 ObjSpec::Bnode(i) => Term::BlankNode(BlankNode::new_unchecked(label(i))),
151:                 ObjSpec::Iri(i) => Term::NamedNode(iri(i)),
152:                 ObjSpec::Lit(i) => literal(i),
153:             };
154:             let graph_name = match q.graph {
155:                 GraphSpec::Default => GraphName::DefaultGraph,
156:                 GraphSpec::Iri(i) => GraphName::NamedNode(iri(i)),
157:                 GraphSpec::Bnode(i) => {
158:                     GraphName::BlankNode(BlankNode::new_unchecked(label(i)))
159:                 }
160:             };
161:             Quad::new(subject, predicate(q.predicate), object, graph_name)
162:         })
163:         .collect()
164: }
165: 
166: fn base_label(i: usize) -> String {
167:     format!("b{}", i)
168: }
169:
```

## crates/sparq-canon/tests/proptest_canon_determinism.rs:269–285

```text
269: fn arb_dataset() -> impl Strategy<Value = Vec<QuadSpec>> {
270:     prop_oneof![
271:         3 => arb_random_quads(),
272:         2 => arb_cycle(),
273:         1 => arb_twin_cycles(),
274:         2 => (arb_random_quads(), arb_cycle()).prop_map(|(mut a, c)| {
275:             a.extend(c);
276:             a
277:         }),
278:     ]
279: }
280: 
281: /// A random permutation of the blank-node pool indices (an injective relabeling
282: /// when composed with a fresh prefix).
283: fn arb_pool_permutation() -> impl Strategy<Value = Vec<usize>> {
284:     Just((0..MAX_BNODES).collect::<Vec<usize>>()).prop_shuffle()
285: }
```

## crates/sparq-canon/tests/proptest_canon_determinism.rs:361–401

```text
361: proptest! {
362:     #![proptest_config(ProptestConfig {
363:         cases: 256,
364:         ..Default::default()
365:     })]
366: 
367:     /// (a) Canonical output is byte-identical under blank-node relabeling: the
368:     /// same structural spec materialized under `b{i}` and under `zz{perm(i)}`
369:     /// (an injective renaming) canonicalizes identically.
370:     #[test]
371:     fn canonical_output_invariant_under_bnode_relabeling(
372:         spec in arb_dataset(),
373:         perm in arb_pool_permutation(),
374:     ) {
375:         let d1 = materialize(&spec, &base_label);
376:         let d2 = materialize(&spec, &|i| format!("zz{}", perm[i]));
377:         let c1 = canonicalize_quads(&d1).expect("canonicalize d1");
378:         let c2 = canonicalize_quads(&d2).expect("canonicalize d2");
379:         prop_assert_eq!(
380:             &c1, &c2,
381:             "canonical output must not depend on input blank-node labels\nspec: {:?}",
382:             spec
383:         );
384:     }
385: 
386:     /// (b) Canonical output is byte-identical under a random permutation of the
387:     /// input quad order.
388:     #[test]
389:     fn canonical_output_invariant_under_quad_permutation(
390:         (original, shuffled) in arb_dataset().prop_flat_map(|spec| {
391:             let d = materialize(&spec, &base_label);
392:             (Just(d.clone()), Just(d).prop_shuffle())
393:         })
394:     ) {
395:         let c1 = canonicalize_quads(&original).expect("canonicalize original");
396:         let c2 = canonicalize_quads(&shuffled).expect("canonicalize shuffled");
397:         prop_assert_eq!(
398:             &c1, &c2,
399:             "canonical output must not depend on input quad order\noriginal: {:?}\nshuffled: {:?}",
400:             original, shuffled
401:         );
```

## crates/sparq-canon/src/concept.rs:271–280

```text
271: 
272:     /// Digests a record's canonical N-Quads document under this algorithm.
273:     fn digest_record(self, record: &[Quad]) -> Result<Vec<u8>, CanonError> {
274:         match self {
275:             ConceptHash::Sha256 => digest_quads_with::<sha2::Sha256>(record),
276:             ConceptHash::Sha512 => digest_quads_with::<sha2::Sha512>(record),
277:             ConceptHash::Sha384 => digest_quads_with::<sha2::Sha384>(record),
278:         }
279:     }
280: }
```

## crates/sparq-wasm/src/canon.rs:21–33

```text
21: /// `_:c14nN`, each line `\n`-terminated). Two N-Quads documents that denote
22: /// RDF-isomorphic datasets — i.e. differ only in blank-node labels and/or quad
23: /// order — produce byte-identical output, so a caller can hash / compare the
24: /// result for an isomorphism-aware dataset `equals` / `contains` / content hash.
25: ///
26: /// `input` is parsed as N-Quads (the default graph is a 3-term line; named
27: /// graphs carry their graph term). A malformed document, or one containing an
28: /// RDF-1.2 triple term (outside the W3C RDFC-1.0 data model), returns the `Err`
29: /// (`JsError`) arm rather than trapping.
30: #[wasm_bindgen(js_name = canonicalizeNQuads)]
31: pub fn canonicalize_nquads(input: &str) -> Result<String, JsError> {
32:     sparq_canon::canonicalize_nquads(input).map_err(|e| JsError::new(&e.to_string()))
33: }
```

## js/src/dataset.ts:266–284

```text
266:    * Whether this dataset denotes the SAME RDF dataset as `quads` — mutual containment, with
267:    * **blank nodes normalized** (RDF-dataset isomorphism). INTEROP: `quads` may be a sparq
268:    * {@link Dataset} or any foreign RDF/JS dataset / `Quad[]`.
269:    *
270:    * [OPUS-4.8] sq-1dd5t (#1047): equality is now decided by RDFC-1.0 — two datasets are equal
271:    * iff their canonical N-Quads are byte-identical — so two isomorphic datasets that differ only
272:    * in blank-node labels (and/or quad order) compare equal. When neither side carries blank
273:    * nodes this short-circuits to the fast exact-label set comparison.
274:    */
275:   equals(quads: QuadSource): boolean {
276:     const otherQuads = toQuadArray(quads);
277:     if (!anyBlankNode(otherQuads) && !this.#hasBlankNode()) {
278:       const other = new QuadSet(otherQuads);
279:       if (other.size !== this.size) return false;
280:       for (const q of this) if (!other.has(q)) return false;
281:       return true;
282:     }
283:     return this.toCanonical() === canonicalizeNQuads(quadsToNQuads(otherQuads));
284:   }
```

## js/src/dataset.ts:386–399

```text
386:    * The dataset's **RDFC-1.0** (RDF Dataset Canonicalization / URDNA2015 successor) canonical
387:    * N-Quads — the form the RDF/JS spec defines `toCanonical` against. Blank-node labels are
388:    * **relabelled to a canonical form** (`_:c14nN`) and the quad lines are canonically sorted, so
389:    * two datasets that are RDF-isomorphic (differ only in blank-node labels and/or quad order)
390:    * produce byte-identical output. This is the basis for dataset hashing, equality and diffing.
391:    *
392:    * [OPUS-4.8] sq-1dd5t (#1047): computed by the engine's RDFC-1.0 implementation
393:    * (`sparq-canon` → the W3C-suite-validated `rdf-canon`) surfaced through the wasm
394:    * `canonicalizeNQuads` binding — no longer the label-sensitive sorted-N-Quads approximation.
395:    * RDF-1.2 triple terms are outside the W3C RDFC-1.0 data model and throw.
396:    */
397:   toCanonical(): string {
398:     return canonicalizeNQuads(quadsToNQuads(this.toArray()));
399:   }
```

## crates/sparq-vc/src/suite.rs:305–321

```text
305: // ---------------------------------------------------------------------------
306: 
307: /// `hashData = SHA-256(canon(proofConfig)) ‖ SHA-256(canon(document))` — the
308: /// 64-byte input the Ed25519 signature covers (vc-di-eddsa §3, proof config first).
309: fn hash_data(triples: &[Triple], config: &ProofConfig) -> Result<Vec<u8>, VcError> {
310:     let doc_canon = sparq_canon::canonicalize_triples(triples)?;
311:     let cfg_triples = proof_config_triples(config);
312:     let cfg_canon = sparq_canon::canonicalize_triples(&cfg_triples)?;
313: 
314:     let doc_hash = Sha256::digest(doc_canon.to_nquads().as_bytes());
315:     let cfg_hash = Sha256::digest(cfg_canon.to_nquads().as_bytes());
316: 
317:     let mut out = Vec::with_capacity(64);
318:     out.extend_from_slice(&cfg_hash); // proof config hash first
319:     out.extend_from_slice(&doc_hash);
320:     Ok(out)
321: }
```

## crates/sparq-zk/src/commit.rs:238–258

```text
238: /// Canonicalizes and commits one named graph's content under `salt`.
239: pub fn commit_triples(triples: &[Triple], salt: Fr) -> Result<GraphCommitment, CommitError> {
240:     let canonical = canon::canonicalize_triples(triples)?;
241:     commit_canonical(canonical, salt)
242: }
243: 
244: /// Commits the content of a `sparq_core::Graph` under `salt`.
245: pub fn commit_graph_content(g: &sparq_core::Graph, salt: Fr) -> Result<GraphCommitment, CommitError> {
246:     let canonical = canon::canonicalize_graph_content(g)?;
247:     commit_canonical(canonical, salt)
248: }
249: 
250: fn commit_canonical(canonical: CanonicalGraph, salt: Fr) -> Result<GraphCommitment, CommitError> {
251:     let mut leaves = Vec::with_capacity(canonical.triples.len());
252:     for t in &canonical.triples {
253:         let leaf = encode::encode_triple(t, &salt)
254:             .ok_or_else(|| CommitError::UncommittableTerm(t.to_string()))?;
255:         leaves.push(leaf);
256:     }
257:     let commitment = poseidon2::hash(&leaves);
258:     Ok(GraphCommitment { canonical, leaves, commitment, salt })
```

## crates/sparq-zk/src/trace.rs:82–108

```text
82:             .map_err(|e| TraceError::Commit(name.as_str().to_string(), e))?;
83:         let label_map = issue_label_map(&triples)
84:             .map_err(|e| TraceError::Commit(name.as_str().to_string(), CommitError::Canon(e)))?;
85:         let leaf_of = commitment
86:             .canonical
87:             .triples
88:             .iter()
89:             .enumerate()
90:             .map(|(i, t)| (t.clone(), i))
91:             .collect();
92:         Ok(CommittedNamedGraph { name, triples, commitment, label_map, leaf_of })
93:     }
94: 
95:     /// Leaf index of a store-form triple (input labels), via the issued
96:     /// identifier map. `None` when the triple is not in this graph.
97:     pub fn leaf_index(&self, t: &Triple) -> Option<usize> {
98:         let canonical = relabel_triple(t, &self.label_map)?;
99:         self.leaf_of.get(&canonical).copied()
100:     }
101: }
102: 
103: /// RDFC-1.0 issued-identifier map for a graph's content (input label →
104: /// canonical `c14nN` label). Single-sourced via the [`sparq_canon`] public API
105: /// (same bridge as [`crate::canon`]). [OPUS-4.8] sq-0qip.
106: fn issue_label_map(triples: &[Triple]) -> Result<HashMap<String, String>, crate::canon::CanonError> {
107:     sparq_canon::issue_triples(triples)
108: }
```

## crates/sparq-zk/src/vc_bridge.rs:318–341

```text
318: pub(crate) fn canonical_nquads(
319:     credential: &[Triple],
320:     proof_config: &[Triple],
321: ) -> Result<(String, String), VcBridgeError> {
322:     let cred_canon = crate::canon::canonicalize_triples(credential)
323:         .map_err(|e| VcBridgeError::Commit(CommitError::Canon(e)))?;
324:     let proof_canon = crate::canon::canonicalize_triples(proof_config)
325:         .map_err(|e| VcBridgeError::Commit(CommitError::Canon(e)))?;
326:     Ok((proof_canon.to_nquads(), cred_canon.to_nquads()))
327: }
328: 
329: /// Build the signed **hashData** from the credential + proof-config **triples**:
330: /// RDFC10-canonicalise each (the DI suites canonicalise before hashing), then
331: /// `proofConfigHash || documentHash`. This is the **SHA-256** derivation, shared by
332: /// `eddsa-rdfc-2022` and the `ecdsa-rdfc-2019` **P-256** profile — the suites
333: /// differ only in the SIGNATURE check over this 64-byte hashData.
334: ///
335: /// The `ecdsa-rdfc-2019` **P-384** profile hashes with SHA-384 instead — see
336: /// [`hash_data_from_triples_sha384`]. Feeding a P-384 proof the 64-byte hashData
337: /// from here cannot verify (the issuer signed 96 different bytes), which is why
338: /// [`verify_source_proof`] resolves the curve profile before choosing.
339: pub fn hash_data_from_triples(
340:     credential: &[Triple],
341:     proof_config: &[Triple],
```

## source/upstream-counter.rs

```text
use crate::CanonicalizationError;
use std::{collections::HashMap, fmt};

const DEFAULT_HNDQ_CALL_LIMIT: usize = 4000;

pub trait HndqCallCounter {
    fn new(max_calls: Option<usize>) -> Self;
    fn add(&mut self, identifier: &str) -> Result<(), CanonicalizationError>;
    fn sum(&self) -> usize;
}

pub struct SimpleHndqCallCounter {
    counter: usize,
    limit: usize,
}

impl Default for SimpleHndqCallCounter {
    fn default() -> Self {
        Self {
            counter: Default::default(),
            limit: DEFAULT_HNDQ_CALL_LIMIT,
        }
    }
}

impl HndqCallCounter for SimpleHndqCallCounter {
    fn new(max_calls: Option<usize>) -> Self {
        let limit = match max_calls {
            Some(limit) => limit,
            None => DEFAULT_HNDQ_CALL_LIMIT,
        };
        Self { counter: 0, limit }
    }

    fn add(&mut self, _identifier: &str) -> Result<(), CanonicalizationError> {
        self.counter += 1;
        if self.counter > self.limit {
            Err(CanonicalizationError::HndqCallLimitExceeded(self.limit))
        } else {
            Ok(())
        }
    }

    fn sum(&self) -> usize {
        self.counter
    }
}

impl fmt::Debug for SimpleHndqCallCounter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("")
            .field("counter", &self.counter)
            .field("limit", &self.limit)
            .finish()
    }
}

pub struct PerNodeHndqCallCounter {
    counter: HashMap<String, usize>,
    limit: usize,
}

impl Default for PerNodeHndqCallCounter {
    fn default() -> Self {
        Self {
            counter: Default::default(),
            limit: DEFAULT_HNDQ_CALL_LIMIT,
        }
    }
}

impl HndqCallCounter for PerNodeHndqCallCounter {
    fn new(max_calls: Option<usize>) -> Self {
        let limit = match max_calls {
            Some(limit) => limit,
            None => DEFAULT_HNDQ_CALL_LIMIT,
        };
        Self {
            counter: Default::default(),
            limit,
        }
    }

    fn add(&mut self, identifier: &str) -> Result<(), CanonicalizationError> {
        let current = self
            .counter
            .entry(identifier.to_string())
            .and_modify(|c| *c += 1)
            .or_insert(1);
        if current > &mut self.limit {
            Err(CanonicalizationError::HndqCallLimitExceeded(self.limit))
        } else {
            Ok(())
        }
    }

    fn sum(&self) -> usize {
        self.counter
            .values()
            .copied()
            .reduce(|acc, v| acc + v)
            .unwrap_or(0)
    }
}

impl fmt::Debug for PerNodeHndqCallCounter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("")
            .field("counter", &self.counter)
            .field("limit", &self.limit)
            .field("sum", &self.sum())
            .finish()
    }
}
```

## source/upstream-error.rs

```text
use oxrdf::BlankNodeIdParseError;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum CanonicalizationError {
    #[error("Base16 encoding failed.")]
    Base16EncodingFailed(base16ct::Error),
    #[error("Reference blank node identifier does not exist in the canonicalization state.")]
    QuadsNotExist,
    #[error("Canonical identifier does not exist for the given blank node.")]
    CanonicalIdentifierNotExist,
    #[error("Parsing blank node identifier failed.")]
    BlankNodeIdParseError,
    #[error(
        "The number of calls to the Hash N-degree Quads algorithm have exceeded the limit of {0}."
    )]
    HndqCallLimitExceeded(usize),
}

impl From<BlankNodeIdParseError> for CanonicalizationError {
    fn from(_: BlankNodeIdParseError) -> Self {
        Self::BlankNodeIdParseError
    }
}
```

## source/upstream-api.rs:132–140

```text
132: pub fn canonicalize_quads(input_quads: &[Quad]) -> Result<String, CanonicalizationError> {
133:     let options = CanonicalizationOptions::default();
134:     canonicalize_quads_with::<Sha256>(input_quads, &options)
135: }
136: 
137: #[derive(Default)]
138: pub struct CanonicalizationOptions {
139:     pub hndq_call_limit: Option<usize>,
140: }
```

## source/upstream-api.rs:283–292

```text
283: pub fn canonicalize_quads_with<D: Digest>(
284:     input_quads: &[Quad],
285:     options: &CanonicalizationOptions,
286: ) -> Result<String, CanonicalizationError> {
287:     let input_dataset = Dataset::from_iter(input_quads);
288:     let issued_identifiers_map = issue_with::<D>(&input_dataset, options)?;
289:     let relabeled_dataset = relabel(&input_dataset, &issued_identifiers_map)?;
290:     Ok(serialize(&relabeled_dataset))
291: }
292:
```

## source/upstream-canon.rs:112–178

```text
112:     /// **issued identifiers map**
113:     ///   An ordered map that relates existing identifiers to issued
114:     ///   identifiers, to prevent issuance of more than one new identifier
115:     ///   per existing identifier, and to allow blank nodes to be
116:     ///   reassigned identifiers some time after issuance.
117:     issued_identifiers_map: HashMap<String, String>,
118: }
119: 
120: impl IdentifierIssuer {
121:     fn new(identifier_prefix: &str) -> IdentifierIssuer {
122:         let issued_identifiers_map = HashMap::<String, String>::new();
123:         IdentifierIssuer {
124:             identifier_prefix: identifier_prefix.to_string(),
125:             identifier_counter: 0,
126:             issued_identifiers_map,
127:         }
128:     }
129: 
130:     fn increment(&mut self) {
131:         self.identifier_counter += 1
132:     }
133: 
134:     fn get(&self, existing_identifier: &str) -> Option<String> {
135:         self.issued_identifiers_map
136:             .get(existing_identifier)
137:             .cloned()
138:     }
139: 
140:     /// **4.5 Issue Identifier Algorithm**
141:     ///   This algorithm issues a new blank node identifier for a given existing
142:     ///   blank node identifier. It also updates state information that tracks
143:     ///   the order in which new blank node identifiers were issued. The order
144:     ///   of issuance is important for canonically labeling blank nodes that are
145:     ///   isomorphic to others in the dataset.
146:     /// **4.5.2 Algorithm**
147:     ///   The algorithm takes an identifier issuer I and an existing identifier as
148:     ///   inputs. The output is a new issued identifier.
149:     fn issue(&mut self, existing_identifier: &str) -> String {
150:         // 1) If there is a map entry for existing identifier in issued identifiers
151:         // map of I, return it.
152:         if let Some(issued_identifier) = self.get(existing_identifier) {
153:             return issued_identifier;
154:         }
155: 
156:         // 2) Generate issued identifier by concatenating identifier prefix with
157:         // the string value of identifier counter.
158:         let issued_identifier = format!("{}{}", self.identifier_prefix, self.identifier_counter);
159: 
160:         // 3) Add an entry mapping existing identifier to issued identifier to
161:         // the issued identifiers map of I.
162:         self.issued_identifiers_map
163:             .insert(existing_identifier.to_string(), issued_identifier.clone());
164: 
165:         // 4) Increment identifier counter.
166:         self.increment();
167: 
168:         // 5) Return issued identifier.
169:         issued_identifier
170:     }
171: 
172:     #[cfg(feature = "log")]
173:     fn serialize_issued_identifiers_map(&self) -> String {
174:         format!(
175:             "{{{}}}",
176:             self.issued_identifiers_map
177:                 .iter()
178:                 .map(|(k, v)| format!("{}: {}", k, v))
```

## source/upstream-canon.rs:431–446

```text
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
```

## Exact unmodified archive proof and locked package versions

```json
{
  "archives": [
    {
      "package": "rdf-canon-0.15.3",
      "crate_sha256": "6783be73f4fa73010f249412ad8f0bb1ac9cdb22b2310bf20a967d80d5906dd0",
      "source_files_verified": 311,
      "all_match": true
    },
    {
      "package": "oxrdf-0.2.4",
      "crate_sha256": "a04761319ef84de1f59782f189d072cbfc3a9a40c4e8bded8667202fbd35b02a",
      "source_files_verified": 16,
      "all_match": true
    },
    {
      "package": "oxttl-0.1.8",
      "crate_sha256": "0d385f1776d7cace455ef6b7c54407838eff902ca897303d06eb12a26f4cf8a0",
      "source_files_verified": 19,
      "all_match": true
    }
  ],
  "packages": [
    {
      "name": "digest",
      "version": "0.10.7",
      "source": "registry+https://github.com/rust-lang/crates.io-index",
      "features": [
        "alloc",
        "block-buffer",
        "core-api",
        "default",
        "std"
      ]
    },
    {
      "name": "oxrdf",
      "version": "0.2.4",
      "source": "registry+https://github.com/rust-lang/crates.io-index",
      "features": [
        "default"
      ]
    },
    {
      "name": "oxttl",
      "version": "0.1.8",
      "source": "registry+https://github.com/rust-lang/crates.io-index",
      "features": [
        "default"
      ]
    },
    {
      "name": "rdf-canon",
      "version": "0.15.3",
      "source": "registry+https://github.com/rust-lang/crates.io-index",
      "features": []
    },
    {
      "name": "sha2",
      "version": "0.10.9",
      "source": "registry+https://github.com/rust-lang/crates.io-index",
      "features": [
        "default",
        "std"
      ]
    }
  ]
}
```

## Retained context and limits

Complete raw expected/output rows, full source files, Cargo.lock, fixture corpus, executable hashes, native toolchain, commands, host resource records and prior-evidence verification are retained locally. This portable packet omits host paths and large full-suite output bodies. The independent review should evaluate the concrete repair boundary after this evidence; no model review is claimed for any proposed algorithm.
