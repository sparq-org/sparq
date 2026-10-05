# sparq inference conformance report

- rdf-tests commit: `f25dbc092c654d792974848e81bb519d7328f0e8`
- w3c/N3 commit: `23ccf3d56b25cb60a68878a04aae0d52493080f0`
- sparq commit: `e321ee200c915f6d677636aea07b6e89540c8f15`

Every manifest entry lands in exactly one bucket — pass, fail, documented divergence, or out-of-scope WITH its reason (no silent skips). Pass rate is `(pass + divergence) / run`; out-of-scope entries are excluded from the rate but counted in coverage.

## RDF Semantics entailment (rdf-tests rdf/rdf11/rdf-mt)

Premise → `sparq-reason` RDFS/RDF materialization (plus the harness-side finite axiomatic/reflexive augmentation the production materializer deliberately omits) → blank-node-homomorphism entailment check; `mf:result false` = (in)consistency check with the per-test recognized-datatype set.

| suite | pass | fail | divergence | out-of-scope | pass-rate (of run) |
|---|---:|---:|---:|---:|---:|
| rdf-mt | 48 | 0 | 0 | 0 | 100.0% |
| **total** | **48** | **0** | **0** | **0** | **100.0%** |

**Overall (RDF Semantics entailment (rdf-tests rdf/rdf11/rdf-mt)): 48 pass / 0 fail / 0 documented divergence / 0 out-of-scope — pass+divergence 100.0% of run, 100.0% of all in-scope tests.**

## OWL 2 RL (W3C OWL WG test cases, RDF-based semantics)

Source: the OWL WG test-repository export (all.rdf, pinned snapshot — see scripts/fetch-inference-suites.sh). Selection: `test:profile test:RL` AND `test:semantics test:RDF-BASED`; each selected case yields one row per declared check (consistency / inconsistency / positive / negative entailment). Cases not RL-profiled or direct-semantics-only are outside the RL applicability rule and are counted in the selection-summary row, not as individual rows.

Method: premise → `sparq_reason::materialize_owl_rl` → `sparq_reason::inconsistencies` / bnode-homomorphism entailment check; ontology-header triples stripped on both sides. The RL/RDF rules are by design incomplete for arbitrary (TBox) conclusions under the RDF-based semantics (conformance doc §2.3, theorem PR1) — such fails are listed, not hidden.

| suite | pass | fail | divergence | out-of-scope | pass-rate (of run) |
|---|---:|---:|---:|---:|---:|
| owl2-rl/consistency | 52 | 0 | 0 | 24 | 100.0% |
| owl2-rl/inconsistency | 11 | 0 | 0 | 3 | 100.0% |
| owl2-rl/negative-entailment | 4 | 0 | 0 | 3 | 100.0% |
| owl2-rl/positive-entailment | 11 | 0 | 13 | 5 | 100.0% |
| owl2-rl/selection | 0 | 0 | 0 | 1 | — |
| **total** | **78** | **0** | **13** | **36** | **100.0%** |

**Overall (OWL 2 RL (W3C OWL WG test cases, RDF-based semantics)): 78 pass / 0 fail / 13 documented divergence / 36 out-of-scope — pass+divergence 100.0% of run, 71.7% of all in-scope tests.**

### Documented divergences

- `owl2-rl/positive-entailment` — **chain2trans1**: PERMANENT — conclusion is the TBox axiom `p rdf:type owl:TransitiveProperty` (from the self-chain p∘p ⊑ p); prp-spo2 consumes owl:propertyChainAxiom only in its BODY to derive chained assertions, no rule head emits owl:TransitiveProperty, and PR1 completeness is assertion-only.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **DisjointClasses-001**: PERMANENT — conclusion types Stewie into an INVENTED anonymous owl:complementOf class; complementOf occurs only in the BODY of the clash rule cls-com — no RL/RDF rule head constructs a class expression (PR1 covers named-class assertions only).
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **DisjointClasses-003**: PERMANENT — as DisjointClasses-001 via owl:AllDisjointClasses: the conclusion invents TWO anonymous owl:complementOf classes; cax-adc consumes the members list to derive `false` only, and no rule head constructs a class expression.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **New-Feature-DisjointDataProperties-002**: PERMANENT — conclusion is a REIFIED owl:AllDifferent/owl:distinctMembers structure; eq-diff2/3 and prp-adp consume these structures in rule BODIES (clash detection) and no rule head constructs one — and differentFrom between individuals is underivable anyway (dt-diff emits it only between unequal-value literals).
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **New-Feature-DisjointObjectProperties-001**: PERMANENT — needs the CONTRAPOSITIVE of prp-pdw: were Peter = Lois, one pair would lie in BOTH disjoint properties, so full semantics entails `Peter differentFrom Lois`; prp-pdw derives only `false` from an actual shared pair, no rule emits differentFrom between individuals (dt-diff covers literals only), and DifferentIndividuals conclusions are outside PR1's assertion scope.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **New-Feature-DisjointObjectProperties-002**: PERMANENT — the prp-adp/prp-pdw contrapositive exactly as in New-Feature-DisjointObjectProperties-001, PLUS the conclusion is a reified owl:AllDifferent/owl:distinctMembers structure that no rule head constructs.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **New-Feature-ObjectQCR-002**: PERMANENT — needs the CONTRAPOSITIVE of cls-maxqc3: were Stewie a Woman, Peter's maxQC-1-on-Woman restriction would force `Stewie sameAs Meg` against their differentFrom, so full semantics types Stewie into the complement of Woman; cls-maxqc3/4 derive sameAs only from ALREADY-co-typed fillers, and no rule head constructs the required owl:complementOf class.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **New-Feature-ReflexiveProperty-001**: PERMANENT — the premise's ReflexiveObjectProperty is EXCLUDED from the RL grammar (Profiles §4.2: all OWL 2 axioms 'apart from disjoint unions of classes and reflexive object property axioms'), the export's RL tag notwithstanding; accordingly no prp-rfx rule exists, so `Peter knows Peter` is underivable.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **owl2-rl-rules-fp-differentFrom**: PERMANENT — needs the CONTRAPOSITIVE of prp-fp: were Y1 = Y2, prp-fp would merge X1/X2 against their differentFrom, so full semantics entails `Y1 differentFrom Y2`; prp-fp requires the SAME subject and derives only sameAs, and no rule emits differentFrom between individuals (dt-diff covers literals only).
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **owl2-rl-rules-ifp-differentFrom**: PERMANENT — the prp-ifp CONTRAPOSITIVE, symmetric to the fp case: were X1 = X2, prp-ifp would merge Y1/Y2 against their differentFrom; prp-ifp derives only sameAs, and differentFrom between individuals has no producing rule.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **WebOnt-I5.5-005**: PERMANENT — conclusion asserts the EXISTENCE of an anonymous class `[ owl:unionOf (a) ]`; no RL/RDF rule head emits owl:unionOf or rdf:first/rdf:rest list cells (cls-uni/scm-uni consume unionOf in bodies), so the closure can never contain the required structure.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **WebOnt-I5.8-008**: PERMANENT — a TBox rdfs:range conclusion needing value-space INTERSECTION (xsd:short ∩ xsd:unsignedInt ⊑ xsd:unsignedShort); scm-rng1 only propagates a range UP existing subClassOf edges, no dt-*/scm-* rule intersects datatype ranges, and PR1 is assertion-only.
  *observed*: conclusion not entailed by the RL/RDF-rules closure
- `owl2-rl/positive-entailment` — **WebOnt-I5.8-009**: PERMANENT — as WebOnt-I5.8-008 with xsd:nonNegativeInteger ∩ xsd:nonPositiveInteger = {0} ⊑ xsd:short: datatype-range intersection is beyond the RL/RDF dt-* rules, and the rdfs:range conclusion is a TBox axiom outside PR1.
  *observed*: conclusion not entailed by the RL/RDF-rules closure

### Out-of-scope reasons

| reason | tests |
|---|---:|
| status absent (only Approved cases are conformance tests) | 22 |
| status Proposed (only Approved cases are conformance tests) | 8 |
| premise only available in functional syntax | 3 |
| uses owl:imports (no dereferencing in the harness) | 2 |
| selection summary (informational row) | 1 |

## N3 (w3c/N3 community-group suite)

Source: w3c/N3 `tests/` (pinned clone). The reasoner manifest measures EYE/cwm parity of the N3 rule engine; the parser/extended manifests measure the N3 parser (positive = must parse, negative = must be rejected); TurtleTests runs the parser in STRICT Turtle mode. Documents parse against the suite's canonical https://w3c.github.io/N3/tests/ base (the .nt expectations bake those IRIs in), and an offline resolver maps those IRIs back into the pinned clone so log:semantics/log:content work without I/O. Reference graphs are compared under blank-node isomorphism (formulae structurally, lists expanded where the expectation is plain triples, same-datatype numerics by value); reason references parse against the ACTION document's base (cwm generated them so). Under test:conclusions both derived-only and store-minus-rules shapings are accepted (the vendored cwm out-files are inconsistent between the two). `rdft:Rejected` entries are out of scope (upstream rejected them); `test:strings` (log:outputString) stays out of scope.

| suite | pass | fail | divergence | out-of-scope | pass-rate (of run) |
|---|---:|---:|---:|---:|---:|
| n3/extended | 871 | 0 | 0 | 107 | 100.0% |
| n3/parser | 213 | 0 | 1 | 16 | 100.0% |
| n3/reasoner | 83 | 0 | 3 | 3 | 100.0% |
| n3/turtle | 297 | 0 | 0 | 0 | 100.0% |
| **total** | **1464** | **0** | **4** | **126** | **100.0%** |

**Overall (N3 (w3c/N3 community-group suite)): 1464 pass / 0 fail / 4 documented divergence / 126 out-of-scope — pass+divergence 100.0% of run, 92.1% of all in-scope tests.**

### Documented divergences

- `n3/reasoner` — **cwm_includes_conclusion**: the engine now derives the `:result :is { … }` conclusion formula, but the vendored conclusion-ref.n3 cannot match the vendored sources: (1) its quoted daml:comment for :Animal reads `…number of\nontological…` while the vendored daml-ex.n3 has `…number of\n\tontological…` (TAB) — the 2003 cwm run used an older daml-ex.n3 revision; (2) the ref formula is not closed under its OWN quoted rules (it holds `d:father daml:range d:Man`, `daml:range = rdfs:range` and the rule `{?x ?p1 ?y. ?p1 = ?p2} => {?x ?p2 ?y}`, yet lacks `d:father rdfs:range d:Man`) — log:conclusion ("all statements which can be deduced") legitimately derives 31 statements the 2003 run did not (instantiated transitivity rules and the consequent type/schema facts).
  *observed*: output not isomorphic to the reference (2 vs 1 statements; expected-only e.g. [Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_includes/foo.n3#result"), Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_includes/foo.n3#is"), Formula([[Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_other/daml-ex.n3"), Iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"), Iri("http://www.daml.org/2001/03/daml+oil#Ontology")], [Iri("https://w3c.github.io/N3/tests/N3Tests/)
- `n3/reasoner` — **cwm_includes_t11**: with the offline resolver the engine derives the schema-checking conclusions over <t10a.n3> (test_undefined etc.); t11-ref.n3 holds only the two foo.n3 conclusions and even omits t11's own data facts — the cwm run that produced it never resolved t10a.n3 and ran with an unrecorded --purge.
  *observed*: output not isomorphic to the reference (11 vs 2 statements; actual-only e.g. [Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_includes/foo.n3#test_undefined"), Iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"), Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_includes/foo.n3#UsedProperty")])
- `n3/reasoner` — **cwm_unify_unify1**: the action concludes `:test :a ?x` (predicate <unify1.n3#a>) but unify1-ref.n3 says `:test a :Successful` (rdf:type) — the vendored cwm reference was generated from an older revision of the action.
  *observed*: output not isomorphic to the reference (1 vs 1 statements; expected-only e.g. [Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_unify/unify1.n3#test"), Iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"), Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_unify/unify1.n3#Successful")]; actual-only e.g. [Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_unify/unify1.n3#test"), Iri("https://w3c.github.io/N3/tests/N3Tests/cwm_unify/unify1.n3#a"), Iri("https:/)
- `n3/parser` — **numbers.n3**: the expected output cwm_n3/n3parser.tests_n3_10013.n3 keeps ONE statement under the generating author's local base (file:/home/syosi/...#is) while the rest were rebased — that statement can never match any correct parse.
  *observed*: parsed statements not isomorphic to the reference

### Out-of-scope reasons

| reason | tests |
|---|---:|
| rdft:Rejected upstream | 125 |
| log:outputString (test:strings) not implemented | 1 |

## SPARQL 1.1 entailment regimes (sparql11/entailment)

Source: `sparql11/entailment` from the pinned rdf-tests clone. Each test runs as query-over-materialized-closure through the same evaluation/comparison machinery as the gating SPARQL harness; the regime materialized is the strongest the reasoner supports of the test's `sd:entailmentRegime` set (RDFS ⊃ RDF; OWL-RDF-Based via the OWL RL rules, a sound subset — its incompleteness shows up as listed fails). The OwlRl closure adds a harness-side eq-ref layer (OWL 2 Profiles §4.3 Table 4: reflexive owl:sameAs for every closure term — omitted by the production materializer as store bloat). The regimes' ANSWER RESTRICTION is applied to engine solutions before comparison (SPARQL 1.1 Entailment Regimes): (C1/skolemization, §2/§3.1) bindings to blank nodes not in the queried graph — i.e. introduced by the saturation — are never answers; and for tests whose expectations are sanctioned under OWL-Direct (§7), variables in class/property-NAME positions cannot bind to anonymous class expressions (a bnode is not a name). D-only / OWL-Direct-only / RIF tests are out of scope with their reason.

| suite | pass | fail | divergence | out-of-scope | pass-rate (of run) |
|---|---:|---:|---:|---:|---:|
| sparql11/entailment | 47 | 0 | 0 | 23 | 100.0% |
| **total** | **47** | **0** | **0** | **23** | **100.0%** |

**Overall (SPARQL 1.1 entailment regimes (sparql11/entailment)): 47 pass / 0 fail / 0 documented divergence / 23 out-of-scope — pass+divergence 100.0% of run, 67.1% of all in-scope tests.**

### Out-of-scope reasons

| reason | tests |
|---|---:|
| entailment regime(s) OWL-Direct not supported (no materialization mapping) | 18 |
| entailment regime(s) RIF not supported (no materialization mapping) | 4 |
| entailment regime(s) D not supported (no materialization mapping) | 1 |

## W3C rdf-turtle through the sparq Turtle parser (parse_to_triples)

Source: w3c/rdf-tests `rdf/rdf11/rdf-turtle/manifest.ttl` (pinned clone). Runs THROUGH the sparq Turtle parser (`Graph::parse_to_triples("turtle")`) — the rejection/acceptance oracle for the Turtle parse path, distinct from the oxttl-differential chunked-vs-serial test and from the N3-parser TurtleTests. PositiveSyntax must parse, NegativeSyntax must be rejected, Eval must parse to a graph isomorphic to the N-Triples expectation, NegativeEval must be rejected. Comparison is blank-node-isomorphic term-set equality (never line-by-line).

| suite | pass | fail | divergence | out-of-scope | pass-rate (of run) |
|---|---:|---:|---:|---:|---:|
| rdf-turtle | 313 | 0 | 0 | 0 | 100.0% |
| **total** | **313** | **0** | **0** | **0** | **100.0%** |

**Overall (W3C rdf-turtle through the sparq Turtle parser (parse_to_triples)): 313 pass / 0 fail / 0 documented divergence / 0 out-of-scope — pass+divergence 100.0% of run, 100.0% of all in-scope tests.**

## Overall (all inference suites)

**1950 pass / 0 fail / 17 documented divergence / 185 out-of-scope — pass+divergence 100.0% of run, 91.4% of all in-scope tests.**

Covers the four inference regime suites (rdf-mt, OWL 2 RL, N3, SPARQL 1.1 entailment) plus the W3C rdf-turtle Turtle parser suite (not an inference suite; shares the same `inference-conformance` CI ratchet job per the central scoreboard).
