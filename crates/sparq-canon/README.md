# sparq-canon

RDFC-1.0 — the W3C **[RDF Dataset Canonicalization]** (the URDNA2015 successor)
— as a small, **opt-in** public API over sparq's term model.

Canonicalization gives an RDF dataset a deterministic, blank-node-relabelled
form: two datasets are RDF-isomorphic **iff** their canonical N-Quads
serializations are byte-for-byte identical. That underpins dataset hashing,
signing, diffing, deduplication, and content-addressing.

[RDF Dataset Canonicalization]: https://www.w3.org/TR/rdf-canon/

> Model: Opus 4.8 (Fable unavailable — flag for re-review when Fable returns).
> Surfaced from `sparq-zk::canon` per bead sq-0qip.

## 🚀 Quickstart

```rust
use oxrdf::{Quad, NamedNode, NamedOrBlankNode, Term, Literal, GraphName, BlankNode};

let q = Quad::new(
    NamedOrBlankNode::BlankNode(BlankNode::new("x").unwrap()),
    NamedNode::new("http://ex/p").unwrap(),
    Term::Literal(Literal::new_simple_literal("v")),
    GraphName::DefaultGraph,
);
let nquads: String = sparq_canon::canonicalize(&[q.clone()]).unwrap();  // c14nN
let map = sparq_canon::issued_identifiers(&[q]).unwrap();  // issuer map
```

## ✨ Features

- **Dataset API** — [`canonicalize`]/`canonicalize_quads` return canonical
  N-Quads; `digest_quads_with::<D>` returns its exact digest bytes; and
  `issued_identifiers`/`issue_quads` return the blank-node issuer map. The other
  `*_with::<D: Digest>` functions select a non-default RDFC-1.0 hash profile.
- **Single-graph API** — `canonicalize_triples` / `canonicalize_graph_content`
  return a `CanonicalGraph` (sorted canonical N-Quads lines + re-parsed canonical
  triples) — what the ZK per-graph commitment pipeline consumes
  (`leaf_index = line index`).
- **Fail-closed on poison graphs** — RDFC-1.0's pathological blow-ups hit the
  HNDQ call-limit guard and surface as `CanonError::Canonicalization`; RDF 1.2
  triple terms are outside the standard data model, so the standard paths fail
  closed with `CanonError::TripleTerm` unless `rdf12-triple-terms` is enabled.
- **W3C-conformant** — validated against the official [rdf-canon test suite]
  (eval + issued-map + negative cases, SHA-256 and SHA-384) through this crate's
  own public API (`tests/rdf_canon_suite.rs`).

[rdf-canon test suite]: https://github.com/w3c/rdf-canon

### ⚠️ Opt-in NON-STANDARD RDF 1.2 triple-term profile (`rdf12-triple-terms`)

**OFF by default; NOT W3C RDFC-1.0.** RDFC-1.0 is defined for RDF 1.1 only and
has no notion of triple terms. W3C has published no RDF-1.2 dataset
canonicalization specification. With the feature OFF the crate is
**byte-identical** to before — the standard paths still return
`CanonError::TripleTerm` on triple terms and the W3C suite still passes.

Enabling `rdf12-triple-terms` adds a **separate, clearly non-standard v2** profile
that natively re-implements the RDFC-1.0 algorithm over oxrdf 0.3 and **descends
the Hash-N-Degree-Quads gossip into `Term::Triple` objects**, so blank nodes
nested inside triple terms get relabelled. It is byte-identical to the standard
path on triple-term-free input (asserted against every W3C suite vector).

```rust
// NON-STANDARD — canonicalizes triple terms incl. nested blank nodes (SHA-256).
let nq = sparq_canon::canonicalize_rdf12(&dataset)?;             // quads
let cg = sparq_canon::canonicalize_triples_rdf12(&triples)?;     // single graph
let m  = sparq_canon::issue_dataset_rdf12(&dataset)?;            // issuer map
// Constrained: requires GROUND triple terms (errors on any nested blank node).
let vc = sparq_canon::canonicalize_rdf12_ground_terms(&dataset)?;
```

**Constrained ground-triple-term variant** (`canonicalize_rdf12_ground_terms` +
`issue_dataset`/`triples`/`*_with` siblings): a thin wrapper that fails closed
with `CanonError::NestedBlankNode` unless every triple term is blank-node-free —
the common credential/VC case. Accepted input never exercises the nested-bnode
HNDQ descent: it is exactly RDFC-1.0 with triple terms as opaque constants.

**Boundary:** SHA-256 is the default; each v2 entry point has a `*_with::<D:
Digest>` sibling (e.g. `sha2::Sha384`) for standard-path parity, and a different
`D` may yield a different (still canonical, isomorphism-stable) relabelling.
Triple terms occur only as objects in oxrdf 0.3; the HNDQ limit still applies.

**Distinguishing power.** An adversarial soundness audit of the nested-bnode
descent (sq-mu1cd / sq-63g0) found the profile **sound** (0 defects / 5 refuted
suspicions); its vectors are pinned as brute-force-anchored regression tests in
`tests/rdf12_triple_term_canon.rs` §5.

### Opt-in `urn:concept:` record verification (`concept`)

**OFF by default.** The multibase/multihash envelope of a `urn:concept:<mb-mh>`
name plus a fail-closed recompute-and-byte-compare guard —
`concept::verify_concept_urn(urn, &record_quads)?` — to run **before** indexing a
received record (#1746). It deliberately does **not** define *which* quads make up
a record; the caller supplies them, because that scope rule belongs to the
concept-hash definition (#1683) and its freeze (#1746), neither vendored here. See
`research/genai-urn-concept-verifier-design.md` — §3: a node-level scope hash is
**not** whole-graph RDFC-1.0; §2: recomputation catches producer-side defects but
is **not** independent of RDFC-1.0 itself.

### Opt-in, single-sourced

`publish = false`, and nothing in sparq's default dependency graph or the wasm
artifact depends on this crate, so both are byte-identical with or without it and
`sparq-core` stays lean. The RDFC-1.0 **algorithm** is the maintained zkp-ld
[`rdf-canon`](https://crates.io/crates/rdf-canon) crate (oxrdf 0.2); this crate
owns the single canonical-N-Quads-text bridge from sparq's oxrdf 0.3, so the
bridge lives in exactly one place. `sparq-zk` depends on it (its `canon` module
is now a re-export).

## 📚 Learn more

- [W3C RDF Dataset Canonicalization (RDFC-1.0)](https://www.w3.org/TR/rdf-canon/)
- [`skills/rdf-canon/SKILL.md`](../../skills/rdf-canon/SKILL.md) — how to use this
  surface; `tests/rdf-canon-testdata/PROVENANCE.md` — the vendored W3C snapshot.

## License

MIT.
