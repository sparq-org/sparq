// [OPUS-4.8] sq-rsd3v.2 (#2614): in-circuit ZK-over-hidden-antecedent
// derivation_step member (RDFS-first). Epic sq-rsd3v; parent #2616.
//! In-circuit RDFS derivation relation -- the GREENFIELD "row A1" member of
//! `research/zk-inference-and-credentials.md sec 3.3`: prove a disclosed row is
//! ENTAILED (SOUNDNESS-of-derivation) under an RDFS regime WITHOUT disclosing
//! the antecedent base facts.
//!
//! # What this replaces (the privacy upgrade)
//! The shipped host-side re-check (`crates/sparq-zk-compose/src/derivation.rs`
//! + `verifier.rs bind_entailment`) grounds every antecedent to a DISCLOSED
//! scan row -- verifiable, but ZERO antecedent privacy (row P4). This relation
//! grounds the DAG LEAVES to committed-graph MEMBERSHIP instead, so the base
//! facts a derivation rests on are HIDDEN private witnesses (row A1).
//!
//! # The statement (design sec 3.3, one `ProofTree` per derived disclosed row)
//! A `ProofTree` is a flat, premises-before-conclusion DAG of `M` nodes. Each
//! active node is EITHER:
//!   - a LEAF (`rule == RULE_LEAF`): an asserted base fact, ANCHORED to
//!     committed-graph membership -- its conclusion triple must occur in some
//!     active slot of some committed graph (design step 3, the soundness
//!     anchor + privacy upgrade); OR
//!   - a RULE FIRING (`rule` in `RULE_RDFS{9,7,2,3,5,11}`): its premise/
//!     conclusion term-encodings satisfy the rule's FIXED variable-sharing
//!     equalities (design step 1), and EVERY premise is CHAINED by
//!     encoding-equality to a STRICTLY EARLIER node's conclusion (design step
//!     2). Chaining bottoms out at anchored leaves (node 0 has no earlier node,
//!     so it must be a leaf), making the DAG well-founded.
//! The disclosed derived row (`root`, PUBLIC) is bound to the LAST active
//! node's conclusion. A forged/ungrounded antecedent has no satisfying witness
//! and the proof fails closed (the `derivation_rejects_ungrounded_antecedent`
//! test pins this).
//!
//! # Commitment binding (anti-row-swap, identical to scan.nr / join.nr step 1)
//! Each witnessed graph re-commits (`commit_fold` over `h3` leaves) to its
//! PUBLIC `commitments[g]`, so leaf membership is membership in the SAME `C(G)`
//! the scan sub-proofs expose and the issuer signed -- not a free-floating set.
//!
//! # Public vs private (the privacy boundary)
//! - PUBLIC:  `commitments` (the per-graph commitments), `schema` (the five
//!            RDFS-vocabulary term encodings the verifier recomputes from fixed
//!            IRIs -- salt-independent PUBLIC constants, NOT trusted schema
//!            edges: the schema ANTECEDENTS are still grounded as ordinary
//!            leaves), `root` (the disclosed derived triple encoding). As in
//!            `scan.nr`, the verifier nonce `challenge` (replay-binding) is a
//!            public input of the bin-package `main` wrapper -- unconstrained in
//!            the relation, byte-bound by the verifier -- not of this core fn.
//! - PRIVATE: `enc`/`counts` (the graph contents + sizes), `node_count`,
//!            `rule`/`prem`/`concl` (the derivation DAG). The antecedent base
//!            facts live only in `enc`/`prem`/`concl` and never appear in a
//!            public input -- that is the antecedent privacy this member adds.
//!
//! # Rule scope v1 (fail-closed outside it; design sec 3.5)
//! rdfs9/rdfs7 (shipped host-side) + rdfs2/rdfs3/rdfs5/rdfs11 -- all the same
//! fixed variable-sharing shape, matching `EntailmentRule` in the host module
//! (single source of truth for the shapes). `owl:sameAs` is gated SEPARATELY
//! (`sq-rsd3v.6`, the `sameas` module -- NOT `.7`, which is the distinct
//! completeness obligation below): encoding-equality is UNSOUND under
//! equality reasoning and MUST NOT ride this fixed-shape path. A node with an
//! unknown `rule` tag has no satisfying shape and is rejected.
//!
//! Note WHERE that refusal lives: this relation does NOT test for the
//! `owl:sameAs` encoding (it has no such public input, and an rdfs7 firing over
//! a `sameAs` fact is a legitimate RDFS instance in-circuit). The gate is
//! HOST-side --
//! `DerivationStep::mentions_equality_predicate` + `bind_entailment`'s
//! `CheckError::EqualityReasoningUnsupported` -- so a manifest cannot present an
//! equality step on this path. Do not read the absence of an in-circuit check
//! as "the circuit permits sameAs reasoning": it permits the RDFS rule, and the
//! equality SEMANTICS come only from `sameas.nr`'s canonicalisation.
//!
//! # HONEST STATUS (sq-qhy4)
//! Research-grade, NOT externally audited. This is the in-circuit RELATION; the
//! off-circuit witness generator (the `sparq-reason explain` ProofTree -> slot
//! mapper) and the compiled bin-package monomorphisation + `bb gates` cost
//! measurement are follow-ups. NO cost claim is made here (repo policy forbids
//! unmeasured numbers; `bb gates` MUST confirm the per-node figure first).
//! This relation states SOUNDNESS of derivation ("every disclosed derived row IS
//! entailed"). COMPLETENESS under entailment ("no entailed answer is MISSING") is
//! the OTHER, DISTINCT obligation (`sq-rsd3v.7`, design sec 3.7) and is explicitly
//! OUT of scope: it would need a closure-sweep over the flat FULL graph (the
//! entailment analogue of `scan.nr` step 3, which sweeps only the asserted base)
//! PLUS a fixpoint-SATURATION proof (no rule fires producing a new triple), and the
//! saturation half is UNBUILT anywhere in sparq and NOT claimed. The host verifier
//! REFUSES a relying party that demands it
//! (`EntailmentPolicy::require_completeness_under_entailment`), so an accept of
//! this relation can never be read as completeness.

use crate::hashes::{commit_fold, h3};

// Rule tags -- the CIRCUIT's private encoding of `EntailmentRule` (the host
// enum serialises as kebab tags; the off-circuit zk-trace mapper translates a
// `ProofNode.rule` to one of these). LEAF marks an asserted base fact anchored
// to graph membership rather than a rule firing.
pub global RULE_LEAF: u32 = 0;
pub global RULE_RDFS9: u32 = 1; // (x type c1),(c1 subClassOf c2) => (x type c2)
pub global RULE_RDFS7: u32 = 2; // (p1 subPropertyOf p2),(x p1 y) => (x p2 y)
pub global RULE_RDFS2: u32 = 3; // (p domain c),(x p y) => (x type c)
pub global RULE_RDFS3: u32 = 4; // (p range c),(x p y) => (y type c)
pub global RULE_RDFS5: u32 = 5; // (p1 sPO p2),(p2 sPO p3) => (p1 sPO p3)
pub global RULE_RDFS11: u32 = 6; // (c1 sCO c2),(c2 sCO c3) => (c1 sCO c3)

// Indices into the PUBLIC `schema` array of RDFS-vocabulary term encodings.
global SCHEMA_TYPE: u32 = 0; // rdf:type
global SCHEMA_SUBCLASS: u32 = 1; // rdfs:subClassOf
global SCHEMA_SUBPROP: u32 = 2; // rdfs:subPropertyOf
global SCHEMA_DOMAIN: u32 = 3; // rdfs:domain
global SCHEMA_RANGE: u32 = 4; // rdfs:range

/// Term-encoding-triple equality (a term-encoding equality IS a term equality
/// over the salt-independent encoding, so this is a sound triple equality).
fn triple_eq(a: [Field; 3], b: [Field; 3]) -> bool {
    (a[0] == b[0]) & (a[1] == b[1]) & (a[2] == b[2])
}

/// `row` occurs in some active slot of the graph `(enc, count)` -- the
/// `scan.nr` / `join.nr` present-in-graph discipline (whole graph in-circuit;
/// membership is equality against witnessed slots, no Merkle machinery). This
/// is the leaf-anchoring primitive of design sec 3.3 step 3.
fn row_present_in<let N: u32>(enc: [[Field; 3]; N], count: u32, row: [Field; 3]) -> bool {
    let mut present = false;
    for i in 0..N {
        let slot_active = i < count;
        present |= slot_active & triple_eq(enc[i], row);
    }
    present
}

/// Whether `(prem0, prem1, concl)` is a WELL-FORMED instance of rule `tag`
/// under the PUBLIC `schema` encodings. Each rule's fixed variable-sharing
/// equalities MIRROR `DerivationStep::is_well_formed` in the host module
/// EXACTLY (single source of truth for the shapes). Exactly one tag-branch can
/// match (the tags are distinct), so the OR selects that rule's check; an
/// unknown tag matches none and yields `false` (fail-closed).
fn rule_shape_ok(
    tag: u32,
    prem0: [Field; 3],
    prem1: [Field; 3],
    concl: [Field; 3],
    schema: [Field; 5],
) -> bool {
    let rtype = schema[SCHEMA_TYPE];
    let sub_class = schema[SCHEMA_SUBCLASS];
    let sub_prop = schema[SCHEMA_SUBPROP];
    let domain = schema[SCHEMA_DOMAIN];
    let range = schema[SCHEMA_RANGE];

    // rdfs9: (x type c1),(c1 subClassOf c2) => (x type c2)
    let s9 = (prem0[1] == rtype)
        & (prem1[1] == sub_class)
        & (prem0[2] == prem1[0]) // shared c1
        & (concl[0] == prem0[0]) // x
        & (concl[1] == rtype)
        & (concl[2] == prem1[2]); // c2

    // rdfs7: (p1 subPropertyOf p2),(x p1 y) => (x p2 y)
    let s7 = (prem0[1] == sub_prop)
        & (prem0[0] == prem1[1]) // shared p1
        & (concl[0] == prem1[0]) // x
        & (concl[1] == prem0[2]) // p2
        & (concl[2] == prem1[2]); // y

    // rdfs2: (p domain c),(x p y) => (x type c)
    let s2 = (prem0[1] == domain)
        & (prem0[0] == prem1[1]) // shared p
        & (concl[0] == prem1[0]) // x
        & (concl[1] == rtype)
        & (concl[2] == prem0[2]); // c

    // rdfs3: (p range c),(x p y) => (y type c)
    let s3 = (prem0[1] == range)
        & (prem0[0] == prem1[1]) // shared p
        & (concl[0] == prem1[2]) // y
        & (concl[1] == rtype)
        & (concl[2] == prem0[2]); // c

    // rdfs5: (p1 subPropertyOf p2),(p2 subPropertyOf p3) => (p1 subPropertyOf p3)
    let s5 = (prem0[1] == sub_prop)
        & (prem1[1] == sub_prop)
        & (prem0[2] == prem1[0]) // shared p2
        & (concl[0] == prem0[0]) // p1
        & (concl[1] == sub_prop)
        & (concl[2] == prem1[2]); // p3

    // rdfs11: (c1 subClassOf c2),(c2 subClassOf c3) => (c1 subClassOf c3)
    let s11 = (prem0[1] == sub_class)
        & (prem1[1] == sub_class)
        & (prem0[2] == prem1[0]) // shared c2
        & (concl[0] == prem0[0]) // c1
        & (concl[1] == sub_class)
        & (concl[2] == prem1[2]); // c3

    ((tag == RULE_RDFS9) & s9)
        | ((tag == RULE_RDFS7) & s7)
        | ((tag == RULE_RDFS2) & s2)
        | ((tag == RULE_RDFS3) & s3)
        | ((tag == RULE_RDFS5) & s5)
        | ((tag == RULE_RDFS11) & s11)
}

/// The generic RDFS derivation relation; a `derivation_k{K}_n{N}_m{M}` bin
/// package would be one monomorphisation (K graphs, N slots/graph, M DAG
/// nodes). See module docs for the full statement and public/private split.
///
/// `commitments` (PUBLIC) MUST byte-equal the scan sub-proofs' `commitments[g]`
/// (the verifier's entailment-binding gate -- NOT enforced here; this is the
/// in-circuit relation). `schema` (PUBLIC) MUST equal the verifier-recomputed
/// RDFS-vocabulary encodings (design sec 3.3: schema-as-PUBLIC is the term
/// encodings only -- the schema ANTECEDENTS are still grounded as leaves, never
/// trusted). `root` (PUBLIC) is the disclosed derived triple encoding, bound to
/// the DAG's terminal conclusion.
pub fn derivation_check<let K: u32, let N: u32, let M: u32>(
    commitments: [Field; K],
    schema: [Field; 5],
    root: [Field; 3],
    counts: [u32; K],
    enc: [[[Field; 3]; N]; K],
    node_count: u32,
    rule: [u32; M],
    prem: [[[Field; 3]; 2]; M],
    concl: [[Field; 3]; M],
) {
    assert(node_count <= M, "node_count exceeds circuit bucket");

    // 1. Commitment recompute -- binds every witnessed graph slot to the PUBLIC
    //    commitment (identical to scan.nr / join.nr step 1). Leaf membership is
    //    therefore membership in the SAME C(G) the issuer signed.
    for g in 0..K {
        assert(counts[g] <= N, "graph size exceeds circuit bucket");
        let mut leaves: [Field; N] = [0; N];
        for i in 0..N {
            leaves[i] = h3(enc[g][i][0], enc[g][i][1], enc[g][i][2]);
        }
        assert_eq(commit_fold(leaves, counts[g]), commitments[g], "graph commitment mismatch");
    }

    // 2. Per-node soundness + root binding.
    let mut root_bound = false;
    for i in 0..M {
        let active = i < node_count;
        let is_leaf = rule[i] == RULE_LEAF;

        // 2a. LEAF anchoring (design step 3): an active leaf's conclusion must
        //     occur in some committed graph -- the hidden-antecedent soundness
        //     anchor. A leaf claiming a fact not in any C(G) has no witness.
        let mut present = false;
        for g in 0..K {
            present |= row_present_in(enc[g], counts[g], concl[i]);
        }
        assert(
            !active | !is_leaf | present,
            "derivation leaf not present in committed graph",
        );

        // 2b. RULE firing (design step 1): a non-leaf node satisfies its rule's
        //     fixed variable-sharing shape over the term encodings.
        assert(
            !active | is_leaf | rule_shape_ok(rule[i], prem[i][0], prem[i][1], concl[i], schema),
            "derivation step violates its rule shape",
        );

        // 2c. Premise CHAINING (design step 2): every premise of a non-leaf node
        //     equals a STRICTLY EARLIER active node's conclusion. Chaining
        //     bottoms out at anchored leaves (node 0 cannot chain, so a non-leaf
        //     node 0 is rejected -> the DAG is well-founded). An ungrounded
        //     antecedent (neither an earlier conclusion nor -- via a leaf node --
        //     a committed fact) fails here: this is the forge-closure.
        for k in 0..2 {
            let mut chained = false;
            for j in 0..M {
                let earlier = (j < i) & (j < node_count);
                chained |= earlier & triple_eq(prem[i][k], concl[j]);
            }
            assert(
                !active | is_leaf | chained,
                "derivation premise not grounded",
            );
        }

        // 2d. Root binding: the LAST active node (i == node_count - 1) is the
        //     terminal conclusion and MUST equal the disclosed derived row.
        //     Folding the equality into the accumulator also forces
        //     node_count >= 1 (else no node binds and the final assert fails).
        let is_last_active = active & !((i + 1) < node_count);
        root_bound |= is_last_active & triple_eq(concl[i], root);
    }
    assert(root_bound, "derivation root not bound to the disclosed derived row");
}
