---
name: shacl-compact-syntax
description: Parse or write SHACL Compact Syntax (SHACL-C/SCS) shapes with the opt-in sparq-shaclc Rust crate. Use when converting strict W3C CG syntax or shaclc-js extended syntax to oxrdf triples, building a sparq-shacl validation graph, handling typed source errors, using the generated streaming parser, or faithfully serialising an expressible shapes graph back to compact syntax.
---

# SHACL Compact Syntax

Use `sparq-shaclc` as a separate opt-in parser/writer crate. Its crate-root API
emits `oxrdf::Triple` values; add `sparq-shacl` only when the resulting shapes
must be validated against data.

<!-- PR #2136: public usage surface for sparq-shaclc. -->

## Parse shapes

`Cargo.toml`:

```toml
[dependencies]
sparq-core = { path = "../sparq-core" }
sparq-shaclc = { path = "../sparq-shaclc" }
sparq-shacl = { path = "../sparq-shacl" } # only for validation/Graph conversion
```

Parse strict SHACL-C and turn the triples into a validation-ready shapes graph:

```rust
use sparq_core::Graph;
use sparq_shaclc::{parse_strict, DEFAULT_BASE};

let source = r#"
PREFIX ex: <http://example.org/>
shape ex:PersonShape -> ex:Person {
  ex:name [1..1] xsd:string .
}
"#;

let (triples, outcome) = parse_strict(source, DEFAULT_BASE)?;
let shapes = sparq_shacl::graph_from_triples(triples);
let data = Graph::load_str("@prefix ex: <http://example.org/> . ex:a a ex:Person .", "turtle")
    .expect("valid Turtle data");
let report = sparq_shacl::validate(&data, &shapes);
assert!(!report.conforms);
assert_eq!(outcome.base, DEFAULT_BASE);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Relative IRIs resolve against the `base` argument. A document `BASE` directive
overrides that argument for following IRIs. `Outcome` returns the final base and
prefixes in declaration order, with `rdf`, `rdfs`, `sh`, `xsd`, and `owl` first.

## Choose a profile

Use the explicit dispatcher when the input profile is selected at runtime:

```rust
use sparq_shaclc::{parse, Profile, DEFAULT_BASE};

let profile = Profile::Extended;
let (triples, outcome) = parse(source, DEFAULT_BASE, profile)?;
# Ok::<(), sparq_shaclc::ShaclcError>(())
```

- `Profile::Strict` / `parse_strict` accepts the W3C Community Group surface
  plus the RDF 1.2 layer supported by the crate.
- `Profile::Extended` / `parse_extended` additionally accepts shaclc-js
  extensions such as annotation lists, `% ... %` property escapes, trailing
  Turtle statements, and the `a` keyword.
- Do not use extended mode as a fallback for input that must be strict; strict
  mode rejects those extensions by construction.

`ShaclcError` exposes 1-based `line` and byte-counted `column`, an optional
stable `code` such as `UNDECLARED_PREFIX`, and a human-readable `message`.

## Write compact syntax

Pass the parse outcome back to the residual-consumption writer:

```rust
use sparq_shaclc::{write, Profile};

let compact = write(
    &triples,
    Some(&outcome.base),
    &outcome.prefixes,
    Profile::Strict,
)?;
# Ok::<(), sparq_shaclc::ShaclcWriteError>(())
```

Writing is all-or-nothing. `ShaclcWriteError::residual` contains exact triples
that the selected profile cannot express; `missing_ontology` identifies a graph
without the required ontology subject. Never treat the error as partial output.

## Stream generated terms

Use `sparq_shaclc::raw::shaclc12` or `raw::shaclc12ext` only when callback or
chunked push parsing is required. These modules expose their generated term
model, `parse`, and `PushParser`; the crate-root functions are preferable when
the consumer needs `oxrdf::Triple` values.

Keep SHACL validation concerns in the `shacl-validation` skill. This crate
parses and writes shapes; it does not validate data itself.
