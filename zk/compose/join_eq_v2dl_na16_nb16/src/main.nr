// sq-j506 (#7103): the DualLeafV2 COMPONENT-WITNESSED hidden-join member.
// Circuit-family member (Q5 lattice): N_A=16, N_B=16 triple slots per graph --
// the same lattice point as `join_eq_na16_nb16`, but both graphs' per-slot term
// encodings are RECOMPUTED in-circuit from witnessed V2 leaf components instead
// of being taken as opaque fields. The relation lives in
// sparq_zk_compose_core::v2_dual_leaf_relations (see its docs for the statement,
// the component ABI and the honest scope).
//
// This is a NEW member. `join_eq_na16_nb16` is untouched, so its verification
// key and every proof already issued against it stay valid.
//
// The PUBLIC inputs are deliberately the same vector, in the same declaration
// order, as `join_eq_na16_nb16`: only the private witness changes. The join
// VALUE stays PRIVATE; only the two graph commitments, the two query-bound slots
// and a HIDING commitment are public. Do not reorder `main`'s parameters: the
// verifier recomputes the public-input vector in declaration order (audit-#1
// discipline).
use sparq_zk_compose_core::v2_dual_leaf_relations::join_eq_v2_dual_leaf_check;

fn main(
    // -- public --
    challenge: pub Field,       // verifier nonce (replay binding S2.5)
    commit_a: pub Field,        // graph-A commitment C(G_a)
    commit_b: pub Field,        // graph-B commitment C(G_b)
    join_commitment: pub Field, // HIDING commitment to the join value (design S2.4)
    slot_a: pub u32,            // graph-A join slot in {0,1,2}
    slot_b: pub u32,            // graph-B join slot in {0,1,2}
    // -- private --
    counts_a: u32, // triples in graph A (<= N_A)
    // Graph-A V2 leaf components, indexed [slot][term position s/p/o].
    type_codes_a: [[Field; 3]; 16],
    has_value_a: [[bool; 3]; 16],
    has_lang_a: [[bool; 3]; 16],
    value_slot0_a: [[Field; 3]; 16],
    datatype_consts_a: [[Field; 3]; 16],
    lang_consts_a: [[Field; 3]; 16],
    identities_a: [[Field; 3]; 16],
    counts_b: u32, // triples in graph B (<= N_B)
    // Graph-B V2 leaf components, same indexing.
    type_codes_b: [[Field; 3]; 16],
    has_value_b: [[bool; 3]; 16],
    has_lang_b: [[bool; 3]; 16],
    value_slot0_b: [[Field; 3]; 16],
    datatype_consts_b: [[Field; 3]; 16],
    lang_consts_b: [[Field; 3]; 16],
    identities_b: [[Field; 3]; 16],
    row_a: [Field; 3], // the joined row of A
    row_b: [Field; 3], // the joined row of B
    blinding: Field,   // per-presentation blinder for join_commitment
) {
    // challenge is bound by the proof's public inputs; no relation constraint.
    let _ = challenge;
    join_eq_v2_dual_leaf_check::<16, 16>(
        commit_a,
        commit_b,
        join_commitment,
        slot_a,
        slot_b,
        counts_a,
        type_codes_a,
        has_value_a,
        has_lang_a,
        value_slot0_a,
        datatype_consts_a,
        lang_consts_a,
        identities_a,
        counts_b,
        type_codes_b,
        has_value_b,
        has_lang_b,
        value_slot0_b,
        datatype_consts_b,
        lang_consts_b,
        identities_b,
        row_a,
        row_b,
        blinding,
    );
}

mod tests {
    use super::main;
    use sparq_dual_leaf_v2::{INTEGER_LANE_CONST, LANG_NONE, VALUE_NONE_V2};
    use sparq_zk_compose_core::hashes::{commit_fold, h3, TYPE_CODE_IRI, TYPE_CODE_LITERAL};
    use sparq_zk_compose_core::join::join_value_commitment;
    use sparq_zk_compose_core::v2_dual_leaf_relations::recompute_graph;

    /// One active triple `(<s>, <p>, "42"^^xsd:integer)` in slot 0, the rest
    /// zero-padded: two IRI terms carrying the flat sentinel and a value-lane
    /// literal object. `subject_identity` is what the two graphs join on.
    fn components(
        subject_identity: Field,
    ) -> ([[Field; 3]; 16], [[bool; 3]; 16], [[bool; 3]; 16], [[Field; 3]; 16], [[Field; 3]; 16], [[Field; 3]; 16], [[Field; 3]; 16]) {
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
        identities[0] = [subject_identity, 22, 33];

        (type_codes, has_value, has_lang, value_slot0, datatype_consts, lang_consts, identities)
    }

    fn commitment_of(enc: [[Field; 3]; 16], count: u32) -> Field {
        let mut leaves: [Field; 16] = [0; 16];
        for i in 0..16 {
            leaves[i] = h3(enc[i][0], enc[i][1], enc[i][2]);
        }
        commit_fold(leaves, count)
    }

    /// This member's relation accepts the honest component witnesses for both
    /// graphs: the two graphs share a subject term, so the join on slot 0 holds
    /// and the hidden join value commits to the public `join_commitment`.
    #[test]
    fn member_accepts_honest_v2_components_on_both_sides() {
        let (tc_a, hv_a, hl_a, vs_a, dc_a, lc_a, id_a) = components(11);
        // Graph B shares the subject identity but differs in its predicate.
        let (tc_b, hv_b, hl_b, vs_b, dc_b, mut lc_b, mut id_b) = components(11);
        id_b[0] = [11, 99, 33];
        lc_b[0] = [LANG_NONE, LANG_NONE, LANG_NONE];

        let enc_a = recompute_graph::<16>(1, tc_a, hv_a, hl_a, vs_a, dc_a, lc_a, id_a);
        let enc_b = recompute_graph::<16>(1, tc_b, hv_b, hl_b, vs_b, dc_b, lc_b, id_b);
        // The shared subject's ENCODING is equal because its components are.
        assert_eq(enc_a[0][0], enc_b[0][0]);
        let blinding = 424242;
        main(
            7,
            commitment_of(enc_a, 1),
            commitment_of(enc_b, 1),
            join_value_commitment(enc_a[0][0], blinding),
            0,
            0,
            1,
            tc_a,
            hv_a,
            hl_a,
            vs_a,
            dc_a,
            lc_a,
            id_a,
            1,
            tc_b,
            hv_b,
            hl_b,
            vs_b,
            dc_b,
            lc_b,
            id_b,
            enc_a[0],
            enc_b[0],
            blinding,
        );
    }

    /// THE MUTATION CHECK FOR THIS MEMBER. Graph A's literal object declares a
    /// language tag while its language constant is still the reserved LANG_NONE
    /// marker.
    /// BY THIS MEMBER's relation that is refused, so the S14.1 reserved-tag check
    /// is load-bearing at the member's witnessed component sites. Deleting the
    /// reserved-tag assert in `compose_core/src/dual_leaf_v2_leaf.nr` turns this
    /// test red to green.
    #[test(should_fail_with = "a carried language constant is the reserved LANG_NONE tag")]
    fn member_refuses_a_lang_none_language_constant() {
        let (tc_a, hv_a, mut hl_a, vs_a, dc_a, lc_a, id_a) = components(11);
        let (tc_b, hv_b, hl_b, vs_b, dc_b, lc_b, mut id_b) = components(11);
        id_b[0] = [11, 99, 33];

        let honest_a = recompute_graph::<16>(1, tc_a, hv_a, hl_a, vs_a, dc_a, lc_a, id_a);
        let enc_b = recompute_graph::<16>(1, tc_b, hv_b, hl_b, vs_b, dc_b, lc_b, id_b);
        let blinding = 424242;
        hl_a[0][2] = true; // ...while lang_consts_a[0][2] stays LANG_NONE.
        main(
            7,
            commitment_of(honest_a, 1),
            commitment_of(enc_b, 1),
            join_value_commitment(honest_a[0][0], blinding),
            0,
            0,
            1,
            tc_a,
            hv_a,
            hl_a,
            vs_a,
            dc_a,
            lc_a,
            id_a,
            1,
            tc_b,
            hv_b,
            hl_b,
            vs_b,
            dc_b,
            lc_b,
            id_b,
            honest_a[0],
            enc_b[0],
            blinding,
        );
    }
}
