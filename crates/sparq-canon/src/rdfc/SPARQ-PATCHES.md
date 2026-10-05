# sparq patches on top of rdf-canon 0.15.3

This module is the `rdf-canon-0.15.3` crate from crates.io
(<https://github.com/zkp-ld/rdf-canon>, MIT, (c) 2023 yamdan; the licence is in
`LICENSE` next to this file and ships inside the `sparq-canon` package). It is a
module of `sparq-canon`, not a `[patch.crates-io]` override, because Cargo ignores
`[patch]` for downstream users and on publish: only code inside the published crate
reaches them. The module stays private; the only API is `sparq-canon`'s.

Changes to the base, apart from the patches below:
- `lib.rs` became `mod.rs`. It re-exports only what `sparq-canon` calls. Upstream's
  manifest-driven test module was dropped (its fixtures were never vendored; sparq runs
  the W3C suite from `crates/sparq-canon/tests/rdf_canon_suite.rs`).
- `api.rs` was trimmed to `canonicalize_quads[_with]`, `issue_with`,
  `issue_quads[_with]` and the private relabel helpers. Their bodies are unchanged.
- Dead code was removed: `serialize_graph`, `PerNodeHndqCallCounter`,
  `HndqCallCounter::sum`, `CanonicalizationError::Base16EncodingFailed`, and `logger.rs`.
- `crate::` became `super::`, and `oxrdf` became `oxrdf02` (sparq's alias for
  oxrdf 0.2).
- The `#[cfg(feature = "log")]` blocks stay verbatim. `sparq-canon` defines no `log`
  feature, so they never compile (a `check-cfg` entry in its `Cargo.toml` allows them).
  `canon.rs` allows two clippy lints that fire only on upstream code.

Retire this module once crates.io publishes an `rdf-canon` release that passes
`crates/sparq-canon/tests/bnode_relabel_regression.rs` and the W3C suite. As of
2026-10-05 the newest release is 0.15.3. `sparq-difftest` and the `canon_compare`
example still use upstream `rdf-canon` 0.15.3 directly: the difftest oracle keeps an
implementation separate from `sparq-canon`, and the example compares against it.

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

**Patch.** In `canonicalize_core` step 5.3, each run of results with tied hashes is
sorted by a secondary key (`canonical_tie_break_key`). The key issues the result's
identifiers, in step 5.3.1 order, on a prospective overlay: a small map that only
reads the canonical issuer and is never cloned from it. It then serializes, through
`structural_key`, the sorted and deduplicated quads of every node the result would
newly issue. Issued blank nodes are written by their (prospective) canonical
identifier, and all other blank nodes by their first-degree hash, which is now kept
on `CanonicalizationState`. The key uses no input labels. When hashes don't tie, the
code path is unchanged. A refactor-only helper, `issued_in_order`, factors out step
5.3.1's existing "order by issued temporary identifier" loop (behaviour unchanged).

## 2. Label-independent choice between equal HNDQ paths

**Symptom.** Add these quads to the §1 dataset:

```nq
_:b1 <http://ex/root0> _:b2 .
_:b1 <http://ex/root0> _:b3 .
_:b5 <http://ex/root0> _:b0 .
_:b5 <http://ex/root0> _:b4 .
```

Swapping `_:b2` and `_:b3` still changed the output, even with §1 applied: the
`<http://ex/q>` quads exchanged `c14n4` and `c14n5`.

**Root cause.** In HNDQ for `_:b1`, the permutations `[b2, b3]` and `[b3, b2]` build
equal paths but different issuers. [§4.8.3][hndq] step 5.4.6 replaces the chosen path
only when the new path is *less than* it, so the first permutation wins, and
permutation order follows the input labels. Upstream also skipped the equal candidate
early: steps 5.4.4.3 and 5.4.5.5 test `path >= chosen_path`, where the spec says
"greater than". The chosen issuer becomes the result's issuer, which drives the
canonical identifiers issued in step 5.3.1. `_:b1` and `_:b5` get different N-degree
hashes, so §1 never sees this tie.

**Patch.** In `hash_n_degree_quads`:
- Steps 5.4.4.3 and 5.4.5.5 skip a permutation on equality only when more will still
  be appended to the path. In that case the final path is strictly greater, so this
  skips exactly what upstream skipped, except a complete tie.
- In step 5.4.6, a complete tie whose issuer differs from the chosen issuer is
  resolved by `path_tie_break_key`. That key covers the quads of the nodes the
  candidate issued on top of the Hn entry's starting issuer, with blank nodes written
  by canonical identifier, then temporary identifier, then first-degree hash. The
  smaller key wins. The chosen candidate's key is computed lazily, once per chosen
  candidate.

The hash returned by HNDQ is unchanged, because both candidates share the same path.
Only the issuer choice changes.

**Cost.** Neither key clones an issuer. Each key is linear (up to a sort) in the quads
of the nodes it covers, and every one of those nodes went through an HNDQ call in the
run that produced the result or candidate. So key work is bounded by work the
`hndq_call_limit` budget already meters, and the patches add no unmetered
amplification.

## Residual and tests

Ties whose keys are also equal keep their input order. That happens when the
labelled one-hop neighbourhoods are identical; non-automorphic nodes could in theory
still differ further out. When the tied nodes are automorphic, every order gives the
same output. The spec leaves both orders unspecified, so the patches stay within
RDFC-1.0 conformance, and the W3C suite results are byte-identical.

`crates/sparq-canon/tests/bnode_relabel_regression.rs` checks:
- the §1 pair and all 24 relabelings of its four-node label pool;
- the §2 b2/b3 swap and all 720 relabelings of its six-node label pool.

The following stay green:
- the W3C rdf-canon suite (`rdf_canon_suite.rs`)
- `proptest_canon_determinism.rs`, including the randomized
  `canonical_output_invariant_under_bnode_relabeling`

[canon]: https://www.w3.org/TR/rdf-canon/#canon-algorithm
[hndq]: https://www.w3.org/TR/rdf-canon/#hash-nd-quads
[hrbn]: https://www.w3.org/TR/rdf-canon/#hash-related-blank-node
