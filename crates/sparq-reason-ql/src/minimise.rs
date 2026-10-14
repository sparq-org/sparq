// sq-g19x0 (epic sq-pbz04): UCQ-containment MINIMISATION — the standard
// query-containment-by-homomorphism test (Chandra–Merlin, STOC 1977), applied soundly to drop
// redundant disjuncts from a rewritten union of conjunctive queries.
//
// THE SOUNDNESS CONTRACT (read this — an over-aggressive minimisation is an UNSOUNDNESS bug).
//
// A conjunctive query q1 is CONTAINED in q2 (q1 ⊆ q2) iff for EVERY database D the answers of q1
// over D are a subset of the answers of q2 over D. Chandra–Merlin: q1 ⊆ q2 iff there is a
// HOMOMORPHISM h from q2's body INTO q1's body that fixes the answer (distinguished) variables —
// i.e. h maps each distinguished variable of q2 to the SAME distinguished variable of q1, maps
// constants to themselves, and maps every atom of q2 to an atom present in q1.
//
// In a UCQ U = { …, q1, …, q2, … }, if q1 ⊆ q2 then every answer q1 contributes is ALSO produced
// by q2, so DROPPING q1 leaves the UCQ's answers UNCHANGED. That is the only move this module
// makes, and it is sound exactly because of the containment direction: we drop the SMALLER
// (contained) query, never the larger one.
//
// FAIL-CLOSED. Query containment is NP-complete (the homomorphism search is the NP witness). We
// run a BOUNDED backtracking homomorphism search; if the search EXHAUSTS its node budget without
// a verdict we return `Containment::Unknown` and the caller KEEPS BOTH disjuncts. Dropping a
// disjunct on an uncertain (budget-exceeded) search would risk losing a non-contained disjunct —
// an unsoundness bug — so uncertainty NEVER drops. The budget is generous relative to the tiny
// (binary-atom, few-atom) CQs DL-Lite rewriting produces; it exists only so a pathological input
// cannot make minimisation non-terminating or quadratically slow.

use crate::perfectref::{Atom, Cq, Term};
use rustc_hash::{FxHashMap, FxHashSet};

/// The verdict of a single containment check `q1 ⊆ q2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Containment {
    /// A frozen homomorphism `q2 → q1` was found: `q1 ⊆ q2` definitely holds.
    Contained,
    /// No homomorphism exists (the search ran to completion): `q1 ⊆ q2` definitely does NOT hold.
    NotContained,
    /// The bounded search exhausted its node budget without a verdict. FAIL-CLOSED: the caller
    /// must treat this as "cannot prove containment" and KEEP the candidate disjunct.
    Unknown,
}

/// The node budget for one homomorphism search. Generous for the binary-atom, few-atom CQs
/// DL-Lite rewriting produces; bounds the NP-complete search so minimisation is always total.
const HOM_BUDGET: u64 = 200_000;

/// Minimise a UCQ by dropping every disjunct that is CONTAINED in another (kept) disjunct.
///
/// SOUND: a dropped disjunct's answers are a subset of a retained disjunct's answers, so the
/// UCQ's certain-answer set is preserved exactly. FAIL-CLOSED: a disjunct is dropped only on a
/// DEFINITE [`Containment::Contained`] verdict; an `Unknown` (budget-exceeded) check never drops.
///
/// The `answer` slice is the UCQ's shared distinguished (projected) variable names — a
/// homomorphism must fix these for the containment to hold. All disjuncts of one rewritten query
/// share the same answer variables (PerfectRef threads them unchanged), so a single slice suffices.
pub fn minimise_ucq(ucq: Vec<Cq>, answer: &[String]) -> Vec<Cq> {
    // Deduplicate structurally first (cheap; the rewriter already normalises, but tree-witness
    // folding can re-introduce a syntactic duplicate). `Cq` is `Eq + Hash`.
    let mut unique: Vec<Cq> = Vec::with_capacity(ucq.len());
    let mut seen: FxHashSet<Cq> = FxHashSet::default();
    for cq in ucq {
        if seen.insert(cq.clone()) {
            unique.push(cq);
        }
    }

    // A disjunct is redundant iff it is contained in SOME OTHER disjunct that we are KEEPING.
    // To stay sound under mutual containment (two structurally-distinct but equivalent CQs), we
    // keep the FIRST of any equivalent pair: a later disjunct is dropped if it is contained in an
    // earlier-indexed disjunct that has not itself been dropped.
    let n = unique.len();
    let mut dropped = vec![false; n];
    for i in 0..n {
        if dropped[i] {
            continue;
        }
        for j in 0..n {
            if i == j || dropped[j] {
                continue;
            }
            // Is unique[j] contained in unique[i]? If so, drop j (its answers ⊆ i's answers).
            // Skip if j is an earlier index that already "owns" i to avoid dropping both of a
            // mutually-contained pair: only drop j when i < j, OR (i > j and i is NOT contained
            // in j) — i.e. break ties by index so exactly one of an equivalent pair survives.
            if contains(&unique[i], &unique[j], answer) == Containment::Contained {
                let keep_j_for_tie = j < i
                    && contains(&unique[j], &unique[i], answer) == Containment::Contained;
                if !keep_j_for_tie {
                    dropped[j] = true;
                }
            }
        }
    }

    unique
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !dropped[*idx])
        .map(|(_, cq)| cq)
        .collect()
}

/// Decide `inner ⊆ outer` (every answer of `inner` is an answer of `outer`) via the frozen
/// homomorphism test: a homomorphism from `outer`'s body INTO `inner`'s body that fixes the
/// distinguished variables. Returns [`Containment::Unknown`] if the bounded search is exhausted.
pub fn contains(outer: &Cq, inner: &Cq, answer: &[String]) -> Containment {
    // Quick necessary condition: every distinguished variable of `outer` must occur in `inner`
    // too (a frozen hom must map it to itself, which must exist in the target). If not, no hom.
    let answer_set: FxHashSet<&str> = answer.iter().map(|s| s.as_str()).collect();
    // The image (target) is `inner`. Pre-bind every distinguished variable to itself.
    let mut subst: FxHashMap<String, Term> = FxHashMap::default();
    for a in answer {
        subst.insert(a.clone(), Term::Var(a.clone()));
    }
    let mut budget = HOM_BUDGET;
    match search_hom(&outer.atoms, 0, &inner.atoms, &mut subst, &answer_set, &mut budget) {
        Verdict::Found => Containment::Contained,
        Verdict::Exhausted => Containment::NotContained,
        Verdict::Budget => Containment::Unknown,
    }
}

enum Verdict {
    /// A complete homomorphism was found.
    Found,
    /// The whole search space was explored with no homomorphism.
    Exhausted,
    /// The node budget ran out before the space was fully explored.
    Budget,
}

/// Backtracking search: extend the partial substitution `subst` so that source atom `idx` (and
/// all later source atoms) map onto some target atom. Distinguished variables are FROZEN (bound
/// to themselves up front, in `answer_set`) and may not be rebound. Constants and `Unbound` are
/// rigid in the SOURCE — they must map to the identical term in the target.
fn search_hom(
    source: &[Atom],
    idx: usize,
    target: &[Atom],
    subst: &mut FxHashMap<String, Term>,
    answer_set: &FxHashSet<&str>,
    budget: &mut u64,
) -> Verdict {
    if *budget == 0 {
        return Verdict::Budget;
    }
    *budget -= 1;
    if idx == source.len() {
        return Verdict::Found; // all source atoms mapped.
    }
    let src = &source[idx];
    let mut hit_budget = false;
    for tgt in target {
        // Snapshot bindings introduced by THIS atom so we can roll back on failure.
        let saved = subst.clone();
        if match_atom(src, tgt, subst, answer_set) {
            match search_hom(source, idx + 1, target, subst, answer_set, budget) {
                Verdict::Found => return Verdict::Found,
                Verdict::Budget => hit_budget = true,
                Verdict::Exhausted => {}
            }
        }
        *subst = saved; // backtrack.
    }
    if hit_budget {
        Verdict::Budget
    } else {
        Verdict::Exhausted
    }
}

/// Try to extend `subst` so that source atom `src` maps to target atom `tgt`. Returns false (no
/// extension) on a predicate/role/constant mismatch or a binding conflict.
fn match_atom(
    src: &Atom,
    tgt: &Atom,
    subst: &mut FxHashMap<String, Term>,
    answer_set: &FxHashSet<&str>,
) -> bool {
    match (src, tgt) {
        (
            Atom::Class { class: c1, arg: a1 },
            Atom::Class { class: c2, arg: a2 },
        ) => c1 == c2 && bind(a1, a2, subst, answer_set),
        (
            Atom::Role { role: r1, s: s1, o: o1 },
            Atom::Role { role: r2, s: s2, o: o2 },
        ) => r1 == r2 && bind(s1, s2, subst, answer_set) && bind(o1, o2, subst, answer_set),
        _ => false,
    }
}

/// Bind one SOURCE term `s` to a TARGET term `t` under the homomorphism. The homomorphism maps
/// source variables to target terms; constants and `Unbound` existentials in the source are RIGID
/// (must equal the target term exactly). A distinguished (frozen) source variable must map to the
/// SAME distinguished variable in the target. Returns false on any conflict.
/// sq-pbz04.3.1: `Literal` arm added for B2 — a source literal constant maps only
/// to the identical target literal (like `Const`).
fn bind(
    s: &Term,
    t: &Term,
    subst: &mut FxHashMap<String, Term>,
    answer_set: &FxHashSet<&str>,
) -> bool {
    match s {
        // A source constant maps only to the identical target constant.
        Term::Const(c) => matches!(t, Term::Const(c2) if c2 == c),
        // B2: a source literal maps only to the identical target literal (same three fields).
        // sq-pbz04.3.1
        Term::Lit(lv, ld, ll) => {
            matches!(t, Term::Lit(lv2, ld2, ll2) if lv2 == lv && ld2 == ld && ll2 == ll)
        }
        // A source `_` existential is rigid under the standard CQ-homomorphism definition we use:
        // it is a NON-shared placeholder, but to stay SOUND we require it to map to the identical
        // target term. (Treating `_` as a mappable variable would be sound too, but only if we
        // also forbid two distinct `_`s collapsing onto one target in a way that loses a join;
        // requiring exact-match is the conservative, always-sound choice — it can only make us
        // KEEP more disjuncts, never wrongly drop one.)
        Term::Unbound(id) => matches!(t, Term::Unbound(id2) if id2 == id),
        Term::Var(v) => {
            if answer_set.contains(v.as_str()) {
                // Frozen distinguished variable: must map to itself (the same answer var).
                matches!(t, Term::Var(v2) if v2 == v)
            } else {
                // A non-distinguished source variable: bind it, or check consistency if bound.
                match subst.get(v) {
                    Some(prev) => prev == t,
                    None => {
                        subst.insert(v.clone(), t.clone());
                        true
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dllite::Role;

    fn class(c: &str, v: &str) -> Atom {
        Atom::Class {
            class: c.to_string(),
            arg: Term::Var(v.to_string()),
        }
    }
    fn class_const(c: &str, k: &str) -> Atom {
        Atom::Class {
            class: c.to_string(),
            arg: Term::Const(k.to_string()),
        }
    }
    fn role(r: &str, s: Term, o: Term) -> Atom {
        Atom::Role {
            role: Role::named(r),
            s,
            o,
        }
    }
    fn cq(atoms: Vec<Atom>, answer: &[&str]) -> Cq {
        Cq {
            atoms: {
                let mut a = atoms;
                a.sort();
                a.dedup();
                a
            },
            answer: answer.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn identical_cqs_are_mutually_contained() {
        let a = cq(vec![class("A", "x")], &["x"]);
        let b = cq(vec![class("A", "x")], &["x"]);
        assert_eq!(contains(&a, &b, &["x".into()]), Containment::Contained);
        assert_eq!(contains(&b, &a, &["x".into()]), Containment::Contained);
    }

    #[test]
    fn more_specific_is_contained_in_more_general() {
        // q1 = A(?x) ∧ B(?x) is contained in q2 = A(?x): every answer of q1 is an answer of q2,
        // because the hom q2 → q1 maps A(?x) onto the A(?x) atom of q1 (fixing ?x).
        let q1 = cq(vec![class("A", "x"), class("B", "x")], &["x"]);
        let q2 = cq(vec![class("A", "x")], &["x"]);
        assert_eq!(contains(&q2, &q1, &["x".into()]), Containment::Contained, "q1 ⊆ q2");
        // The reverse does NOT hold (q2 has answers q1 lacks): no hom q1 → q2 (B(?x) unmatched).
        assert_eq!(contains(&q1, &q2, &["x".into()]), Containment::NotContained);
    }

    #[test]
    fn distinct_classes_not_contained() {
        let q1 = cq(vec![class("A", "x")], &["x"]);
        let q2 = cq(vec![class("B", "x")], &["x"]);
        assert_eq!(contains(&q2, &q1, &["x".into()]), Containment::NotContained);
    }

    #[test]
    fn frozen_answer_var_must_match() {
        // q2 = A(?x) is NOT a superset of q1 = A(?y) when ?x and ?y are BOTH distinguished and
        // different: the frozen hom cannot map ?x to ?y.
        let q1 = cq(vec![class("A", "y")], &["y"]);
        let q2 = cq(vec![class("A", "x")], &["x"]);
        // Different answer signatures — there is no shared answer slice, but exercise the freeze:
        assert_eq!(contains(&q2, &q1, &["x".into(), "y".into()]), Containment::NotContained);
    }

    #[test]
    fn const_folds_into_existential_role_one_way() {
        // q1 = R(?x, :c) ; q2 = R(?x, ?w) with ?w NON-distinguished. q1 ⊆ q2: the hom q2 → q1
        // maps ?w ↦ :c (a non-distinguished source var maps to a constant). So R(?x,:c) ⊆ ∃w R(?x,w).
        let q1 = cq(
            vec![role("R", Term::Var("x".into()), Term::Const("c".into()))],
            &["x"],
        );
        let q2 = cq(
            vec![role("R", Term::Var("x".into()), Term::Var("w".into()))],
            &["x"],
        );
        assert_eq!(contains(&q2, &q1, &["x".into()]), Containment::Contained);
        // Reverse fails: ∃w R(?x,w) ⊄ R(?x,:c) (source const :c cannot map to a variable).
        assert_eq!(contains(&q1, &q2, &["x".into()]), Containment::NotContained);
    }

    #[test]
    fn minimise_drops_the_contained_disjunct() {
        // UCQ { A(?x), A(?x)∧B(?x) }: the second is contained in the first ⇒ minimise to { A(?x) }.
        let general = cq(vec![class("A", "x")], &["x"]);
        let specific = cq(vec![class("A", "x"), class("B", "x")], &["x"]);
        let out = minimise_ucq(vec![general.clone(), specific], &["x".into()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], general);
    }

    #[test]
    fn minimise_keeps_incomparable_disjuncts() {
        let a = cq(vec![class("A", "x")], &["x"]);
        let b = cq(vec![class("B", "x")], &["x"]);
        let out = minimise_ucq(vec![a, b], &["x".into()]);
        assert_eq!(out.len(), 2, "incomparable disjuncts must both survive");
    }

    #[test]
    fn minimise_dedups_structural_duplicates() {
        let a = cq(vec![class("A", "x")], &["x"]);
        let out = minimise_ucq(vec![a.clone(), a.clone(), a], &["x".into()]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn minimise_keeps_one_of_mutually_equivalent_pair() {
        // Two structurally-distinct but logically-equivalent CQs (a redundant self-join):
        //   q1 = R(?x,?y)             q2 = R(?x,?y) ∧ R(?x,?z)
        // q2 ⊆ q1 (hom q1→q2: identity) and q1 ⊆ q2 (hom q2→q1: ?z↦?y). They are EQUIVALENT.
        // Minimisation must keep EXACTLY ONE (never drop both → that would lose all answers).
        let q1 = cq(
            vec![role("R", Term::Var("x".into()), Term::Var("y".into()))],
            &["x", "y"],
        );
        let q2 = cq(
            vec![
                role("R", Term::Var("x".into()), Term::Var("y".into())),
                role("R", Term::Var("x".into()), Term::Var("z".into())),
            ],
            &["x", "y"],
        );
        let out = minimise_ucq(vec![q1, q2], &["x".into(), "y".into()]);
        assert_eq!(out.len(), 1, "exactly one of an equivalent pair survives (never zero)");
    }

    #[test]
    fn const_clash_not_contained() {
        let q1 = cq(vec![class_const("A", "c1")], &[]);
        let q2 = cq(vec![class_const("A", "c2")], &[]);
        assert_eq!(contains(&q2, &q1, &[]), Containment::NotContained);
    }
}
