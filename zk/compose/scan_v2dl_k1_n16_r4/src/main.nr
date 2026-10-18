// sq-j506 (#7103): the DualLeafV2 COMPONENT-WITNESSED scan member.
// Circuit-family member (Q5 lattice): K=1 graphs, N=16 triple slots per graph,
// R=4 disclosed match rows -- the same lattice point as `scan_k1_n16_r4`, but
// the per-slot term encodings are RECOMPUTED in-circuit from witnessed V2 leaf
// components instead of being taken as opaque fields. The relation lives in
// sparq_zk_compose_core::v2_dual_leaf_relations (see its docs for the statement,
// the component ABI and the honest scope).
//
// This is a NEW member. `scan_k1_n16_r4` is untouched, so its verification key
// and every proof already issued against it stay valid.
//
// The PUBLIC inputs are deliberately the same vector, in the same declaration
// order, as `scan_k1_n16_r4`: only the private witness changes. The two members
// are told apart by their verification keys, not by their public layout. Do not
// reorder `main`'s parameters: the verifier recomputes the public-input vector in
// declaration order.
use sparq_zk_compose_core::scan::PatternSpec;
use sparq_zk_compose_core::v2_dual_leaf_relations::scan_v2_dual_leaf_check;

fn main(
    // -- public --
    challenge: pub Field,              // verifier nonce (replay binding S2.5)
    commitments: pub [Field; 1],       // per-graph flat Poseidon2 commitments
    pattern_is_const: pub [bool; 3],   // BGP slot constancy (s, p, o)
    pattern_const_enc: pub [Field; 3], // term encodings of constant slots (0 = var)
    rows: pub [[Field; 3]; 4],         // disclosed matched rows (term encodings)
    row_count: pub u32,
    attribution: pub [bool; 1],        // per-graph source attribution (audit #8)
    // -- private --
    counts: [u32; 1],                  // triples per graph (<= N)
    // V2 leaf components, indexed [graph][slot][term position s/p/o].
    type_codes: [[[Field; 3]; 16]; 1],      // 1 = IRI, 2 = literal, 3 = blank node
    has_value: [[[bool; 3]; 16]; 1],        // term carries a value-lane handle
    has_lang: [[[bool; 3]; 16]; 1],         // term carries a language tag
    value_slot0: [[[Field; 3]; 16]; 1],     // value handle, or VALUE_NONE_V2
    datatype_consts: [[[Field; 3]; 16]; 1], // literal datatype lane constant
    lang_consts: [[[Field; 3]; 16]; 1],     // language constant, or LANG_NONE
    identities: [[[Field; 3]; 16]; 1],      // lexical identity component
) {
    // challenge is bound by the proof's public inputs; no relation constraint.
    let _ = challenge;
    scan_v2_dual_leaf_check::<1, 16, 4>(
        commitments,
        PatternSpec { is_const: pattern_is_const, const_enc: pattern_const_enc },
        rows,
        row_count,
        attribution,
        counts,
        type_codes,
        has_value,
        has_lang,
        value_slot0,
        datatype_consts,
        lang_consts,
        identities,
    );
}

mod tests {
    use super::main;
    use sparq_dual_leaf_v2::{INTEGER_LANE_CONST, LANG_NONE, VALUE_NONE_V2};
    use sparq_zk_compose_core::hashes::{commit_fold, h3, TYPE_CODE_IRI, TYPE_CODE_LITERAL};
    use sparq_zk_compose_core::v2_dual_leaf_relations::recompute_graph;

    /// One active triple `(<s>, <p>, "42"^^xsd:integer)` in slot 0, the rest
    /// zero-padded -- the components this member takes as its private witness.
    fn components() -> ([[Field; 3]; 16], [[bool; 3]; 16], [[bool; 3]; 16], [[Field; 3]; 16], [[Field; 3]; 16], [[Field; 3]; 16], [[Field; 3]; 16]) {
        let mut type_codes: [[Field; 3]; 16] = [[0; 3]; 16];
        let mut has_value: [[bool; 3]; 16] = [[false; 3]; 16];
        let has_lang: [[bool; 3]; 16] = [[false; 3]; 16];
        let mut value_slot0: [[Field; 3]; 16] = [[0; 3]; 16];
        let mut datatype_consts: [[Field; 3]; 16] = [[0; 3]; 16];
        let mut lang_consts: [[Field; 3]; 16] = [[0; 3]; 16];
        let mut identities: [[Field; 3]; 16] = [[0; 3]; 16];

        type_codes[0] = [TYPE_CODE_IRI, TYPE_CODE_IRI, TYPE_CODE_LITERAL];
        has_value[0] = [false, false, true];
        value_slot0[0] = [VALUE_NONE_V2, VALUE_NONE_V2, 42];
        datatype_consts[0] = [0, 0, INTEGER_LANE_CONST];
        lang_consts[0] = [LANG_NONE, LANG_NONE, LANG_NONE];
        identities[0] = [11, 22, 33];

        (type_codes, has_value, has_lang, value_slot0, datatype_consts, lang_consts, identities)
    }

    /// This member's relation accepts the honest component witness: the graph
    /// re-commits to the public commitment and the one disclosed row is the
    /// recomputed encoding triple.
    #[test]
    fn member_accepts_honest_v2_components() {
        let (tc, hv, hl, vs, dc, lc, id) = components();
        let enc = recompute_graph::<16>(1, tc, hv, hl, vs, dc, lc, id);
        let mut leaves: [Field; 16] = [0; 16];
        for i in 0..16 {
            leaves[i] = h3(enc[i][0], enc[i][1], enc[i][2]);
        }
        let commitment = commit_fold(leaves, 1);
        let mut rows: [[Field; 3]; 4] = [[0; 3]; 4];
        rows[0] = enc[0];
        main(
            7,
            [commitment],
            [false, true, false],
            [0, enc[0][1], 0],
            rows,
            1,
            [true],
            [1],
            [tc],
            [hv],
            [hl],
            [vs],
            [dc],
            [lc],
            [id],
        );
    }

    /// THE MUTATION CHECK FOR THIS MEMBER. Declaring the reserved no-value
    /// sentinel as a value-lane handle is refused BY THIS MEMBER's relation --
    /// not merely by the library helper -- so the S3.3 band check is load-bearing
    /// at the member's witnessed component sites. Deleting the band assert in
    /// `compose_core/src/dual_leaf_v2_leaf.nr` turns this test red to green.
    #[test(should_fail_with = "value-lane hook is outside the handle band")]
    fn member_refuses_the_sentinel_as_a_value_handle() {
        let (tc, hv, hl, mut vs, dc, lc, id) = components();
        let honest = recompute_graph::<16>(1, tc, hv, hl, vs, dc, lc, id);
        let mut leaves: [Field; 16] = [0; 16];
        for i in 0..16 {
            leaves[i] = h3(honest[i][0], honest[i][1], honest[i][2]);
        }
        let commitment = commit_fold(leaves, 1);
        let mut rows: [[Field; 3]; 4] = [[0; 3]; 4];
        rows[0] = honest[0];
        vs[0][2] = VALUE_NONE_V2; // ...while has_value[0][2] stays true.
        main(
            7,
            [commitment],
            [false, true, false],
            [0, honest[0][1], 0],
            rows,
            1,
            [true],
            [1],
            [tc],
            [hv],
            [hl],
            [vs],
            [dc],
            [lc],
            [id],
        );
    }
}
