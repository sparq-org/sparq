// [OPUS-5] sq-rsd3v.3 (#2946): witnessed-rule-shape derivation_step for N3
// {premise} => {conclusion} rules + the math: builtin gadget whitelist.
// Epic sq-rsd3v; parent #2616. Blocked-by sq-rsd3v.2 (#2614, derivation.nr).
//! In-circuit N3 derivation relation -- "row A2" of
//! `research/zk-inference-and-credentials.md` sec 4: prove a disclosed row is
//! ENTAILED under a DATASET-SUPPLIED N3 rule graph, WITHOUT disclosing the
//! antecedent base facts.
//!
//! # Why this is the HARDER sibling of `derivation.nr` (sec 4.1)
//! For RDFS/OWL-RL the rule set is a FIXED PUBLIC TABLE, so `derivation.nr`
//! can hard-code six variable-sharing shapes behind a `rule` tag and be done.
//! For N3 the rule set is **dataset-supplied**: `{p1 . p2 ...} => {c1 ...}`.
//! Two design moves follow, and they are the whole content of this module.
//!
//! 1. **The rule author becomes an ISSUER.** The rule graph is committed into
//!    the signed-input set exactly like a TBox: `rules_root` (PUBLIC) is a
//!    `commit_fold` over per-rule leaves, so a derivation is only as sound as
//!    its rules and **the proof says WHOSE rules**. A prover cannot invent a
//!    convenient rule -- an uncommitted shape changes `rules_root` and the
//!    recompute in step 2 fails closed. (This is the `sq-z9l` key-set-membership
//!    discipline reused at the rule-graph level; the SIGNATURE over
//!    `rules_root` is the existing `issuer.nr` machinery and is NOT restated
//!    here -- this relation binds to the root, the issuer member binds the root
//!    to a key.)
//! 2. **The rule SHAPE is itself a WITNESS**, not a fixed enum tag. Each
//!    pattern slot is a `(kind, konst, var)` triple: a CONSTANT term encoding,
//!    or a VARIABLE index into a per-node substitution. The circuit checks the
//!    node's conclusion IS the rule's conclusion pattern under a CONSISTENT
//!    variable substitution -- consistency is STRUCTURAL here, because the
//!    substitution is an ARRAY indexed by variable id, so one variable cannot
//!    take two values within a firing.
//!
//! So this module is `derivation.nr` generalised from a fixed rule table to a
//! witnessed rule shape, plus the builtin gadget whitelist (sec 4.3).
//!
//! # The statement (one `ProofTree` per derived disclosed row)
//! A flat, premises-before-conclusion DAG of `M` nodes. Each active node is
//! EITHER:
//!   - a LEAF (`node_is_leaf`): an asserted base fact, ANCHORED to committed-
//!     graph membership -- its triple must occur in an active slot of some
//!     committed graph (the soundness anchor AND the privacy upgrade: the base
//!     fact is a private witness, never a public input); OR
//!   - a RULE FIRING citing `node_rule[i]`, a rule of the COMMITTED rule set,
//!     where (a) every JOIN premise atom, instantiated under the node's
//!     substitution, equals a STRICTLY EARLIER active node's conclusion, (b)
//!     every BUILTIN premise atom satisfies its whitelisted gadget over the
//!     instantiated operands, and (c) the node's triple is the rule's selected
//!     conclusion atom under that same substitution.
//! Chaining bottoms out at anchored leaves (node 0 has no earlier node), so the
//! DAG is well-founded. The disclosed derived row (`root`, PUBLIC) is bound to
//! the LAST active node's conclusion.
//!
//! Note a rule whose premise is PURELY builtin (e.g.
//! `{ (1 2) math:sum ?c } => { :x :v ?c }`) legitimately fires at node 0 with
//! no chaining: its conclusion is determined by CONSTANTS through the gadgets,
//! so it is genuinely entailed. That is not a hole in well-foundedness -- there
//! is simply nothing to ground.
//!
//! # SAFETY (range-restriction) is enforced IN-CIRCUIT, and it is load-bearing
//! The substitution is a PRIVATE witness. If a conclusion variable were not
//! bound by the premise, the prover would be free to CHOOSE its value and
//! "derive" an arbitrary triple. So the declared subset's safety condition --
//! every conclusion variable is bound in the premise -- is not a stylistic
//! restriction, it is the soundness condition, and step 3 below enforces it
//! over the witnessed shape with a BINDING SCHEDULE that mirrors how
//! `sparq-reason` reorders premises (a builtin runs only after the atoms that
//! produce its inputs; cwm's "when ready"). A join atom BINDS every variable
//! slot it carries; an arithmetic builtin REQUIRES its input variables already
//! bound and BINDS its output variable; a comparison builtin binds nothing and
//! requires both operands bound. The `n3_rejects_unsafe_rule` test pins this.
//!
//! # The PROVABLE SUBSET v1 (declared; fail-closed outside it -- sec 4.3)
//! INCLUDED: safe, ground-deriving FORWARD rules `{p1 . p2 ...} => {c1 ...}`
//! with universals, plus the whitelisted `math:` builtins as fixed gadgets --
//! comparisons (`greaterThan` / `lessThan` / `notGreaterThan` / `notLessThan` /
//! `equalTo`), which reuse `filter_signed`'s audited `signed_verdict` and its
//! canonical-`xsd:integer` operand binding VERBATIM, and arithmetic
//! (`sum` / `difference` / `product`) as field ops with range checks. Path
//! syntax (`!` / `^`) is fine: `sparq-reason` desugars it into fresh-variable
//! join triples BEFORE the proof step, so it never reaches this relation.
//!
//! EXCLUDED and fail-closed: existentials-in-conclusion (no in-circuit fresh
//! blank-node minting -- there is no `SLOT_*` kind for one, so such a rule
//! cannot even be expressed here, and the host gate rejects it at issuance);
//! scoped negation-as-failure (`log:notIncludes` / `log:collectAllIn` --
//! proving a NEGATIVE needs the saturation machinery, deferred with
//! completeness, `sq-rsd3v.7`); `math:quotient` / `exponentiation` / floats;
//! `string:` builtins (no in-circuit UTF-8 reasoning); `list:` generators;
//! `time:` decomposition; `log:semantics` / `log:includes`; backward rules that
//! do not reduce to a forward closure; any rule whose closure is unbounded.
//! A rule outside the subset is REJECTED, never silently approximated: an
//! unwhitelisted builtin tag has no satisfying gadget (step 3 asserts
//! `tag <= BUILTIN_MAX`), and the host-side gate
//! (`sparq_zk_compose::n3::N3Rule::admit`) refuses the rule GRAPH at issuance
//! so an out-of-subset rule never reaches `rules_root` in the first place.
//!
//! # Public vs private (the privacy boundary)
//! - PUBLIC:  `commitments` (the per-graph commitments -- the SAME `C(G)` the
//!            scan sub-proofs expose and the issuer signed), `rules_root` (WHOSE
//!            rules), `root` (the disclosed derived triple encoding). As in
//!            `scan.nr`, a bin-package `main` wrapper would additionally carry
//!            the verifier nonce `challenge` (unconstrained in the relation,
//!            byte-bound verifier-side).
//! - PRIVATE: `enc`/`counts` (the graph contents + sizes), the ENTIRE rule
//!            table (only its ROOT is public -- so the rule TEXT stays hidden
//!            while its authorship is proved), the derivation DAG
//!            (`node_*`), and the builtin operand witnesses. The antecedent
//!            base facts appear in no public input: that is the antecedent
//!            privacy this member adds over the host-side disclosed-base
//!            re-check.
//!
//! # `owl:sameAs` (the same gate as `derivation.nr`)
//! This relation re-checks variable sharing by ENCODING EQUALITY, which is term
//! IDENTITY -- the wrong proxy once `owl:sameAs` quotients the term universe.
//! Equality reasoning must ride `sameas.nr`'s canonicalisation instead, and the
//! host gate refuses an equality-mentioning step. Do not read the absence of an
//! in-circuit `sameAs` test here as permission: this relation simply has no
//! equality semantics at all.
//!
//! # Cost (NO number is claimed)
//! The dominant terms are `M*R*P*(V + M)` (per-node instantiation + join
//! chaining against every candidate rule) and `M*P*3` blake3 token rebuilds for
//! the builtin operand bindings, plus `K*N` for the commitment recompute and
//! `R*(P + C)` hashes for the rule-set recompute. Repo policy FORBIDS an
//! unmeasured cost figure: `bb gates` over a compiled monomorphisation MUST be
//! run before ANY cost claim, and it has NOT been run for this module.
//!
//! # HONEST STATUS (sq-qhy4)
//! Research-grade, NOT externally audited; it inherits every soundness caveat
//! of sec 3 (`derivation.nr`). This is the in-circuit RELATION only -- the
//! off-circuit witness generator (the `sparq-reason` `reason_n3_proof` /
//! `n3_proof_tree` `ProofStep` -> slot mapper), a compiled
//! `n3_k{K}_n{N}_r{R}_m{M}` bin package, its `bb gates` measurement, and the
//! `verify_manifest` dispatch that would bind such a proof are follow-ups.
//! Nothing here makes the composition verifier sound.
//!
//! This relation states SOUNDNESS of derivation ("every disclosed derived row
//! IS entailed by the committed rules"). COMPLETENESS under entailment ("no
//! entailed answer is MISSING") is the OTHER, DISTINCT obligation
//! (`sq-rsd3v.7`, design sec 3.7) and is explicitly OUT of scope -- it needs a
//! closure-sweep over the flat FULL graph PLUS a fixpoint-SATURATION proof, and
//! the saturation half is UNBUILT anywhere in sparq and NOT claimed.

use crate::filter_int::{OP_EQ, OP_GE, OP_GT, OP_LE, OP_LT};
use crate::filter_signed::{signed_int_encoding, signed_verdict};
use crate::hashes::{commit_fold, h2, h3, h4};

/// A pattern slot holding a CONSTANT term encoding (read from `*_konst`).
pub global SLOT_CONST: u32 = 0;
/// A pattern slot holding a VARIABLE index (read from `*_var`), resolved
/// through the firing node's substitution array.
pub global SLOT_VAR: u32 = 1;

/// An ordinary JOIN premise atom: matched against the derivation DAG (and so,
/// transitively, against the committed graphs).
pub global BUILTIN_NONE: u32 = 0;
/// `?a math:greaterThan ?b` -- slots 0 and 2 are the operands.
pub global BUILTIN_GREATER_THAN: u32 = 1;
/// `?a math:lessThan ?b`.
pub global BUILTIN_LESS_THAN: u32 = 2;
/// `?a math:notGreaterThan ?b` (i.e. `<=`).
pub global BUILTIN_NOT_GREATER_THAN: u32 = 3;
/// `?a math:notLessThan ?b` (i.e. `>=`).
pub global BUILTIN_NOT_LESS_THAN: u32 = 4;
/// `?a math:equalTo ?b`.
pub global BUILTIN_EQUAL_TO: u32 = 5;
/// `(?a ?b) math:sum ?c` -- slots 0 and 1 are the list elements, slot 2 the
/// output. See the slot-1 repurposing note on [`n3_derivation_check`].
pub global BUILTIN_SUM: u32 = 6;
/// `(?a ?b) math:difference ?c`.
pub global BUILTIN_DIFFERENCE: u32 = 7;
/// `(?a ?b) math:product ?c`.
pub global BUILTIN_PRODUCT: u32 = 8;
/// The largest whitelisted tag. A rule citing anything above this has no
/// satisfying gadget and is rejected -- the fail-closed edge of the declared
/// subset (design sec 4.3).
pub global BUILTIN_MAX: u32 = 8;

/// Term-encoding-triple equality (an encoding equality IS a term equality over
/// the salt-independent encoding -- the same primitive `derivation.nr` and
/// `sameas.nr` use).
fn triple_eq(a: [Field; 3], b: [Field; 3]) -> bool {
    (a[0] == b[0]) & (a[1] == b[1]) & (a[2] == b[2])
}

/// `row` occurs in some active slot of the graph `(enc, count)` -- the
/// `scan.nr` / `derivation.nr` whole-graph-in-circuit membership discipline
/// (membership is equality against witnessed slots; no Merkle machinery).
fn row_present_in<let N: u32>(enc: [[Field; 3]; N], count: u32, row: [Field; 3]) -> bool {
    let mut present = false;
    for i in 0..N {
        present |= (i < count) & triple_eq(enc[i], row);
    }
    present
}

/// The canonical commitment leaf of ONE witnessed rule shape. Folding every
/// slot (`kind`, `konst`, `var`) plus the builtin tags and the two lengths means
/// `rules_root` pins the rule EXACTLY: a prover who edits any slot, retags a
/// builtin, or shortens a premise gets a different leaf and fails the step-2
/// recompute.
///
/// Padding atoms (`>= prem_len` / `>= concl_len`) are hashed too. That costs
/// nothing in soundness -- the padding is whatever the ISSUER committed, and
/// the prover must reproduce it -- but the host builder zeroes it so the leaf
/// is canonical for a given rule. The host mirror
/// (`sparq_zk_compose::n3::rule_leaf`) folds in exactly this order.
pub(crate) fn rule_leaf<let P: u32, let C: u32>(
    prem_len: u32,
    concl_len: u32,
    builtin: [u32; P],
    prem_kind: [[u32; 3]; P],
    prem_konst: [[Field; 3]; P],
    prem_var: [[u32; 3]; P],
    concl_kind: [[u32; 3]; C],
    concl_konst: [[Field; 3]; C],
    concl_var: [[u32; 3]; C],
) -> Field {
    let mut acc = h2(prem_len as Field, concl_len as Field);
    for a in 0..P {
        acc = h2(acc, builtin[a] as Field);
        for s in 0..3 {
            acc = h4(acc, prem_kind[a][s] as Field, prem_konst[a][s], prem_var[a][s] as Field);
        }
    }
    for c in 0..C {
        for s in 0..3 {
            acc = h4(acc, concl_kind[c][s] as Field, concl_konst[c][s], concl_var[c][s] as Field);
        }
    }
    acc
}

/// Instantiate one pattern slot under a node's substitution: a CONSTANT slot
/// yields its committed encoding, a VARIABLE slot the substitution's value for
/// that variable id.
///
/// The linear select (rather than a dynamic index) matches the discipline of
/// `sameas.nr` / `path.nr`. Because the substitution is an ARRAY indexed by
/// variable id, a variable occurring in several slots necessarily resolves to
/// the SAME value -- that is where "consistent variable substitution" (design
/// sec 4.1 move 2) comes from: it is structural, not an extra constraint that
/// could be forgotten.
fn instantiate<let V: u32>(kind: u32, konst: Field, var: u32, subst: [Field; V]) -> Field {
    let mut out = konst;
    for v in 0..V {
        let hit = (kind == SLOT_VAR) & (var == v);
        out = if hit { subst[v] } else { out };
    }
    out
}

/// The whitelisted `math:` builtin gadgets, evaluated over the INSTANTIATED
/// operand encodings of one premise atom.
///
/// `enabled` masks every constraint that involves `inst`, so a JOIN atom (or an
/// inactive padding atom) pays the shape but asserts nothing about it. The
/// canonical-digit checks inside [`signed_int_encoding`] are NOT masked (they
/// constrain only the witness, never `inst`), so the host MUST fill unused
/// operand slots with canonical padding digits -- e.g. `1` followed by `0`s,
/// since a magnitude of two or more digits may not lead with `0`.
///
/// Operand layout, matching how `sparq-reason` presents the atom:
/// - comparison `?a math:<cmp> ?b` -> slot 0 is `?a`, slot 2 is `?b`;
/// - arithmetic `(?a ?b) math:<op> ?c` -> slots 0 and 1 are the list elements,
///   slot 2 is the output.
///
/// **Soundness of the numeric lane.** Each operand's value is tied to its term
/// ENCODING by [`signed_int_encoding`] -- the same canonical-`xsd:integer`
/// token rebuild `filter_signed_int_check` uses. Because the lexical form is
/// canonical, the encoding determines the value uniquely and vice versa, so a
/// prover can neither read one encoding as two numbers nor emit a second
/// encoding for a computed result. The comparison verdict is
/// [`signed_verdict`] VERBATIM (design sec 4.3's "reuse the existing circuits").
///
/// **Range checks on the arithmetic.** Every magnitude is `< 2^64` (`MD <= 19`
/// digits), so `v0 + v1`, `v0 - v1` and `v0 * v1` are bounded by `2^128`, far
/// below the ~`2^254` field modulus: the field equations are EXACT and no
/// wraparound can alias a wrong answer. A true result outside the representable
/// `MD`-digit range simply has no canonical witness, so the proof fails CLOSED
/// rather than returning a truncated answer.
fn builtin_gadget<let MD: u32>(
    enabled: bool,
    tag: u32,
    inst: [Field; 3],
    neg: [bool; 3],
    digits: [[u8; MD]; 3],
) {
    let (e0, n0, m0) = signed_int_encoding(neg[0], digits[0]);
    let (e1, n1, m1) = signed_int_encoding(neg[1], digits[1]);
    let (e2, n2, m2) = signed_int_encoding(neg[2], digits[2]);

    let is_arith = (tag == BUILTIN_SUM) | (tag == BUILTIN_DIFFERENCE) | (tag == BUILTIN_PRODUCT);
    let is_cmp = !is_arith;

    // Operand binding: the witnessed value must BE the instantiated term. Slot
    // 1 carries an operand only for arithmetic (for a comparison it is the
    // atom's unused predicate slot).
    assert(!enabled | (e0 == inst[0]), "builtin operand encoding mismatch");
    assert(!enabled | !is_arith | (e1 == inst[1]), "builtin operand encoding mismatch");
    assert(!enabled | (e2 == inst[2]), "builtin operand encoding mismatch");

    // Comparisons -- `signed_verdict` reused verbatim, value = slot 0, bound =
    // slot 2. All five map onto the existing FILTER operator selectors.
    let op = if tag == BUILTIN_GREATER_THAN {
        OP_GT
    } else if tag == BUILTIN_LESS_THAN {
        OP_LT
    } else if tag == BUILTIN_NOT_GREATER_THAN {
        OP_LE
    } else if tag == BUILTIN_NOT_LESS_THAN {
        OP_GE
    } else {
        OP_EQ
    };
    assert(
        !enabled | !is_cmp | signed_verdict(n0, m0, n2, m2, op),
        "math: comparison builtin does not hold",
    );

    // Arithmetic -- exact field equations over the signed values.
    let zero: Field = 0;
    let v0 = if n0 { zero - (m0 as Field) } else { m0 as Field };
    let v1 = if n1 { zero - (m1 as Field) } else { m1 as Field };
    let v2 = if n2 { zero - (m2 as Field) } else { m2 as Field };
    assert(
        !enabled | (tag != BUILTIN_SUM) | (v0 + v1 == v2),
        "math:sum builtin does not hold",
    );
    assert(
        !enabled | (tag != BUILTIN_DIFFERENCE) | (v0 - v1 == v2),
        "math:difference builtin does not hold",
    );
    assert(
        !enabled | (tag != BUILTIN_PRODUCT) | (v0 * v1 == v2),
        "math:product builtin does not hold",
    );
}

/// Check that every ACTIVE committed rule lies in the PROVABLE SUBSET: its
/// slot kinds and variable indices are well formed, its builtins are
/// whitelisted, its builtin inputs are bound before use, and -- the load-bearing
/// one -- every CONCLUSION variable is bound by the premise (safety /
/// range-restriction).
///
/// This is checked over the WHOLE committed rule set rather than only the rules
/// a given proof fires, which is both cheaper (`O(R)`, not `O(M*R)`) and the
/// strict reading of design sec 4.3's "a rule outside the subset MUST be
/// rejected, not silently approximated". It is also exactly consistent with the
/// host gate: `sparq_zk_compose::n3::N3RuleSet::admit` refuses to COMMIT a rule
/// graph containing an out-of-subset rule, so an honest issuer's `rules_root`
/// always satisfies this.
///
/// The binding schedule walks the premise atoms IN ORDER, which is the order
/// `sparq-reason` has already reordered them into (a builtin runs only after the
/// atoms producing its inputs).
fn rule_subset_check<let R: u32, let P: u32, let C: u32, let V: u32>(
    rule_count: u32,
    prem_len: [u32; R],
    concl_len: [u32; R],
    prem_builtin: [[u32; P]; R],
    prem_kind: [[[u32; 3]; P]; R],
    prem_var: [[[u32; 3]; P]; R],
    concl_kind: [[[u32; 3]; C]; R],
    concl_var: [[[u32; 3]; C]; R],
) {
    for r in 0..R {
        let rule_active = r < rule_count;
        assert(!rule_active | (prem_len[r] >= 1), "rule has an empty premise");
        assert(!rule_active | (concl_len[r] >= 1), "rule has an empty conclusion");
        assert(!rule_active | (prem_len[r] <= P), "rule premise exceeds circuit bucket");
        assert(!rule_active | (concl_len[r] <= C), "rule conclusion exceeds circuit bucket");

        // Variables bound so far, in premise order.
        let mut bound: [bool; V] = [false; V];

        for a in 0..P {
            let atom_active = rule_active & (a < prem_len[r]);
            let tag = prem_builtin[r][a];
            assert(
                !atom_active | (tag <= BUILTIN_MAX),
                "rule uses a builtin outside the whitelist",
            );
            let is_join = tag == BUILTIN_NONE;
            let is_arith =
                (tag == BUILTIN_SUM) | (tag == BUILTIN_DIFFERENCE) | (tag == BUILTIN_PRODUCT);
            let is_cmp = !is_join & !is_arith;

            for s in 0..3 {
                let kind = prem_kind[r][a][s];
                let var = prem_var[r][a][s];
                let is_var = kind == SLOT_VAR;
                assert(!atom_active | (kind <= SLOT_VAR), "unknown pattern slot kind");
                assert(
                    !atom_active | !is_var | (var < V),
                    "variable index exceeds circuit bucket",
                );

                // A builtin's INPUT slots must already be bound: both operands
                // of a comparison (slots 0, 2), the two list elements of an
                // arithmetic op (slots 0, 1). Slot 2 of an arithmetic op is the
                // OUTPUT and is bound BY the gadget, below.
                let is_input =
                    (is_cmp & ((s == 0) | (s == 2))) | (is_arith & ((s == 0) | (s == 1)));
                let mut already = false;
                for v in 0..V {
                    already |= (var == v) & bound[v];
                }
                assert(
                    !atom_active | !is_input | !is_var | already,
                    "builtin input variable is not bound by an earlier premise atom",
                );
            }

            // Bind: a join atom binds every variable slot it carries; an
            // arithmetic builtin binds its output slot (the gadget determines
            // its value from the already-bound inputs).
            for s in 0..3 {
                let binds = is_join | (is_arith & (s == 2));
                let is_var = prem_kind[r][a][s] == SLOT_VAR;
                for v in 0..V {
                    let hit = atom_active & binds & is_var & (prem_var[r][a][s] == v);
                    bound[v] = bound[v] | hit;
                }
            }
        }

        // SAFETY: every conclusion variable is bound in the premise. Without
        // this the prover picks the substitution freely and "derives" anything.
        for c in 0..C {
            let catom_active = rule_active & (c < concl_len[r]);
            for s in 0..3 {
                let kind = concl_kind[r][c][s];
                let var = concl_var[r][c][s];
                let is_var = kind == SLOT_VAR;
                assert(!catom_active | (kind <= SLOT_VAR), "unknown pattern slot kind");
                assert(
                    !catom_active | !is_var | (var < V),
                    "variable index exceeds circuit bucket",
                );
                let mut ok = false;
                for v in 0..V {
                    ok |= (var == v) & bound[v];
                }
                assert(
                    !catom_active | !is_var | ok,
                    "rule is unsafe: a conclusion variable is not bound in the premise",
                );
            }
        }
    }
}

/// The generic N3 witnessed-rule-shape derivation relation; an
/// `n3_k{K}_n{N}_r{R}_m{M}` bin package would be one monomorphisation
/// (`K` graphs, `N` slots/graph, `R` rules, `P` premise atoms/rule,
/// `C` conclusion atoms/rule, `M` DAG nodes, `V` variables/rule, `MD` builtin
/// magnitude digits). See the module docs for the full statement and the
/// public/private split.
///
/// `commitments` (PUBLIC) MUST byte-equal the scan sub-proofs' `commitments[g]`
/// (the verifier's entailment-binding gate -- NOT enforced here; this is the
/// in-circuit relation). `rules_root` (PUBLIC) MUST be the rule-graph root the
/// ISSUER signed (the `issuer.nr` member binds root to key; this relation binds
/// the fired rules to the root). `root` (PUBLIC) is the disclosed derived
/// triple encoding, bound to the DAG's terminal conclusion.
///
/// **Slot-1 repurposing.** For a BUILTIN atom the predicate is carried by the
/// tag, so pattern slot 1 holds no predicate; it is REUSED as the second list
/// element of a binary arithmetic builtin (`(?a ?b) math:sum ?c`). For a
/// comparison it is unconstrained. It is still folded into [`rule_leaf`], so
/// the repurposing is committed like every other slot.
pub fn n3_derivation_check<let K: u32, let N: u32, let R: u32, let P: u32, let C: u32, let M: u32, let V: u32, let MD: u32>(
    // PUBLIC
    commitments: [Field; K],
    rules_root: Field,
    root: [Field; 3],
    // PRIVATE -- the committed graphs
    counts: [u32; K],
    enc: [[[Field; 3]; N]; K],
    // PRIVATE -- the committed rule graph (only its ROOT is public)
    rule_count: u32,
    prem_len: [u32; R],
    concl_len: [u32; R],
    prem_builtin: [[u32; P]; R],
    prem_kind: [[[u32; 3]; P]; R],
    prem_konst: [[[Field; 3]; P]; R],
    prem_var: [[[u32; 3]; P]; R],
    concl_kind: [[[u32; 3]; C]; R],
    concl_konst: [[[Field; 3]; C]; R],
    concl_var: [[[u32; 3]; C]; R],
    // PRIVATE -- the derivation DAG
    node_count: u32,
    node_is_leaf: [bool; M],
    node_rule: [u32; M],
    node_concl: [u32; M],
    node_subst: [[Field; V]; M],
    node_out: [[Field; 3]; M],
    // PRIVATE -- builtin operand witnesses, per node per premise atom
    op_neg: [[[bool; 3]; P]; M],
    op_digits: [[[[u8; MD]; 3]; P]; M],
) {
    assert(node_count <= M, "node_count exceeds circuit bucket");
    assert(rule_count <= R, "rule_count exceeds circuit bucket");
    assert(rule_count >= 1, "empty rule set proves nothing");

    // 1. Commitment recompute -- binds every witnessed graph slot to the PUBLIC
    //    commitment (identical to scan.nr / derivation.nr step 1), so leaf
    //    anchoring is membership in the SAME C(G) the issuer signed.
    for g in 0..K {
        assert(counts[g] <= N, "graph size exceeds circuit bucket");
        let mut leaves: [Field; N] = [0; N];
        for i in 0..N {
            leaves[i] = h3(enc[g][i][0], enc[g][i][1], enc[g][i][2]);
        }
        assert_eq(commit_fold(leaves, counts[g]), commitments[g], "graph commitment mismatch");
    }

    // 2. Rule-set commitment recompute -- the rule author as ISSUER (design sec
    //    4.1 move 1). Every rule shape the DAG may cite is pinned by the PUBLIC
    //    `rules_root`; a fabricated rule changes the root and fails here.
    let mut rule_leaves: [Field; R] = [0; R];
    for r in 0..R {
        rule_leaves[r] = rule_leaf(
            prem_len[r],
            concl_len[r],
            prem_builtin[r],
            prem_kind[r],
            prem_konst[r],
            prem_var[r],
            concl_kind[r],
            concl_konst[r],
            concl_var[r],
        );
    }
    assert_eq(commit_fold(rule_leaves, rule_count), rules_root, "rule-set commitment mismatch");

    // 3. Every committed rule lies in the declared PROVABLE SUBSET -- notably
    //    SAFETY, without which the private substitution would let the prover
    //    choose an unbound conclusion variable's value.
    // `V` appears only in the body (the binding-schedule accumulator), so it
    // cannot be inferred from the arguments -- name it explicitly.
    rule_subset_check::<R, P, C, V>(
        rule_count,
        prem_len,
        concl_len,
        prem_builtin,
        prem_kind,
        prem_var,
        concl_kind,
        concl_var,
    );

    // 4. Per-node soundness + root binding.
    let mut root_bound = false;
    for i in 0..M {
        let active = i < node_count;
        let is_leaf = node_is_leaf[i];

        // 4a. LEAF anchoring: an active leaf's fact must occur in some
        //     committed graph -- the hidden-antecedent soundness anchor. A leaf
        //     claiming a fact in no C(G) has no witness.
        let mut present = false;
        for g in 0..K {
            present |= row_present_in(enc[g], counts[g], node_out[i]);
        }
        assert(
            !active | !is_leaf | present,
            "n3 derivation leaf not present in committed graph",
        );

        // 4b. RULE firing, checked against each CANDIDATE rule under a mask.
        //     Exactly one `r` can be selected (the equality test is on a single
        //     witnessed index), and `rule_selected` forces one to be.
        let mut rule_selected = false;
        for r in 0..R {
            let sel = active & !is_leaf & (r < rule_count) & (node_rule[i] == r);
            rule_selected |= sel;

            for a in 0..P {
                let atom_active = sel & (a < prem_len[r]);
                let is_join = prem_builtin[r][a] == BUILTIN_NONE;

                let mut inst: [Field; 3] = [0; 3];
                for s in 0..3 {
                    inst[s] = instantiate(
                        prem_kind[r][a][s],
                        prem_konst[r][a][s],
                        prem_var[r][a][s],
                        node_subst[i],
                    );
                }

                // JOIN atom: the instantiated premise must equal a STRICTLY
                // EARLIER active node's conclusion. Chaining bottoms out at
                // anchored leaves, so an ungrounded antecedent -- neither a
                // committed fact nor an earlier conclusion -- fails here. This
                // is the forge-closure.
                let mut chained = false;
                for j in 0..M {
                    chained |= (j < i) & (j < node_count) & triple_eq(inst, node_out[j]);
                }
                assert(!atom_active | !is_join | chained, "n3 premise not grounded");

                // BUILTIN atom: the whitelisted gadget must hold over the
                // instantiated operands. Nothing else grounds it -- a builtin
                // premise is COMPUTED, not matched.
                builtin_gadget(
                    atom_active & !is_join,
                    prem_builtin[r][a],
                    inst,
                    op_neg[i][a],
                    op_digits[i][a],
                );
            }

            // 4c. The node's triple IS the rule's selected conclusion atom under
            //     the SAME substitution (design sec 4.1 move 2).
            assert(
                !sel | (node_concl[i] < concl_len[r]),
                "node selects a conclusion atom outside the rule",
            );
            for c in 0..C {
                let csel = sel & (c < concl_len[r]) & (node_concl[i] == c);
                let mut inst: [Field; 3] = [0; 3];
                for s in 0..3 {
                    inst[s] = instantiate(
                        concl_kind[r][c][s],
                        concl_konst[r][c][s],
                        concl_var[r][c][s],
                        node_subst[i],
                    );
                }
                assert(
                    !csel | triple_eq(inst, node_out[i]),
                    "node conclusion is not the rule's conclusion under the substitution",
                );
            }
        }
        assert(
            !active | is_leaf | rule_selected,
            "node cites a rule outside the committed rule set",
        );

        // 4d. Root binding: the LAST active node is the terminal conclusion and
        //     MUST equal the disclosed derived row. Folding the equality into
        //     the accumulator also forces `node_count >= 1`.
        let is_last_active = active & !((i + 1) < node_count);
        root_bound |= is_last_active & triple_eq(node_out[i], root);
    }
    assert(root_bound, "n3 derivation root not bound to the disclosed derived row");
}
