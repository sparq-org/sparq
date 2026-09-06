// [FABLE-5] sq-p6yb7 (epic sq-pbz04.3, design record research/owl2-ql-cq-gate-broadening.md §8):
// DL-Lite_R KB consistency checking via VIOLATION-QUERY COMPOSITION. Opt-in `ql-consistency`
// feature (OFF by default; pulls `experimental` for the PerfectRef machinery it reuses).
//
// # The algorithm (standard DL-Lite technique)
//
// For each structurally-captured negative inclusion (`TBox::neg_incl`):
//
//   B1 ⊑ ¬B2  ⇒  violation CQ   q() ← B1(x), B2(x)      (some individual in both B1 and B2)
//   R1 ⊑ ¬R2  ⇒  violation CQ   q() ← R1(x,y), R2(x,y)  (some pair in both R1 and R2)
//
// each violation CQ is rewritten with the EXISTING PerfectRef saturation (`perfect_ref`) under
// the captured POSITIVE inclusions, and the resulting boolean UCQ is evaluated over the given
// triples (the ABox read off the data — an `rdf:type` triple is a class assertion, any other
// triple a role assertion). The KB is INCONSISTENT iff some violation UCQ has a nonempty answer.
//
// # Soundness of `Inconsistent` (holds at ANY capture level)
//
// Every captured axiom (positive or negative) is an EXACT reading of an asserted triple, and
// PerfectRef is sound for certain answers (Calvanese et al., JAR 39(3) 2007): a nonempty answer
// of the rewritten violation UCQ over the data witnesses `T ∪ A ⊨ ∃x. B1(x) ∧ B2(x)` (resp. the
// role pair), which together with the asserted `B1 ⊑ ¬B2` is a contradiction. Description logic
// is MONOTONIC, so an inconsistency derived from a SUBSET of the KB's axioms is an inconsistency
// of the whole KB — `Inconsistent` is therefore sound even when the TBox is only partially
// captured.
//
// # Completeness of `Consistent` (claimed ONLY over the fully-captured fragment)
//
// By the DL-Lite satisfiability theorem (Calvanese et al., JAR 39(3) 2007, §4: KB satisfiability
// reduces to evaluating the negative-inclusion closure cln(T) over db(A)), a DL-Lite_R KB (T, A)
// is UNSATISFIABLE iff some negative inclusion in cln(T) is violated in db(A). PerfectRef applied
// to the violation query of each ASSERTED negative inclusion simulates that closure exactly —
// each cln rule corresponds to a rewrite step on a violation-CQ atom:
//
//   * `B1 ⊑ B2, B2 ⊑ ¬B3 ⇒ B1 ⊑ ¬B3`: rewriting the `B2(x)` atom of q() ← B2(x), B3(x)
//     backward by the positive inclusion `B1 ⊑ B2` yields q() ← B1(x), B3(x) — the violation
//     query of the derived negative inclusion. (`∃R ⊑ A` turns a class atom into `R(x, _)`;
//     `A ⊑ ∃R` fires on `R(x, _)` because the introduced filler is `_` — always `_`-eligible.)
//   * `R1 ⊑ R2, R2 ⊑ ¬R3 ⇒ R1 ⊑ ¬R3` and `R1 ⊑ R2, ∃R2 ⊑ ¬B ⇒ ∃R1 ⊑ ¬B`: the role-inclusion
//     rewrite (inverse directions included) on the corresponding role atom.
//   * Violations witnessed by ANONYMOUS individuals of the canonical model (e.g. `A ⊑ ∃R`,
//     `∃R⁻ ⊑ B1`, `∃R⁻ ⊑ B2`, `B1 ⊑ ¬B2`, ABox `A(a)`) are found through PerfectRef's REDUCE
//     step: q() ← B1(x), B2(x) rewrites to q() ← R(_,x), R(_,x)-shaped pairs, reduce unifies
//     them, the now-unshared `x` anonymises to `_`, and `A ⊑ ∃R` folds `R(_,_)` to `A(_)`,
//     which the data satisfies. The same mechanism covers role-disjointness violations on
//     TBox-generated edges (a generated edge belongs to every super-role of its generator).
//
// The argument is stated over ASSERTED negative inclusions, NOT over the RDF syntax that carried
// them, so it is stable under a broadened capture: `TBox::neg_incl` is the set of asserted NIs,
// however spelled. Re-checked for the subClassOf-complement capture (sq-fj8lj follow-up),
// `B1 rdfs:subClassOf [ owl:complementOf B2 ]`: the axiom it denotes is `B1 ⊑ ¬B2` — EXACTLY the
// DL-Lite_R negative inclusion, not an approximation of a stronger axiom (contrast the
// NAMED-subject `A owl:complementOf B`, which asserts the biconditional `A ≡ ¬B`; its `¬B ⊑ A`
// half has no NI form, so it stays uncaptured and keeps `Consistent` fail-closed). `B1 ⊑ ¬B2` is
// interderivable with the disjointness `B1 ⊓ B2 ⊑ ⊥` that `owl:disjointWith` asserts, so the
// captured value lands in the SAME `NegativeInclusion::Concept(Basic, Basic)` shape, composes the
// SAME violation query `q() ← B1(x), B2(x)`, and enters cln(T) by the same rules — every step of
// the closure simulation above applies verbatim, with no new rule and no new witness shape. What
// changes is only WHICH KBs reach the gates: a graph whose sole negative axiom is a
// subClassOf-complement now has `consistency_uncaptured == 0` and (the shape no longer being
// counted `skipped`) can satisfy `fully_captured()`, so it can graduate a definitive verdict on
// either side instead of an unavoidable `Unknown`.
//
// So if NO rewritten violation UCQ matches the data, no negative inclusion of cln(T) is violated
// in db(A), and the canonical model of the positive inclusions is a model of the whole captured
// KB — `Consistent`. That argument needs EVERY axiom captured: `fully_captured()` must hold
// (nothing skipped or unrecognised) AND every negative axiom must be structurally in `neg_incl`
// (`consistency_uncaptured == 0`). A KB failing either stays `Unknown` — fail-closed, NEVER
// silently assumed consistent.
//
// (DL-Lite_F/A functionality assertions — not in OWL 2 QL — reach neither bucket: extraction
// counts `owl:FunctionalProperty` typings in `unrecognised_schema`, blocking `fully_captured()`.)
//
// # Entailment by inconsistency
//
// From an inconsistent KB everything is entailed: the certain answers of ANY query are all
// tuples over the active domain. A consumer holding an `Inconsistent` verdict must therefore
// not use the positive UCQ rewriting's answers as THE certain answers (they under-approximate);
// the verdict itself is the definitive answer. Consumers that need fail-closed behaviour on an
// implementation-defined surface (e.g. the W3C entailment-regime arm, where inconsistent-graph
// behaviour is left to the system) should hold rather than guess — see the graduation predicate
// in sparq-conformance.

use crate::dllite::{Basic, NegativeInclusion, Role, TBox};
use crate::perfectref::{perfect_ref, Atom, Cq, Term};
use oxrdf::{NamedOrBlankNode, Term as RdfTerm, Triple};
use rustc_hash::FxHashMap;

/// The verdict of [`check_consistency`]: definitive where the captured fragment supports it,
/// honest `Unknown` everywhere else. Never a guessed `Consistent`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QlConsistency {
    /// The KB is satisfiable. Returned ONLY when the TBox is `fully_captured()` AND every
    /// negative axiom was structurally captured (`consistency_uncaptured == 0`) AND no
    /// violation query matched — the completeness argument in the module header applies.
    Consistent,
    /// The KB is unsatisfiable: the carried negative inclusion's violation query (rewritten
    /// under the positive inclusions) matched the data. Sound at ANY capture level
    /// (monotonicity). Everything is entailed from an inconsistent KB.
    Inconsistent(QlViolation),
    /// No violation was found, but the captured fragment cannot support a `Consistent` claim.
    /// Fail-closed: an uncaptured axiom could make the KB inconsistent.
    Unknown(QlConsistencyGap),
}

/// The witness carried by [`QlConsistency::Inconsistent`]: the asserted negative inclusion
/// whose (rewritten) violation query matched the data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QlViolation {
    /// The violated negative inclusion, as asserted in the TBox.
    pub axiom: NegativeInclusion,
}

/// Why a no-violation KB stays [`QlConsistency::Unknown`] instead of `Consistent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QlConsistencyGap {
    /// `TBox::fully_captured()` is false — an axiom outside the captured DL-Lite_R fragment
    /// (or unrecognised schema vocabulary) could make the KB inconsistent in a way the
    /// violation queries cannot see.
    NotFullyCaptured {
        /// Axioms recognised-but-skipped as outside the DL-Lite_R fragment.
        skipped: usize,
        /// OWL/RDFS constructs the extractor does not model.
        unrecognised_schema: usize,
    },
    /// Some consistency-relevant axiom was not structurally captured into `TBox::neg_incl`
    /// (a NAMED-subject `owl:complementOf`, or a disjointness / subClassOf-complement with a
    /// non-basic operand) — its violations are not covered by the composed queries.
    UncapturedNegativeAxioms {
        /// `TBox::consistency_uncaptured`.
        uncaptured: usize,
    },
}

/// Check DL-Lite_R KB consistency over `kb` (schema + data triples together, as extraction
/// reads them). Extracts the TBox itself and delegates to [`check_consistency_with`].
pub fn check_consistency(kb: &[Triple]) -> QlConsistency {
    check_consistency_with(&TBox::extract(kb), kb)
}

/// Check DL-Lite_R KB consistency with an already-extracted `tbox` (which MUST come from
/// `TBox::extract` over the same `kb` — the verdict is only meaningful for the extraction's
/// own accounting). See the module header for the soundness/completeness argument.
pub fn check_consistency_with(tbox: &TBox, kb: &[Triple]) -> QlConsistency {
    // Violation sweep FIRST: `Inconsistent` is sound at any capture level (monotonicity), so
    // a partially-captured KB can still get a definitive INCONSISTENT verdict.
    for ni in &tbox.neg_incl {
        let ucq = perfect_ref(violation_atoms(ni), Vec::new(), tbox);
        if ucq.iter().any(|cq| cq_nonempty(cq, kb)) {
            return QlConsistency::Inconsistent(QlViolation { axiom: ni.clone() });
        }
    }
    // No violation found. `Consistent` needs the completeness argument, which needs total
    // capture — otherwise fail-closed `Unknown`.
    if !tbox.fully_captured() {
        return QlConsistency::Unknown(QlConsistencyGap::NotFullyCaptured {
            skipped: tbox.skipped,
            unrecognised_schema: tbox.unrecognised_schema,
        });
    }
    if tbox.consistency_uncaptured > 0 {
        return QlConsistency::Unknown(QlConsistencyGap::UncapturedNegativeAxioms {
            uncaptured: tbox.consistency_uncaptured,
        });
    }
    QlConsistency::Consistent
}

/// The violation CQ body of one negative inclusion (boolean query — empty answer tuple).
/// `B1 ⊑ ¬B2` → `B1(x), B2(x)`; `R1 ⊑ ¬R2` → `R1(x,y), R2(x,y)`. Fresh `Unbound` fillers for
/// `∃R` operands start at 0 — `perfect_ref`'s `seed_fresh` seeds its counter above them.
fn violation_atoms(ni: &NegativeInclusion) -> Vec<Atom> {
    let mut next = 0u32;
    let mut fresh = move || {
        let t = Term::Unbound(next);
        next += 1;
        t
    };
    match ni {
        NegativeInclusion::Concept(b1, b2) => {
            let x = Term::Var("x".into());
            vec![
                basic_atom(b1, x.clone(), &mut fresh),
                basic_atom(b2, x, &mut fresh),
            ]
        }
        NegativeInclusion::Role(r1, r2) => {
            let (x, y) = (Term::Var("x".into()), Term::Var("y".into()));
            vec![
                role_atom_forward(r1, x.clone(), y.clone()),
                role_atom_forward(r2, x, y),
            ]
        }
    }
}

/// The atom asserting membership of `arg` in basic concept `b`; `∃R` membership is an R-edge
/// to a fresh unshared existential.
fn basic_atom(b: &Basic, arg: Term, fresh: &mut impl FnMut() -> Term) -> Atom {
    match b {
        Basic::Class(c) => Atom::Class {
            class: c.clone(),
            arg,
        },
        Basic::Exists(r) => role_atom_forward(r, arg, fresh()),
    }
}

/// Build `R(s, o)` normalised to the FORWARD named property (the invariant every atom in the
/// PerfectRef pipeline maintains): an inverse role swaps subject/object.
fn role_atom_forward(r: &Role, s: Term, o: Term) -> Atom {
    if r.inverse {
        Atom::Role {
            role: Role::named(r.iri.clone()),
            s: o,
            o: s,
        }
    } else {
        Atom::Role {
            role: r.clone(),
            s,
            o,
        }
    }
}

/// A binding key: named variables and `Unbound` ids are BOTH joinable positions (an `Unbound`
/// id can be shared after emit-side blank-node lifting or a reduce unification, so repeated
/// ids must bind consistently — mirroring `emit::cq_to_bgp`'s one-variable-per-id rule).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum BindKey {
    Var(String),
    Unbound(u32),
}

/// Boolean evaluation: does `cq` (all variables existential) have a match in `kb`? Plain
/// backtracking join, linear scan per atom — violation CQs have ≤ 2 atoms and this runs on
/// consistency checks, not the query hot path.
fn cq_nonempty(cq: &Cq, kb: &[Triple]) -> bool {
    let mut binding: FxHashMap<BindKey, RdfTerm> = FxHashMap::default();
    sat(&cq.atoms, 0, kb, &mut binding)
}

fn sat(atoms: &[Atom], i: usize, kb: &[Triple], binding: &mut FxHashMap<BindKey, RdfTerm>) -> bool {
    let Some(atom) = atoms.get(i) else {
        return true; // every atom matched
    };
    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    for t in kb {
        // Collect this triple's (pattern, value) pairs for the atom, or skip the triple.
        let mut pairs: Vec<(&Term, RdfTerm)> = Vec::with_capacity(2);
        match atom {
            // Class atom A(arg): an `rdf:type` triple whose object is exactly the named class.
            Atom::Class { class, arg } => {
                if t.predicate.as_str() != RDF_TYPE
                    || !matches!(&t.object, RdfTerm::NamedNode(n) if n.as_str() == class)
                {
                    continue;
                }
                pairs.push((arg, subject_term(&t.subject)));
            }
            // Role atom R(s, o): a triple with predicate R; an inverse role reads it swapped.
            Atom::Role { role, s, o } => {
                if t.predicate.as_str() != role.iri {
                    continue;
                }
                let (sp, op) = if role.inverse { (o, s) } else { (s, o) };
                pairs.push((sp, subject_term(&t.subject)));
                pairs.push((op, t.object.clone()));
            }
        }
        let mut undo: Vec<BindKey> = Vec::new();
        let ok = pairs
            .iter()
            .all(|(pat, val)| match_term(pat, val, binding, &mut undo));
        if ok && sat(atoms, i + 1, kb, binding) {
            return true;
        }
        for k in undo {
            binding.remove(&k);
        }
    }
    false
}

fn subject_term(s: &NamedOrBlankNode) -> RdfTerm {
    match s {
        NamedOrBlankNode::NamedNode(n) => RdfTerm::NamedNode(n.clone()),
        NamedOrBlankNode::BlankNode(b) => RdfTerm::BlankNode(b.clone()),
    }
}

/// Match one atom-term against one data term, extending `binding` (recording insertions in
/// `undo` so the caller can backtrack). Constants/literals are rigid; variables and `Unbound`
/// ids bind-or-compare.
fn match_term(
    pat: &Term,
    val: &RdfTerm,
    binding: &mut FxHashMap<BindKey, RdfTerm>,
    undo: &mut Vec<BindKey>,
) -> bool {
    let key = match pat {
        Term::Const(iri) => {
            return matches!(val, RdfTerm::NamedNode(n) if n.as_str() == iri);
        }
        Term::Lit(lv, ld, lang) => {
            return matches!(val, RdfTerm::Literal(l)
                if l.value() == lv
                    && l.datatype().as_str() == ld
                    && l.language() == lang.as_deref());
        }
        Term::Var(v) => BindKey::Var(v.clone()),
        Term::Unbound(id) => BindKey::Unbound(*id),
    };
    if let Some(bound) = binding.get(&key) {
        return bound == val;
    }
    binding.insert(key.clone(), val.clone());
    undo.push(key);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn triple(s: &str, p: &str, o: &str) -> Triple {
        Triple::new(iri(s), iri(p), iri(o))
    }

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    const SUBCLASS: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
    const SUBPROP: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";
    const DOMAIN: &str = "http://www.w3.org/2000/01/rdf-schema#domain";
    const DISJOINT: &str = "http://www.w3.org/2002/07/owl#disjointWith";
    const PROP_DISJOINT: &str = "http://www.w3.org/2002/07/owl#propertyDisjointWith";
    const COMPLEMENT: &str = "http://www.w3.org/2002/07/owl#complementOf";

    // Direct violation: an individual asserted into two disjoint classes.
    #[test]
    fn direct_class_disjointness_violation_is_inconsistent() {
        let kb = vec![
            triple("http://ex/A", DISJOINT, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
            triple("http://ex/i", RDF_TYPE, "http://ex/B"),
        ];
        let QlConsistency::Inconsistent(v) = check_consistency(&kb) else {
            panic!("expected Inconsistent, got {:?}", check_consistency(&kb));
        };
        assert_eq!(
            v.axiom,
            NegativeInclusion::Concept(
                Basic::Class("http://ex/A".into()),
                Basic::Class("http://ex/B".into())
            )
        );
    }

    // DERIVED violation — the closure/rewriting must fire: i is only ASSERTED into subclasses.
    #[test]
    fn violation_through_positive_inclusions_is_detected() {
        let kb = vec![
            triple("http://ex/A1", SUBCLASS, "http://ex/A"),
            triple("http://ex/B1", SUBCLASS, "http://ex/B"),
            triple("http://ex/A", DISJOINT, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A1"),
            triple("http://ex/i", RDF_TYPE, "http://ex/B1"),
        ];
        assert!(matches!(
            check_consistency(&kb),
            QlConsistency::Inconsistent(_)
        ));
        // MUTATION WITNESS: without the disjointness the same data is Consistent — the
        // Inconsistent verdict above is carried by the violation-query composition, not by
        // an accident of the fixture.
        let no_neg: Vec<Triple> = kb[..2]
            .iter()
            .cloned()
            .chain(kb[3..].iter().cloned())
            .collect();
        assert_eq!(check_consistency(&no_neg), QlConsistency::Consistent);
    }

    // Domain-derived violation: p's domain is A, A disjoint B, i has a p-edge and is a B.
    #[test]
    fn violation_via_domain_inclusion_is_detected() {
        let kb = vec![
            triple("http://ex/p", DOMAIN, "http://ex/A"),
            triple("http://ex/A", DISJOINT, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/B"),
            triple("http://ex/i", "http://ex/p", "http://ex/j"),
        ];
        assert!(matches!(
            check_consistency(&kb),
            QlConsistency::Inconsistent(_)
        ));
    }

    // Role disjointness, including through a role inclusion.
    #[test]
    fn role_disjointness_violation_is_detected() {
        let kb = vec![
            triple("http://ex/p1", SUBPROP, "http://ex/p"),
            triple("http://ex/p", PROP_DISJOINT, "http://ex/q"),
            triple("http://ex/a", "http://ex/p1", "http://ex/b"),
            triple("http://ex/a", "http://ex/q", "http://ex/b"),
        ];
        let QlConsistency::Inconsistent(v) = check_consistency(&kb) else {
            panic!("expected Inconsistent");
        };
        assert_eq!(
            v.axiom,
            NegativeInclusion::Role(Role::named("http://ex/p"), Role::named("http://ex/q"))
        );
        // The role pair must match as a PAIR: crossing edges (a p1 b, b q a) are no violation.
        let crossed = vec![
            kb[0].clone(),
            kb[1].clone(),
            kb[2].clone(),
            triple("http://ex/b", "http://ex/q", "http://ex/a"),
        ];
        assert_eq!(check_consistency(&crossed), QlConsistency::Consistent);
    }

    // Self-disjointness A ⊑ ¬A: any member is a violation (the reduce step dedups the atoms).
    #[test]
    fn self_disjoint_class_with_member_is_inconsistent() {
        let kb = vec![
            triple("http://ex/A", DISJOINT, "http://ex/A"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
        ];
        assert!(matches!(
            check_consistency(&kb),
            QlConsistency::Inconsistent(_)
        ));
    }

    // A satisfiable KB WITH negative inclusions gets a definitive Consistent.
    #[test]
    fn satisfiable_kb_with_negative_inclusions_is_consistent() {
        let kb = vec![
            triple("http://ex/A", DISJOINT, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
            triple("http://ex/j", RDF_TYPE, "http://ex/B"),
        ];
        assert_eq!(check_consistency(&kb), QlConsistency::Consistent);
    }

    // sq-fj8lj follow-up — the subClassOf-complement negative inclusion `A ⊑ ¬B` decides BOTH
    // sides: violated → Inconsistent, satisfiable → definitive Consistent (the verdict this
    // shape could not reach while it was counted `skipped` + `consistency_uncaptured`).
    #[test]
    fn subclass_complement_decides_both_verdicts() {
        const COMPLEMENT: &str = "http://www.w3.org/2002/07/owl#complementOf";
        let bnode = oxrdf::BlankNode::new("c0").unwrap();
        // :A rdfs:subClassOf [ owl:complementOf :B ] — i.e. A ⊑ ¬B.
        let tbox = vec![
            Triple::new(iri("http://ex/A"), iri(SUBCLASS), bnode.clone()),
            Triple::new(bnode, iri(COMPLEMENT), iri("http://ex/B")),
        ];
        // Violated: `i` is in both A and B.
        let mut violating = tbox.clone();
        violating.extend([
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
            triple("http://ex/i", RDF_TYPE, "http://ex/B"),
        ]);
        let QlConsistency::Inconsistent(v) = check_consistency(&violating) else {
            panic!("expected Inconsistent, got {:?}", check_consistency(&violating));
        };
        assert_eq!(
            v.axiom,
            NegativeInclusion::Concept(
                Basic::Class("http://ex/A".into()),
                Basic::Class("http://ex/B".into())
            )
        );
        // Satisfiable: distinct individuals — a DEFINITIVE Consistent, not Unknown.
        let mut satisfiable = tbox.clone();
        satisfiable.extend([
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
            triple("http://ex/j", RDF_TYPE, "http://ex/B"),
        ]);
        assert_eq!(check_consistency(&satisfiable), QlConsistency::Consistent);
        // MUTATION WITNESS: drop the complement triple and the same data is merely a subClassOf
        // of an opaque blank node — `skipped`, hence fail-closed Unknown. The Consistent above is
        // carried by the capture, not by the fixture happening to be violation-free.
        let no_complement: Vec<Triple> = violating
            .iter()
            .filter(|t| t.predicate.as_str() != COMPLEMENT)
            .cloned()
            .collect();
        assert_eq!(
            check_consistency(&no_complement),
            QlConsistency::Unknown(QlConsistencyGap::NotFullyCaptured {
                skipped: 1,
                unrecognised_schema: 0
            })
        );
    }

    // A NAMED-subject complementOf is NEVER structurally captured: no violation found →
    // fail-closed Unknown.
    #[test]
    fn complement_of_stays_unknown_fail_closed() {
        let kb = vec![
            triple("http://ex/A", COMPLEMENT, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
        ];
        assert_eq!(
            check_consistency(&kb),
            QlConsistency::Unknown(QlConsistencyGap::UncapturedNegativeAxioms { uncaptured: 1 })
        );
    }

    // An uncaptured (non-QL) axiom blocks Consistent but NOT a found Inconsistent.
    #[test]
    fn partial_capture_blocks_consistent_but_not_inconsistent() {
        let functional = triple(
            "http://ex/p",
            RDF_TYPE,
            "http://www.w3.org/2002/07/owl#FunctionalProperty",
        );
        // No violation + unrecognised schema → Unknown(NotFullyCaptured).
        let kb = vec![
            functional.clone(),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
        ];
        assert_eq!(
            check_consistency(&kb),
            QlConsistency::Unknown(QlConsistencyGap::NotFullyCaptured {
                skipped: 0,
                unrecognised_schema: 1
            })
        );
        // A violated captured negative inclusion + the same unrecognised axiom → still
        // Inconsistent (monotonicity: a subset-derived inconsistency stands).
        let kb = vec![
            functional,
            triple("http://ex/A", DISJOINT, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
            triple("http://ex/i", RDF_TYPE, "http://ex/B"),
        ];
        assert!(matches!(
            check_consistency(&kb),
            QlConsistency::Inconsistent(_)
        ));
    }

    // Anonymous-witness violation: A ⊑ ∃R, ∃R⁻ ⊑ B1, ∃R⁻ ⊑ B2, B1 ⊑ ¬B2, A(a). The violating
    // individual exists only in the canonical model — reduce + existential folding must find it.
    #[test]
    fn anonymous_witness_violation_is_detected() {
        const RANGE: &str = "http://www.w3.org/2000/01/rdf-schema#range";
        const ON_PROPERTY: &str = "http://www.w3.org/2002/07/owl#onProperty";
        const SOME_VALUES: &str = "http://www.w3.org/2002/07/owl#someValuesFrom";
        const OWL_THING: &str = "http://www.w3.org/2002/07/owl#Thing";
        let bnode = oxrdf::BlankNode::new("r0").unwrap();
        let kb = vec![
            // A ⊑ ∃R (via the unqualified someValuesFrom restriction on the RHS).
            Triple::new(iri("http://ex/A"), iri(SUBCLASS), bnode.clone()),
            Triple::new(bnode.clone(), iri(ON_PROPERTY), iri("http://ex/R")),
            Triple::new(bnode, iri(SOME_VALUES), iri(OWL_THING)),
            // ∃R⁻ ⊑ B1 and ∃R⁻ ⊑ B2 (range axioms), B1 disjoint B2.
            triple("http://ex/R", RANGE, "http://ex/B1"),
            triple("http://ex/R", RANGE, "http://ex/B2"),
            triple("http://ex/B1", DISJOINT, "http://ex/B2"),
            triple("http://ex/a", RDF_TYPE, "http://ex/A"),
        ];
        assert!(matches!(
            check_consistency(&kb),
            QlConsistency::Inconsistent(_)
        ));
    }

    // Trivial cases: empty KB, and a KB with no negative axioms at all.
    #[test]
    fn no_negative_axioms_is_consistent() {
        assert_eq!(check_consistency(&[]), QlConsistency::Consistent);
        let kb = vec![
            triple("http://ex/A", SUBCLASS, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
        ];
        assert_eq!(check_consistency(&kb), QlConsistency::Consistent);
    }

    // Direct unit tests for the small public surface (coverage-floor discipline).
    #[test]
    fn negative_inclusion_display_is_readable() {
        let ni = NegativeInclusion::Concept(
            Basic::Class("http://ex/A".into()),
            Basic::Exists(Role::named("http://ex/r").inv()),
        );
        assert_eq!(ni.to_string(), "<http://ex/A> ⊑ ¬∃<http://ex/r>⁻");
        let nr = NegativeInclusion::Role(Role::named("http://ex/p"), Role::named("http://ex/q"));
        assert_eq!(nr.to_string(), "<http://ex/p> ⊑ ¬<http://ex/q>");
    }

    #[test]
    fn check_consistency_with_matches_check_consistency() {
        let kb = vec![
            triple("http://ex/A", DISJOINT, "http://ex/B"),
            triple("http://ex/i", RDF_TYPE, "http://ex/A"),
            triple("http://ex/i", RDF_TYPE, "http://ex/B"),
        ];
        let tbox = TBox::extract(&kb);
        assert_eq!(check_consistency_with(&tbox, &kb), check_consistency(&kb));
    }
}
