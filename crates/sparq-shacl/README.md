<!-- [OPUS-4.8] sq-inzv: README brought to template. -->
# sparq-shacl

<p>
  <a href="https://crates.io/crates/sparq-shacl"><img src="https://img.shields.io/crates/v/sparq-shacl.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-shacl"><img src="https://docs.rs/sparq-shacl/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

Opt-in **SHACL Core + SHACL-SPARQL (`sh:sparql`)** validation over
[`sparq_core::Graph`]s. Core constraints run at the dictionary-id level via direct,
index-backed permutation scans (no SPARQL round-trip; terms materialise only at the
report boundary); `sh:sparql` routes its `sh:select` through `sparq-engine`. Reports:
`conforms` + per-result detail; `to_turtle()` / `to_ntriples()` (W3C report graph), `to_json()`, and `to_text()`.

Like `sparq-reason`, this crate is **isolated**: no other sparq crate depends on it, so
the core engine and the default wasm bundle carry zero SHACL code. The browser/JS
consumer opts in through `sparq-wasm`'s non-default `shacl` feature
(`Store.validate(data, shapes, format)`, a drop-in for `rdf-validate-shacl`).

## 🚀 Quickstart

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;

let data = Graph::load_str(r#"
    @prefix ex: <http://example.org/> .
    ex:alice a ex:Person ; ex:age "thirty" .
"#, "turtle")?;

let shapes = Graph::load_str(r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property [ sh:path ex:age ; sh:datatype xsd:integer ; sh:minCount 1 ] .
"#, "turtle")?;

let report = sparq_shacl::validate(&data, &shapes);
if !report.conforms {
    eprintln!("{}", report.to_text());   // or report.to_turtle() for the report graph
}
// SHACL-1.2 default `conforms` fails on {Violation,Warning,Info}; the stricter toggle:
if report.conforms_violations_only() { /* fails on sh:Violation only */ }
# assert!(!report.conforms);
# Ok(()) }
```

For many data graphs against one shapes graph, parse once with `ShapesModel::parse(&shapes)`
and call `validate_with_model`. CLI: `cargo run -p sparq-shacl --example validate -- data.ttl shapes.ttl [--turtle]`.

## ✨ Features

- **W3C core conformance — 98 / 98 (100%)** of the core `sht:Validate` entries of the
  official [w3c/data-shapes](https://github.com/w3c/data-shapes) suite (pinned
  `b6e73695`); `fetch-shacl-tests.sh` then `cargo test -p sparq-shacl --test w3c_core`.
- **SHACL 1.2 core constraints** — `sh:memberShape`/`sh:uniqueMembers`/
  `sh:{min,max}ListLength`/`sh:uniqueValuesFor`/`sh:closed sh:ByTypes` (sq-vg3y),
  disjunctive-list `sh:datatype`/`sh:nodeKind`/`sh:class`, path-valued comparands for
  `sh:equals`/`sh:disjoint`/`sh:lessThan`/`sh:lessThanOrEquals`, `sh:subsetOf`/
  `sh:someValue`/`sh:singleLine`/`sh:rootClass`, severity-threshold `conforms`
  (shapes-graph `sh:conformanceDisallows` overrides the default {Violation,Warning,Info},
  sq-5q76d), `sh:reifierShape`/`sh:reificationRequired` over RDF-1.2 reifiers, the
  `rdf:dirLangString` `sh:uniqueLang` key (sq-0mjfd), and **per-constraint-statement
  reified-annotation overrides** (`{| sh:deactivated/message/severity … |}` apply to ONLY that
  occurrence, sq-pb0wm). The **full** vendored 1.2 suite is gated by a two-sided ratchet
  (sq-6glcr): core / SPARQL (incl. `sht:Failure` rejection) / node-expr, in both feature
  states — gap map in [`research/shacl12-conformance-gap.md`](../../research/shacl12-conformance-gap.md).
- **SHACL-SPARQL + custom §6 components** — `sh:sparql` (§5.2, results carry
  `sh:sourceConstraint`; `$this`/`$value` pre-binding propagates into UNION branches,
  sibling joins and projecting sub-SELECTs, sq-mue75) and SPARQL-based constraint
  *components* (`sh:ConstraintComponent` + `sh:parameter`/`sh:validator`, §6), pinned
  by the W3C `sparql/*` sub-suites.
- **SHACL Advanced Features (opt-in `shacl-af`)** — a rule **inference** step
  (`sh:rule` / `sh:values`, not part of `validate`): `sh:TripleRule`, `sh:SPARQLRule`,
  value rules, node expressions, and `sh:expression` / `sh:nodeByExpression`;
  `validate_with_domain` selects asserted facts or `data ∪ inferred`; its model variant reuses parsed shapes. Off ⇒ none compiles.
- **Differential fuzzing** — a deterministic SplitMix64 fuzzer (`tests/diff_fuzz.rs`)
  cross-checks reports against pluggable reference engines (pySHACL, Jena SHACL, the
  Zazuko / `rdf-validate-shacl` Node engines); `#[ignore]`d, a nightly CI lane.
- **Full path + target support** — all SHACL path forms (sequence / alternative /
  inverse / `zeroOrMore`·`oneOrMore`·`zeroOrOne`), `sh:targetNode`/`Class`/
  `SubjectsOf`/`ObjectsOf` + implicit class targets, the SHACL-1.2 targets
  `sh:targetWhere`/`sh:shape`/`sh:ShapeClass` + SPARQL-valued targets & value nodes
  (`sh:select`/`sh:sparqlExpr`, sq-rnkdh), plus `sh:severity`/`sh:message`/`sh:deactivated`.
- **SHACL Compact Syntax parser (opt-in `scs`)** — `parse_scs(text, base)` /
  `parse_scs_to_graph(text, base)` turn a [W3C SHACL Compact Syntax](https://w3c.github.io/shacl/shacl-compact-syntax/)
  document into the same shapes triples `validate` consumes; round-trips **32/32** of the
  vendored W3C fixtures graph-isomorphically; zero new deps. Off ⇒ none compiles.
- **Shapes-graph assembly (opt-in `imports`)** — `resolve_shapes_graph`/`resolve_imports`:
  `sh:shapesGraph` + cycle-guarded `owl:imports` closure via a caller-supplied loader. Off ⇒ none compiles.

## 📚 Learn more

- **How-to** — [`skills/shacl-validation/SKILL.md`](../../skills/shacl-validation/SKILL.md)
  (the exhaustive supported-constraint list and the report shape). [GPT-6] SPARQL constraints, validators and targets retain VERSION metadata through prebinding. Unsupported labels use the existing ill-formed-shape policy; see the [EBV dialect contract](../../skills/sparql-query/ebv-dialects.md).
- **API reference** — [docs.rs/sparq-shacl](https://docs.rs/sparq-shacl).
- **Spec** — W3C SHACL (Core, SPARQL, AF); the surface is pinned by [`tests/`](tests/).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## Scope and non-goals

`validate_strict` returns `Err(ShaclFailure)` for an unsound SHACL-SPARQL pre-binding (`MINUS`/
`VALUES`/`SERVICE` / a sub-`SELECT` dropping `$this` / a `BIND` re-binding it, sq-0mjfd) and for
**ill-formed shapes-graph constructs** — the W3C-suite `sht:Failure` outcome: an unparsable
`sh:path` or pathless property shape, a non-`xsd:integer` count/length, a malformed SHACL list,
a non-node-kind `sh:nodeKind`, a non-IRI `sh:severity`, EITHER qualified parameter without the
other, ANY `sh:entailment` (no regime is supported, SHACL §3.4), or a PRESENT `sh:select`/
`sh:sparqlExpr`/`shacl-af` node expression that does not build — **fail-closed relative to this
engine's parsers** (sq-11a, sq-ehq4g, sq-c1v3e); construct-local checks, not a full
SHACL-of-SHACL pass. `validate` skips all of these leniently, surfacing each skip in
`report.diagnostics` (joining the sq-lz99x uncompilable-`sh:pattern` skip there). Out of scope:
`$shapesGraph` — see `bd list -l area:sparq-shacl`. Results are **not deduplicated** across
traversal routes (a nested shape reports once per parent route); re-entrant recursion conforms.

## License

[MIT](../../LICENSE).
