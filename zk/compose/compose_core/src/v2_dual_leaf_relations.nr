//! The `DualLeafV2` COMPONENT-WITNESSED variants of the scan and hidden-join
//! relations (research/zk-field-native-encoding.md S11 bead 3 / S14).
//!
//! Each one takes a V2 leaf's components instead of an opaque term encoding,
//! recomputes the encodings through
//! [`crate::dual_leaf_v2_leaf::recompute_term`] -- which applies the S3.3
//! reserved-sentinel band check and the S14.1 reserved language-tag check at the
//! witnessed component sites -- and then hands the recomputed encodings to the
//! UNCHANGED shipped relation body.
//!
//! # Why wrappers and not edits
//!
//! `scan::scan_check` and `join::join_eq_check` are called verbatim. Their
//! sources are byte-identical to the shipped versions, so every existing
//! `scan_*` / `join_eq_*` member keeps its verification key and all issued
//! material stays valid. The V2 variants are NEW members
//! (`scan_v2dl_*` / `join_eq_v2dl_*`) with their own public-input layouts and
//! their own keys. Nothing that verified before changes.
//!
//! # The component ABI
//!
//! The components arrive as PARALLEL per-graph / per-slot / per-term-position
//! arrays rather than a struct array, matching how the shipped members pass
//! `pattern_is_const` / `pattern_const_enc`. Index order throughout is
//! `[graph][slot][term position]`, with term position 0/1/2 =
//! subject/predicate/object -- the same order `h3(s, p, o)` folds.
//!
//! All component arrays are PRIVATE. The public-input layout of each V2 member
//! is its shipped counterpart's, unchanged, so the verifier's public-input
//! reconstruction is the same shape it already knows.
//!
//! # Honest scope
//!
//! The recompute constrains leaf component SHAPES; it does not bind a leaf's
//! identity component to the term's bytes, so the documented INV-VL downgrade is
//! unchanged. No value-lane verdict becomes acceptable through these members --
//! the verifier's Stage-0 value-lane refusals are untouched. Research-grade, NOT
//! externally audited (sq-qhy4).

use crate::dual_leaf_v2_leaf::{recompute_term, TermComponents};
use crate::join::join_eq_check;
use crate::scan::{scan_check, PatternSpec};

/// Recompute one graph's `N` triple slots from their component witnesses.
///
/// Slot `i` is ACTIVE when `i < count`; an inactive slot encodes to zero and is
/// exempt from the component checks, matching the zero padding the host writes.
pub fn recompute_graph<let N: u32>(
    count: u32,
    type_codes: [[Field; 3]; N],
    has_value: [[bool; 3]; N],
    has_lang: [[bool; 3]; N],
    value_slot0: [[Field; 3]; N],
    datatype_consts: [[Field; 3]; N],
    lang_consts: [[Field; 3]; N],
    identities: [[Field; 3]; N],
) -> [[Field; 3]; N] {
    let mut enc: [[Field; 3]; N] = [[0; 3]; N];
    for i in 0..N {
        let active = i < count;
        for s in 0..3 {
            enc[i][s] = recompute_term(
                TermComponents {
                    type_code: type_codes[i][s],
                    has_value: has_value[i][s],
                    has_lang: has_lang[i][s],
                    value_slot0: value_slot0[i][s],
                    datatype_const: datatype_consts[i][s],
                    lang_const: lang_consts[i][s],
                    identity: identities[i][s],
                },
                active,
            );
        }
    }
    enc
}

/// The V2 component-witnessed SCAN relation: recompute every graph's leaves from
/// their components, then run the unchanged `scan::scan_check` statement over
/// them.
pub fn scan_v2_dual_leaf_check<let K: u32, let N: u32, let R: u32>(
    commitments: [Field; K],
    pattern: PatternSpec,
    rows: [[Field; 3]; R],
    row_count: u32,
    attribution: [bool; K],
    counts: [u32; K],
    type_codes: [[[Field; 3]; N]; K],
    has_value: [[[bool; 3]; N]; K],
    has_lang: [[[bool; 3]; N]; K],
    value_slot0: [[[Field; 3]; N]; K],
    datatype_consts: [[[Field; 3]; N]; K],
    lang_consts: [[[Field; 3]; N]; K],
    identities: [[[Field; 3]; N]; K],
) {
    let mut enc: [[[Field; 3]; N]; K] = [[[0; 3]; N]; K];
    for g in 0..K {
        enc[g] = recompute_graph::<N>(
            counts[g],
            type_codes[g],
            has_value[g],
            has_lang[g],
            value_slot0[g],
            datatype_consts[g],
            lang_consts[g],
            identities[g],
        );
    }
    scan_check::<K, N, R>(commitments, pattern, rows, row_count, attribution, counts, enc);
}

/// The V2 component-witnessed HIDDEN-JOIN relation: recompute both graphs'
/// leaves from their components, then run the unchanged `join::join_eq_check`
/// statement over them.
pub fn join_eq_v2_dual_leaf_check<let N_A: u32, let N_B: u32>(
    commit_a: Field,
    commit_b: Field,
    join_commitment: Field,
    slot_a: u32,
    slot_b: u32,
    counts_a: u32,
    type_codes_a: [[Field; 3]; N_A],
    has_value_a: [[bool; 3]; N_A],
    has_lang_a: [[bool; 3]; N_A],
    value_slot0_a: [[Field; 3]; N_A],
    datatype_consts_a: [[Field; 3]; N_A],
    lang_consts_a: [[Field; 3]; N_A],
    identities_a: [[Field; 3]; N_A],
    counts_b: u32,
    type_codes_b: [[Field; 3]; N_B],
    has_value_b: [[bool; 3]; N_B],
    has_lang_b: [[bool; 3]; N_B],
    value_slot0_b: [[Field; 3]; N_B],
    datatype_consts_b: [[Field; 3]; N_B],
    lang_consts_b: [[Field; 3]; N_B],
    identities_b: [[Field; 3]; N_B],
    row_a: [Field; 3],
    row_b: [Field; 3],
    blinding: Field,
) {
    let enc_a = recompute_graph::<N_A>(
        counts_a,
        type_codes_a,
        has_value_a,
        has_lang_a,
        value_slot0_a,
        datatype_consts_a,
        lang_consts_a,
        identities_a,
    );
    let enc_b = recompute_graph::<N_B>(
        counts_b,
        type_codes_b,
        has_value_b,
        has_lang_b,
        value_slot0_b,
        datatype_consts_b,
        lang_consts_b,
        identities_b,
    );
    join_eq_check::<N_A, N_B>(
        commit_a,
        commit_b,
        join_commitment,
        slot_a,
        slot_b,
        enc_a,
        counts_a,
        enc_b,
        counts_b,
        row_a,
        row_b,
        blinding,
    );
}

mod tests {
    use super::{recompute_graph, scan_v2_dual_leaf_check};
    use crate::hashes::{commit_fold, h3, TYPE_CODE_IRI, TYPE_CODE_LITERAL};
    use crate::scan::PatternSpec;
    use sparq_dual_leaf_v2::{INTEGER_LANE_CONST, LANG_NONE, VALUE_NONE_V2};

    global N: u32 = 4;

    /// One graph of ONE triple `(<s>, <p>, "42"^^xsd:integer)`: two IRI terms
    /// carrying the flat sentinel, and a value-lane literal carrying hook 42.
    /// Returns the component arrays and the recomputed commitment.
    fn fixture() -> ([[Field; 3]; N], [[bool; 3]; N], [[bool; 3]; N], [[Field; 3]; N], [[Field; 3]; N], [[Field; 3]; N], [[Field; 3]; N]) {
        let mut type_codes: [[Field; 3]; N] = [[0; 3]; N];
        let mut has_value: [[bool; 3]; N] = [[false; 3]; N];
        let has_lang: [[bool; 3]; N] = [[false; 3]; N];
        let mut value_slot0: [[Field; 3]; N] = [[0; 3]; N];
        let mut datatype_consts: [[Field; 3]; N] = [[0; 3]; N];
        let mut lang_consts: [[Field; 3]; N] = [[0; 3]; N];
        let mut identities: [[Field; 3]; N] = [[0; 3]; N];

        // Slot 0 is the one active triple.
        type_codes[0] = [TYPE_CODE_IRI, TYPE_CODE_IRI, TYPE_CODE_LITERAL];
        has_value[0] = [false, false, true];
        value_slot0[0] = [VALUE_NONE_V2, VALUE_NONE_V2, 42];
        datatype_consts[0] = [0, 0, INTEGER_LANE_CONST];
        lang_consts[0] = [LANG_NONE, LANG_NONE, LANG_NONE];
        identities[0] = [11, 22, 33];

        (type_codes, has_value, has_lang, value_slot0, datatype_consts, lang_consts, identities)
    }

    /// The recomputed encodings are the host's V2 shapes, and the fold over them
    /// is the graph commitment the scan statement checks.
    #[test]
    fn the_graph_recompute_matches_the_documented_v2_encodings() {
        let (tc, hv, hl, vs, dc, lc, id) = fixture();
        let enc = recompute_graph::<N>(1, tc, hv, hl, vs, dc, lc, id);
        assert_eq(enc[0][0], h3(VALUE_NONE_V2, 11, TYPE_CODE_IRI));
        assert_eq(enc[0][1], h3(VALUE_NONE_V2, 22, TYPE_CODE_IRI));
        assert_eq(
            enc[0][2],
            h3(h3(42, INTEGER_LANE_CONST, LANG_NONE), 33, TYPE_CODE_LITERAL),
        );
        // Padding slots are zero, so the fold sees only the active leaf.
        assert_eq(enc[1], [0, 0, 0]);
    }

    /// END TO END: the V2 scan statement accepts the honest component witnesses
    /// against the commitment folded from the recomputed leaves, and the
    /// disclosed row is the recomputed encoding triple.
    #[test]
    fn the_v2_scan_relation_accepts_honest_component_witnesses() {
        let (tc, hv, hl, vs, dc, lc, id) = fixture();
        let enc = recompute_graph::<N>(1, tc, hv, hl, vs, dc, lc, id);
        let mut leaves: [Field; N] = [0; N];
        for i in 0..N {
            leaves[i] = h3(enc[i][0], enc[i][1], enc[i][2]);
        }
        let commitment = commit_fold(leaves, 1);
        scan_v2_dual_leaf_check::<1, N, 1>(
            [commitment],
            // Predicate-constant pattern `(?s, <p>, ?o)`.
            PatternSpec { is_const: [false, true, false], const_enc: [0, enc[0][1], 0] },
            [enc[0]],
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

    /// THE BAND CHECK REACHES THROUGH THE RELATION. Swapping the literal's
    /// value-lane hook for the reserved sentinel -- while still declaring it a
    /// value handle -- is refused by the recompute inside the scan statement,
    /// not merely by the leaf helper in isolation.
    #[test(should_fail_with = "value-lane hook is outside the handle band")]
    fn the_v2_scan_relation_refuses_the_sentinel_as_a_value_handle() {
        let (tc, hv, hl, mut vs, dc, lc, id) = fixture();
        vs[0][2] = VALUE_NONE_V2;
        let _ = recompute_graph::<N>(1, tc, hv, hl, vs, dc, lc, id);
    }

    /// THE RESERVED-TAG CHECK REACHES THROUGH THE RELATION.
    #[test(should_fail_with = "a carried language constant is the reserved LANG_NONE tag")]
    fn the_v2_scan_relation_refuses_a_lang_none_language_constant() {
        let (tc, hv, mut hl, vs, dc, lc, id) = fixture();
        hl[0][2] = true; // the literal now declares a language tag...
        let _ = recompute_graph::<N>(1, tc, hv, hl, vs, dc, lc, id);
        // ...while its lang slot still carries LANG_NONE.
    }

    /// A PADDING slot is exempt: the same all-zero components that would fail
    /// every check when active are accepted when the slot is inactive, which is
    /// what lets the host zero-pad as the shipped members already do.
    #[test]
    fn padding_slots_need_no_canonical_components() {
        let (tc, hv, hl, vs, dc, lc, id) = fixture();
        // count = 1, so slots 1..N carry all-zero components (type code 0).
        let enc = recompute_graph::<N>(1, tc, hv, hl, vs, dc, lc, id);
        assert_eq(enc[3], [0, 0, 0]);
    }

    /// ...and the control: those same zero components ARE refused once the slot
    /// is active, so the exemption above is activity-scoped and not a hole.
    #[test(should_fail_with = "term type code is not a committed term kind")]
    fn an_active_slot_may_not_carry_padding_components() {
        let (tc, hv, hl, vs, dc, lc, id) = fixture();
        let _ = recompute_graph::<N>(2, tc, hv, hl, vs, dc, lc, id);
    }
}
