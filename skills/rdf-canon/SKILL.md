---
name: rdf-canon
description: Canonicalize an RDF dataset with RDFC-1.0 (the W3C RDF Dataset Canonicalization, the URDNA2015 successor) using the opt-in sparq-canon crate — turn a set of oxrdf quads (or one graph's triples) into a deterministic, blank-node-relabelled canonical N-Quads string, or get just the canonical blank-node issuer map. Use when you need to hash, sign, diff, deduplicate, or content-address an RDF dataset, or to test two graphs for RDF-isomorphism (identical canonical form). Native + wasm; does not touch sparq-core's default build.
---

# sparq-canon — RDFC-1.0 dataset canonicalization

[RDFC-1.0](https://www.w3.org/TR/rdf-canon/) (the W3C **RDF Dataset
Canonicalization**, the URDNA2015 successor) computes a deterministic form of an
RDF dataset: blank nodes are relabelled to `c14n0`, `c14n1`, … and the quads are
emitted in a stable code-point order. Two datasets are **RDF-isomorphic iff their
canonical N-Quads serializations are byte-for-byte identical** — the basis for
dataset hashing, signing, diffing, deduplication, and content-addressing.

`sparq-canon` is the **opt-in public surface** for this. Add it explicitly; it is
**not** in sparq's default build (`sparq-core` stays lean, the wasm artifact is
unchanged unless you pull this in). The RDFC-1.0 algorithm itself is the
maintained zkp-ld [`rdf-canon`](https://crates.io/crates/rdf-canon) crate;
`sparq-canon` owns the single oxrdf-0.3 ↔ oxrdf-0.2 canonical-N-Quads bridge and
exposes a clean API over sparq's term model. `sparq-zk` depends on it.

## Add the dependency

```toml
[dependencies]
sparq-canon = { path = "crates/sparq-canon" }   # or your workspace path
oxrdf = { version = "0.3", features = ["rdf-12"] }
```

## Dataset API — quads in, canonical N-Quads out

The general case. Pass any slice of `oxrdf::Quad` (named graphs and the default
graph both work); get back the canonical N-Quads document.

```rust
use oxrdf::{Quad, NamedNode, NamedOrBlankNode, Term, Literal, GraphName, BlankNode};

let q = Quad::new(
    NamedOrBlankNode::BlankNode(BlankNode::new("x").unwrap()),
    NamedNode::new("http://example.org/p").unwrap(),
    Term::Literal(Literal::new_simple_literal("v")),
    GraphName::DefaultGraph,
);

// Full canonical N-Quads (input blank-node labels are erased -> c14nN):
let nquads: String = sparq_canon::canonicalize(&[q.clone()]).unwrap();
assert!(nquads.contains("_:c14n0"));

// Just the issuer map: input blank-node label -> canonical label.
let map = sparq_canon::issued_identifiers(&[q]).unwrap();
assert_eq!(map.get("x").map(String::as_str), Some("c14n0"));
```

`canonicalize_quads` / `issue_quads` are aliases (the `rdf_canon`-style names).

### Text in, text out (`canonicalize_nquads`)

When you already hold serialized RDF — e.g. across a language boundary — skip the
oxrdf term plumbing: `canonicalize_nquads(&str) -> Result<String, _>` parses an
N-Quads document and returns its canonical N-Quads (`parse_nquads` exposes the parse
alone). This is the seam the `@sparq-org/sparq` RDF/JS `Dataset` uses for
isomorphism-aware `toCanonical` / `equals` / `contains`, surfaced over wasm as the
`canonicalizeNQuads(nquads)` binding behind `sparq-wasm`'s opt-in `canon` feature.

```rust
let canon = sparq_canon::canonicalize_nquads(
    "_:b0 <http://ex/p> _:b1 .\n_:b1 <http://ex/q> \"v\" .\n",
).unwrap();
assert!(canon.contains("_:c14n"));
```

### Hash profiles and final digests

The spec default is SHA-256. To use the SHA-384 profile (or any
`digest::Digest`), use the `*_with` functions; `Digest` is re-exported so you do
not need a direct `digest` dependency:

```rust
let nq = sparq_canon::canonicalize_quads_with::<sha2::Sha384>(&dataset).unwrap();
```

To hash the exact standard canonical N-Quads bytes in one call, including the
final newline on a non-empty dataset, supply the final digest algorithm to
`digest_quads_with`:

```rust
let digest = sparq_canon::digest_quads_with::<sha2::Sha256>(&dataset).unwrap();
assert_eq!(digest.len(), 32);
```

Here `D` selects only the digest of the finished canonical document;
canonicalization still follows `canonicalize_quads` and its default RDFC-1.0
hash profile. Callers choose the digest crate and algorithm; `sparq-canon` adds
no default digest alias.

## Single-graph API — one graph's triples

When you have one graph's content (a default-graph-only dataset), use the
triple-level API. It returns a `CanonicalGraph` with the sorted canonical lines
and the re-parsed canonical triples (line index = a stable total order — this is
exactly what the ZK per-graph commitment pipeline uses as the leaf order).

```rust
use sparq_canon::{canonicalize_triples, canonicalize_graph_content};

let canon = canonicalize_triples(&triples).unwrap();
let doc: String = canon.to_nquads();         // joined lines, each + '\n'
let n = canon.lines.len();                    // = canon.triples.len()

// Or straight from a stored graph:
let canon = canonicalize_graph_content(&graph).unwrap();   // graph: &sparq_core::Graph
```

`graph_triples(&graph)` materializes a stored graph's triples as `oxrdf::Triple`s
if you want them without canonicalizing.

## Errors — fail closed

`CanonError` is `#[non_exhaustive]` (variants have been added ungated as the
crate grew — keep a wildcard arm) and has five variants:

- `TripleTerm` — the dataset contains an RDF 1.2 triple term as an object; these
  are outside W3C RDFC-1.0's data model, so the **standard** paths fail closed.
  Enable the opt-in `rdf12-triple-terms` profile (below) to canonicalize them.
- `NestedBlankNode` — a blank node occurs **inside** a triple term. Only the
  opt-in profile's constrained `*_ground_terms` entry points (below) construct
  it: they require blank-node-free (ground) triple terms and fail closed
  otherwise. The variant itself is always present (not feature-gated), so
  matching on `CanonError` is identical in both feature states.
- `TripleTermDepthExceeded` — a triple term nests deeper than the crate-wide
  `MAX_TRIPLE_TERM_DEPTH` bound (128). Only the opt-in profile's entry points
  construct it: they pre-check depth **iteratively** before any recursive
  descent (HNDQ gossip, relabelling, serialization — and oxrdf's own
  `Drop`/`Clone` recurse), so adversarially deep nesting fails closed instead
  of risking a stack overflow. Ungated, like `NestedBlankNode`.
- `Canonicalization(String)` — `rdf-canon` rejected the dataset. This includes
  the **HNDQ call-limit guard**: RDFC-1.0 has pathological-input blow-ups, so a
  poison graph trips the limit and fails closed rather than running unbounded.
- `Bridge(String)` — an internal serialize/parse error (should not occur for
  well-formed RDFC-1.0-model input; surfaced rather than swallowed).

## ⚠️ Opt-in NON-STANDARD RDF 1.2 triple-term profile

**OFF by default; NOT W3C RDFC-1.0.** RDFC-1.0 is an RDF-1.1-only spec with no
notion of triple terms. W3C has published no RDF-1.2 dataset canonicalization
specification. With the feature OFF, behaviour is byte-identical to the standard
surface (triple terms still raise `CanonError::TripleTerm`; the W3C suite still
passes).

Enable the cargo feature to opt in to a **separate, clearly non-standard** v2
profile. It natively re-implements the RDFC-1.0 algorithm over oxrdf 0.3 and
**descends the Hash-N-Degree-Quads gossip into `Term::Triple` objects**, so blank
nodes nested inside triple terms are relabelled to `c14nN`. On triple-term-free
input it is byte-identical to the standard path (asserted against every W3C suite
vector — the strongest correctness anchor).

```toml
sparq-canon = { path = "crates/sparq-canon", features = ["rdf12-triple-terms"] }
```

```rust
// NON-STANDARD profile (SHA-256). Canonicalizes triple terms incl. nested bnodes.
let nq = sparq_canon::canonicalize_rdf12(&dataset)?;          // quads -> N-Quads
let cg = sparq_canon::canonicalize_triples_rdf12(&triples)?;  // one graph
let m  = sparq_canon::issue_dataset_rdf12(&dataset)?;         // issuer map

// Hash-profile parity with the standard path: each v2 entry point has a
// `*_with::<D: Digest>` sibling (SHA-384 = parity target).
let nq384 = sparq_canon::canonicalize_rdf12_with::<sha2::Sha384>(&dataset)?;
let cg384 = sparq_canon::canonicalize_triples_rdf12_with::<sha2::Sha384>(&triples)?;
let m384  = sparq_canon::issue_dataset_rdf12_with::<sha2::Sha384>(&dataset)?;

// CONSTRAINED ground-triple-term variant (sq-iaxd): errors with
// `CanonError::NestedBlankNode` if ANY blank node occurs inside a triple term
// (at any depth — inner subject or inner object). Same `*_with` siblings.
let vc  = sparq_canon::canonicalize_rdf12_ground_terms(&dataset)?;   // quads
let vcg = sparq_canon::canonicalize_triples_rdf12_ground_terms(&triples)?;
let vm  = sparq_canon::issue_dataset_rdf12_ground_terms(&dataset)?;  // issuer map
let vgc = sparq_canon::canonicalize_graph_content_rdf12_ground_terms(&graph)?; // &sparq_core::Graph
```

**When to prefer the constrained `*_ground_terms` entry points.** The common
credential/VC shape quotes or asserts **ground** content: triple terms with no
blank nodes inside. On that input the constrained wrappers are byte-identical
to the full profile (they are thin guard + delegate wrappers), but their
well-definedness does **not** rest on the novel nested-bnode HNDQ-descent
design — with every triple term ground, the run is exactly the RDFC-1.0
algorithm over an alphabet extended with opaque triple-term constants, and a
nested blank node is **rejected** (`CanonError::NestedBlankNode`) instead of
silently canonicalized under the extension. Top-level blank nodes (quad
subject/object/graph name) are ordinary RDFC-1.0 bnodes and stay fine. Use the
full `canonicalize_rdf12` descent only when you actually need nested blank
nodes relabelled. Still non-standard either way: the canonical N-Quads carry
RDF-1.2 `<<( … )>>` tokens, which W3C RDFC-1.0 has no notion of.

**Boundary / honesty:** SHA-256 is the default; a `*_with::<D: Digest>` sibling of
each v2 entry point selects another hash (notably `sha2::Sha384`) for parity with
the standard path's `canonicalize_quads_with`. The non-generic entry points are
SHA-256 and byte-identical to before; a different `D` may produce a different
(still canonical, isomorphism-stable under that `D`) relabelling. Triple terms
appear only as objects in oxrdf 0.3 (a triple's subject is `NamedOrBlankNode`), so
nesting descends strictly through the object position; the HNDQ poison-graph call
limit still fails closed. This is a sparq-local extension, **not** a W3C standard
— do not represent its output as W3C RDFC-1.0.

**Soundness audit + distinguishing-power regression (sq-mu1cd / sq-63g0).** An
adversarial audit of the nested-bnode descent found the profile **sound** on every
constructed nested-bnode case (0 defects / 5 refuted suspicions): triple-term-
internal bnodes share one HNDQ position marker (object position + the outer
predicate), yet distinct nested structure never collapses, because the final
serialization re-renders the real `Term::Triple` with c14n labels and the issuer
map separates genuinely-distinct bnodes via the first-degree descent. The audit's
sharper non-isomorphic vectors are pinned as permanent regression tests
(`tests/rdf12_triple_term_canon.rs` §5) — asymmetric / anchored inner swap,
inner-subject vs inner-object leaf, depth-1 vs depth-2 nesting, self-loop
inner-subject vs inner-object — each first **brute-force-proven non-isomorphic** by
an independent oracle before asserting the canon differs, so a future
`hash_n_degree_quads` / `hash_related_blank_node` refactor that introduced a
false-equal would be caught. The whether-the-marker-needs-a-sub-discriminator
question is a spec-clarity / robustness matter, not a latent defect.

## Opt-in `urn:concept:` record verification (`concept` feature) — [SONNET-4.6] issue #1746

**OFF by default.** A federated concept record is named by a content address,
`urn:concept:<multibase-multihash>`; before indexing a record you were handed,
recompute that name from the bytes and refuse the record if it disagrees.

```toml
sparq-canon = { path = "crates/sparq-canon", features = ["concept"] }
```

```rust
use sparq_canon::concept::{concept_urn, verify_concept_urn, ConceptHash, Multibase, ConceptUrn};

// GUARD — run this BEFORE indexing. `Ok` means the quads you hold reproduce the
// declared multihash byte-for-byte; every other outcome is a refusal.
verify_concept_urn(urn, &record_quads)?;

// Mint the name for a record you produce (same envelope, other direction).
let urn = concept_urn(&record_quads, ConceptHash::Sha256, Multibase::Base58Btc)?;

// Pin one algorithm rather than accepting any this build supports.
let parsed = ConceptUrn::parse(urn_str)?;
if parsed.hash_code() != ConceptHash::Sha256.code() { /* refuse */ }
```

Supported envelope: multibase `f`/`F` (base16), `b`/`B` (base32, unpadded), `z`
(base58btc), `u` (base64url, unpadded); multihash codes `sha2-256` (`0x12`),
`sha2-512` (`0x13`), `sha2-384` (`0x20`) at their full, untruncated lengths.

**Fail-closed, exhaustively.** Not a `urn:concept:` URN; an unknown multibase
prefix; a name-specific string longer than any well-formed multihash encodes to
(refused on length *before* the body is decoded, so a hostile name cannot buy
decoder work — base58btc decoding is quadratic in the body length);
a character outside the declared alphabet or non-canonical padding bits;
a truncated, non-minimal, or trailing-byte multihash; a declared length that
disagrees with the digest present or with the code's natural size; a hash code
this build cannot recompute; an empty record; an RDF 1.2 triple term; a digest
mismatch — all `Err`, nothing indexed. Unknown never means accepted. The
comparison is over the **full multihash** (prefix included) and is constant-time,
so a record cannot pass by declaring a code other than the one that produced its
digest.

### The two things this deliberately does not do

1. **It does not define which quads are "the record".** You pass them. The
   scope-extraction rule (whole named graph? node neighbourhood? SCC?) belongs to
   the concept-hash definition (GitHub #1683) and its profile freeze (#1746),
   neither of which is vendored here. §3 of
   `research/genai-urn-concept-verifier-design.md` shows, with counterexamples
   from this crate's own `issue_quads`, that a node-level scope hash is **not** an
   RDFC-1.0 property of the surrounding graph — so the choice is load-bearing and
   is not sparq's to make. Two conventions this implementation *does* fix, so a
   disagreement is visible rather than silent: the hashed pre-image is the exact
   canonical N-Quads document including its trailing newline, and the multihash
   code selects the **outer** digest only (canonicalization keeps RDFC-1.0's
   spec-default SHA-256 profile).
2. **It is not an independent implementation of RDFC-1.0.** Recomputing at the
   ingestion boundary catches producer-side defects and tampering. It does not
   catch a bug in RDFC-1.0 itself: `sparq-canon` delegates the algorithm to
   `rdf-canon`, so a producer on the same lineage would reproduce such a bug
   identically and it would cancel out. Do not describe this surface as
   "double-implementation verified" or "independently proven".

Suite: `crates/sparq-canon/tests/concept_urn.rs` — positive vectors are known
answers derived **outside** this crate, plus a mutation check that flips every
byte of a correct multihash in turn and requires each to be rejected.

## Conformance

Validated against the official [W3C rdf-canon test
suite](https://github.com/w3c/rdf-canon) — all eval (canonical-output),
issued-map, and negative (poison-graph) cases, under both SHA-256 and SHA-384 —
through this crate's own public API. See `crates/sparq-canon/tests/`.

## Comparative panel (bench/canon) — [FABLE-5] sq-hmd7l.16

`bash bench/canon/run.sh --smoke` drives the public API over the vendored W3C
suite via `crates/sparq-canon/examples/canon_bench.rs` (also a deterministic
`gen-clique <n>` poison generator) and asserts **byte-identical** canonical
N-Quads on the non-pathological set, then records poison-graph DoS outcomes
(`ok`/`guard`/`capped`/`wrong`/`accepted`) under a HARD per-graph wall-clock
cap. Full mode adds rdf-canonize (JS — the independent implementation; its
shipped `maxWorkFactor=1` guard fail-closes on 18/64 approved evals, so parity
runs at its raised `maxWorkFactor=3` posture, both postures recorded on the
poison panel) and the rdf-canon crate driven directly (at the default matching
pin that column measures this crate's bridge overhead — sparq-canon delegates
its algorithm to that same crate). Honesty notes + tunables:
`bench/canon/README.md`; first-read gap record: `research/gap-canon-2026-07.md`.

## When NOT to use it

- You only need a *syntactic* serialization (N-Triples/N-Quads as-is): use the
  oxrdf/oxttl serializers directly. Canonicalization is more expensive.
- The dataset has no blank nodes and a fixed order you already control:
  canonicalization is still correct but buys you nothing.

## Status

Verified against `sparq-canon` 0.1.0 source. The standard RDFC-1.0 path is
`rdf-canon` 0.15.3 (W3C-suite validated); `sparq-canon` is the single-sourced
bridge + public API. The opt-in, off-by-default `rdf12-triple-terms` profile
(sq-hslb [OPUS-4.8]) is a native RDFC-1.0 re-implementation extended to RDF 1.2
triple terms — **non-standard** (W3C RDFC-1.0 is RDF-1.1-only) — now with a
`*_with::<D: Digest>` hash-profile sibling on every entry point for SHA-384
parity (sq-5i1d [OPUS-4.8]) and a constrained ground-triple-term
(error-on-nested-bnode) `*_ground_terms` wrapper family for the common
credential/VC case (sq-iaxd [FABLE-5]). The `canonicalize_nquads` / `parse_nquads` text seam
and the `sparq-wasm` opt-in `canon` feature (`canonicalizeNQuads` binding for the
`@sparq-org/sparq` RDF/JS `Dataset`) are sq-1dd5t [OPUS-4.8]; that wasm consumer pulls
`sparq-canon` with `default-features = false` (the crate now disables
`sparq-core`'s default `parallel` and re-enables it via its own default `parallel`
feature, so native builds are byte-identical and the wasm build drops rayon).
The opt-in, off-by-default `concept` feature ([SONNET-4.6] issue #1746) adds the
`urn:concept:` multibase/multihash envelope plus the recompute-and-byte-compare
ingestion guard; it fixes no scope-extraction rule and makes no independence
claim beyond producer-side re-derivation (see that section).
`publish = false`, non-default workspace member — nothing in sparq's default graph
depends on it, so the default build and lean wasm artifact are byte-identical with
or without it.
