// sq-j506 (#7103): the METHOD-BOUND successful-result relation.
//! The method-bound (`ZKSIG_M5`) successful-result relation -- `result.nr`'s
//! statement over a credential attestation that names its commitment method.
//!
//! # Why this is a NEW relation and not an edit
//!
//! `result::result_check` authenticates credentials over the method-UNBOUND
//! `ZKSIG_C3` message, and 30 shipped members compile it: the four `result_v1_*`
//! members are pinned to an IMMUTABLE verification-key baseline (the foundation
//! commit recorded as `comparison_base` in
//! `bench/zk-compose/result_v1_compatibility.json`, re-derived independently by
//! the `zk-toolchain` workflow), and the `result_v2_*` / `result_v3_*` members
//! are a retained contract for proofs already issued against them. Changing the
//! message in place would invalidate every one of them. So `result_check` is
//! untouched, and new issuance moves to the members that compile the relation
//! below.
//!
//! # The one difference
//!
//! The credential message. `result_check` signs
//! `h4(ZKSIG_C3, root, salt, reference)`; this relation signs
//! `h5(ZKSIG_M5, root, salt, reference, SCHEME_STRING_CANONICAL_V1)`. Every other
//! line is `result_check`'s, and
//! `crates/sparq-zk-compose/tests/result_m5_relation_pin.rs` DERIVES the function
//! below from `result.nr`'s text by exactly that substitution and asserts the
//! result is byte-equal to this file -- so an edit to one relation that is not
//! mirrored in the other turns that pin red.
//!
//! Research-grade, NOT externally audited (sq-qhy4).

use crate::hashes::{commit_fold, h2, h3, h4, h5};
use crate::issuer::{schnorr_verify, Point};
use crate::revoke::revoke_unset_check;
use crate::result::{ResultInteger, N, P, R, V, POLICY_DEPTH};

/// `ZKSIG_M5` -- the METHOD-BOUND presentation attestation tag
/// (`sig::presentation_message_with_status_and_scheme`). This relation
/// authenticates credentials over this message, so a credential whose issuer
/// attested no commitment method (the method-UNBOUND `ZKSIG_C3`) does not open
/// here at all.
global PRESENTATION_STATUS_SCHEME: Field = 0x5a4b5349475f4d35;
// Exact existing sig.rs domain tags, as in `result.nr`.
global STATUS_REF: Field = 0x5a4b5349475f5352;
global ACCEPTED_STATUS: Field = 0x5a4b5349475f414c;

/// The folded `sig::scheme_iri_to_field` digest of the STRING-CANONICAL
/// commitment method's `zk:scheme` IRI (`CommitmentMethod::StringCanonicalV1`).
///
/// It is a compile-time CONSTANT rather than a public input because this
/// relation proves string-canonical leaves ONLY: every leaf is
/// `h3(Enc(s), Enc(p), Enc(o))` over the string-canonical term encoding, so no
/// other commitment method can be the one a verifying proof is about. Baking it
/// in is what makes the method part of the relation instead of a prover choice --
/// a dual-leaf credential's attestation simply fails `schnorr_verify` here.
///
/// `crates/sparq-zk-compose/tests/result_scheme_pin.rs` pins this literal to
/// `sig::scheme_iri_to_field(CommitmentMethod::StringCanonicalV1.scheme_iri())`,
/// so the two sides cannot drift apart silently.
pub global SCHEME_STRING_CANONICAL_V1: Field =
    0x057ecc168a5cf8f59c4b3a3f3aaae1b630d6bb875a0fb3255d801e28db485042;

pub fn result_check_m5<T, let K: u32, let F: u32, let SD: u32, let INTEGER_BITS: u32, let CONTRACT_VERSION: u32>(
    version: Field,
    accepted_root: Field,
    issuer_keys: [[Field; 2]; K],
    pattern_count: u32,
    pattern_vars: [[u32; 3]; P],
    pattern_constants: [[Field; 3]; P],
    projection: [bool; V],
    result_count: u32,
    results: [[Field; V]; R],
    filter_count: u32,
    filter_vars: [u32; F],
    filter_ops: [u32; F],
    filter_bounds: [u64; F],
    counts: [u32; K],
    salts: [Field; K],
    enc: [[[Field; 3]; N]; K],
    status_lists: [Field; K],
    status_versions: [u64; K],
    status_indices: [Field; K],
    status_roots: [Field; K],
    status_siblings: [[Field; SD]; K],
    policy_indices: [Field; K],
    policy_siblings: [[Field; POLICY_DEPTH]; K],
    signatures: [[Field; 5]; K],
    selected_graphs: [[u32; P]; R],
    selected_leaves: [[u32; P]; R],
    values: [[Field; V]; R],
    selected_types: [[[u32; 3]; P]; R],
    selected_hashes: [[[Field; 3]; P]; R],
    filter_values: [[T; F]; R],
) where T: ResultInteger {
    // [GPT-6] Every published member fixes these capacities at compile time.
    std::static_assert((SD == 10) | (SD == 17) | (SD == 20), "unsupported status depth");
    std::static_assert((INTEGER_BITS == 0) | (INTEGER_BITS == 8) | (INTEGER_BITS == 64), "unsupported integer capacity");
    std::static_assert((F == 0) == (INTEGER_BITS == 0), "numeric capacity must match filter member");
    assert_eq(version, CONTRACT_VERSION as Field, "unsupported successful-result contract");
    assert((pattern_count > 0) & (pattern_count <= P), "pattern capacity");
    assert((result_count > 0) & (result_count <= R), "result capacity");
    assert(filter_count <= F, "filter capacity");
    for p in 0..P {
        for s in 0..3 {
            assert(pattern_vars[p][s] <= V, "variable capacity");
        }
    }
    for f in 0..F {
        assert((filter_vars[f] > 0) & (filter_vars[f] <= V), "filter variable capacity");
    }
    for g in 0..K {
        assert((counts[g] > 0) & (counts[g] <= N), "graph capacity");
        let mut leaves = [0; N];
        for i in 0..N {
            leaves[i] = h3(enc[g][i][0], enc[g][i][1], enc[g][i][2]);
            for s in 0..3 {
                assert((i < counts[g]) | (enc[g][i][s] == 0), "noncanonical graph padding");
            }
        }
        let root = commit_fold(leaves, counts[g]);
        // Independently bind each credential's own list, version, and index.
        let reference = h4(STATUS_REF, status_lists[g], status_indices[g], status_versions[g] as Field);
        let message = h5(
            PRESENTATION_STATUS_SCHEME,
            root,
            salts[g],
            reference,
            SCHEME_STRING_CANONICAL_V1,
        );
        let sig = signatures[g];
        schnorr_verify(
            Point { x: issuer_keys[g][0], y: issuer_keys[g][1] },
            Point { x: sig[0], y: sig[1] }, sig[2], sig[3], sig[4], message,
        );
        // The policy leaf atomically binds list identity, version, and live-bit root.
        let mut node = h4(ACCEPTED_STATUS, status_lists[g], status_versions[g] as Field, status_roots[g]);
        let directions: [bool; POLICY_DEPTH] = policy_indices[g].to_le_bits();
        for d in 0..POLICY_DEPTH {
            let sib = policy_siblings[g][d];
            node = if directions[d] { h2(sib, node) } else { h2(node, sib) };
        }
        assert_eq(node, accepted_root, "credential status outside verifier policy");
        revoke_unset_check::<SD>(status_roots[g], status_indices[g], 0, status_siblings[g]);
    }
    for r in 0..R {
        let row_active = r < result_count;
        for v in 0..V {
            assert(!row_active | !projection[v] | (values[r][v] == results[r][v]), "projected binding mismatch");
            assert((row_active & projection[v]) | (results[r][v] == 0), "noncanonical public result padding");
        }
        for p in 0..P {
            let active = row_active & (p < pattern_count);
            let g = selected_graphs[r][p];
            let i = selected_leaves[r][p];
            assert(g < K, "selected credential out of range");
            assert(i < N, "selected leaf out of range");
            assert(!active | (i < counts[g]), "selected padding leaf");
            for s in 0..3 {
                let term = enc[g][i][s];
                let ty = selected_types[r][p][s];
                assert(!active | ((ty == 1) | (ty == 2)), "selected blank or unsupported term");
                if s < 2 {
                    assert(!active | (ty == 1), "subject and predicate must be IRIs");
                }
                assert(!active | (term == h2(ty as Field, selected_hashes[r][p][s])), "selected term type opening mismatch");
                let variable = pattern_vars[p][s];
                if variable == 0 {
                    assert(!active | (term == pattern_constants[p][s]), "pattern constant mismatch");
                } else {
                    // One shared row binding enforces every repeated occurrence,
                    // including repeats within one pattern and cross-credential joins.
                    assert(!active | (term == values[r][variable - 1]), "joined variable mismatch");
                }
            }
        }
        for f in 0..F {
            if row_active & (f < filter_count) {
                filter_values[r][f].check(values[r][filter_vars[f] - 1], filter_ops[f], filter_bounds[f], INTEGER_BITS);
            }
        }
    }
}
