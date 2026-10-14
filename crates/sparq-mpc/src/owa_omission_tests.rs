// sq-2fms: federation-level OWA / omission negative suite. Written
// while Fable unavailable — re-review on return.
//! Federation-level Open-World-Assumption (OWA) / omission negative suite
//! (sq-2fms).
//!
//! Architecture §2 convention #8 (OWA-safe semantics) and §4.2 require that an
//! *omission* — a holder dropping a graph, returning a truncated partial, or
//! omitting a row/contributor — must **not forge a valid result**, and that a
//! short-count / dropped-graph view should be *detectable*. The intended
//! cryptographic defence is the issuer-signed true-triple-count + salt of §4.3
//! step 1 (a holder commits to HOW MANY true facts it holds, so a short count is
//! caught against the signed bound). That binding is **gated on M4** (the
//! collaborative proof + attestation, see [`crate::proof`]) — it does NOT exist
//! yet, and this suite does not pretend it does.
//!
//! So this module pins the ACTUAL pre-M4 behaviour along two honest axes, exactly
//! as the bead asks:
//!
//! 1. **Omission cannot FORGE.** Whatever a malicious/faulty holder omits, the
//!    federated disclosed result it can produce is always a SUBSET of (or equal
//!    to) the honest result over the data that was actually contributed — it can
//!    never manufacture a binding, a row, or a join that no holder's data
//!    supports. This is the part that IS guaranteed today, and it is the security-
//!    relevant half: omission degrades *completeness*, never *soundness*. The
//!    disclosed-key join is a faithful inner join over the partials it is GIVEN
//!    (`join.rs` doc: "join == union-store evaluation"), so a dropped input simply
//!    shrinks `D = ⋃ contributed graphs`; the result still satisfies
//!    `Disclosed(π) ⊆ Eval_PAG(Q, D_contributed)`.
//!
//! 2. **Omission is NOT YET cryptographically DETECTABLE (the honest gap).** With
//!    no signed-count binding (pre-M4) the federation has no way to tell a
//!    genuinely-empty holder from a holder that DROPPED its rows: both present an
//!    empty/short partial, and both are accepted. The ONLY thing that surfaces a
//!    short-count today is the out-of-band **differential-vs-union-store** anchor
//!    (`pipeline.rs` / `join.rs` differential tests) — i.e. a verifier who already
//!    knows the true union. We pin that the discrepancy is *visible in the
//!    differential* (so the test is a real regression guard for when the signed-
//!    count defence lands) while it is *invisible to the protocol itself*. We do
//!    NOT claim detection the code cannot deliver — that would be fake crypto.
//!
//! Each test therefore asserts the real behaviour and labels which half it is.
//! When the §4.3-step-1 signed-count binding lands (M4), the "not yet detectable"
//! pins here become the failing red tests that drive its acceptance.

use crate::join::{DisclosedKeyJoin, GlobalJoin, JoinPlan};
use crate::partial::{HolderId, PartialResult};
use oxrdf::{Term, Variable};
use sparq_core::Graph;
use sparq_engine::query as engine_query;

const PFX: &str = "@prefix ex: <http://ex/> .\n";

fn var(n: &str) -> Variable {
    Variable::new_unchecked(n)
}

/// Canonical, order-independent multiset render of a (vars, rows) result: each
/// row becomes a SORTED list of `(?var, term-debug)` pairs, and the rows are
/// sorted. Two results are equal as SPARQL solution multisets iff this is equal.
/// (Same shape the `join.rs` differential tests use, lifted here so the omission
/// tests can compare a federated join against a union-store evaluation regardless
/// of column/row order.)
fn canonical_multiset(vars: &[Variable], rows: &[Vec<Option<Term>>]) -> Vec<Vec<(String, String)>> {
    let mut out: Vec<Vec<(String, String)>> = rows
        .iter()
        .map(|row| {
            let mut pairs: Vec<(String, String)> = vars
                .iter()
                .zip(row.iter())
                .map(|(v, t)| {
                    let val = match t {
                        Some(t) => format!("{t:?}"),
                        None => "<UNBOUND>".to_string(),
                    };
                    (v.as_str().to_string(), val)
                })
                .collect();
            pairs.sort();
            pairs
        })
        .collect();
    out.sort();
    out
}

/// `true` iff every row of `sub` (as a canonical solution) also appears in
/// `sup` — i.e. `sub`'s solution multiset is contained in `sup`'s SET of rows.
/// Used to assert the *no-forgery* property: an omitted-input result must be a
/// subset of the honest full result (it can drop rows, never invent them).
fn rows_subset_of(
    sub_vars: &[Variable],
    sub_rows: &[Vec<Option<Term>>],
    sup_vars: &[Variable],
    sup_rows: &[Vec<Option<Term>>],
) -> bool {
    let sup: std::collections::BTreeSet<Vec<(String, String)>> =
        canonical_multiset(sup_vars, sup_rows).into_iter().collect();
    canonical_multiset(sub_vars, sub_rows)
        .into_iter()
        .all(|r| sup.contains(&r))
}

/// Build a single union store from several turtle bodies (the
/// "evaluate the whole query over `D = ⋃ contributed graphs`" differential side).
fn union_eval(docs: &[&str], q: &str) -> PartialResult {
    let mut combined = String::from(PFX);
    for d in docs {
        combined.push_str(d);
        combined.push('\n');
    }
    let g = Graph::load_str(&combined, "turtle").expect("union graph parses");
    let r = engine_query(&g, q).expect("union query ok");
    PartialResult {
        holder: HolderId::new("federation"),
        vars: r.vars,
        rows: r.rows,
    }
}

/// One holder's disclosed partial: evaluate `fragment` over `doc` locally.
fn partial_of(id: &str, doc: &str, fragment: &str) -> PartialResult {
    let h = crate::holder::Holder::from_rdf(id, &format!("{PFX}{doc}"), "turtle")
        .expect("holder wallet parses");
    h.evaluate_local(fragment).expect("local eval ok")
}

// Fixture: a 3-holder chain `?p knows ?x . ?x supervisedBy ?y . ?y name ?n`,
// each holder owning one pattern over the shared global IRIs. Honest evaluation
// yields exactly the chain rows; the omission tests drop a holder / row from THIS.
const A_DOC: &str = "ex:p1 ex:knows ex:x1 . ex:p2 ex:knows ex:x2 .";
const B_DOC: &str = "ex:x1 ex:supervisedBy ex:y1 . ex:x2 ex:supervisedBy ex:y1 .";
const C_DOC: &str = "ex:y1 ex:name \"Boss\" .";
const A_FRAG: &str = "PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }";
const B_FRAG: &str = "PREFIX ex: <http://ex/> SELECT ?x ?y WHERE { ?x ex:supervisedBy ?y }";
const C_FRAG: &str = "PREFIX ex: <http://ex/> SELECT ?y ?n WHERE { ?y ex:name ?n }";
const CHAIN_QUERY: &str = "PREFIX ex: <http://ex/> \
    SELECT ?p ?x ?y ?n WHERE { ?p ex:knows ?x . ?x ex:supervisedBy ?y . ?y ex:name ?n }";

fn chain_plan_on(v: &str) -> JoinPlan {
    JoinPlan {
        join_var: var(v),
        key_disclosed: true,
    }
}

/// Fold the 3-holder chain join exactly as the federation driver would: A⋈B on
/// `?x`, then ⋈C on `?y`. Returns the federated disclosed result.
fn fold_chain(pa: &PartialResult, pb: &PartialResult, pc: &PartialResult) -> PartialResult {
    let ab = DisclosedKeyJoin
        .join(&[pa.clone(), pb.clone()], &chain_plan_on("x"))
        .expect("A⋈B ok");
    DisclosedKeyJoin
        .join(&[ab, pc.clone()], &chain_plan_on("y"))
        .expect("(A⋈B)⋈C ok")
}

// =====================================================================
// NEGATIVE TEST 1 — a DROPPED HOLDER / dropped graph.
// =====================================================================

/// A holder dropping its WHOLE graph (contributing an empty partial) collapses
/// the inner join to empty — it CANNOT forge a result. Two honest halves:
///
/// (no-forgery)   the dropped-holder result is a SUBSET of the honest full result
///                (here: empty ⊆ full) — omission never manufactures a row.
/// (not-yet-detectable) a holder that DROPPED its graph is byte-indistinguishable
///                from a holder that genuinely HAS no matching facts: the join
///                accepts both and yields the SAME empty result. Only the
///                differential against the true union (which a protocol party does
///                NOT possess pre-M4) reveals the short-count. We pin both.
#[test]
fn dropped_holder_cannot_forge_and_is_not_yet_detectable() {
    let pa = partial_of("a", A_DOC, A_FRAG);
    let pb = partial_of("b", B_DOC, B_FRAG);
    let pc = partial_of("c", C_DOC, C_FRAG);

    // Honest full federation = the differential anchor (chain over the union).
    let honest = fold_chain(&pa, &pb, &pc);
    let expected = union_eval(&[A_DOC, B_DOC, C_DOC], CHAIN_QUERY);
    assert_eq!(
        canonical_multiset(&honest.vars, &honest.rows),
        canonical_multiset(&expected.vars, &expected.rows),
        "sanity: honest 3-holder chain equals union-store eval"
    );
    assert_eq!(honest.rows.len(), 2, "p1→x1→y1→Boss and p2→x2→y1→Boss");

    // Holder B DROPS its whole graph (e.g. refuses to disclose / returns nothing).
    // Its partial keeps the schema (?x ?y) but carries no rows — exactly what an
    // honest empty holder ALSO presents (OWA: absent is absent).
    let pb_dropped = PartialResult {
        holder: HolderId::new("b"),
        vars: pb.vars.clone(),
        rows: vec![],
    };
    let dropped = fold_chain(&pa, &pb_dropped, &pc);

    // (no-forgery) The dropped result is EMPTY — a strict subset of honest. The
    // malicious holder produced FEWER answers, never a forged one.
    assert!(
        dropped.is_empty(),
        "dropping a holder's graph empties the inner join"
    );
    assert!(
        rows_subset_of(&dropped.vars, &dropped.rows, &honest.vars, &honest.rows),
        "no-forgery: the dropped-holder result must be a subset of the honest result"
    );

    // (not-yet-detectable) An honest holder that simply HAS no supervisedBy facts
    // presents a byte-identical empty partial → the SAME empty join. The protocol
    // cannot tell "dropped" from "genuinely empty": no signed-count binding (M4).
    let pb_genuinely_empty = partial_of("b", "ex:x1 ex:unrelated ex:z .", B_FRAG);
    assert!(pb_genuinely_empty.is_empty());
    assert_eq!(
        pb_dropped.rows, pb_genuinely_empty.rows,
        "a dropped graph is indistinguishable from a genuinely-empty one (OWA, pre-M4)"
    );
    let genuine = fold_chain(&pa, &pb_genuinely_empty, &pc);
    assert_eq!(
        canonical_multiset(&dropped.vars, &dropped.rows),
        canonical_multiset(&genuine.vars, &genuine.rows),
        "the join yields the same result either way — omission is NOT detectable by the \
         protocol pre-M4 (only the differential vs the TRUE union, held out-of-band, \
         distinguishes them — the signed-count defence is gated on §4.3 step 1 / M4)"
    );
}

// =====================================================================
// NEGATIVE TEST 2 — a TRUNCATED partial (holder returns FEWER rows).
// =====================================================================

/// A holder returning a TRUNCATED partial (some of its true rows dropped) can
/// only SHRINK the join — never forge. Pinned with both honest halves:
///
/// (no-forgery)   the truncated-partial result ⊆ honest full result, AND it equals
///                the union-store evaluation over the data ACTUALLY contributed
///                (so the result stays sound w.r.t. what was disclosed).
/// (not-yet-detectable) the truncated partial is structurally identical to a holder
///                that simply owns fewer facts; the protocol accepts it. The
///                short-count is only visible in the differential vs the true union.
#[test]
fn truncated_partial_cannot_forge_and_equals_contributed_union() {
    let pa = partial_of("a", A_DOC, A_FRAG);
    let pb = partial_of("b", B_DOC, B_FRAG);
    let pc = partial_of("c", C_DOC, C_FRAG);

    let honest = fold_chain(&pa, &pb, &pc);
    assert_eq!(honest.rows.len(), 2, "sanity: both chains present honestly");

    // Holder B TRUNCATES: it really has supervisedBy for BOTH x1 and x2 but
    // discloses ONLY the x1 row (drops the x2 supervisedBy fact). This is the
    // canonical short-count: fewer rows than its signed true count WOULD bind.
    assert_eq!(pb.rows.len(), 2, "B honestly has two supervisedBy rows");
    let x1_term = Term::from(oxrdf::NamedNode::new_unchecked("http://ex/x1"));
    let x_col = pb
        .vars
        .iter()
        .position(|v| v == &var("x"))
        .expect("B projects ?x");
    let pb_truncated = PartialResult {
        holder: pb.holder.clone(),
        vars: pb.vars.clone(),
        rows: pb
            .rows
            .iter()
            .filter(|r| r[x_col].as_ref() == Some(&x1_term))
            .cloned()
            .collect(),
    };
    assert_eq!(pb_truncated.rows.len(), 1, "B truncated to only the x1 row");

    let truncated = fold_chain(&pa, &pb_truncated, &pc);

    // (no-forgery) the truncated chain yields ONLY p1→x1→y1→Boss — a subset of the
    // honest two-row result; the p2 chain is silently missing, never replaced by a
    // fabricated row.
    assert_eq!(
        truncated.rows.len(),
        1,
        "only the x1 chain survives truncation"
    );
    assert!(
        rows_subset_of(&truncated.vars, &truncated.rows, &honest.vars, &honest.rows),
        "no-forgery: truncated result must be a subset of the honest full result"
    );

    // (soundness w.r.t. contributed data) the truncated result EQUALS evaluating
    // the whole query over the union of what B ACTUALLY contributed (only the x1
    // supervisedBy fact) — so the disclosed answer is still a faithful PAG eval of
    // the contributed graphs, never a forgery.
    let b_contributed = "ex:x1 ex:supervisedBy ex:y1 .";
    let contributed_eval = union_eval(&[A_DOC, b_contributed, C_DOC], CHAIN_QUERY);
    assert_eq!(
        canonical_multiset(&truncated.vars, &truncated.rows),
        canonical_multiset(&contributed_eval.vars, &contributed_eval.rows),
        "truncated join == union-store eval over the CONTRIBUTED graphs (sound, not forged)"
    );

    // (not-yet-detectable) a holder that genuinely owns only the x1 fact presents
    // the SAME partial → the SAME result. The protocol cannot distinguish a
    // truncation from honest scarcity without the signed true-count (M4).
    let pb_genuinely_short = partial_of("b", b_contributed, B_FRAG);
    assert_eq!(
        canonical_multiset(&pb_truncated.vars, &pb_truncated.rows),
        canonical_multiset(&pb_genuinely_short.vars, &pb_genuinely_short.rows),
        "a truncated partial is indistinguishable from a genuinely-short one (pre-M4)"
    );
}

// =====================================================================
// NEGATIVE TEST 3 — an OMITTED ROW from a contributor.
// =====================================================================

/// An OMITTED row (a contributor drops ONE solution it should have disclosed)
/// removes exactly the answers that depended on it — and CANNOT forge a different
/// answer. Driven through the full disclosed-key join.
///
/// (no-forgery)   removing holder A's `p2 knows x2` row deletes the p2 chain from
///                the result; the surviving rows are a subset of honest, and no new
///                row appears.
/// (not-yet-detectable) the omitted-row partial is identical to a holder that never
///                held that row; the join accepts it. Only the differential
///                surfaces the missing answer.
#[test]
fn omitted_row_drops_dependent_answers_without_forging() {
    let pa = partial_of("a", A_DOC, A_FRAG);
    let pb = partial_of("b", B_DOC, B_FRAG);
    let pc = partial_of("c", C_DOC, C_FRAG);

    let honest = fold_chain(&pa, &pb, &pc);
    assert_eq!(honest.rows.len(), 2);

    // Holder A OMITS its `p2 knows x2` row (discloses only p1).
    assert_eq!(pa.rows.len(), 2, "A honestly knows two pairs");
    let p1_term = Term::from(oxrdf::NamedNode::new_unchecked("http://ex/p1"));
    let p_col = pa
        .vars
        .iter()
        .position(|v| v == &var("p"))
        .expect("A projects ?p");
    let pa_omitted = PartialResult {
        holder: pa.holder.clone(),
        vars: pa.vars.clone(),
        rows: pa
            .rows
            .iter()
            .filter(|r| r[p_col].as_ref() == Some(&p1_term))
            .cloned()
            .collect(),
    };
    assert_eq!(pa_omitted.rows.len(), 1, "A omitted the p2 row");

    let omitted = fold_chain(&pa_omitted, &pb, &pc);

    // (no-forgery) exactly the p1 chain survives; the p2→x2→y1→Boss answer is gone,
    // and nothing was invented in its place.
    assert_eq!(
        omitted.rows.len(),
        1,
        "only the p1 chain survives the omission"
    );
    assert!(
        rows_subset_of(&omitted.vars, &omitted.rows, &honest.vars, &honest.rows),
        "no-forgery: the omitted-row result is a subset of the honest result"
    );
    // The surviving row binds ?p to p1 (the omitted p2 answer is absent, not faked).
    let omitted_p: std::collections::BTreeSet<String> = omitted
        .rows
        .iter()
        .filter_map(|r| {
            omitted
                .vars
                .iter()
                .position(|v| v == &var("p"))
                .and_then(|i| r[i].as_ref())
                .map(|t| format!("{t:?}"))
        })
        .collect();
    assert!(
        omitted_p.iter().all(|p| p.contains("p1")) && !omitted_p.iter().any(|p| p.contains("p2")),
        "omission removes the p2 answer; it is NOT replaced by a forged binding"
    );

    // (not-yet-detectable) a holder that genuinely knows only p1 presents the SAME
    // partial → the same result; the omission is invisible to the protocol pre-M4.
    let pa_genuinely_one = partial_of("a", "ex:p1 ex:knows ex:x1 .", A_FRAG);
    assert_eq!(
        canonical_multiset(&pa_omitted.vars, &pa_omitted.rows),
        canonical_multiset(&pa_genuinely_one.vars, &pa_genuinely_one.rows),
        "an omitted row is indistinguishable from a genuinely-absent one (pre-M4)"
    );
}

// =====================================================================
// NEGATIVE TEST 4 — an omitted CONTRIBUTOR (one fewer holder in the fold).
// =====================================================================

/// Omitting a whole CONTRIBUTOR from the fold (joining only A and B, never C)
/// cannot forge: it yields a join over the contributed subset, whose ?p/?x/?y
/// bindings are all genuinely supported by A and B. The point distinct from test
/// 1: here the dropped contributor is removed from the fold ENTIRELY (not handed
/// an empty partial), modelling a planner/orchestrator that silently leaves a
/// source out. The result must still be sound (a real A⋈B), never a forgery, and
/// — crucially — the protocol does not flag the missing source.
#[test]
fn omitted_contributor_yields_sound_subset_join_not_a_forgery() {
    let pa = partial_of("a", A_DOC, A_FRAG);
    let pb = partial_of("b", B_DOC, B_FRAG);

    // Drop C entirely: fold only A⋈B on ?x.
    let ab = DisclosedKeyJoin
        .join(&[pa.clone(), pb.clone()], &chain_plan_on("x"))
        .expect("A⋈B ok");

    // (soundness) A⋈B equals the 2-pattern BGP over the union of ONLY A and B —
    // a faithful PAG eval of the contributed subset, never a forged ?n binding
    // (the dropped C's ?name column simply does not appear).
    let ab_query =
        "PREFIX ex: <http://ex/> SELECT ?p ?x ?y WHERE { ?p ex:knows ?x . ?x ex:supervisedBy ?y }";
    let ab_expected = union_eval(&[A_DOC, B_DOC], ab_query);
    assert_eq!(
        canonical_multiset(&ab.vars, &ab.rows),
        canonical_multiset(&ab_expected.vars, &ab_expected.rows),
        "omitting contributor C yields a sound A⋈B over the contributed subset, not a forgery"
    );
    // No ?n column was fabricated for the missing contributor.
    assert!(
        !ab.vars.contains(&var("n")),
        "the dropped contributor's column must NOT appear (no forged binding)"
    );

    // (not-yet-detectable) the join of {A, B} succeeds and reports nothing about a
    // missing third source — there is no signed manifest of WHICH contributors
    // must be present, so a silently-omitted contributor is not flagged (M4 gap).
    assert_eq!(
        ab.rows.len(),
        2,
        "both A⋈B chains present; no missing-source error"
    );
}

// =====================================================================
// HONESTY PIN — the signed-count defence does NOT exist yet.
// =====================================================================

/// The §4.3-step-1 issuer-signed true-triple-count + salt is the intended
/// defence that would make a short-count DETECTABLE. It is gated on M4 (it is part
/// of the collaborative proof / attestation, [`crate::proof`]). This pin asserts
/// that NOTHING in the disclosed-key join consumes or checks a per-holder signed
/// count today: a [`PartialResult`] carries only `(holder, vars, rows)` — there is
/// no count field, no signature, no manifest — so the join HAS no count to verify
/// against. When the signed-count binding lands, this pin (and the
/// "not-yet-detectable" halves above) are the red tests its acceptance must flip.
#[test]
fn no_signed_count_binding_exists_to_detect_omission_pre_m4() {
    // A PartialResult is purely (holder, vars, rows): no signed count to check
    // omission against. We construct one with a deliberately-short row set and the
    // join accepts it without complaint — there is no binding it could violate.
    let short = PartialResult {
        holder: HolderId::new("any"),
        vars: vec![var("x"), var("y")],
        rows: vec![], // a holder claiming zero rows — nothing binds this to a true count
    };
    let other = PartialResult {
        holder: HolderId::new("other"),
        vars: vec![var("y"), var("n")],
        rows: vec![vec![
            Some(Term::from(oxrdf::NamedNode::new_unchecked("http://ex/y1"))),
            Some(Term::Literal(oxrdf::Literal::new_simple_literal("Boss"))),
        ]],
    };
    // The join accepts the (possibly short-counted) empty partial and returns empty
    // — fail-OPEN to completeness loss, by design, because there is no signed count
    // to fail-closed against. This is the documented pre-M4 gap, pinned honestly.
    let joined = DisclosedKeyJoin
        .join(&[short, other], &chain_plan_on("y"))
        .expect("join accepts a short-counted partial — no count binding to reject it");
    assert!(
        joined.is_empty(),
        "with no signed-count binding the join cannot detect the short count; it simply \
         propagates the omission as completeness loss (detection is gated on §4.3 step 1 / M4)"
    );
}
