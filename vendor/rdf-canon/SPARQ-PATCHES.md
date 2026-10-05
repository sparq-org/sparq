# sparq patches on top of rdf-canon 0.15.3

This tree started as a byte-identical copy of the `rdf-canon-0.15.3` sources from
crates.io (`src/`, `Cargo.toml`, `README.md`, `CHANGELOG.md`, `LICENSE`; upstream's
`tests/` fixtures are not copied — sparq runs the W3C suite from
`crates/sparq-canon/tests/rdf-canon-testdata`). The only non-upstream change in the
base is a `[workspace]` table in `Cargo.toml`, required for the root
`[patch.crates-io]` path patch. Upstream: <https://github.com/zkp-ld/rdf-canon>.

Retire this tree once crates.io publishes an `rdf-canon` release that passes
`crates/sparq-canon/tests/bnode_relabel_regression.rs` and the W3C suite
(`crates/sparq-canon/tests/rdf_canon_suite.rs`). As of 2026-10-05 the newest
release is 0.15.3. Like the spargebra patch, the root patch table does not reach
the separate `fuzz/` workspace.

## 1. Label-independent tie-break for tied N-degree hashes (sparq-org/sparq#6475)

**Symptom.** `canonicalize_quads` returned different canonical N-Quads for two
datasets that differ only by a bijective blank-node rename:

```nq
_:b4 <http://ex/q> _:b2 _:b3 .
_:b4 <http://ex/p> _:b0 .
_:b0 <http://ex/q> _:b3 _:b2 .
```

Unmodified 0.15.3 reproduces it.

**Root cause.** `_:b2` and `_:b3` share a first-degree hash, so they go to the
Hash N-Degree Quads algorithm ([RDFC-1.0 §4.8][hndq]). Hn ([§4.8.3][hndq] step 3)
records, for each blank-node component, a hash of only the *related* node's position,
predicate, and identifier ([§4.7][hrbn]). It does not record which related nodes
appear in the same quad, or the reference node's own position. Both nodes get
`{s<q>c14n1, s<q>c14n0, o<q>…, g…}` and identical N-degree hashes. But they are
**not** automorphic: `_:b2` is the object in the quad whose subject is `c14n1`, and
`_:b3` is the graph name there. [§4.4.3][canon] step 5.3 only orders the hash path
list "by the hash in result", so tied results stay in identifier-list order. That
order comes from the input blank-node labels, so the issued canonical identifiers
(and the output) depend on the labels.

**Patch.** In `canonicalize_core` step 5.3, results with tied hashes get a
secondary sort key (`tie_break_key`). The key simulates step 5.3.1 for that result
on a copy of the canonical issuer. It then serializes, sorted and deduplicated,
every quad that mentions a node this result would newly issue. Issued blank nodes
are written by their prospective canonical identifier, and all other blank nodes by
their first-degree hash. The key uses no input labels. When hashes don't tie, the
code path is unchanged. When the tied nodes are automorphic, every order gives the
same output, so all W3C suite results stay byte-identical. The spec leaves the order
of tied results unspecified, so the patch stays within RDFC-1.0 conformance. A
refactor-only helper, `issued_in_order`, factors out step 5.3.1's existing
"order by issued temporary identifier" loop (behaviour unchanged).

**Residual.** Two failure modes remain:
- Tied results with an equal key keep input order. This happens when the labelled
  neighbourhoods are identical, but non-automorphic nodes could in theory still
  differ further out.
- The same incompleteness can show up inside HNDQ step 5.4.6, where tied paths with
  different issuers are chosen by permutation order.

Neither has been observed. The randomized property
`canonical_output_invariant_under_bnode_relabeling` keeps testing for them.

**Tests.** `crates/sparq-canon/tests/bnode_relabel_regression.rs` checks the fixed
pair from the issue and all 24 relabelings of its four-node label pool. The
following stay green:
- the W3C rdf-canon suite (`rdf_canon_suite.rs`)
- `proptest_canon_determinism.rs`

[canon]: https://www.w3.org/TR/rdf-canon/#canon-algorithm
[hndq]: https://www.w3.org/TR/rdf-canon/#hash-nd-quads
[hrbn]: https://www.w3.org/TR/rdf-canon/#hash-related-blank-node
