// [FABLE-5] sq-rvgr2.4 — SPARQL Vector & GenAI Extension, an Unofficial Proposal Draft.
// Revision 2 (review round 1): normativity decoupled from implementation status.
// Revision 3 ([SONNET-4.6] sq-vv2l0): the Editor's-note TODOs for code-level details confirmed
// against crates/sparq-vectors and resolved into the text.
//
// GROUNDING. Revisions 1-2 were grounded ONLY in the read-only estate recon for the
// vector/GenAI surface (research/specs/vector-genai-estate-recon.md, branch
// docs/fable-program-recon-records) — the built-and-tested surface of the opt-in sparq-vectors
// crate plus the adjacent sparq-nlq / sparq-introspect crates — and marked what that prose
// digest could not pin with an "Editor's note" rather than guessing. Revision 3 checked those
// notes against the crate sources; two of them changed a verdict rather than confirming one
// (VG-MET-2 is NOT satisfied; VG-GOV-2 is unsatisfiable from this repository), and both are
// recorded as such. Normative requirements are NOT gated on build status: the normative
// sections state what a conforming implementation MUST do, and the informative
// implementation-status section reports which requirements the current sparq build satisfies.
// Designs offered for a FUTURE revision are labelled with a "Proposal" aside; a detail this
// draft deliberately leaves to a future revision keeps an "Editor's note".
//
// HONESTY. This draft cites NO measured performance or accuracy figures: every measurement in
// the underlying estate is work-box non-canonical, and the spec factory's build-boundary
// honesty scan enforces the no-perf-numbers rule over this source. Approximation, uncalibrated
// confidence, and no-accuracy-claim features are stated plainly in the normative text, and the
// implementation-status section states plainly where the sparq build does NOT yet satisfy this
// revision (most importantly the embedding-provenance record).

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "SPARQL Vector & GenAI Extension")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

// A "Proposal" aside: marks a design offered for a FUTURE revision of this specification. It
// is outside this draft's normative surface (which is otherwise independent of implementation
// status — see the conformance section). Mirrors _lib/spec.typ's note() (HTML aside + PDF
// block) so both targets render it, with an amber accent to distinguish a proposal from an
// informative note.
#let proposal(body) = context {
  if target() == "html" {
    html.elem("aside", attrs: (class: "note proposal"))[
      #html.elem("span", attrs: (class: "note-title"))[Proposal]
      #body
    ]
  } else {
    block(width: 100%, inset: 8pt, radius: 3pt, stroke: (left: 3pt + rgb("#c80")), fill: rgb("#fdf8ee"))[
      #strong[Proposal] — #body
    ]
  }
}

// An "Editor's note" aside: a detail this draft does not yet pin (the grounding digest does
// not capture it); to be confirmed against the implementation before the draft advances.
#let ednote(body) = context {
  if target() == "html" {
    html.elem("aside", attrs: (class: "note ednote"))[
      #html.elem("span", attrs: (class: "note-title"))[Editor's note]
      #body
    ]
  } else {
    block(width: 100%, inset: 8pt, radius: 3pt, stroke: (left: 3pt + rgb("#88a")), fill: rgb("#f4f5fb"))[
      #strong[Editor's note] — #body
    ]
  }
}

// A testable-assertion identifier. Every normative assertion carries one, rendered at the
// assertion site, so conformance is checkable assertion-by-assertion (see the
// testable-assertions section). In HTML each id is an anchor (`#req-vg-…`).
#let rid(id) = context {
  let label-text = "[" + id + "]"
  if target() == "html" {
    html.elem("span", attrs: (class: "req-id", id: "req-" + lower(id)))[#label-text]
  } else {
    text(size: 8pt, weight: "bold", fill: rgb("#555"))[#label-text]
  }
}

#spec-head()

#intro-section("abstract", "Abstract")[
  This document specifies a vector-search and generative-AI extension to SPARQL 1.1, developed
  in the sparq engine. It is explicit about what is #strong[implemented] versus what is
  #strong[specified ahead of implementation]. The implemented core comprises the `vec:`
  vocabulary and the two magic-predicate graph patterns `vec:nearest` and `vec:search`
  (k-nearest-neighbour retrieval with scores exposed as ordinary solution bindings); the
  separation of answer-exact and approximate execution modes; the answer-safety obligation for
  predicate-filtered vector search; the persisted vector-store container and its
  graph-staleness contract; the embedding-acquisition contract and remote-provider protocol;
  and the grounded-generation (natural-language-to-SPARQL) processing obligations. Specified
  ahead of implementation — normative in this revision, and #strong[not yet satisfied by the
  sparq build] (@sec-impl-status) — are the REQUIRED embedding-provenance record (model
  identifier, model version, distance metric, normalisation regime, dimension), its
  MUST-reject rule for provenance-incompatible query vectors, and a deterministic tie-break
  that makes answer-exact top-k membership reproducible across implementations.
  Clearly-labelled Proposals (in-query hybrid fusion, reranking, multi-vector stores,
  constrained decoding) and open design questions are recorded for a future Working or
  Community Group. Every normative assertion carries a testable identifier (@sec-testable).
]

#sotd()

= Introduction

RDF stores increasingly pair symbolic query answering with dense-vector retrieval: entities
and literals are embedded into a vector space by a (typically external) embedding model, and a
query can ask for the _k_ nearest neighbours of a node or of an ad-hoc vector, optionally
constrained by ordinary graph patterns. Every major vendor has grown a proprietary idiom for
this (@sec-related surveys them), but there is no standard SPARQL surface.

This draft writes down one coherent design so it can be reviewed, criticised, and compared —
and it is explicit about what is built versus what is specified ahead of implementation. The
retrieval core (the `vec:` patterns, the execution modes, filtered answer-safety, the primary
container and its staleness contract, the embedding-client contract, the grounded-generation
loop) is implemented and tested in the sparq engine's opt-in vector estate. The
#emph[reproducibility] core — the embedding-provenance record of @sec-provenance-model and the
metric and normalisation declaration of @sec-metric — is normative in this revision but not
yet satisfied by the sparq build. @sec-impl-status reports the split assertion-by-assertion;
nothing in the normative sections is conditioned on it.

The design philosophy is deliberately minimal:

- The #strong[only] vector query surface is a pair of _magic predicates_ in a dedicated
  namespace, recognised inside ordinary basic graph patterns. No new grammar productions, no
  new algebra operators, and no change to the SPARQL parser are required.
- Vector search results enter the solution multiset as ordinary bindings — a neighbour
  variable and an optional score variable — so every downstream SPARQL construct (joins,
  `FILTER`, `ORDER BY`, aggregation, federation) applies unchanged.
- Answer-exact execution is the default. Approximate nearest-neighbour (ANN) acceleration is
  an explicit opt-in whose non-exactness must be surfaced, never silent.

An implementation technique note: the sparq implementation realises these patterns as a
query-rewrite from the recognised pattern to inline data (a `VALUES`-equivalent block) over
the algebra, leaving the engine, planner, and WASM build unchanged. The technique is
informative; only the observable semantics in this document are normative.

This document also covers the surrounding estate a vector surface needs to be trustworthy:
persisted store formats and their staleness contract (@sec-formats), the embedding
acquisition contract (@sec-embedding), embedding provenance (@sec-provenance-model), and the
grounded-generation loop that turns natural-language questions into validated SPARQL
(@sec-grounded). Where a surface is offered for a future revision rather than specified now,
it appears as a labelled #emph[Proposal] (@sec-conformance).

= Terminology and conformance <sec-conformance>

== Requirement keywords

The key words #strong[MUST], #strong[MUST NOT], #strong[REQUIRED], #strong[SHALL],
#strong[SHALL NOT], #strong[SHOULD], #strong[SHOULD NOT], #strong[RECOMMENDED], #strong[MAY],
and #strong[OPTIONAL] in this document are to be interpreted as described in
#cite("RFC2119") and #cite("RFC8174") when, and only when, they appear in all capitals, as
shown here.

== Terms

This document uses the following terms:

- A #dfn[vector store] is a component that persists a mapping from RDF term identifiers to
  dense vectors (one vector per term in the mainline model) and answers nearest-neighbour
  queries over it. The mainline #dfn[embeddable domain] is exactly #strong[IRIs and literals];
  a blank node is never embedded in this model, so it is never a query seed (@sec-typing) nor a
  candidate result — the property the boundary tie-break of @sec-ordering relies on to stay
  well-defined across implementations.
- A #dfn[vec-extended processor] is a SPARQL query processor that recognises and evaluates the
  graph patterns of @sec-patterns against a vector store.
- An #dfn[embedding client] is a component that obtains vectors for text inputs from an
  embedding model under the contract of @sec-embedding.
- A #dfn[grounded-generation processor] is a component that answers natural-language questions
  by generating, validating, and executing SPARQL against an RDF dataset per @sec-grounded.
- #dfn[Answer-exact] execution returns exactly the true top-k under the store's declared
  metric (@sec-metric), with ties at the k boundary broken deterministically per
  @sec-ordering; #dfn[approximate] execution may miss true neighbours (its recall is strictly
  below one).

== Conformance classes

Conformance is claimed per class — vector store, vec-extended processor, embedding client,
grounded-generation processor. An implementation MAY implement any subset of the four
classes, but MUST satisfy every normative assertion of each class it claims.

== Normativity and implementation status

The normative assertions of this specification bind every implementation that claims the
relevant conformance class, #strong[independently of whether any existing implementation —
including the sparq engine — satisfies them today]. This document keeps two questions
separate:

+ #emph[What does the specification require?] — answered by the normative sections. A
  requirement is never demoted from the normative surface merely because it is not yet built;
  in particular, the embedding-provenance record of @sec-provenance-model is REQUIRED even
  though the current sparq container does not carry it by default.
+ #emph[Does a given build comply?] — answered, for sparq, by the informative
  implementation-status report of @sec-impl-status, which records per assertion group what is
  implemented, what is not, and what remains unconfirmed.

The following are #strong[non-normative]: material marked #emph[Proposal] (a design offered
for a future revision, outside this draft's normative surface); material marked
#emph[Editor's note]; all examples and notes; @sec-related; and @sec-impl-status.

== Testable assertions

Every normative assertion in this document carries a stable identifier of the form
`[VG-<group>-<n>]`, rendered at the assertion site. @sec-testable indexes the groups and
states the conformance-testing commitment.

= Related work <sec-related>

#emph[This section is informative.]

== Vendor vector-query surfaces

GraphDB exposes similarity-index queries through special predicates inside SPARQL
#cite("GRAPHDB-SIM"); Neo4j exposes vector indexes through Cypher procedure calls
#cite("NEO4J-VEC"); Amazon Neptune Analytics exposes vector-similarity functions in its query
surface #cite("NEPTUNE-AN"); pgvector adds vector distance operators to SQL
#cite("PGVECTOR"); and Stardog's Voicebox couples LLM-driven question answering to the graph
#cite("STARDOG-VB") — the grounded-generation half of this draft's scope rather than the k-NN
half. Dedicated vector databases (Qdrant, Weaviate, Milvus) sit outside the query language
entirely, behind JSON APIs. Across these systems three surface shapes recur — (i)
pattern-level magic predicates, (ii) procedure or function calls, and (iii) comparison
operators — and no two vendors agree on one. This draft picks shape (i) because it needs no
grammar change and composes with basic graph patterns; the equivalence or deliberate absence
of the other two shapes is an open question (@sec-open). All vendor entries here are
catalogued from vendor documentation; none has been measured or conformance-tested by the
sparq project.

== Prior RDF/SPARQL vector work

Vec2SPARQL #cite("VEC2SPARQL") proposed exposing embedding similarity to SPARQL as extension
functions over knowledge-graph and image embeddings — the function-shaped alternative this
draft records in @sec-open. RDF2Vec #cite("RDF2VEC") established entity-embedding pipelines
for RDF graphs; this draft is agnostic about how vectors are produced and instead treats the
embedding function as an external artifact whose identity the store must record
(@sec-provenance-model). Relative to this prior work, the present draft's contribution is not
a similarity function but a pattern-level k-NN surface with declared execution modes, a
filtered-search answer-safety contract, and a reproducibility (provenance) record.

== Filtered and approximate nearest-neighbour search

The ANN backends this design admits follow HNSW #cite("HNSW"), DiskANN/Vamana
#cite("DiskANN"), and product quantisation #cite("PQ"). Predicate-constrained ("filtered")
ANN is its own literature: Filtered-DiskANN #cite("FILTERED-DISKANN") builds filter-aware
proximity graphs, and ACORN #cite("ACORN") targets predicate-agnostic search under arbitrary
structured constraints — precisely the workload a `vec:` pattern joined to ordinary triple
patterns produces. This draft does not standardise any algorithm from this family; it
standardises the answer-safety contract (@sec-filtered) that any of them must meet to be
admissible inside a SPARQL answer: the filtered result must be indistinguishable from
post-filtering the unfiltered result in the same execution mode. Rank fusion in the hybrid
proposal follows reciprocal-rank fusion #cite("RRF").

= The vec: vocabulary <sec-vocab>

The vector extension vocabulary lives in a single namespace:

```turtle
PREFIX vec: <http://sparq.dev/vec#>
```

Two predicate IRIs are recognised in this revision:

#table(
  columns: 2,
  align: (left, left),
  table.header[Predicate][Role],
  [`vec:nearest` (`http://sparq.dev/vec#nearest`)], [k-nearest-neighbour pattern binding the
    neighbour variable only (@sec-patterns).],
  [`vec:search` (`http://sparq.dev/vec#search`)], [k-nearest-neighbour pattern additionally
    binding a similarity score (@sec-patterns).],
)

A vec-extended processor MUST raise a query error when a triple pattern uses any other IRI in
the `vec:` namespace as its predicate. #rid("VG-VOC-1") Unrecognised `vec:` terms are reserved
for future revisions of this specification; treating them as ordinary (data-matched)
predicates would silently change meaning between revisions.

== Namespace governance and vocabulary versioning <sec-governance>

`http://sparq.dev/vec#` is a vendor namespace administered by the sparq project. For a
standards-track document that is a liability, not a virtue; this subsection states the
governance rules that hold while the namespace remains vendor-administered, and what happens
on adoption.

- #strong[Persistence.] The namespace IRI is stable. A term, once assigned a meaning by a
  revision of this specification, MUST NOT be removed or semantically redefined by a later
  revision; the only permitted lifecycle transition is deprecation (the term keeps its
  meaning and is marked deprecated). #rid("VG-GOV-1")
- #strong[Versioning.] Each revision of this specification carries a vocabulary revision
  number; this document defines #strong[vocabulary revision 1], whose recognised terms are
  exactly those of @sec-vocab. A namespace document SHOULD be published at the namespace IRI,
  listing every recognised term together with the revision that introduced it.
  #rid("VG-GOV-2")
- #strong[Extension.] New `vec:` terms are introduced only by a new revision of this
  specification, never ad hoc. Because of the unrecognised-term rule (VG-VOC-1), a query
  using a newer revision's term against an older processor fails loudly instead of silently
  matching data; that hard error #emph[is] the extension mechanism's forward-compatibility
  behaviour, and it is intentional. A processor SHOULD state, in its documentation and in its
  VG-VOC-1 error message, the highest vocabulary revision it implements. #rid("VG-GOV-3")
- #strong[Migration.] If a Working or Community Group adopts this document, terms are
  expected to move to a namespace under that group's control. The sparq project commits to
  publishing an equivalence mapping from `vec:` terms to their successors and to maintaining
  the vendor namespace, deprecated, for existing data and queries.

#note[
  #strong[Dereferenceability, confirmed against the deployment.] No namespace document is
  published at the namespace IRI, and none can be published from the sparq repository as it
  stands: the project's site is served from a #emph[different] host than the one the namespace
  IRI names, and the repository carries no route or asset that answers at `sparq.dev` at all.
  The namespace IRI therefore does not dereference. Satisfying VG-GOV-2 requires either serving
  the namespace document at the IRI this specification already fixed, or re-basing the
  vocabulary onto a host the project demonstrably serves — and the second option collides with
  the persistence rule above, which is exactly the liability a vendor-administered namespace
  carries. A Working or Community Group adopting this document inherits the choice
  (@sec-open). The machine-readable form of the per-term revision annotation remains unchosen
  independently of where the document is served.
]

= Vector search graph patterns <sec-patterns>

== Recognised patterns

A vec-extended processor MUST recognise the following two triple-pattern shapes inside a basic
graph pattern: #rid("VG-PAT-1")

```sparql
?node vec:nearest ( query k ) .
( ?node ?score ) vec:search ( query k ) .
```

Evaluating `vec:nearest` MUST bind `?node` to the `k` nearest neighbours of `query` in the
vector store under the store's declared metric (@sec-metric), best-first at generation time
(see @sec-ordering for what "best-first" does and does not guarantee downstream).
#rid("VG-PAT-2") Evaluating `vec:search` MUST produce the same neighbour bindings and MUST
additionally bind `?score` to the similarity between the query vector and each neighbour's
vector under the store's declared metric — in this revision, the cosine similarity — as an
`xsd:double` literal. #rid("VG-PAT-3")

== Argument grammar

The parenthesised forms are #strong[ordinary SPARQL RDF collections] — syntactic sugar for
`rdf:first`/`rdf:rest` lists per SPARQL 1.1 #cite("SPARQL11-QUERY") and RDF 1.1
#cite("RDF11-CONCEPTS") — not new grammar. The extension recognises the list structurally:

- The object of both predicates MUST be a two-element collection, exactly `( query k )`.
  #rid("VG-GRM-1")
- The subject of `vec:search` MUST be a two-element collection, exactly `( ?node ?score )`.
  #rid("VG-GRM-2")
- The neighbour position (`?node`) MUST be a variable. #rid("VG-GRM-3")
- The score position of `vec:search` (`?score`) MUST likewise be a variable. #rid("VG-GRM-4")

Both subject positions carry the same requirement for the same reason: each names a binding the
pattern #emph[produces]. A constant in either position asks the processor to produce a binding
for something that is already fixed, which cannot mean what its author intended — so it is an
error (VG-ERR-2, VG-ERR-7), not a filter and not an empty result. In particular a constant in the
score position MUST NOT be read as a similarity threshold; thresholding is expressed by an
ordinary `FILTER` over the bound score variable.

== Argument typing <sec-typing>

The `query` argument MUST be a constant, one of: #rid("VG-TYP-1")

+ a #strong[node IRI]: the query vector is that node's stored vector, and the seed node itself
  is excluded from the results; or
+ a #strong[literal] whose lexical form is a comma-separated list of floating-point values:
  the query vector is parsed from the literal, and its dimension MUST equal the store's
  declared dimension.

The `k` argument MUST be a constant, non-negative integer literal (a negative `k` is thereby
excluded by typing). #rid("VG-TYP-2")

== Error conditions

A vec-extended processor MUST fail the query with a hard query error (not an empty result) in
each of the following cases:

+ the predicate is an unrecognised IRI in the `vec:` namespace (@sec-vocab); #rid("VG-ERR-1")
+ the neighbour position is not a variable; #rid("VG-ERR-2")
+ the `query` or `k` argument is not a constant; #rid("VG-ERR-3")
+ either argument list has the wrong number of elements; #rid("VG-ERR-4")
+ the query vector's dimension does not equal the store's dimension; #rid("VG-ERR-5")
+ an argument list is malformed, dangling, or cyclic (the `rdf:first`/`rdf:rest` structure
  does not form a well-formed finite list); #rid("VG-ERR-6")
+ the score position of `vec:search` is not a variable (@sec-patterns). #rid("VG-ERR-7")

Failing loudly is a design decision: every one of these conditions indicates a query that
cannot mean what its author intended, and returning an empty (or worse, partial) result would
convert an authoring bug into a silent wrong answer.

== Result multiplicity and ordering <sec-ordering>

The solutions produced by a `vec:` pattern enter the surrounding query as an #strong[unordered]
solution multiset, exactly as if they had been written as an inline `VALUES` block: SPARQL
joins do not preserve generation order. Consequently:

- A consumer that requires best-first order MUST apply an explicit
  `ORDER BY DESC(?score)` over a `vec:search` score variable. #rid("VG-ORD-1")
- A processor MUST NOT promise any implicit ordering of `vec:nearest` results after joins.
  #rid("VG-ORD-2")

The score is the cosine similarity in the closed interval from −1 to 1 (see @sec-metric),
exposed as `xsd:double`.

#strong[Tie-breaking.] When candidates tie on score at the boundary of the top-k (the k-th
best and the next-best scores are equal), result-set #emph[membership] MUST be decided
deterministically. The #dfn[tie-break key] of a candidate is the N-Triples serialisation of
its RDF term #cite("RDF11-NTRIPLES"): an IRI serialises as `<`, the IRI, then `>`; a literal
as its quoted, escaped lexical form followed by its `^^<datatype-IRI>` or `@language-tag`
suffix. This key is fixed by the term alone — the serialisation grammar leaves a conforming
processor no freedom over it — so equal terms yield identical keys on every implementation. The
tied candidates are admitted in ascending Unicode-codepoint order of their tie-break keys until
`k` results are reached. #rid("VG-TIE-1") The key is total and stable because the mainline
embeddable domain is exactly IRIs and literals (see the #emph[vector store] definition in
@sec-conformance): no blank node is ever a candidate, so blank-node labels — which N-Triples
scopes to a single document and does #emph[not] hold stable across implementations — never
enter the ordering. A future revision that admits blank-node embedding MUST first assign each
blank node a canonical label under RDF Dataset Canonicalization #cite("RDFC10") so this
stability is preserved. Ties strictly inside the top-k need no rule: the solution multiset is
unordered, so the relative order of admitted equal-score rows is unobservable. This rule exists
so that two conforming answer-exact implementations return the #emph[same] top-k set on the
same store; without it, "exact" would not imply "reproducible" (see @sec-accuracy). An
implementation MAY realise the rule through any internal order that agrees with it.

== Empty and degenerate queries

The following degenerate cases MUST be handled as stated:

- If the `query` argument is a node IRI that is absent from the store (the node exists in the
  graph but was never embedded, or does not exist at all), the pattern MUST yield zero
  solutions — not an error, and not arbitrary neighbours. #rid("VG-DEG-1")
- If the query vector is all-zero, the pattern MUST yield zero solutions. (Cosine similarity
  against the zero vector is undefined; refusing to rank is the only honest answer.)
  #rid("VG-DEG-2")
- If `k` is zero, the pattern MUST yield zero solutions — not an error. #rid("VG-DEG-3")

== Worked example

The following query retrieves the ten reports most similar to a seed document, keeps only
those in a thematic class, and recovers best-first order explicitly:

```sparql
PREFIX vec: <http://sparq.dev/vec#>
PREFIX ex:  <http://example.org/>

SELECT ?report ?score WHERE {
  ( ?report ?score ) vec:search ( ex:seed-document 10 ) .
  ?report a ex:ClimateReport .
}
ORDER BY DESC(?score)
```

#note[
  The class constraint `?report a ex:ClimateReport` interacts with the vector pattern under
  the filtered-search semantics of @sec-filtered: the ten neighbours are the top ten
  #emph[within] the constrained candidate set, not the global top ten post-filtered (which
  could return fewer than ten).
]

= Answer-exact and approximate execution modes <sec-modes>

This specification distinguishes two normative execution modes:

+ #strong[Answer-exact mode] (the default). The processor MUST return exactly the true top-k
  under the store's declared metric (@sec-metric), with ties at the k boundary broken per
  VG-TIE-1 (@sec-ordering). #rid("VG-MODE-1") The exact backend is the only backend a
  processor MAY use without an explicit opt-in.
+ #strong[Approximate mode] (explicit opt-in). The processor MAY accelerate retrieval with an
  approximate index — for example an in-memory HNSW graph #cite("HNSW"), an on-disk
  DiskANN/Vamana graph #cite("DiskANN"), or product-quantisation candidate generation
  #cite("PQ") — whose recall is strictly below one.

For approximate mode, a processor:

- MUST require an explicit opt-in (a distinct entry point, feature flag, or equivalent) —
  approximate execution MUST NOT be substituted silently for answer-exact execution;
  #rid("VG-MODE-2")
- MUST surface, in its API contract and documentation, that results are approximate with
  recall below one; #rid("VG-MODE-3")
- MUST NOT represent approximate results as answer-exact to any downstream consumer.
  #rid("VG-MODE-4")

The rationale is the project's empirical-honesty rule: a SPARQL answer is a claim about the
data, and an ANN index can silently drop true answers. Retrieval-and-estimate workloads may
accept that trade; answer-serving workloads must not have it made for them.

#note[
  This document deliberately cites no recall or throughput figures: every measurement in the
  underlying estate is machine-local and non-canonical. A future revision backed by a
  canonical benchmark authority could add an informative recall annex; the normative content
  here does not depend on one.
]

Approximate-mode entry points are additionally staleness-guarded against the store contract
of @sec-staleness. Quantised representations (a scalar quantiser from 32-bit floats to 8-bit
integers, and a product quantiser with an asymmetric-distance table and a persistable
codebook) exist as storage/acceleration options; a processor using them for candidate
generation is in approximate mode and carries that mode's obligations.

= Filtered vector search <sec-filtered>

When a `vec:` pattern occurs in a basic graph pattern alongside ordinary triple patterns that
constrain the neighbour variable, the processor MAY evaluate the search #emph[filtered]: only
candidates satisfying the constraints are eligible neighbours, so the top-k is taken within
the eligible set. This is the standard "filtered ANN" problem (@sec-related), and it carries
the specification's central soundness obligation:

- The #dfn[candidate mask] MUST be derived from the #strong[connected component] of the
  neighbour variable in the join graph of the surrounding basic graph pattern — every triple
  pattern transitively join-connected to the neighbour variable participates, including
  cyclic sub-patterns. #rid("VG-FILT-1")
- The filtered top-k MUST equal the result of post-filtering the #emph[unfiltered] retrieval
  by that same transitive constraint, evaluated in the same execution mode — filtering is an
  optimisation, never a semantic change. #rid("VG-FILT-2")
- Every returned identifier MUST be a member of the mask. #rid("VG-FILT-3")
- An empty mask MUST yield zero solutions. #rid("VG-FILT-4")

#strong[Mode-relativity.] The equality of VG-FILT-2 is defined #emph[relative to the
execution mode] (@sec-modes). In answer-exact mode both sides are exact, so the filtered
result is exactly the true filtered top-k (with VG-TIE-1 ties). In approximate mode both
sides are the same approximate retrieval, so the invariant guarantees only that filtering
itself introduces no #emph[additional] error; it is not an exactness guarantee, and recall
remains below one. VG-FILT-2 must not be read as promising exact answers under approximate
execution.

A processor MAY choose between pre-filtering (masking candidates before ranking) and
post-filtering (ranking then discarding) on cost grounds; the choice MUST NOT be observable in
the result set. #rid("VG-FILT-5") (The sparq implementation drives this choice with a cost
model and an iterative over-fetch strategy for approximate backends; both are informative
here.)

A processor MAY cache derived masks across query preparations. A cached mask MUST be keyed by
both the constraining sub-pattern and the graph's fingerprint (@sec-staleness), and MUST be
invalidated when the fingerprint changes; serving a mask computed against a different graph
state is unsound. #rid("VG-FILT-6")

= Distance metric and normalisation <sec-metric>

== Declared metric

Every vector store MUST declare the distance metric and the normalisation regime under which
its vectors are comparable; the declaration is part of the provenance record of
@sec-provenance-model. #rid("VG-MET-1") Evaluation of the patterns of @sec-patterns is
defined relative to the store's declared metric.

This revision defines exactly one metric for the mainline query surface: #strong[cosine
similarity over L2-normalised vectors], with scores in the closed interval from −1 to 1.
Future revisions may register additional metrics through the extension mechanism of
@sec-governance. The mainline query surface offers no #emph[per-query] metric selection —
evaluation is always under the store's declared metric — and whether a per-query metric
argument is wanted remains an open question (@sec-open).

== Normalisation regime

For the cosine metric:

- Stored vectors MUST be L2-normalised at store-build time, before persistence.
  #rid("VG-MET-2")
- The query vector MUST be L2-normalised at query-evaluation time, before comparison.
  #rid("VG-MET-3")

On vectors normalised this way the cosine relates to Euclidean distance by
`cos = 1 − d²/2`, which is how an implementation may derive it from a Euclidean-native index
(informative).

#note[
  #strong[Where sparq normalises, confirmed against the implementation.] The two requirements
  above bite differently on the two execution modes, and the sparq build satisfies them
  unevenly.

  On the #emph[answer-exact] path nothing is normalised at either end. The primary `.spqv`
  container persists each vector exactly as it was supplied — the write path validates
  dimension, finiteness, and non-zeroness, and rejects an all-zero vector, but it does not
  rescale — so #strong[VG-MET-2 is not satisfied]. Instead the exact scan computes the cosine
  with both norms taken at comparison time, which is scale-invariant: the #emph[scores] are
  identical to those the normalised regime would produce, so VG-MET-3 is satisfied in effect
  rather than as a distinct query-time step. The residue of VG-MET-2 is therefore not about
  score values but about #emph[interchange]: a consumer that reads a `.spqv` and compares the
  raw `f32` rows itself — with a dot product, or through a Euclidean-native index via the
  `cos = 1 − d²/2` identity above — gets wrong answers from an unnormalised container, and the
  container declares nothing that would warn it (the VG-MET-1 gap of @sec-impl-status).

  On the #emph[approximate] path normalisation is explicit, but at #emph[index]-build time
  rather than store-build time: the in-memory graph caches L2-normalised copies of the store's
  vectors, the persisted `.spqg` node records hold the normalised vector (@sec-formats), and the
  query vector is normalised before the search — an all-zero query has no direction and yields
  no results, agreeing with VG-DEG-2 and with the exact path. That regime satisfies VG-MET-3
  literally and satisfies VG-MET-2's intent one container removed from where this specification
  places it.

  The gap this leaves is not that vectors are compared wrongly today; it is that "L2-normalised"
  is a property of neither the persisted primary container nor any declaration it carries, so
  nothing detects a store whose rows were never normalised being handed to a consumer that
  assumes they were. Closing it means normalising on the `.spqv` write path, or recording the
  normalisation regime in the provenance record and enforcing it — the latter being what
  VG-PROV-1 already REQUIRES.
]

== Cross-metric and cross-normalisation comparison

A processor MUST NOT evaluate a query against a store under a metric or normalisation regime
different from the store's declared record; on detecting the mismatch it MUST reject the
query rather than return well-formed but meaningless numbers. #rid("VG-MET-4")

#note[
  The sparq structure-aware store extension tags each typed-literal encoder block with a
  `Metric` value of `Euclidean` or `NonEuclidean` and rejects an attempt to apply a Euclidean
  or cosine comparison to a `NonEuclidean` block (for example, a hyperbolic taxonomy
  embedding). That guard is the block-granularity instance of VG-MET-4; it is described
  informatively because the container that carries it is not normatively specified in this
  draft (@sec-formats).
]

= Embedding provenance and versioning <sec-provenance-model>

Vector answers are only reproducible if the store can say what function produced its vectors.
Two stores built by different embedding models — or the same model at different versions, or
with different normalisation — are geometrically incomparable, yet produce well-formed,
meaningless numbers when mixed. This section therefore states the reproducibility record
normatively.

== The provenance record

A vector store MUST record: #rid("VG-PROV-1")

+ the embedding #strong[model identifier];
+ the #strong[model version] (or a content digest, where the provider exposes one);
+ the #strong[distance metric] (@sec-metric);
+ the #strong[normalisation regime] (@sec-metric); and
+ the #strong[dimension].

A vector store SHOULD additionally record the verbalisation regime (@sec-embedding) used to
produce the embedded text. #rid("VG-PROV-2")

== Compatibility rejection

A processor MUST reject a query whose query-vector provenance is incompatible with the
store's record — a different model identifier, model version, metric, normalisation regime,
or dimension — rather than return well-formed meaningless results. #rid("VG-PROV-3")

In the mainline surface the query vector arises three ways: a node-IRI seed draws its vector
from the store itself and is compatible by construction; a literal vector (@sec-typing)
carries no provenance; and a processor that embeds query #emph[text] at query time obtains a
vector from an embedding client (@sec-embedding), whose model identifier and version MUST
match the store's record under VG-PROV-3. For literal query vectors: the dimension check is
VG-ERR-5, and the literal MUST be treated as being under the store's declared metric and
normalisation regime (normalised per VG-MET-3); a processor MAY require an explicit override
before accepting literal query vectors at all. #rid("VG-PROV-4")

== RDF expression

The provenance record SHOULD be expressible in RDF, so datasets can describe their vector
indexes; the natural home is the `vec:` namespace (terms for the model identifier, model
version, metric, normalisation regime, and dimension of a store), aligned with PROV-O
#cite("PROV-O") for derivation. #rid("VG-PROV-5")

#ednote[
  The concrete `vec:` term names for the record, and the byte-level binding of the record
  into the persisted container (a version-3 header), are left to a future revision
  (@sec-formats). Until then the record's serialisation is implementation-defined, which
  impairs interchange but not the obligation to keep the record.
]

#note[
  The sparq implementation does #strong[not] yet satisfy the REQUIRED record by default: the
  default write path emits the version-2 container, which records the dimension and the graph
  fingerprint only. An opt-in feature can persist the full record in a version-3 header, and
  the record is expressible in RDF (VG-PROV-5), but until a record is required rather than
  optional this remains the most consequential known gap between this specification and its
  reference implementation, and closing it is the highest-priority item this draft asks
  reviewers to weigh (@sec-impl-status).
]

= Persisted vector store formats <sec-formats>

== The primary container

The primary container (file extension `.spqv`) is a flat, memory-mappable file holding
exactly one 32-bit-float vector per embedded RDF term identifier. All integers are
little-endian. A version-2 container MUST follow this layout, in order: #rid("VG-FMT-1")

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Field][Type / size][Meaning],
  [magic], [4 bytes], [the ASCII bytes `SPQV`],
  [version], [`u32`], [container version; this table describes version 2],
  [dimension], [`u32`, at byte offset 8], [vector dimensionality, one value for the whole
    store],
  [count], [`u64`], [number of stored vectors],
  [reserved], [12 bytes], [reserved for future use],
  [fingerprint], [24 bytes, at byte offset 32], [graph fingerprint: dictionary length,
    triple count, and content hash, each `u64` (@sec-staleness)],
  [data], [dense `f32`], [the vectors, densely packed],
  [index], [sorted pairs of `u32`], [an (identifier, slot) index sorted by identifier],
)

A store builder SHOULD support a streaming write path whose memory use is independent of the
store size #rid("VG-FMT-2"), and a reader SHOULD support opening from a byte slice (enabling
memory-mapped and in-memory use alike) #rid("VG-FMT-3").

The version-2 header records the dimension and the graph fingerprint — and nothing about the
embedding model that produced the vectors. It therefore #strong[cannot, by itself, satisfy
the REQUIRED provenance record] (VG-PROV-1): a vector store persisted only as a version-2
container does not conform to this revision. The byte-level binding of the provenance record
(a version-3 header) is left to a future revision; until it is pinned, a conforming store
must persist the record alongside the container in an implementation-defined form and SHOULD
also express it in RDF (VG-PROV-5). @sec-impl-status records that the sparq implementation
writes version 2 by default and does not yet satisfy VG-PROV-1.

== Sibling containers

#emph[This subsection is informative.] Three sibling containers exist in the sparq estate and
share the same magic-plus-version discipline — a four-byte ASCII magic, a `u32` version, and
little-endian integers throughout. Their layouts are recorded here as confirmed against the
implementation, so a future revision can adopt or delegate them from a known starting point
rather than from a blank.

`.spqg` (magic `SPQG`, version 2): an on-disk DiskANN/Vamana proximity graph for approximate
mode.

```text
0   magic        b"SPQG"                              4 bytes
4   version      u32 = 2                              4
8   dim          u32                                  4
12  degree R     u32   (max out-neighbours)           4
16  count        u64   (nodes)                        8
24  medoid       u32   (entry-point slot)             4
28  enc_tag      u32   (0 = none, 1 = PQ cache)       4
32  fingerprint  dict_len u64, triples u64, hash u64  24   (@sec-staleness)
56  nodes        count × node record

node record: id u32 | degree u32 | vector [dim] f32 | nbrs [R] u32 (slot indices)
optional PQ section (present iff enc_tag = 1, appended after the nodes):
  magic b"SPQP" | cb_len u64 | codebook [cb_len] bytes | stride u32 | codes
```

A version-1 `.spqg` has the same first 32 bytes and no fingerprint block; it still opens, and
is reported as unverifiable rather than silently assumed fresh. Note that the per-node vector
is stored #strong[L2-normalised] — the observation that pins the normalisation question of
@sec-metric.

`.spqd` (magic `SPQD`, version 1): a crash-durable incremental delta sidecar, tied to the store
generation it patches.

```text
0   magic         b"SPQD"                              4 bytes
4   version       u32 = 1                              4
8   dim           u32                                  4
12  append_count  u64                                  8
20  tomb_count    u64                                  8
28  reserved      zero                                 4
32  fingerprint   base graph fingerprint               24   (@sec-staleness)
56  appends       append_count × (id u32, [dim] f32)
    tombstones    tomb_count × (id u32)
```

The header fingerprint is the base generation the delta is keyed against, so replaying a delta
onto a base of a different generation is rejected rather than silently mis-keyed; an all-zero
block decodes as unbound, exactly as the `.spqv` fingerprint block does. The decoder recomputes
the exact expected total length from the header and rejects a file that is shorter #emph[or]
longer, so a truncated or trailing-garbage sidecar is a descriptive error rather than a short
replay.

`.spqs` (magic `SPQS`, version 2): a self-describing schema header for structure-aware
vectorisation.

```text
0   magic     b"SPQS"                              4 bytes
4   version   u32 = 2                              4
8   dim       u32   (total structured row width)   4
12  n_blocks  u32                                  4
16  blocks    n_blocks × block record

block record (v2, 14 bytes): encoder u8 | metric u8 | offset u32 | width u32 | weight f32
block record (v1, 10 bytes): as v2 without the trailing fusion weight
```

Each block carries the per-block `Metric` tag noted in @sec-metric. The parser re-validates that
the blocks partition `[0, dim)` contiguously with no gap or overlap and cross-checks the
recomputed width against the stored `dim` field, so a header that lies about its own dimension
is rejected. A version-1 header parses with every fusion weight defaulted.

These layouts remain #strong[outside this revision's normative surface] — not now because their
bytes are unknown, but because each is an implementation-internal acceleration or sidecar format
carrying its own independent version line, and a conformance class for them would freeze
implementation detail that the query semantics of this specification do not depend on. A future
revision must decide deliberately: adopt one or more of them normatively, or delegate all three
to a named container registry.

== Graph fingerprint and staleness contract <sec-staleness>

A vector store is only meaningful relative to the exact dictionary state of the graph it was
built from: the store maps #emph[term identifiers], not terms, to vectors. The staleness
contract is therefore normative:

- The fingerprint MUST comprise the dictionary length, the triple count, and a content hash
  computed over the dictionary's term #emph[set] in an identifier-order-independent manner.
  The hash MUST be stable across thread counts (parallel and serial ingest of the same data
  MUST fingerprint identically). #rid("VG-STAL-1")
- A fingerprint check before serving is #strong[necessary but not sufficient]: two graphs with
  identical term sets can assign identifiers in different orders, and the fingerprint —
  being deliberately order-independent — cannot tell them apart.
- To serve queries from a persisted store, the graph MUST have been round-tripped through the
  engine's native save/open path, which freezes identifier order. A graph #strong[re-parsed]
  from a source serialisation MUST NOT be paired with a previously persisted store, even when
  the fingerprint matches: identifier assignment may have changed, and every vector would
  silently attach to the wrong term. #rid("VG-STAL-2")

The last clause is the sharpest edge in the whole design; see also the integrity discussion in
@sec-security.

== Bulk import validation

A bulk import path (from `.npy` arrays or raw numeric dumps, under an explicit
row-to-identifier contract) MUST be fail-closed: the importer MUST reject wrong element type,
wrong shape, wrong row order, wrong length, and non-finite values, and MUST bound the bytes it
reads while parsing the `.npy` header. #rid("VG-IMP-1") Fail-open import would let a
malformed artifact manufacture plausible-looking neighbours.

= Embedding acquisition <sec-embedding>

== The embedding client contract

An embedding client MUST return, for a batch of text inputs, #strong[exactly one] vector per
input, #strong[in input order], each of exactly the declared dimension and containing only
finite values. #rid("VG-EMB-1")

When the vectors come from a live remote provider, the client MUST validate the response and
MUST reject it on any of: wrong vector count, wrong dimension, non-finite values, a duplicate
result index, a missing result index, or an out-of-range result index. #rid("VG-EMB-2")
Silently re-ordering or padding a provider response would attach vectors to the wrong terms —
the same failure class as the staleness edge of @sec-staleness.

== Remote provider protocol

The remote protocol is OpenAI-compatible #cite("OPENAI-EMB"):

- `POST` to the provider base URL's `/v1/embeddings` endpoint with a JSON body carrying the
  model identifier and the input batch, authenticated with a bearer token;
- the response's per-item #emph[index] field MUST be honoured when reassembling the batch
  (order of arrival is not trusted). #rid("VG-EMB-3")

Client configuration is drawn from the environment: `SPARQ_EMBEDDINGS_API_KEY` (REQUIRED),
`SPARQ_EMBEDDINGS_BASE_URL`, and `SPARQ_EMBEDDINGS_MODEL`, the latter two with implementation
defaults (the public OpenAI endpoint and a small text-embedding model, respectively). The
defaults are informative; the validation obligations above are not.

== Test-only embedders

The sparq estate ships a hash-based embedder for deterministic tests. It captures #strong[no]
semantics whatsoever — semantically related words are unrelated vectors under it. It MUST NOT
be shipped or configured as a retrieval feature; it exists so the machinery around embeddings
can be tested without a model. #rid("VG-EMB-4")

== Verbalisation

Entities are turned into embeddable text by a verbalisation pipeline (label-chain resolution
with multilingual support and a length budget). Verbalisation choices change what a vector
means; @sec-provenance-model folds the verbalisation regime into the provenance record
(VG-PROV-2). Note that verbalised text is exactly what leaves the deployment when a remote
provider is used (@sec-security).

= Hybrid symbolic–vector retrieval <sec-hybrid>

The hybrid surface this revision specifies is #strong[library-only]: reciprocal-rank fusion
#cite("RRF") (plain and weighted, with the conventional constant of sixty in the rank
denominator) and score fusion exist as host-language helpers that combine a vector ranking
with a symbolic (for example, full-text) ranking. #strong[No SPARQL syntax for hybrid fusion
is defined]; within a query, hybrid behaviour today is what you compose yourself from a
`vec:` pattern, other ranked sources, and explicit arithmetic over score bindings.

#proposal[
  A future revision may add an in-query hybrid surface: a fusion form (a `vec:hybrid` pattern
  or equivalent) exposing weighted reciprocal-rank fusion over two or more ranked
  sub-retrievals, and a second-stage rerank seam (for a cross-encoder) behind the same
  opt-in-and-flagged discipline as approximate mode (@sec-modes). Named and multi-vector
  stores (more than one vector per term, addressed by name) are likewise future-revision
  designs: the version-2 container holds exactly one vector per identifier. These are
  proposals; no conformance language attaches to them in this draft.
]

= Provenance-weighted retrieval <sec-pkg>

#emph[This section is informative.] The `secx:` prefix is now pinned against the implementation
(below), so the original blocker — a vocabulary term whose IRI is unknown cannot carry
conformance language — is cleared. A different one has taken its place: the implementation
recognises #emph[two] namespaces for the assurance classes and designates neither as canonical,
and a normative section cannot require recognition of an alias set without saying which IRI a
conforming producer writes. A future revision must designate one form canonical (retaining the
other as a recognised-on-input alias, or dropping it) before this section can carry conformance
language.

Retrieval over a knowledge graph whose triples carry assurance metadata can weight candidates
by how well-supported they are. The sparq implementation's reader recognises the following
vocabulary:

#table(
  columns: 2,
  align: (left, left),
  table.header[Term][Meaning],
  [`pkg:confidence` (`https://sparq.dev/ns/pkg#confidence`)], [a numeric confidence asserted
    for a statement],
  [`pkg:assurance` (`https://sparq.dev/ns/pkg#assurance`)], [an assurance class; recognised
    objects are `secx:Proven`, `secx:Claimed`, and `secx:Conjectured`],
  [`prov:wasDerivedFrom` (`http://www.w3.org/ns/prov#wasDerivedFrom`)], [PROV-O derivation
    #cite("PROV-O"), used to trace a statement to its source],
  [`pkg:discoveredFrom` (`https://sparq.dev/ns/pkg#discoveredFrom`)], [the discovery channel a
    statement arrived through — a sub-property of `prov:wasDerivedFrom`, scanned in addition to
    the super-property so lineage resolves on a graph whose sub-to-super entailment has not been
    materialised],
)

The `secx:` prefix, confirmed against the implementation, is #strong[two] namespaces rather than
one: the reader recognises each assurance class under both
`https://sparq.dev/ns/secx#` and `https://w3id.org/zkp-sparql/sec-prop#`, so
`https://sparq.dev/ns/secx#Proven` and `https://w3id.org/zkp-sparql/sec-prop#Proven` are treated
alike, and correspondingly for `Claimed` and `Conjectured`. The first is a sparq vendor form
carried by the project's own fixtures; the second is the `w3id.org` form of the sec-prop
extension the vocabulary reuses for this axis. Neither is designated canonical, which is why
this section stays informative.

The triple weight is computed as
`w(t) = clamp(assurance_mult × confidence × source_reliability, floor, 1.0)` and the reader
is #strong[fail-open]: on a plain graph carrying none of this vocabulary every weight is 1.0
and retrieval is unchanged.

#note[
  The fail-open default is a security property as much as an ergonomic one, and it cuts both
  ways: it means an attacker who cannot write the vocabulary cannot #emph[suppress] retrieval
  either, but also that omitting the vocabulary silently restores uniform weighting. See
  VG-SEC-3 in @sec-security.
]

#note[
  Provenance weighting ships with #strong[no accuracy claim]. The literature reports
  inconsistent gains from this family of techniques, and the implementation gates adoption
  behind an on/off ablation with an explicit adopt-only-on-measured-lift-else-abandon bar.
  This draft records the vocabulary and the weight formula so they can be reviewed; it does
  not recommend enabling the feature.
]

The adjacent structure-aware vectorisation estate (typed-literal encoders, taxonomy encoders
with Euclidean and hyperbolic geometries, SHACL/OWL prior extraction, knowledge-graph-embedding
training) similarly carries #strong[no accuracy claim] and is out of normative scope for this
draft. A propose-then-verify neuro-symbolic gate exists — vector search proposes candidate
triples and a deductive gate admits a candidate only if it introduces no new SHACL violation
and no new OWL inconsistency, failing closed — and is likewise recorded here informatively.

= Grounded generation over RDF <sec-grounded>

The generative-AI half of this extension is a natural-language question-answering loop that
is #strong[grounded] in the dataset: a language model writes SPARQL, but only the dataset
answers questions. A grounded-generation processor:

+ MUST ground generation in a machine-derived description of the dataset (for example schema
  cards derived from introspection: vocabulary usage, VoID-style statistics, SHACL shapes,
  characteristic sets) rather than letting the model guess the schema; #rid("VG-GG-1")
+ MUST validate a generated query before execution, and MUST NOT execute a query that fails
  validation (the loop instead enters repair); #rid("VG-GG-2")
+ MUST derive answers exclusively from the bindings returned by executing the validated query
  against the dataset — never from the language model's parametric knowledge; #rid("VG-GG-3")
+ MAY iterate a bounded generate–validate–execute–repair loop on failure; #rid("VG-GG-4")
+ SHOULD attach citations that trace each answer to the solution bindings (and thus the
  triples) that support it; #rid("VG-GG-5")
+ MUST NOT present an assurance qualifier derived from #emph[asserted] metadata
  (@sec-pkg) as a #emph[calibrated] confidence. The qualifier band reflects what the data
  claims about itself; no reliability measurement backs it, and a future revision can only
  upgrade this once calibration is measured. #rid("VG-GG-6")

Deterministic replay of recorded model completions is the testing regime this draft assumes
for the loop. Replay-based scores validate the #emph[mechanism] — the loop, the validator,
the harness — and MUST NOT be quoted as any real model's accuracy. #rid("VG-GG-7")

#proposal[
  Grammar-constrained (logit-masked) decoding of SPARQL — constraining generation itself so
  only grammatical queries and in-dictionary terms can be produced — is a future-revision
  design; the current sparq implementation applies a post-hoc dictionary constraint only. A
  future revision may specify constrained decoding as a SHOULD for grounded-generation
  processors once implementation experience exists.
]

= Accuracy and determinism considerations <sec-accuracy>

This section states plainly what this extension does #strong[not] promise.

- #strong[No accuracy figures are part of this specification.] Every recall, fidelity, and
  answer-quality measurement in the underlying estate was taken on a non-canonical work
  machine; none is citable as a floor. Conformance to this draft is a property of semantics
  (exactness, error behaviour, validation), never of a benchmark score.
- #strong[Approximate mode is honestly approximate.] Its recall is below one by construction;
  the normative obligations of @sec-modes exist so no consumer mistakes it for answer-exact,
  and the filtered-search invariant is exactness-preserving only relative to the mode
  (@sec-filtered).
- #strong[Exactness implies reproducibility only together with the tie rule.] This revision
  pins a deterministic boundary tie-break (VG-TIE-1, @sec-ordering), so two implementations
  conforming to answer-exact mode return the same top-k #emph[set] on the same store. The
  sparq build implements VG-TIE-1 on its answer-exact paths — unfiltered and mask-filtered
  alike — and enforces the embeddable-domain restriction the rule relies on fail-closed: a
  candidate containing a blank node is a hard query error wherever its term, rather than its
  score alone, would decide top-k membership (@sec-impl-status).
- #strong[Embedding models are external nondeterminism.] Two embedding runs are comparable
  only under identical model, version, normalisation, and verbalisation — which the REQUIRED
  provenance record of @sec-provenance-model exists to attest. Until an implementation
  persists that record (@sec-impl-status), "same query, same store" reproducibility holds,
  but "rebuild the store, get the same answers" does not in general.
- #strong[The learned-feature estate makes no claims.] Knowledge-graph-embedding training and
  provenance weighting are explicitly no-accuracy-claim features gated on unmeasured lift
  (@sec-pkg).

= Security and privacy considerations <sec-security>

#strong[Data disclosure to embedding providers.] Under the remote protocol of
@sec-embedding, the verbalised text of entities — labels, literals, and whatever else the
verbalisation budget admits — is transmitted to a third-party provider. Deployments MUST
treat everything embedded through a remote provider as disclosed to that provider, and
SHOULD NOT embed graphs containing personal or confidential data through an external endpoint
absent a data-processing agreement. #rid("VG-SEC-1") Bearer credentials are read from the
environment (`SPARQ_EMBEDDINGS_API_KEY`); standard secret-handling hygiene applies, and
endpoints SHOULD be HTTPS (the implementation default is).

#strong[Embedding inversion and membership inference.] Dense embeddings are not an
anonymisation: published research demonstrates partial reconstruction of source text from
embeddings and membership inference against embedding stores. A `.spqv` file derived from
sensitive text SHOULD be access-controlled as if it were the text. (Informative; the
grounding estate takes no position beyond this warning.)

#strong[Malformed artifacts.] The fail-closed import obligations (@sec-formats) — typed,
bounded, finite-checked parsing of foreign arrays — and the fail-closed provider-response
validation (@sec-embedding) are this extension's parsing attack surface controls.

#strong[Opening an untrusted container.] Importing foreign numeric data (VG-IMP-1) and
#emph[opening a container] are distinct attack surfaces, and this revision states the second
normatively:

- A processor opening a vector store from an untrusted source MUST validate the container
  before any vector read: the magic and version; every length-prefixed header block against the
  remaining length; the total file length against the size implied by the declared dimension and
  count, computed so that an overflowing declaration is rejected rather than wrapping; and every
  entry of the identifier index, so that identifiers are strictly ascending and every slot is in
  range and used exactly once. After that pass no subsequent read may depend on further
  validation. #rid("VG-SEC-4")
- A successful open MUST NOT be treated as evidence of a container's authenticity or
  provenance. Validation establishes well-formedness only. #rid("VG-SEC-5")

VG-SEC-5 is the sharper of the two. A #emph[well-formed] `.spqv` is entirely attacker-authorable:
its vectors are arbitrary, and its embedded graph fingerprint (@sec-staleness) is a plain header
field, so a hostile store can be made to pass the staleness check against a graph it was never
built from and then return whatever neighbours its author chose. The fingerprint is an integrity
control against #emph[accident] — a stale or mismatched store — and not against an adversary.
Deployments that accept stores from outside their trust boundary need an authenticity control of
their own (a signature over the container bytes, or transport the store never leaves), and this
draft does not specify one.

#note[
  The sparq implementation satisfies VG-SEC-4 for the primary container: the header, the exact
  total length under checked arithmetic, and the whole identifier index are validated eagerly on
  open — from a memory map and from an in-memory byte buffer alike, through the same validator —
  so the vector data itself, which is left to be paged in lazily, is reached only behind an index
  already proved consistent. Big-endian targets are rejected outright rather than
  byte-swapped. The property that this validator neither panics nor reads out of bounds for
  #emph[any] input buffer carries a bounded model-checking proof over all buffers up to a small
  size, complementing coverage-guided fuzzing, a deterministic corruption sweep, and
  sanitiser runs, which reach further but only on the inputs they happen to generate. The
  sibling containers of @sec-formats are validated in the same style — exact-length checks with
  descriptive errors rather than short reads — though without the model-checking proof.

  Nothing in the estate addresses VG-SEC-5: sparq has no signature over, or authenticated
  channel for, a `.spqv` container.
]

#strong[Integrity via the staleness contract.] Serving vectors against the wrong dictionary
state silently attaches every answer to the wrong term (@sec-staleness). The fingerprint
check and the round-trip-only rule are integrity controls, not performance niceties; the
mask-cache invalidation rule of @sec-filtered is the same control applied to cached derived
state.

#strong[Approximation as an integrity property.] An ANN index can drop true answers. In
adversarial settings the gap between approximate and exact answers is itself a manipulation
surface (an attacker who can influence index construction can influence what goes missing);
answer-serving paths MUST use answer-exact mode per @sec-modes. #rid("VG-SEC-2")

#strong[Prompt injection in grounded generation.] A grounded-generation processor feeds
model-bound context derived from the dataset (schema cards, labels), and dataset content can
embed adversarial instructions. The validate-before-execute obligation of @sec-grounded is
the primary control: whatever the model was talked into writing, only a validated SPARQL
query executes, and answers come only from its bindings. Deployments SHOULD additionally
restrict the execution surface available to generated queries (for example, to read-only
operations); this draft does not yet pin that restriction normatively.

#strong[Provenance metadata is attacker-writable data.] The vocabulary of @sec-pkg weights
retrieval by #emph[asserted] confidence and assurance. Anyone who can write
`pkg:confidence` triples can bias retrieval; the weights inherit the write-access-control
story of the graph itself, and the fail-open default means an attacker can also simply omit
the vocabulary. Consumers MUST NOT treat provenance-weighted rank as evidence of truth.
#rid("VG-SEC-3")

= Open design questions <sec-open>

The following are the known open questions a Working or Community Group adopting this draft
would need to settle. Each is a real gap in the design, recorded honestly rather than papered
over:

+ #strong[Variable-driven queries.] The `query` argument is constant-only (@sec-typing); a
  per-row (correlated / lateral) k-NN — "for each ?paper, its five nearest" — is
  inexpressible. Options include lateral-join semantics or relaxing the constant rule with
  defined binding-time semantics.
+ #strong[Surface form.] Only the magic-predicate form is specified. A `SERVICE`-based form
  and an extension-function form were considered in the estate's design records, and vendor
  idioms split across all three shapes (@sec-related). A standard should pick one normative
  form and define the others' equivalence or absence.
+ #strong[Per-query metric selection.] This revision requires a store-declared metric
  (@sec-metric) but offers no per-query metric argument; whether one is wanted — and how it
  would interact with VG-MET-4's rejection rule — is open.
+ #strong[Named / multi-vector stores and in-query hybrid.] Both are future-revision designs
  (@sec-hybrid).
+ #strong[Container bindings.] The byte-level binding of the provenance record (a version-3
  `.spqv` header) still needs to be pinned or delegated to a registry (@sec-formats). The
  sibling-container layouts are now recorded (@sec-formats) but the adopt-or-delegate decision
  is open, and the `secx:` vocabulary is now pinned to two namespaces (@sec-pkg) of which one
  must be designated canonical.
+ #strong[Namespace dereferenceability.] The namespace IRI this document fixes is not served by
  the project (@sec-governance), so VG-GOV-2 cannot be satisfied without either serving it or
  re-basing the vocabulary against the persistence rule of VG-GOV-1.
+ #strong[Container authenticity.] VG-SEC-5 (@sec-security) states that a valid container is not
  an authentic one, but this draft specifies no authenticity mechanism. Whether one belongs in
  this specification — a signature block in the container, or an out-of-band obligation — is
  open.
+ #strong[Calibrated confidence.] Upgrading the asserted-assurance band of @sec-grounded to a
  measured, calibrated confidence requires a reliability evaluation that does not yet exist.

= Testable assertions and conformance testing <sec-testable>

== Assertion identifiers

Every normative assertion in this document carries a stable identifier of the form
`[VG-<group>-<n>]`, rendered at the assertion site. Identifiers are never reused; a withdrawn
assertion's identifier is retired, not reassigned. The groups:

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Group][Section][Scope],
  [`VG-VOC`], [@sec-vocab], [vocabulary recognition],
  [`VG-GOV`], [@sec-governance], [namespace governance and versioning],
  [`VG-PAT`, `VG-GRM`, `VG-TYP`, `VG-ERR`, `VG-ORD`, `VG-TIE`, `VG-DEG`], [@sec-patterns],
    [query patterns: recognition, grammar, typing, errors, ordering, ties, degenerate cases],
  [`VG-MODE`], [@sec-modes], [answer-exact and approximate execution modes],
  [`VG-FILT`], [@sec-filtered], [filtered-search answer safety],
  [`VG-MET`], [@sec-metric], [metric and normalisation declaration],
  [`VG-PROV`], [@sec-provenance-model], [embedding provenance record and rejection],
  [`VG-FMT`, `VG-STAL`, `VG-IMP`], [@sec-formats], [container layout, staleness, import],
  [`VG-EMB`], [@sec-embedding], [embedding acquisition],
  [`VG-GG`], [@sec-grounded], [grounded generation],
  [`VG-SEC`], [@sec-security], [deployment obligations and container trust],
)

== Conformance test suite

#strong[No public conformance test suite accompanies this draft yet.] The sparq repository's
crate tests exercise the implemented assertions and are the seed corpus, but they are engine
tests, not an implementation-independent suite. The commitment this draft makes is: before
this document advances beyond Unofficial Proposal Draft, the project intends to publish a
runnable conformance suite in which every `VG-` assertion is paired with at least one test,
together with machine-readable (EARL-style) implementation reports; an assertion that cannot
be paired with a test will be reworded until testable, or demoted to informative.
@sec-impl-status is the first — hand-written — implementation report against the assertion
inventory.

= sparq implementation status <sec-impl-status>

#emph[This section is informative.] It reports, per assertion group, whether the current
sparq build satisfies this revision. It was originally derived from the project's read-only
estate recon digest (`research/specs/vector-genai-estate-recon.md`) rather than a fresh code
audit — the first revision was deliberately authored without opening crate sources. The rows
that revision marked #emph[unconfirmed] have since been checked against the crate sources, and
the `VG-GOV-3`, `VG-TIE-1`, `VG-MET-4`, and `VG-PROV-5` rows record behaviour implemented and
tested after the digest was taken; the table below is maintained against the code. Two of those
checks changed a verdict rather than merely confirming one: `VG-MET-2` moved from
#emph[unconfirmed] to #emph[not satisfied] (@sec-metric), and `VG-GOV-2` is now known to be
unsatisfiable from this repository rather than merely unimplemented (@sec-governance).

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Assertions][sparq status][Notes],
  [`VG-VOC-1`], [Implemented, tested], [unrecognised `vec:` IRIs are a hard query error],
  [`VG-GOV-1`], [Satisfied], [no term has been removed or redefined; this is revision 1],
  [`VG-GOV-2`], [Not satisfied — unsatisfiable as the estate stands], [the `vec:` namespace IRI
    is not served by the project's site, which publishes under a different host, and the
    repository carries no asset that would answer at it; the machine-readable per-term revision
    annotation is separately unchosen (@sec-governance)],
  [`VG-GOV-3`], [Implemented, tested], [the build states vocabulary revision 1 in its crate
    documentation and names it in the VG-VOC-1 unrecognised-term hard error],
  [`VG-PAT-1..3`, `VG-GRM-1..4`, `VG-TYP-1..2`, `VG-ERR-1..7`], [Implemented, tested],
    [both `vec:search` subject positions go through one bare-variable requirement, so a
    non-variable #emph[score] position is a hard query error exactly as a non-variable
    neighbour position is (VG-GRM-4 / VG-ERR-7) — confirmed and pinned by a test],
  [`VG-ORD-1..2`], [Implemented], [unordered `VALUES`-equivalent multiset semantics],
  [`VG-TIE-1`], [Implemented, tested], [boundary-tie membership by ascending N-Triples key on
    both answer-exact paths (unfiltered and mask-filtered); a candidate containing a blank
    node is rejected with a fail-closed hard error wherever its term — rather than its score
    alone — would decide membership. The build additionally embeds ground triple terms, an
    extension beyond the mainline embeddable domain whose `<<( … )>>` keys are likewise fixed
    by the serialisation grammar],
  [`VG-DEG-1..2`], [Implemented, tested], [absent seed and all-zero query yield zero
    solutions],
  [`VG-DEG-3`], [Implemented, tested], [`k` is typed as a non-negative integer, so `0` parses
    and reaches the search, where both answer-exact entry points return the empty result — not
    an error — for `vec:nearest` and `vec:search`, by vector seed and by node seed alike;
    confirmed and pinned by a test],
  [`VG-MODE-1..4`], [Implemented], [exact full scan is the default; approximate backends are
    opt-in and documented as recall-below-one],
  [`VG-FILT-1..6`], [Implemented, tested], [mask derivation, post-filter equality, and
    fingerprint-keyed cache invalidation],
  [`VG-MET-1`], [Not satisfied], [no metric or normalisation regime is recorded anywhere;
    cosine is implicit in the mainline path],
  [`VG-MET-2`], [Not satisfied (primary container)], [the `.spqv` write path persists vectors as
    supplied — it validates dimension, finiteness, and non-zeroness but does not rescale — so
    stored vectors are not L2-normalised before persistence. The `.spqg` proximity graph does
    store normalised vectors, one container removed from where this assertion places the
    requirement (@sec-metric)],
  [`VG-MET-3`], [Satisfied], [explicitly on the approximate path (the query vector is normalised
    before the search); in effect on the answer-exact path, whose cosine takes both norms at
    comparison time and is therefore scale-invariant, yielding the scores the normalised regime
    would yield (@sec-metric)],
  [`VG-MET-4`], [Implemented, tested (declared metrics)], [the mainline query surface rejects
    a store whose persisted provenance declares a non-cosine metric with a hard error; a
    legacy store carrying no record declares nothing, so nothing can be checked against it —
    that absence is the VG-MET-1 gap, not a VG-MET-4 one. The structure-feature per-block
    `Euclidean`/`NonEuclidean` guard remains the block-granularity instance],
  [`VG-PROV-1..4`], [Not satisfied (default build)], [#strong[the headline gap]: the default
    write path emits the version-2 container, which records dimension and graph fingerprint
    only; an opt-in feature can persist the full record in a version-3 header, but a record
    remains optional, so the MUST of VG-PROV-1 — and the rejection rules that depend on a
    record being present — are not met by default],
  [`VG-PROV-5`], [Implemented, tested], [the provenance record is expressible as RDF triples
    (model, metric, normalisation, and dimension always; version and verbalisation axes when
    present) in a sparq vendor provenance vocabulary — the concrete `vec:` term names remain
    left to a future revision per the editor's note of @sec-provenance-model],
  [`VG-FMT-1..3`], [Implemented, tested], [version-2 layout, streaming writer, byte-slice
    open],
  [`VG-STAL-1..2`], [Implemented, tested], [order-independent fingerprint; round-trip-only
    serving rule],
  [`VG-IMP-1`], [Implemented, tested], [fail-closed `.npy`/numeric-dump import],
  [`VG-EMB-1..4`], [Implemented, tested], [client contract, response validation, index
    honouring; the hash embedder is test-only],
  [`VG-GG-1..7`], [Implemented], [in the adjacent sparq-nlq / sparq-introspect crates;
    replay-based mechanism testing; citations feature; the assurance band is documented as
    uncalibrated],
  [`VG-SEC-1..3`], [Deployment obligations], [not machine-checkable per se; the HTTPS
    default of @sec-embedding is implemented],
  [`VG-SEC-4`], [Implemented, tested], [header, exact total length under checked arithmetic, and
    the whole identifier index are validated eagerly on open — from a memory map and from an
    in-memory buffer through one validator; big-endian targets are refused. The
    no-panic/no-out-of-bounds property carries a bounded model-checking proof alongside fuzz,
    corruption-sweep, and sanitiser coverage (@sec-security)],
  [`VG-SEC-5`], [Not addressed], [there is no signature over, and no authenticated channel for,
    a `.spqv` container; a well-formed hostile store can carry arbitrary vectors #emph[and] a
    forged graph fingerprint, so it passes the staleness check of @sec-staleness],
  [Proposals (@sec-hybrid, @sec-grounded)], [Not built], [in-query fusion, rerank,
    multi-vector stores, constrained decoding — outside the normative surface],
)

Summary: the sparq build conforms to the retrieval core of this revision (patterns, modes,
filtered answer-safety, staleness, import, embedding acquisition, container validation, grounded
generation) but #strong[does not conform to this revision as a whole]. No row is unconfirmed any
longer. The known deltas are `VG-GOV-2`, `VG-MET-1`, `VG-MET-2`, `VG-PROV-1..4`, and `VG-SEC-5`
— and they are one story rather than five: the estate does not record what its vectors are, so
it cannot enforce that they are comparable (`VG-MET-1`, `VG-MET-2`, `VG-PROV-1..4`), cannot
publish what its terms mean (`VG-GOV-2`), and cannot attest where a container came from
(`VG-SEC-5`).

= References

#references((
  ("RFC2119", [Bradner, S. #emph[Key words for use in RFCs to Indicate Requirement Levels].
    RFC 2119, IETF, March 1997.]),
  ("RFC8174", [Leiba, B. #emph[Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words].
    RFC 8174, IETF, May 2017.]),
  ("SPARQL11-QUERY", [Harris, S.; Seaborne, A. (eds.) #emph[SPARQL 1.1 Query Language].
    W3C Recommendation, 21 March 2013. https://www.w3.org/TR/sparql11-query/.]),
  ("RDF11-CONCEPTS", [Cyganiak, R.; Wood, D.; Lanthaler, M. (eds.) #emph[RDF 1.1 Concepts and
    Abstract Syntax]. W3C Recommendation, 25 February 2014.
    https://www.w3.org/TR/rdf11-concepts/.]),
  ("RDF11-NTRIPLES", [Beckett, D. (ed.) #emph[RDF 1.1 N-Triples: A line-based syntax for an RDF
    graph]. W3C Recommendation, 25 February 2014. https://www.w3.org/TR/n-triples/.]),
  ("RDFC10", [Longley, D.; Kellogg, G. (eds.) #emph[RDF Dataset Canonicalization (RDFC-1.0)].
    W3C Recommendation, 2024. https://www.w3.org/TR/rdf-canon/.]),
  ("PROV-O", [Lebo, T.; Sahoo, S.; McGuinness, D. (eds.) #emph[PROV-O: The PROV Ontology].
    W3C Recommendation, 30 April 2013. https://www.w3.org/TR/prov-o/.]),
  ("HNSW", [Malkov, Y.; Yashunin, D. #emph[Efficient and Robust Approximate Nearest Neighbor
    Search Using Hierarchical Navigable Small World Graphs]. IEEE TPAMI, 2020.]),
  ("DiskANN", [Subramanya, S. J. et al. #emph[DiskANN: Fast Accurate Billion-point Nearest
    Neighbor Search on a Single Node]. NeurIPS, 2019.]),
  ("FILTERED-DISKANN", [Gollapudi, S. et al. #emph[Filtered-DiskANN: Graph Algorithms for
    Approximate Nearest Neighbor Search with Filters]. WWW, 2023.]),
  ("ACORN", [Patel, L.; Kraft, P.; Guestrin, C.; Zaharia, M. #emph[ACORN: Performant and
    Predicate-Agnostic Search Over Vector Embeddings and Structured Data]. SIGMOD, 2024.]),
  ("PQ", [Jégou, H.; Douze, M.; Schmid, C. #emph[Product Quantization for Nearest Neighbor
    Search]. IEEE TPAMI, 2011.]),
  ("RRF", [Cormack, G. V.; Clarke, C. L. A.; Büttcher, S. #emph[Reciprocal Rank Fusion
    Outperforms Condorcet and Individual Rank Learning Methods]. SIGIR, 2009.]),
  ("VEC2SPARQL", [Kulmanov, M. et al. #emph[Vec2SPARQL: Integrating SPARQL Queries and
    Knowledge Graph Embeddings]. bioRxiv, 2018.]),
  ("RDF2VEC", [Ristoski, P.; Paulheim, H. #emph[RDF2Vec: RDF Graph Embeddings for Data
    Mining]. ISWC, 2016.]),
  ("OPENAI-EMB", [OpenAI. #emph[Embeddings API reference (v1/embeddings)].
    https://platform.openai.com/docs/api-reference/embeddings. (Vendor documentation;
    catalogued, not measured.)]),
  ("GRAPHDB-SIM", [Ontotext. #emph[GraphDB Semantic Similarity Searches]. Vendor
    documentation; catalogued, not measured.]),
  ("NEO4J-VEC", [Neo4j. #emph[Vector Indexes]. Vendor documentation; catalogued, not
    measured.]),
  ("NEPTUNE-AN", [Amazon Web Services. #emph[Vector Similarity in Neptune Analytics]. Vendor
    documentation; catalogued, not measured.]),
  ("STARDOG-VB", [Stardog. #emph[Voicebox]. Vendor documentation; catalogued, not
    measured.]),
  ("PGVECTOR", [pgvector contributors. #emph[pgvector: Open-source Vector Similarity Search
    for Postgres]. https://github.com/pgvector/pgvector. (Vendor documentation; catalogued,
    not measured.)]),
))
