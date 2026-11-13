# Validation details and prerequisites

## Gotchas / feature flags / prerequisites

- **Base SHACL is engaged purely by depending on `sparq-shacl`** (no feature
  needed). It transitively pulls in `sparq-engine` (to run `sh:sparql`/§6 queries).
  Neither is in the **default** wasm dependency graph, so the default browser bundle
  stays SHACL-free; they enter the wasm graph ONLY when a consumer opts in via
  `sparq-wasm`'s non-default `shacl` feature, on which build `sparq-engine`'s defaults
  (rayon/regex/digest) are dropped so the bundle stays lean. The native build is
  unaffected (full engine defaults).
- **SHACL-AF rules (`sh:rule`) are OPT-IN behind the `shacl-af` cargo feature.**
  With the feature off, the base validation path carries zero rule code/parse cost
  and the `apply_rules` / `apply_rules_with_model` / `expand` / `Inference` /
  `apply_rules_in_scope*` / `expand_dataset` / `GraphScope` / `Destination` /
  `DatasetExpansion` / `FactDomain` / `validate_with_domain*` symbols are absent. SHACL-AF rules are an
  INFERENCE step (they produce triples), not part of the existing `validate(..)`
  path. Use `FactDomain::AssertedPlusInferred` when constraints should see the rule
  closure without expanding manually.
- **SHACL 1.2 inference rules also require `shacl-af`.** The separate `rules12`
  module implements their scheduling semantics. Declared `sh:RulesEntailment`
  and `sh:RuleSet` regimes execute before validation; without the feature these
  regimes are unsupported. See [Advanced Features](advanced-features.md) for
  selection, scheduling and failure behavior.
- **The SHACL Compact Syntax parser is OPT-IN behind the `scs` cargo feature.**
  With it off the `scs` module and the `parse_scs` / `parse_scs_to_graph` / `ScsError`
  / `DEFAULT_BASE` symbols are absent (zero parser code compiled in). It adds no new
  dependencies. Coverage is honest: any construct outside the supported grammar
  returns a typed `ScsError` rather than mis-parsing. Both the SCS parse and the
  reference Turtle must resolve relative IRIs against the same `base` to agree, so
  the round-trip test passes the fixture's `BASE` (or `DEFAULT_BASE`) to both sides.
- **Shapes-graph assembly is OPT-IN behind the `imports` cargo feature** (sq-uz0).
  With it off the `imports` module and the `resolve_shapes_graph` / `resolve_imports`
  / `ShapesGraphResolution` symbols are absent (zero assembly code compiled in; no
  new dependencies). The library never dereferences an IRI itself — the loader
  callback does — so there is no network / no SSRF surface here by construction.
- **Rule fixpoint is bounded.** `apply_rules` iterates the rule schedule until a
  pass infers nothing, capped at `rules::MAX_ITERATIONS` (100); `Inference::capped`
  flags a non-terminating rule set (e.g. a CONSTRUCT minting a fresh blank node each
  pass) whose inferred set may be incomplete.
- **`sh:conforms` uses the SHACL-1.2 default disallowed set {Violation,Warning,Info}**
  (sq-sx15d): a Debug/Trace-only report conforms; a Warning/Info result does NOT. For a
  stricter "only Violation fails" gate use `conforms_violations_only()`; for a custom
  `sh:conformanceDisallows` set use `conforms_with_disallowed(&[..])`.
- **Ill-formed shapes are skipped by `validate`, reported as a failure by
  `validate_strict` (sq-11a, sq-ehq4g).** A shape never declared, an unparsable path,
  or an `sh:select` that fails to parse (e.g. undeclared prefix) contributes no
  results; the rest of validation still runs. `validate` never returns a
  `Result`/panics on bad shapes — so a silently-empty report can mean "no targets"
  rather than "conforms". When the distinction matters (CI shape linting, the suite's
  `sht:Failure` entries), `validate_strict` rejects ill-formed constructs with
  `ShaclFailure.ill_formed` (see the strict-validation list above). A PRESENT
  `sh:select`/`sh:sparqlExpr` whose text does not parse is rejected strictly too
  (sq-ehq4g) — FAIL-CLOSED relative to this engine's vendored SPARQL parser, so a
  valid query using syntax the parser lacks is also rejected; prefer fixing the query
  (or filing the parser gap) over weakening the strict gate.
- **An uncompilable `sh:pattern` is SKIPPED, not fail-closed (sq-lz99x).** The Rust
  `regex` crate has no lookahead/lookbehind — neither does the XML Schema regex flavour
  the SHACL spec ties `sh:pattern` to — so e.g. `^(?!(TODO|TBD)).*` does not compile.
  That constraint is skipped (it reports no violations) and surfaced once in
  `report.diagnostics` (a `ShapeDiagnostic` carrying the shape, component, and the
  `regex` crate's error), so the skip is not silent. Earlier this wrongly flagged
  EVERY value. To express a "must NOT start with X" check, use a POSITIVE-match
  `sh:sparql` `REGEX(?str, "^\\s*(TODO|...)")` constraint (flag when it matches) instead.
- **XPath-regex divergences are translated, not passed through (sq-8ro).** `sh:pattern`
  is matched by the Rust `regex` crate, but the XPath/XSD constructs it lacks are
  translated first (`eval.rs::compose_pattern`): the `q` flag in `sh:flags` gives XPath
  F&O literal-pattern mode (only `i` combines with it, matching the engine's SPARQL
  `REGEX`), and `\i` / `\I` / `\c` / `\C` (XML NameStartChar / NameChar classes and
  complements) expand to explicit character classes — including inside `[...]`, via
  nested classes. Look-around remains genuinely unsupported → the sq-lz99x skip path.
- **Temporal comparison is exact, and cross-family pairs are INCOMPARABLE (GH #3526).**
  `sh:lessThan`, `sh:lessThanOrEquals` and `sh:min/maxInclusive` / `sh:min/maxExclusive`
  decide `xsd:dateTime` / `xsd:dateTimeStamp` / `xsd:date` / `xsd:time` through the
  engine's shared exact comparator (`sparq_core::temporal::ExactTemporal`), not on f64
  instants, so the ±14h mixed-timezone indeterminate window is decided on exact instants
  rather than rounded. A temporal paired with another family compares as nothing — a date
  against a dateTime, either against an `xsd:time`, a temporal against a non-temporal —
  and so does an ill-formed raw lexical (a timezone-free `xsd:dateTimeStamp`, a padded
  value: XSD whitespace processing does not apply to an RDF literal). Incomparable means a
  validation result, not a silent pass. `sh:in` and `sh:hasValue` are RDF-**term**
  membership (SHACL §§4.8.1–4.8.2), not value equality, so two distinct literals denoting
  one instant are different members.
- **Numeric comparison is exact for `xsd:decimal` and the integer family (GH #7294).** The
  same six components compare two decimal / integer values by their lexicals, at any length,
  so values that differ only beyond `f64` precision (`1.000000000000000015` against
  `1.00000000000000002`, integers past 2^53) are told apart. A pair with an `xsd:float` or
  `xsd:double` operand is promoted to the least common floating type, as SPARQL `<` is specified,
  each operand converted once from its lexical whatever its length. `NaN` and an ill-typed
  numeric lexical (`"1.5"^^xsd:integer`, an out-of-range `xsd:byte`, a padded value) compare
  as nothing. A numeric bound is read once per validation run.
- **Results are NOT deduplicated** across traversal routes / component occurrences — a
  nested shape reached via two parents reports twice (intentional, matches the suite).
- **Recursion is treated as conforming.** Re-entering the same (focus, shape) pair
  counts as conforming (SHACL leaves recursion undefined); cyclic `sh:node`/`sh:property`
  terminate without stack overflow.
- **`sh:sparql` pre-binding:** `$this` (and `$PATH` on property shapes) is injected via
  an algebra-level VALUES on the parsed query — it lands below solution modifiers (so
  `LIMIT`/`ORDER BY`/`DISTINCT` behave correctly) AND propagates into every scope the
  variable can reach: both UNION branches, sibling joins, both sides of a LATERAL
  (its right side is a required part evaluated per left solution, so both sides take
  the pre-binding exactly as a JOIN's siblings do — GH #4291), and a sub-SELECT that
  hides or explicitly projects the variable. OPTIONAL branches and EXISTS graph
  patterns also receive pre-bindings. Subquery projections stay unchanged.
  Blank focus nodes retain their identity through a term-valued algebra binding;
  ground triple terms are accepted in VALUES. Each `sh:sparql`
  result carries `sh:sourceConstraint` (the `sh:SPARQLConstraint` node).  `sh:prefixes`
  chases `sh:declare`(`sh:prefix`/`sh:namespace`) transitively through `owl:imports`.
- **§6 limits:** the W3C `sparql/component/*` suite `owl:imports` the external
  `http://datashapes.org/dash` vocabulary; it is run offline (`tests/w3c_sparql_component.rs`)
  by resolving that import against a vendored, minimal pinned excerpt at
  `crates/sparq-shacl/tests/vendor/dash.ttl`. The `sparql/pre-binding` *rejection*
  channel is `validate_strict` (SHACL 1.2) or `validate_strict_shacl10` (SHACL 1.0).
- **W3C conformance:** at suite pin `04ecbb4` (2026-11-12), full SHACL 1.2
  Core passes 162 entries and SHACL-SPARQL passes 98, in both feature states.
  Node-expression evaluation passes 272 entries in both feature states, with four
  failures comparing decimal lexical forms and no unsupported evaluation entries.
  With `shacl-af`, node-expression constraints pass eight entries. The eight validation
  entries are excluded from evaluation's EARL assertions. See
  [`research/shacl12-conformance-gap.md`](../../../research/shacl12-conformance-gap.md)
  for the exact gap map and ratchets.

- §6 SPARQL-based constraint *components* are implemented and tested
  (`tests/sparql_components.rs` plus the W3C `sparql/component` sub-suite in
  `tests/w3c_sparql_component.rs`); the crate README documents them under its
  "SHACL-SPARQL + custom §6 components" feature bullet.

*Id-level core-constraint fast path (perf, sq-7d3dj.33.4).* Core constraints are
evaluated at the **dictionary-id level**: each shape's `sh:path` is compiled once per
`validate` (predicate IRIs → ids), the per-focus path walk and dedup run over `u32`
ids, and the hot value checks (`sh:datatype` / `sh:pattern` / `sh:nodeKind` /
`sh:minCount`·`maxCount` / `sh:minLength`·`maxLength` / `sh:node`, which also gets an
id-keyed conformance memo) read the dictionary's zero-copy literal records — a term is
materialised only for a VIOLATING value, at the report boundary. Compiled `sh:pattern`
regexes are `Rc`-shared across focus nodes (a per-focus `Regex` clone would rebuild the
lazy-DFA cache on every match). All of it is internal — no API or feature flag — and the
report is byte-identical to the Term-level route: a focus node absent from the data
dictionary (e.g. a `sh:targetNode` naming a ghost IRI) falls back to the Term-level walk,
and the in-crate `idfast_*` differential tests diff full reports fast-vs-forced-slow.
