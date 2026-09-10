# Issue5183 pinned reference witness

Authored by GPT-6 Astra xhigh. Public synthetic data only; no production patch.

## Observations and limits

{
  "conclusion": "Oxigraph0.5.9 collapses the two same-value integer lexical terms before WHERE evaluation, changing solution cardinality and fresh template blank-node count. Sparq parent/main public witnesses retain both RDF terms. This establishes a reference-cardinality explanation for the reduction independently of canonicalization.",
  "scope": {
    "operations": 2,
    "cases": 3,
    "reference_runs": 1,
    "retries": 0,
    "reference_api": [
      "Store::new",
      "Store::update",
      "Store::query",
      "Store::iter"
    ],
    "reference_features": [
      "rdf-12"
    ],
    "disabled": [
      "default/rocksdb",
      "http-client"
    ],
    "resolved_packages": 62,
    "production_lock_dependency_drift": [],
    "patched_parser": "vendored spargebra0.4.6 at repository head781f667c",
    "canonicalizer_used": false,
    "controls": "8-only yields1/1/1; 8+9 yields2/2/2, matching both earlier Sparq revisions and both update paths."
  },
  "normative_interpretation": "RDF literal term identity includes lexical form; same integer value does not make these two RDF terms identical. INSERT blank nodes are fresh per WHERE solution. Sparq preserves two terms/solutions; the reference stores a smaller graph. This does not yet establish the cause of all eight CI mismatches.",
  "limits": [
    "NOT the full ten-operation seed4141222487 CI replay; seven other seeds unexecuted.",
    "Earlier Sparq witnesses used qualified cached parent/main-equivalent libraries with default engine/core features, not CI feature parity; exact attribution copied unchanged.",
    "Rust1.97.1/aarch64 macOS/O3/unwind/no-LTO/codegen16 is not Linux CI release-fast thin-LTO/abort.",
    "Blank-node labels are incidental; compare counts within results, not identifiers across executions.",
    "No comparator, canonicalizer, generator, production input, engine source, dependency version or gate changed.",
    "Dependency warnings retained; not a full crate/workspace lint/conformance run.",
    "No unsupported whole-cache source authentication claim; actual selected source, manifest and artifact hashes retained."
  ],
  "next_small_step": "Separately authorize unchanged update_fuzz exact seed4141222487 on qualified parent/main sources with CI-relevant features, strict rebuild/in-place check, generator/LOAD sandbox, allowlist and comparator preserved. Capture failure step and raw pre-step graphs. Do not normalize production inputs or weaken comparisons based only on this reduction."
}

## Comparison

[
  {
    "case": "lexical-pair",
    "oxigraph": {
      "before_quads": 1,
      "where_rows": 1,
      "distinct_blank_nodes": 1,
      "after_quads": 2,
      "lexical_terms": [
        "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
      ]
    },
    "sparq": {
      "parent/rebuild": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "parent/in-place": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "main/rebuild": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "main/in-place": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      }
    }
  },
  {
    "case": "canonical-eight",
    "oxigraph": {
      "before_quads": 1,
      "where_rows": 1,
      "distinct_blank_nodes": 1,
      "after_quads": 2,
      "lexical_terms": [
        "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
      ]
    },
    "sparq": {
      "parent/rebuild": {
        "before_quads": 1,
        "where_rows": 1,
        "distinct_blank_nodes": 1,
        "after_quads": 2,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "parent/in-place": {
        "before_quads": 1,
        "where_rows": 1,
        "distinct_blank_nodes": 1,
        "after_quads": 2,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "main/rebuild": {
        "before_quads": 1,
        "where_rows": 1,
        "distinct_blank_nodes": 1,
        "after_quads": 2,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "main/in-place": {
        "before_quads": 1,
        "where_rows": 1,
        "distinct_blank_nodes": 1,
        "after_quads": 2,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      }
    }
  },
  {
    "case": "different-values",
    "oxigraph": {
      "before_quads": 2,
      "where_rows": 2,
      "distinct_blank_nodes": 2,
      "after_quads": 4,
      "lexical_terms": [
        "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
        "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
      ]
    },
    "sparq": {
      "parent/rebuild": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "parent/in-place": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "main/rebuild": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      },
      "main/in-place": {
        "before_quads": 2,
        "where_rows": 2,
        "distinct_blank_nodes": 2,
        "after_quads": 4,
        "lexical_terms": [
          "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>",
          "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ]
      }
    }
  }
]

## Exact raw reference records

{"case":"lexical-pair","terms":["\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>", "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>"],"first":"INSERT DATA { <http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> . <http://ex/s2> <http://ex/p1> \"008\"^^<http://www.w3.org/2001/XMLSchema#integer> . }","second":"INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }","before":["<http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"where_rows":[["<http://ex/s2>", "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"]],"after":["<http://ex/s2> <http://ex/p0> _:da7fb4115dde12fea36166f498af3d0 .", "<http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"blank_rows":[["_:da7fb4115dde12fea36166f498af3d0"]],"distinct_blank_nodes":1}
{"case":"canonical-eight","terms":["\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"],"first":"INSERT DATA { <http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> . }","second":"INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }","before":["<http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"where_rows":[["<http://ex/s2>", "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"]],"after":["<http://ex/s2> <http://ex/p0> _:d49f312c3112a1b2c34e957a85350f1a .", "<http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"blank_rows":[["_:d49f312c3112a1b2c34e957a85350f1a"]],"distinct_blank_nodes":1}
{"case":"different-values","terms":["\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>", "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"],"first":"INSERT DATA { <http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> . <http://ex/s2> <http://ex/p1> \"9\"^^<http://www.w3.org/2001/XMLSchema#integer> . }","second":"INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }","before":["<http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> .", "<http://ex/s2> <http://ex/p1> \"9\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"where_rows":[["<http://ex/s2>", "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>"], ["<http://ex/s2>", "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>"]],"after":["<http://ex/s2> <http://ex/p0> _:b53334a95a5d71689e4cf79df3ef129d .", "<http://ex/s2> <http://ex/p0> _:f6d8bf3d6fac0ca8d708ddcf69972e5a .", "<http://ex/s2> <http://ex/p1> \"8\"^^<http://www.w3.org/2001/XMLSchema#integer> .", "<http://ex/s2> <http://ex/p1> \"9\"^^<http://www.w3.org/2001/XMLSchema#integer> ."],"blank_rows":[["_:b53334a95a5d71689e4cf79df3ef129d"], ["_:f6d8bf3d6fac0ca8d708ddcf69972e5a"]],"distinct_blank_nodes":2}


## Actual harness

```rust

// [GPT-6 ASTRA] Fixed synthetic UPDATE comparison through the public reference API.
use oxigraph::{sparql::QueryResults, store::Store};
use std::{collections::BTreeSet, error::Error};

type TestResult<T> = Result<T, Box<dyn Error>>;

fn quads(store: &Store) -> TestResult<Vec<String>> {
    let mut result = Vec::new();
    for quad in store.iter() {
        let quad = quad?;
        result.push(format!("{} {} {} .", quad.subject, quad.predicate, quad.object));
    }
    result.sort();
    Ok(result)
}

#[expect(deprecated, reason = "Match the existing UPDATE differential query API")]
fn rows(store: &Store, query: &str, variables: &[&str]) -> TestResult<Vec<Vec<String>>> {
    let QueryResults::Solutions(solutions) = store.query(query)? else {
        return Err("Expected SELECT solutions".into());
    };
    let mut result = Vec::new();
    for solution in solutions {
        let solution = solution?;
        let mut row = Vec::new();
        for variable in variables {
            row.push(solution.get(*variable).ok_or("Missing binding")?.to_string());
        }
        result.push(row);
    }
    result.sort();
    Ok(result)
}

fn main() -> TestResult<()> {
    let eight = "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let padded = "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let nine = "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let cases: [(&str, &[&str]); 3] = [
        ("lexical-pair", &[eight, padded]),
        ("canonical-eight", &[eight]),
        ("different-values", &[eight, nine]),
    ];
    for (case, terms) in cases {
        let store = Store::new()?;
        let data: String = terms.iter().map(|term| format!("<http://ex/s2> <http://ex/p1> {term} . ")).collect();
        let first = format!("INSERT DATA {{ {data}}}");
        let second = "INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }";
        store.update(first.as_str())?;
        let before = quads(&store)?;
        let where_rows = rows(&store, "SELECT ?s ?o WHERE { ?s <http://ex/p1> ?o }", &["s", "o"])?;
        store.update(second)?;
        let after = quads(&store)?;
        let blank_rows = rows(&store, "SELECT ?b WHERE { <http://ex/s2> <http://ex/p0> ?b }", &["b"])?;
        let distinct: BTreeSet<_> = blank_rows.iter().map(|row| &row[0]).collect();
        println!("{{\"case\":{case:?},\"terms\":{terms:?},\"first\":{first:?},\"second\":{second:?},\"before\":{before:?},\"where_rows\":{where_rows:?},\"after\":{after:?},\"blank_rows\":{blank_rows:?},\"distinct_blank_nodes\":{}}}", distinct.len());
        if !distinct.iter().all(|value| value.starts_with("_:")) || distinct.len() != where_rows.len() {
            return Err("Fresh blank-node count does not match observed WHERE solutions".into());
        }
    }
    Ok(())
}


```

## Pinned source

oxigraph0.5.9, default-features=false, rdf-12; repository-pinned dependencies; vendored spargebra0.4.6. No RocksDB/HTTP.

```rust

583:             "http://www.w3.org/2001/XMLSchema#integer"
584:             | "http://www.w3.org/2001/XMLSchema#byte"
585:             | "http://www.w3.org/2001/XMLSchema#short"
586:             | "http://www.w3.org/2001/XMLSchema#int"
587:             | "http://www.w3.org/2001/XMLSchema#long"
588:             | "http://www.w3.org/2001/XMLSchema#unsignedByte"
589:             | "http://www.w3.org/2001/XMLSchema#unsignedShort"
590:             | "http://www.w3.org/2001/XMLSchema#unsignedInt"
591:             | "http://www.w3.org/2001/XMLSchema#unsignedLong"
592:             | "http://www.w3.org/2001/XMLSchema#positiveInteger"
593:             | "http://www.w3.org/2001/XMLSchema#negativeInteger"
594:             | "http://www.w3.org/2001/XMLSchema#nonPositiveInteger"
595:             | "http://www.w3.org/2001/XMLSchema#nonNegativeInteger" => parse_integer_str(value),

```

```rust

870: pub fn parse_integer_str(value: &str) -> Option<EncodedTerm> {
871:     value.parse().map(EncodedTerm::IntegerLiteral).ok()
872: }

```

```rust

1137:             EncodedTerm::BooleanLiteral(value) => Ok(Literal::from(*value).into()),
1138:             EncodedTerm::FloatLiteral(value) => Ok(Literal::from(*value).into()),
1139:             EncodedTerm::DoubleLiteral(value) => Ok(Literal::from(*value).into()),
1140:             EncodedTerm::IntegerLiteral(value) => Ok(Literal::from(*value).into()),
1141:             EncodedTerm::DecimalLiteral(value) => Ok(Literal::from(*value).into()),
1142:             EncodedTerm::DateTimeLiteral(value) => Ok(Literal::from(*value).into()),

```

## Primary references

RDF1.1 Concepts §3.3 https://www.w3.org/TR/rdf11-concepts/#section-Graph-Literal

SPARQL1.1 Update §3.1.3/4.2.3 https://www.w3.org/TR/sparql11-update/#deleteInsert

## Build identity

{
  "compiler": "rustc 1.97.1 (8bab26f4f 2026-07-14)\nbinary: rustc\ncommit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452\ncommit-date: 2026-07-14\nhost: aarch64-apple-darwin\nrelease: 1.97.1\nLLVM version: 22.1.6\n",
  "harness_sha256": "8a30b422264921a29a56cfc8baf5c62d046b415f9b1263f9a0eee57354d47974",
  "binary": {
    "path": "oxigraph-witness",
    "sha256": "c1f581d01715b130367300cabdc43f75273bd7027bdd9b9d982ed617da52f292",
    "bytes": 6840432
  },
  "resolved_lock_sha256": "12174af43de70fc2197ec8d5594b56a5e81a11dc4aefcce36ad608989d887483",
  "command_outcomes": [
    {
      "name": "metadata",
      "exit": 0,
      "elapsed_seconds": 4.1829495
    },
    {
      "name": "build",
      "exit": 0,
      "elapsed_seconds": 186.297422625
    },
    {
      "name": "fixed-cases",
      "exit": 0,
      "elapsed_seconds": 2.0537173750000193
    }
  ],
  "resources": {
    "elapsed_compile_run_seconds": 193.949075291,
    "maximum_sampled_aggregate_growth_bytes": 72196096,
    "minimum_sampled_free_bytes": 6275842048,
    "sample_interval_seconds": 2,
    "limits": {
      "seconds": 600,
      "growth_bytes": 536870912,
      "free_bytes": 2147483648
    },
    "meaning": "Allocated disk bytes; no performance, allocator heap or RSS measurement claim."
  }
}

Omitted host paths, raw environment/build logs and full dependency sources from this public packet. Named diagnostics remain in the local bundle. No full-source or full-workspace review claim.
