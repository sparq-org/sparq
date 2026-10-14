//! [Kern] Quoted-triple (RDF 1.2 reifier) inference for the OWL 2 RL profile —
//! FIRST INCREMENT of the kern/quoted-triple-infer track. Opt-in behind the
//! `quoted-triples` feature; when off this module is cfg'd out entirely (the
//! lean-core posture, like `dtype` / `rif` / `datalog`) and `Profile::OwlRl`
//! closures are byte-identical to before it existed.
//!
//! # Representation
//!
//! The loaders (sparq-core `ttl`/`nt`) desugar every RDF 1.2 quotation form —
//! `<< :s :p :o >>`, `:s :p :o ~ :r`, `{| … |}` annotation blocks — to ONE shape: a
//! reifier node `R` carrying `R rdf:reifies <<( s p o )>>`, where the triple term
//! `<<( s p o )>>` is interned STRUCTURALLY (one content-addressed dictionary id
//! holding its component ids, [`TermParts::Triple`]) and is a single opaque id
//! everywhere else. This module reasons over exactly that shape.
//!
//! # Rules (both monotone, datalog-style over ids)
//!
//! | rule | premise ⊢ conclusion |
//! |---|---|
//! | reif-dtr (destructure) | `(r rdf:reifies tt)`, `tt = <<( s p o )>>` ⊢ `(r rdf:type rdf:Statement)`, `(r rdf:subject s)`, `(r rdf:predicate p)`, `(r rdf:object o)` |
//! | reif-ctr (construct)   | `(r rdf:subject s)`, `(r rdf:predicate p)`, `(r rdf:object o)`, **`(s p o)` in the closure**, `o` not a triple term ⊢ `(r rdf:reifies <<( s p o )>>)` |
//!
//! These are the RDF 1.2 ↔ classic-reification BRIDGE rules: reif-dtr is the
//! "unstar" projection into the classic vocabulary (`rdf:subject`/`rdf:predicate`/
//! `rdf:object`, whose RDF-Schema reading a reifier still fits), so the ordinary RL
//! rules (domain/range, subPropertyOf, sameAs, …) can reason over the recovered
//! components; reif-ctr is its converse, surfacing classic-reification data as
//! native RDF 1.2 reifiers. They are NOT normative RDF 1.2 entailments — which is
//! exactly why the layer is opt-in rather than silently changing `Profile::OwlRl`
//! results (and the committed inference-conformance ratchet) for data that uses the
//! classic vocabulary innocently.
//!
//! # Finite Herbrand base / termination
//!
//! Forward chaining with a term CONSTRUCTOR is not obviously terminating: an
//! unrestricted construct rule can quote its own conclusions — e.g. under
//! `rdf:reifies rdfs:subPropertyOf rdf:object`, minting `tt1` derives
//! `(r rdf:object tt1)`, which licenses minting `<<( r rdf:reifies tt1 )>>`, and so
//! on forever (pinned by `construct_never_quotes_a_triple_term` below). reif-ctr
//! therefore carries two restrictions:
//!
//! 1. **Only EXISTING triples are reified**: the quoted triple `(s p o)` must
//!    already be present in the closure (asserted or derived). reif-ctr never
//!    invents a triple.
//! 2. **No quoting of triple terms**: no component of a CONSTRUCTED triple term may
//!    itself be a triple term (`s`/`p` are excluded by the RDF 1.2 kind guards —
//!    subject IRI/blank, predicate IRI, the same kinds `Dict` enforces for stored
//!    triple terms; `o` is checked explicitly). Destructuring of user-written
//!    nested quotes still works — only the constructor is depth-0.
//!
//! Termination: (i) no rule invents reifier nodes or blank nodes (no existentials);
//! (ii) reif-dtr adds at most 4 triples per `(r, tt)` pair, all over ids that
//! already exist (quotation components are interned at load time even when they
//! occur in no asserted triple); (iii) by restriction 2 every term reif-ctr mints
//! has only LEAF (non-triple-term) components, and neither the RL core nor these
//! rules ever create a new leaf term beyond the fixed rule vocabulary — so the
//! mintable set is bounded by the triples over a FIXED finite leaf universe, and by
//! restriction 1 each mint corresponds to a triple already in the closure. The
//! reachable term universe U is therefore finite, every derivable triple lies in
//! the finite set U³, and all rules are monotone — the alternating
//! reify-step / RL-closure loop in [`crate::materialize_owl_rl`] adds at least one
//! triple per round or stops, reaching the least fixpoint in ≤ |U|³ rounds.
//!
//! Restriction 2 is deliberately decided from TERM STRUCTURE, not from "was this id
//! minted earlier in this run?" — together with the closure-membership form of
//! restriction 1 (membership in the CURRENT set, not a pre-derivation base
//! snapshot) this keeps [`crate::materialize`] IDEMPOTENT as documented: [`step`]
//! is a pure function of `(dict, triples, mode)`, so re-running materialization on
//! its own fixpoint derives nothing new. (A base-snapshot guard would NOT be
//! idempotent: a triple derived by, say, rdfs7 on the first call is "base" on the
//! second call and would suddenly become quotable.)
//!
//! # Opacity (coordinated with the sibling opacity-semantics PR; no dependency)
//!
//! * NO rule derives `(s p o)` from `(r rdf:reifies <<( s p o )>>)` — quotation
//!   never asserts. Pinned by the leak tests here and in tests/owl_reify.rs.
//! * A triple term is ONE opaque id to every RL rule AND to the `owl:sameAs`
//!   union-find (entity rewriting canonicalizes triple POSITIONS, never the
//!   component ids stored INSIDE a triple term) — no rule ever REWRITES a
//!   quotation. Note the boundary precisely, though: the DESTRUCTURED classic
//!   vocabulary is transparent by design, so the bridge COMPOSITION
//!   (reif-dtr, then eq-rep on `rdf:subject` values, then reif-ctr) can derive
//!   `r rdf:reifies <<( x p o )>>` from `r rdf:reifies <<( s p o )>>` +
//!   `x sameAs s` — but ONLY when the variant triple `(x p o)` itself holds, and
//!   without touching the original quotation. That is classic-vocabulary
//!   reasoning, not substitution inside a quote; it is pinned by
//!   `same_as_variants_arrive_only_via_the_transparent_bridge` in
//!   tests/owl_reify.rs. For consumers that want even that composition
//!   suppressed, [`ReifyMode::DestructureOnly`] (the STRICT-OPACITY mode, second
//!   increment) runs reif-dtr alone: inference then never mints a triple term at
//!   all, so no quotation — variant or otherwise — can ever be introduced by
//!   reasoning. Drive it via [`crate::materialize_owl_rl_reify`] (batch) or
//!   [`crate::MaterializedOwlGraph::with_reify_mode`] (incremental);
//!   [`crate::materialize_owl_rl`] / [`crate::MaterializedOwlGraph::new`] keep the
//!   full bridge.
//! * Only the OUTER quotation is destructured — a component that is itself a
//!   triple term stays one opaque id.
//! * Reasoning over reifier ANNOTATIONS (`{| … |}` blocks) needs no new machinery:
//!   annotations are ordinary triples about the reifier node, and the full RL rule
//!   set applies to them without ever touching the quoted triple. What reif-dtr
//!   adds is a deliberate, monotone projection: the DESTRUCTURED components are
//!   transparent ordinary terms, while the triple term itself remains opaque.
//!
//! # Increment scope
//!
//! One naive O(|closure|) pass per outer round, single-threaded, alternated with
//! the batch RL core (correct, not yet fused into the semi-naive delta loop);
//! `MaterializedOwlGraph` routes any base that mentions the reification vocabulary
//! to its documented Fallback mode (re-materialize per mutation) rather than
//! teaching the counting modes these rules. See the first-increment PR body (#2135)
//! for the follow-up list. The second increment (sq-afun3) added the
//! strict-opacity [`ReifyMode`] surface (batch only). The third increment
//! (sq-afun3) plumbed that mode through the incremental handle:
//! [`crate::MaterializedOwlGraph::with_reify_mode`] fixes the mode at construction
//! and every Fallback re-materialization — the initial one and every mutation's —
//! runs it, so the type's closure-equals-from-scratch invariant now holds against
//! the MATCHING batch oracle ([`crate::materialize_owl_rl_reify`] in the same mode)
//! and strict opacity is reachable incrementally. Still open (unchanged): the
//! counting modes model neither bridge rule, so reify-vocabulary bases stay in
//! Fallback in BOTH modes, and the reify step is still the naive per-round scan
//! rather than a semi-naive delta.

use crate::owl::RDF;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{is_inline, Dict, Id, TermParts, NO_ID};

/// Which of the bridge rules the reify layer runs — see the opacity section of the
/// module docs. sq-afun3 (second increment of kern/quoted-triple-infer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReifyMode {
    /// Both bridge rules (reif-dtr + reif-ctr) to their joint fixpoint — what
    /// [`crate::materialize_owl_rl`] runs when the `quoted-triples` feature is on.
    #[default]
    Bridge,
    /// STRICT OPACITY: destructure only. reif-ctr never runs, so inference never
    /// mints a triple term — classic-reification data is never surfaced as
    /// `rdf:reifies`, and the documented bridge composition (reif-dtr → eq-rep on
    /// the classic vocabulary → reif-ctr) that can quote a `owl:sameAs`-variant
    /// spelling is suppressed with it. Everything reif-dtr recovers (the classic
    /// vocabulary over as-written quotations, `rdf:Statement` typing, annotation
    /// reasoning) is unchanged. A strict subset of [`ReifyMode::Bridge`], so the
    /// finite-Herbrand-base / termination / idempotency arguments hold a fortiori
    /// (no constructor, no minting).
    DestructureOnly,
}

/// The interned rule vocabulary. `intern` is idempotent (fixed labels only), the
/// same discipline as [`crate::Vocab`] / `Owl::intern`.
pub(crate) struct ReifyVocab {
    pub reifies: Id,   // rdf:reifies (RDF 1.2)
    pub ty: Id,        // rdf:type
    pub statement: Id, // rdf:Statement
    pub subject: Id,   // rdf:subject
    pub predicate: Id, // rdf:predicate
    pub object: Id,    // rdf:object
}

impl ReifyVocab {
    pub fn intern(dict: &mut Dict) -> ReifyVocab {
        let mut i = |frag: &str| dict.intern_iri(&format!("{RDF}{frag}"));
        ReifyVocab {
            reifies: i("reifies"),
            ty: i("type"),
            statement: i("Statement"),
            subject: i("subject"),
            predicate: i("predicate"),
            object: i("object"),
        }
    }
}

/// Occurrence gate: can either rule possibly fire? Checked WITHOUT interning (four
/// read-only [`Dict::lookup`]s), so the closure of reify-free data stays
/// byte-identical and pays no scan. A trigger id — `rdf:reifies` or the classic
/// `rdf:subject`/`rdf:predicate`/`rdf:object` — counts in ANY position:
/// `ex:s rdfs:subPropertyOf rdf:subject` mentions `rdf:subject` only as an OBJECT,
/// yet rdfs7 derives `rdf:subject` assertions reif-ctr must see. The RL core mints
/// no new IRIs beyond its fixed vocabulary, so a closure where none of the four ids
/// occur can never grow a premise for either rule — this check is complete.
pub(crate) fn occurs(dict: &Dict, triples: &[[Id; 3]]) -> bool {
    let look = |frag: &str| {
        dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
            format!("{RDF}{frag}"),
        )))
    };
    let trigger: Vec<Id> = ["reifies", "subject", "predicate", "object"]
        .iter()
        .map(|f| look(f))
        .filter(|&id| id != NO_ID)
        .collect();
    !trigger.is_empty()
        && triples
            .iter()
            .any(|t| t.iter().any(|id| trigger.contains(id)))
}

/// One naive reify round over the whole current closure — reif-dtr + reif-ctr in
/// [`ReifyMode::Bridge`], reif-dtr alone in [`ReifyMode::DestructureOnly`]. Appends
/// the new triples (set semantics, deduplicated against `triples` and within the
/// round) and returns how many were added. Driven to the joint fixpoint by
/// [`crate::materialize_owl_rl`] / [`crate::materialize_owl_rl_reify`], which
/// alternate one round with one RL closure — a per-round full scan is fine for a
/// first increment because rounds are rare (each must be separated by an RL pass
/// that derived new reify-relevant facts); the semi-naive delta form is a named
/// follow-up.
pub(crate) fn step(dict: &mut Dict, triples: &mut Vec<[Id; 3]>, mode: ReifyMode) -> usize {
    let v = ReifyVocab::intern(dict);
    let mut set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let mut add: Vec<[Id; 3]> = Vec::new();

    // Component values per reifier, from classic-vocabulary assertions (asserted or
    // derived). A reifier with several values per slot yields the cross product in
    // reif-ctr — the standard reading of classic reification, filtered hard by the
    // existing-triple restriction. Collected only in Bridge mode: DestructureOnly
    // never runs reif-ctr, so the maps stay empty and the construct pass below is
    // a no-op.
    let mut subj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut pred: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut obj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();

    for &[r, p, o] in triples.iter() {
        if p == v.reifies {
            // reif-dtr. `term_parts` is only valid for real dictionary ids, so skip
            // inline-literal ids; a reifies object that is NOT a triple term
            // (freehand data like `r rdf:reifies "x"`) simply matches no rule
            // (premise failure, not a fallback).
            if !is_inline(o) {
                if let TermParts::Triple([qs, qp, qo]) = dict.term_parts(o) {
                    add.extend([
                        [r, v.ty, v.statement],
                        [r, v.subject, qs],
                        [r, v.predicate, qp],
                        [r, v.object, qo],
                    ]);
                }
            }
        } else if mode == ReifyMode::Bridge {
            if p == v.subject {
                subj.entry(r).or_default().push(o);
            } else if p == v.predicate {
                pred.entry(r).or_default().push(o);
            } else if p == v.object {
                obj.entry(r).or_default().push(o);
            }
        }
    }

    // reif-ctr (Bridge mode only — the maps are empty in DestructureOnly) — under
    // the finiteness restrictions (module doc) and the RDF 1.2
    // well-formedness kind guards. An ill-kinded candidate satisfies no premise and
    // is skipped — never interned, so a construct can never produce a triple term
    // the dict's open-time component-kind validation would reject.
    for (&r, ss) in &subj {
        let (Some(ps), Some(os)) = (pred.get(&r), obj.get(&r)) else {
            continue;
        };
        for &s in ss {
            if is_inline(s)
                || !matches!(
                    dict.term_parts(s),
                    TermParts::Iri { .. } | TermParts::Blank(_)
                )
            {
                continue;
            }
            for &p in ps {
                if is_inline(p) || !matches!(dict.term_parts(p), TermParts::Iri { .. }) {
                    continue;
                }
                for &o in os {
                    // Restriction 2: never quote a triple term (finiteness, module doc).
                    if !is_inline(o) && matches!(dict.term_parts(o), TermParts::Triple(_)) {
                        continue;
                    }
                    // Restriction 1: only reify a triple that already EXISTS.
                    if !set.contains(&[s, p, o]) {
                        continue;
                    }
                    let tt = mint(dict, s, p, o);
                    add.push([r, v.reifies, tt]);
                }
            }
        }
    }

    let mut added = 0;
    for t in add {
        if set.insert(t) {
            triples.push(t);
            added += 1;
        }
    }
    added
}

/// Intern the triple term `<<( s p o )>>` from already-interned component ids,
/// through the same public structural [`Dict::intern`] path the loaders use
/// (content-addressed by component ids, so it dedupes against loader-interned
/// terms). The reasoner deliberately stays on the public dict surface — the
/// id-level `intern_triple_ids` is dict-private, and the reconstruction round-trip
/// only runs for the rare guarded construct firings.
fn mint(dict: &mut Dict, s: Id, p: Id, o: Id) -> Id {
    let subject: oxrdf::NamedOrBlankNode = match dict.term(s) {
        oxrdf::Term::NamedNode(n) => n.into(),
        oxrdf::Term::BlankNode(b) => b.into(),
        // Kind-guarded at the call site (reif-ctr skips non-IRI/blank subjects).
        _ => unreachable!("reif-ctr subject kind guard"),
    };
    let oxrdf::Term::NamedNode(predicate) = dict.term(p) else {
        // Kind-guarded at the call site (reif-ctr skips non-IRI predicates).
        unreachable!("reif-ctr predicate kind guard")
    };
    dict.intern(&oxrdf::Term::Triple(Box::new(oxrdf::Triple::new(
        subject,
        predicate,
        dict.term(o),
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{NamedNode, Term};

    fn iri(d: &mut Dict, s: &str) -> Id {
        d.intern_iri(&format!("http://ex/{s}"))
    }

    /// Intern `<<( s p o )>>` over `http://ex/` IRIs through the public path.
    fn tt(d: &mut Dict, s: &str, p: &str, o: &str) -> Id {
        d.intern(&Term::Triple(Box::new(oxrdf::Triple::new(
            NamedNode::new_unchecked(format!("http://ex/{s}")),
            NamedNode::new_unchecked(format!("http://ex/{p}")),
            Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{o}"))),
        ))))
    }

    #[test]
    fn occurs_is_false_without_trigger_vocab() {
        let mut d = Dict::default();
        let (a, p, b) = (iri(&mut d, "a"), iri(&mut d, "p"), iri(&mut d, "b"));
        let triples = vec![[a, p, b]];
        assert!(
            !occurs(&d, &triples),
            "no reification vocabulary → layer stays inactive"
        );
    }

    #[test]
    fn occurs_triggers_on_any_position() {
        let mut d = Dict::default();
        let sp = d.intern_iri("http://www.w3.org/2000/01/rdf-schema#subPropertyOf");
        let rdf_subject = d.intern_iri(&format!("{RDF}subject"));
        let mine = iri(&mut d, "mySubj");
        // rdf:subject occurs only as an OBJECT — still must arm the layer.
        let triples = vec![[mine, sp, rdf_subject]];
        assert!(occurs(&d, &triples));
    }

    #[test]
    fn destructure_recovers_components_and_types_the_reifier() {
        let mut d = Dict::default();
        let (s, p, o, r) = (
            iri(&mut d, "s"),
            iri(&mut d, "p"),
            iri(&mut d, "o"),
            iri(&mut d, "r"),
        );
        let v = ReifyVocab::intern(&mut d);
        let q = tt(&mut d, "s", "p", "o");
        let mut triples = vec![[s, p, o], [r, v.reifies, q]];
        let added = step(&mut d, &mut triples, ReifyMode::Bridge);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[r, v.ty, v.statement]));
        assert!(set.contains(&[r, v.subject, s]));
        assert!(set.contains(&[r, v.predicate, p]));
        assert!(set.contains(&[r, v.object, o]));
        // reif-ctr also re-derives (r reifies q) from the fresh components — already
        // present, so exactly the 4 destructure conclusions are new.
        assert_eq!(added, 4);
        // Idempotent: a second round adds nothing.
        assert_eq!(step(&mut d, &mut triples, ReifyMode::Bridge), 0);
    }

    #[test]
    fn construct_mints_only_existing_triples() {
        let mut d = Dict::default();
        let (s, p, o, r, r2, x) = (
            iri(&mut d, "s"),
            iri(&mut d, "p"),
            iri(&mut d, "o"),
            iri(&mut d, "r"),
            iri(&mut d, "r2"),
            iri(&mut d, "x"),
        );
        let v = ReifyVocab::intern(&mut d);
        let mut triples = vec![
            [s, p, o], // the asserted triple r quotes
            [r, v.subject, s],
            [r, v.predicate, p],
            [r, v.object, o],
            // r2 describes (x p o) — NOT an existing triple: reif-ctr must not fire.
            [r2, v.subject, x],
            [r2, v.predicate, p],
            [r2, v.object, o],
        ];
        let added = step(&mut d, &mut triples, ReifyMode::Bridge);
        assert_eq!(added, 1, "exactly the guarded construction fires");
        let q = tt(&mut d, "s", "p", "o");
        assert!(triples.contains(&[r, v.reifies, q]));
        // The unguarded term was never interned — the finite-Herbrand-base pin.
        let bogus = d.lookup(&Term::Triple(Box::new(oxrdf::Triple::new(
            NamedNode::new_unchecked("http://ex/x"),
            NamedNode::new_unchecked("http://ex/p"),
            Term::NamedNode(NamedNode::new_unchecked("http://ex/o")),
        ))));
        assert_eq!(
            bogus, NO_ID,
            "no triple term is minted for a non-existing combination"
        );
    }

    #[test]
    fn no_unquoting_leak_and_nested_terms_stay_opaque() {
        let mut d = Dict::default();
        let (a, q, b, s, p, r) = (
            iri(&mut d, "a"),
            iri(&mut d, "q"),
            iri(&mut d, "b"),
            iri(&mut d, "s"),
            iri(&mut d, "p"),
            iri(&mut d, "r"),
        );
        let v = ReifyVocab::intern(&mut d);
        let inner = tt(&mut d, "a", "q", "b");
        let outer = d.intern(&Term::Triple(Box::new(oxrdf::Triple::new(
            NamedNode::new_unchecked("http://ex/s"),
            NamedNode::new_unchecked("http://ex/p"),
            d.term(inner),
        )))); // <<( s p <<( a q b )>> )>>
        let mut triples = vec![[r, v.reifies, outer]];
        step(&mut d, &mut triples, ReifyMode::Bridge);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        // The quoted triple is NOT asserted (opacity)…
        assert!(
            !set.contains(&[s, p, inner]),
            "quotation must not assert the quoted triple"
        );
        assert!(
            !set.contains(&[a, q, b]),
            "nested quotation must not assert either"
        );
        // …but the OUTER components are recovered, the inner term staying one opaque id.
        assert!(set.contains(&[r, v.object, inner]));
        assert!(
            !set.iter().any(|t| t[0] == inner),
            "nothing destructures the nested term (no reifies premise)"
        );
    }

    #[test]
    fn construct_never_quotes_a_triple_term() {
        // The divergence cascade from the module docs, one level deep: r classically
        // describes the triple (r reifies q) — which EXISTS — but its object is a
        // triple term, so restriction 2 refuses to mint <<( r reifies q )>>.
        let mut d = Dict::default();
        let (s, p, o, r) = (
            iri(&mut d, "s"),
            iri(&mut d, "p"),
            iri(&mut d, "o"),
            iri(&mut d, "r"),
        );
        let v = ReifyVocab::intern(&mut d);
        let q = tt(&mut d, "s", "p", "o");
        let mut triples = vec![
            [s, p, o],
            [r, v.reifies, q],
            [r, v.subject, r],
            [r, v.predicate, v.reifies],
            [r, v.object, q],
        ];
        // Drive this rule set alone to fixpoint — it must terminate.
        while step(&mut d, &mut triples, ReifyMode::Bridge) > 0 {}
        let nested = d.lookup(&Term::Triple(Box::new(oxrdf::Triple::new(
            NamedNode::new_unchecked("http://ex/r"),
            NamedNode::new_unchecked(format!("{RDF}reifies")),
            d.term(q),
        ))));
        assert_eq!(
            nested, NO_ID,
            "a construct-reachable quotation-of-a-quotation must never be minted"
        );
    }

    #[test]
    fn construct_then_destructure_close_in_two_rounds() {
        let mut d = Dict::default();
        let (s, p, o, r) = (
            iri(&mut d, "s"),
            iri(&mut d, "p"),
            iri(&mut d, "o"),
            iri(&mut d, "r"),
        );
        let v = ReifyVocab::intern(&mut d);
        let mut triples = vec![
            [s, p, o],
            [r, v.subject, s],
            [r, v.predicate, p],
            [r, v.object, o],
        ];
        // Round 1: reif-ctr mints the reifies triple.
        assert_eq!(step(&mut d, &mut triples, ReifyMode::Bridge), 1);
        // Round 2: reif-dtr types the reifier from the new premise; the subject/
        // predicate/object conclusions already exist. Round 3: fixpoint.
        assert_eq!(step(&mut d, &mut triples, ReifyMode::Bridge), 1);
        let q = tt(&mut d, "s", "p", "o");
        assert!(triples.contains(&[r, v.reifies, q]));
        assert!(triples.contains(&[r, v.ty, v.statement]));
        assert_eq!(
            step(&mut d, &mut triples, ReifyMode::Bridge),
            0,
            "terminates — the guarded universe is exhausted"
        );
    }

    #[test]
    fn construct_skips_ill_kinded_candidates() {
        // A literal "subject" / literal "predicate" value satisfies no premise: the
        // kind guards must skip it (never intern a triple term the dict's open-time
        // component-kind validation would reject) — and must not panic.
        let mut d = Dict::default();
        let (s, p, o, r) = (
            iri(&mut d, "s"),
            iri(&mut d, "p"),
            iri(&mut d, "o"),
            iri(&mut d, "r"),
        );
        let lit = d.intern_lit("x", "http://www.w3.org/2001/XMLSchema#string", None);
        let v = ReifyVocab::intern(&mut d);
        let mut triples = vec![
            [s, p, o],
            [r, v.subject, lit],   // ill-kinded subject value
            [r, v.predicate, lit], // ill-kinded predicate value
            [r, v.object, o],
            [r, v.subject, s],
            [r, v.predicate, p],
        ];
        assert_eq!(
            step(&mut d, &mut triples, ReifyMode::Bridge),
            1,
            "only the well-kinded (s p o) combination fires"
        );
        let q = tt(&mut d, "s", "p", "o");
        assert!(triples.contains(&[r, v.reifies, q]));
    }

    /// sq-afun3: DestructureOnly recovers exactly the reif-dtr conclusions
    /// and NEVER constructs — the fixture that fires reif-ctr once in Bridge mode
    /// mints nothing in strict mode, and no triple term is ever interned.
    #[test]
    fn destructure_only_never_constructs() {
        let mut d = Dict::default();
        let (s, p, o, r, r2) = (
            iri(&mut d, "s"),
            iri(&mut d, "p"),
            iri(&mut d, "o"),
            iri(&mut d, "r"),
            iri(&mut d, "r2"),
        );
        let v = ReifyVocab::intern(&mut d);
        let q = tt(&mut d, "s", "p", "o");
        let mut triples = vec![
            [s, p, o],
            [r, v.reifies, q], // reif-dtr premise: the strict mode still fires this
            // A complete classic description of the EXISTING (s p o) — the exact
            // shape reif-ctr fires on in Bridge mode (construct_mints_only_…).
            [r2, v.subject, s],
            [r2, v.predicate, p],
            [r2, v.object, o],
        ];
        let mut strict = triples.clone();
        while step(&mut d, &mut strict, ReifyMode::DestructureOnly) > 0 {}
        let set: FxHashSet<[Id; 3]> = strict.iter().copied().collect();
        // reif-dtr conclusions are unchanged…
        assert!(set.contains(&[r, v.ty, v.statement]));
        assert!(set.contains(&[r, v.subject, s]));
        assert!(set.contains(&[r, v.predicate, p]));
        assert!(set.contains(&[r, v.object, o]));
        // …but reif-ctr never runs: r2 gains NO reifies triple in strict mode,
        // while Bridge mode derives it from the same input.
        assert!(
            !set.contains(&[r2, v.reifies, q]),
            "strict mode must not surface classic reification as rdf:reifies"
        );
        while step(&mut d, &mut triples, ReifyMode::Bridge) > 0 {}
        assert!(
            triples.contains(&[r2, v.reifies, q]),
            "the same fixture DOES construct in Bridge mode (non-vacuity)"
        );
    }

    /// sq-afun3: strict mode derives no quotation AT ALL — not even the
    /// leaf-component depth-1 mint Bridge mode allows on the adversarial-cascade
    /// shape (`r` classically describing its own asserted `(r reifies o)` triple) —
    /// and the fixpoint still terminates and is idempotent.
    #[test]
    fn destructure_only_mints_nothing_on_the_adversarial_cascade() {
        let mut d = Dict::default();
        let (o, r) = (iri(&mut d, "o"), iri(&mut d, "r"));
        let v = ReifyVocab::intern(&mut d);
        let mut triples = vec![
            // Freehand `r reifies :o` (not a triple term): dtr's premise fails, but
            // the triple EXISTS, so Bridge's reif-ctr may quote it (all leaf).
            [r, v.reifies, o],
            [r, v.subject, r],
            [r, v.predicate, v.reifies],
            [r, v.object, o],
        ];
        let mut strict = triples.clone();
        while step(&mut d, &mut strict, ReifyMode::DestructureOnly) > 0 {}
        assert_eq!(strict.len(), 4, "strict mode derives nothing here");
        // No quotation is minted by inference: <<( r reifies o )>> must not exist.
        let quote = |d: &Dict| {
            d.lookup(&Term::Triple(Box::new(oxrdf::Triple::new(
                NamedNode::new_unchecked("http://ex/r"),
                NamedNode::new_unchecked(format!("{RDF}reifies")),
                Term::NamedNode(NamedNode::new_unchecked("http://ex/o")),
            ))))
        };
        assert_eq!(quote(&d), NO_ID, "strict mode must never mint a triple term");
        // Idempotent: the fixpoint is a pure function of (dict, triples, mode).
        assert_eq!(step(&mut d, &mut strict, ReifyMode::DestructureOnly), 0);
        // Non-vacuity: Bridge mode DOES mint the depth-1 quotation on this fixture.
        while step(&mut d, &mut triples, ReifyMode::Bridge) > 0 {}
        assert_ne!(quote(&d), NO_ID, "Bridge mints the depth-1 quotation");
    }
}
